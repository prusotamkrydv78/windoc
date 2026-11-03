package com.notchy.app

import android.animation.ValueAnimator
import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.view.MotionEvent
import android.view.View
import android.view.animation.DecelerateInterpolator
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.max
import kotlin.math.sin

/**
 * The robot around the front camera: a black pill wrapping the punch hole, with two glowing
 * eyes either side of it.
 *
 * Gestures: tap (pet, peek at time and battery), double-tap (silence a ringing call, or the
 * privacy screen), long-press or swipe down (quick panel), swipe left / right (previous /
 * next song), hold while charging (feed it).
 *
 * States, most visible first: a live activity (timer, ride, delivery) keeps the pill a bit
 * wider with a progress ring; a charging ring fills to the battery level; a ringing call
 * pulses; the eyes take the colour of the app that just notified, bounce to music, spin when
 * the phone is shaken, and wear sunglasses or a scarf with the weather. A red or orange dot
 * shows while any app uses the camera or microphone.
 *
 * @param hole diameter of the camera cut-out in pixels; the pill is sized from it.
 */
@SuppressLint("ViewConstructor")
class RobotView(context: Context, private val hole: Float) : View(context) {
    private val density = resources.displayMetrics.density
    private val pill = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.BLACK }
    private val eye = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = EYE }
    private val glow = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = EYE; alpha = 60 }
    private val label = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.WHITE
        textSize = 12f * density
        isFakeBoldText = true
    }
    private val small = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.WHITE
        textSize = 9.5f * density
        isFakeBoldText = true
    }
    private val bolt = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.rgb(255, 214, 61) }
    private val ring = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.STROKE
        strokeCap = Paint.Cap.ROUND
    }
    private val fill = Paint(Paint.ANTI_ALIAS_FLAG)

    /** 0 = resting pill, 1 = widened with time and battery. */
    private var open = 0f
    private var blink = 0f
    private var hop = 0f
    private var look = 0f
    private var battery = -1
    private var charging = false
    /** Shown instead of the time and battery while set. */
    private var message: String? = null
    /** 0..1 while the robot is thrown up to the PC: the eyes fly off and come back. */
    private var thrown = 0f

    // ---- Public state ---------------------------------------------------------------------

    /** A Focus session runs on the PC: narrow, determined eyes. */
    var focused = false
        set(value) { field = value; invalidate() }

    /** Accessories unlocked on the PC (0 to 3) and the Focus streak, drawn as tiny badges. */
    var badges = 0
        set(value) { field = value; invalidate() }
    var streak = 0
        set(value) { field = value; invalidate() }

    /** A running timer, ride or delivery: shown in the pill all the time. */
    data class LiveActivity(val text: String, val progress: Float?, val until: Long = 0L)
    var activity: LiveActivity? = null
        set(value) { field = value; invalidate() }

    /** 0..1 loudness of the music playing, for bouncing eyes. */
    var beat = 0f
        set(value) { field = value; invalidate() }

    /** The phone is ringing: the pill pulses. */
    var ringing = false
        set(value) {
            field = value
            if (value) pulse() else invalidate()
        }

    /** Camera (red) or microphone (orange) in use by some app, or 0. */
    var privacyDot = 0
        set(value) { field = value; invalidate() }

    /** "sun" for sunglasses, "cold" for a scarf, or null. */
    var wear: String? = null
        set(value) { field = value; invalidate() }

    /** A tiny download speed readout next to the camera, or null. */
    var speed: String? = null
        set(value) { field = value; invalidate() }

    /** Face down in a pocket: the robot sleeps and the pill shrinks to nothing. */
    var pocketed = false
        set(value) {
            field = value
            animate().alpha(if (value) 0f else 1f).setDuration(300).start()
        }

    // ---- Callbacks ------------------------------------------------------------------------

    var onPet: (() -> Unit)? = null
    /** Long-press or swipe down: opens the quick panel. */
    var onThrow: (() -> Unit)? = null
    var onSwipe: ((direction: Int) -> Unit)? = null
    var onDoubleTap: (() -> Unit)? = null
    /** Held for two seconds while charging. */
    var onFeed: (() -> Unit)? = null
    /** A tap while a screenshot is waiting to be sent. */
    var onScreenshotTap: (() -> Boolean)? = null

    private val restW get() = hole * 3.4f
    private val activityW get() = hole * 3.4f + 86f * density
    private val openW get() = hole * 3.4f + 150f * density
    val pillH get() = hole * 1.35f
    val maxWidth get() = openW

    private var openAnimator: ValueAnimator? = null

    init {
        scheduleBlink()
        scheduleLook()
    }

    fun setBattery(percent: Int, isCharging: Boolean) {
        val plugged = isCharging && !charging
        battery = percent
        charging = isCharging
        if (plugged) chargeRing()
        invalidate()
    }

    /** Widens and shows [text] for a few seconds. */
    fun say(text: String, millis: Long = 3500) {
        message = text
        doHop()
        peek(millis)
    }

    /** Widens for a few seconds, then settles back. */
    fun peek(millis: Long = 3500) {
        animateOpen(1f)
        removeCallbacks(close)
        postDelayed(close, millis)
    }

    private val close = Runnable {
        animateOpen(0f)
        postDelayed({ if (open == 0f) message = null }, 450)
    }

    /** The eyes shoot up out of the pill and drop back in. */
    fun throwNow() = animate01(1400) { thrown = it }

    // ---- Effects --------------------------------------------------------------------------

    private var tint: Int? = null
    private val untint = Runnable { tint = null; invalidate() }

    /** The eyes glow [color] for a few seconds (the app that just notified). */
    fun flash(color: Int) {
        tint = color
        removeCallbacks(untint)
        postDelayed(untint, 3000)
        doHop()
    }

    private var charge = 0f
    private fun chargeRing() = animate01(2600) { charge = it }

    private var dizzy = 0f
    /** The phone was shaken: spinning eyes. */
    fun dizzy() = animate01(1600) { dizzy = it }

    private var pulsePhase = 0f
    private fun pulse() {
        ValueAnimator.ofFloat(0f, 1f).apply {
            duration = 900
            repeatCount = 12
            addUpdateListener {
                pulsePhase = it.animatedValue as Float
                if (!ringing) cancel()
                invalidate()
            }
            start()
        }
    }

    private var yawn = 0f
    /** A morning yawn: the pill stretches and the eyes close. */
    fun yawn() = animate01(1800) { yawn = it }

    private fun animate01(millis: Long, update: (Float) -> Unit) {
        ValueAnimator.ofFloat(0f, 1f).apply {
            duration = millis
            addUpdateListener { update(it.animatedValue as Float); invalidate() }
            addListener(object : android.animation.AnimatorListenerAdapter() {
                override fun onAnimationEnd(animation: android.animation.Animator) { update(0f); invalidate() }
            })
            start()
        }
    }

    // ---- The notch game: tap the dot as it appears -----------------------------------------

    private var gameRound = -1
    private var dotSide = 0
    private var dotAt = 0L
    private val gameTimes = mutableListOf<Long>()

    fun startGame() {
        gameRound = 0
        gameTimes.clear()
        say("Tap the dot!", 1500)
        postDelayed({ nextDot() }, 1700)
    }

    private fun nextDot() {
        if (gameRound < 0) return
        dotSide = if (Math.random() < 0.5) -1 else 1
        dotAt = System.currentTimeMillis()
        invalidate()
        // Missed: next one.
        postDelayed(missed, 1500)
    }

    private val missed = Runnable { gameTap(null) }

    private fun gameTap(side: Int?) {
        removeCallbacks(missed)
        if (side != null && side == dotSide) gameTimes += System.currentTimeMillis() - dotAt
        dotSide = 0
        gameRound++
        invalidate()
        if (gameRound >= 10) {
            gameRound = -1
            val hits = gameTimes.size
            say(if (hits == 0) "Missed them all!" else "$hits/10 · ${gameTimes.average().toInt()} ms")
            return
        }
        postDelayed({ nextDot() }, (500 + Math.random() * 900).toLong())
    }

    // ---- Touch ----------------------------------------------------------------------------

    private var downX = 0f
    private var downY = 0f
    private var pressedAt = 0L
    private var lastTap = 0L
    private val longPress = Runnable {
        pressedAt = 0L
        if (!charging) onThrow?.invoke()
    }
    private val feed = Runnable {
        pressedAt = 0L
        onFeed?.invoke()
    }
    private val singleTap = Runnable {
        if (onScreenshotTap?.invoke() == true) return@Runnable
        doHop()
        peek()
        onPet?.invoke()
    }

    @SuppressLint("ClickableViewAccessibility")
    override fun onTouchEvent(event: MotionEvent): Boolean {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                downX = event.x
                downY = event.y
                pressedAt = event.eventTime
                if (gameRound >= 0) {
                    gameTap(if (event.x < width / 2f) -1 else 1)
                    pressedAt = 0L
                    return true
                }
                // Charging: a long hold feeds it; otherwise a long press opens the panel.
                postDelayed(if (charging) feed else longPress, if (charging) 2000 else 550)
            }
            MotionEvent.ACTION_MOVE -> {
                val dx = event.x - downX
                val dy = event.y - downY
                if (pressedAt != 0L && (abs(dx) > 28 * density || dy > 24 * density)) {
                    removeCallbacks(longPress)
                    removeCallbacks(feed)
                    pressedAt = 0L
                    if (abs(dx) > abs(dy)) {
                        onSwipe?.invoke(if (dx > 0) 1 else -1)
                        look = if (dx > 0) 1f else -1f
                        doHop()
                    } else {
                        onThrow?.invoke()
                    }
                }
            }
            MotionEvent.ACTION_UP -> {
                removeCallbacks(longPress)
                removeCallbacks(feed)
                if (pressedAt != 0L) {
                    if (event.eventTime - lastTap < 300) {
                        removeCallbacks(singleTap)
                        lastTap = 0L
                        onDoubleTap?.invoke()
                    } else {
                        lastTap = event.eventTime
                        postDelayed(singleTap, 300)
                    }
                }
                pressedAt = 0L
            }
            MotionEvent.ACTION_CANCEL -> {
                removeCallbacks(longPress)
                removeCallbacks(feed)
                pressedAt = 0L
            }
        }
        return true
    }

    // ---- Animation helpers ----------------------------------------------------------------

    private fun animateOpen(target: Float) {
        openAnimator?.cancel()
        openAnimator = ValueAnimator.ofFloat(open, target).apply {
            duration = 420
            interpolator = DecelerateInterpolator(2f)
            addUpdateListener { open = it.animatedValue as Float; invalidate() }
            start()
        }
    }

    private fun scheduleBlink() {
        postDelayed({
            ValueAnimator.ofFloat(0f, 1f, 0f).apply {
                duration = 180
                addUpdateListener { blink = it.animatedValue as Float; invalidate() }
                start()
            }
            scheduleBlink()
        }, (2500 + Math.random() * 3500).toLong())
    }

    private fun scheduleLook() {
        postDelayed({
            val target = listOf(-1f, 0f, 0f, 1f).random()
            ValueAnimator.ofFloat(look, target).apply {
                duration = 350
                addUpdateListener { look = it.animatedValue as Float; invalidate() }
                start()
            }
            scheduleLook()
        }, (3000 + Math.random() * 5000).toLong())
    }

    private fun doHop() = animate01(520) { hop = it }

    private val sleepy: Boolean
        get() {
            val hour = java.util.Calendar.getInstance().get(java.util.Calendar.HOUR_OF_DAY)
            return hour >= 23 || hour < 6
        }

    // ---- Drawing --------------------------------------------------------------------------

    override fun onDraw(canvas: Canvas) {
        val live = activity
        val base = if (live != null) activityW else restW
        val stretch = sin(yawn * Math.PI).toFloat() * 12f * density
        val w = base + (openW - base) * open + stretch
        val h = pillH
        val cx = width / 2f
        val top = (height - h) / 2f
        val rect = RectF(cx - w / 2f, top, cx + w / 2f, top + h)

        // Ringing: a soft pulse around the pill.
        if (ringing) {
            fill.color = Color.rgb(88, 214, 141)
            fill.alpha = ((1f - pulsePhase) * 120).toInt()
            val grow = pulsePhase * 6f * density
            val halo = RectF(rect).apply { inset(-grow, -grow) }
            canvas.drawRoundRect(halo, halo.height() / 2f, halo.height() / 2f, fill)
        }
        canvas.drawRoundRect(rect, h / 2f, h / 2f, pill)

        // Charging: a ring traced around the pill up to the battery level.
        if (charge > 0f && battery >= 0) {
            ring.color = Color.rgb(124, 242, 192)
            ring.strokeWidth = 2f * density
            ring.alpha = (sin(charge * Math.PI).toFloat().coerceIn(0f, 1f) * 255).toInt()
            val outline = RectF(rect).apply { inset(1.5f * density, 1.5f * density) }
            val path = Path().apply { addRoundRect(outline, outline.height() / 2f, outline.height() / 2f, Path.Direction.CW) }
            val measure = android.graphics.PathMeasure(path, true)
            val traced = Path()
            measure.getSegment(0f, measure.length * (battery / 100f) * (charge * 1.6f).coerceAtMost(1f), traced, true)
            canvas.drawPath(traced, ring)
        }

        // Eyes sit either side of the camera hole and squash when blinking.
        val fly = if (thrown in 0.001f..0.999f) sin(thrown * Math.PI).toFloat() * h * 2.2f else 0f
        val bounce = sin(hop * Math.PI).toFloat() * h * 0.18f + fly
        val eyeW = hole * 0.42f
        val mood = when {
            focused -> 0.45f
            sleepy && open == 0f && live == null -> 0.35f
            else -> 1f
        }
        val yawnShut = 1f - sin(yawn * Math.PI).toFloat() * 0.9f
        val music = 1f + beat * 0.5f
        val eyeHeight = max(eyeH(h) * mood * music * yawnShut * (1f - blink * 0.85f), 1.5f * density)
        val gap = hole * 0.95f
        val shift = look * hole * 0.12f
        val color = tint ?: EYE
        eye.color = color
        glow.color = color
        glow.alpha = 60
        // Live activity: the eyes move left of the camera to make room for it.
        val eyeOffset = if (live != null && open < 0.4f) -(activityW - restW) / 2f else 0f
        for (side in listOf(-1f, 1f)) {
            val ex = cx + side * gap + shift + eyeOffset
            val ey = top + h / 2f - bounce
            if (dizzy > 0f) {
                // Spinning eyes: a little rotating bar each.
                val angle = dizzy * 6f * Math.PI.toFloat() * side
                val r = eyeW * 0.7f
                eye.strokeWidth = 2f * density
                canvas.drawLine(ex - cos(angle) * r, ey - sin(angle) * r, ex + cos(angle) * r, ey + sin(angle) * r, eye)
                continue
            }
            val r = RectF(ex - eyeW / 2f, ey - eyeHeight / 2f, ex + eyeW / 2f, ey + eyeHeight / 2f)
            val g = RectF(r).apply { inset(-3f * density, -3f * density) }
            canvas.drawRoundRect(g, g.height() / 2f, g.height() / 2f, glow)
            canvas.drawRoundRect(r, r.height() / 2f, r.height() / 2f, eye)
            if (wear == "sun" && open < 0.4f) {
                // Sunglasses: a dark lens over each eye, joined over the camera.
                fill.color = Color.rgb(30, 30, 36)
                val lens = RectF(r).apply { inset(-2.5f * density, -1f * density) }
                canvas.drawRoundRect(lens, 3f * density, 3f * density, fill)
            }
        }
        if (wear == "cold" && open < 0.4f) {
            // A scarf: a warm band along the bottom of the pill.
            fill.color = Color.rgb(232, 92, 72)
            canvas.drawRoundRect(
                RectF(rect.left + h * 0.4f, rect.bottom - 3f * density, rect.right - h * 0.4f, rect.bottom),
                1.5f * density, 1.5f * density, fill,
            )
        }

        // Camera or microphone in use.
        if (privacyDot != 0) {
            fill.color = privacyDot
            canvas.drawCircle(rect.right - h * 0.38f, top + h * 0.32f, 2.6f * density, fill)
        }

        // The notch game's dot.
        if (gameRound >= 0 && dotSide != 0) {
            fill.color = Color.rgb(255, 214, 61)
            canvas.drawCircle(cx + dotSide * (w / 2f - h * 0.5f), top + h / 2f, h * 0.22f, fill)
        }

        // Earned on the PC: one dot per accessory, and a warm dot for a 3+ day Focus streak.
        if (open < 0.2f && live == null && (badges > 0 || streak >= 3)) {
            val colors = listOf(Color.rgb(255, 122, 89), Color.rgb(120, 170, 255), Color.rgb(255, 214, 61))
            var x = rect.right - h * 0.42f
            val y = top + h * 0.5f
            for (index in 0 until badges.coerceAtMost(3)) {
                fill.color = colors[index]
                canvas.drawCircle(x, y, 1.8f * density, fill)
                x -= 5f * density
            }
            if (streak >= 3) {
                fill.color = Color.rgb(255, 150, 60)
                canvas.drawCircle(rect.left + h * 0.42f, y, 2.2f * density, fill)
            }
        }

        // Live activity: a progress ring and a short label right of the camera.
        if (live != null && open < 0.4f) {
            val ringCx = rect.right - h * 0.55f
            val ringR = h * 0.28f
            ring.strokeWidth = 2f * density
            ring.color = Color.argb(70, 255, 255, 255)
            canvas.drawCircle(ringCx, top + h / 2f, ringR, ring)
            val fraction = live.progress ?: if (live.until > 0) {
                1f - ((live.until - System.currentTimeMillis()).coerceAtLeast(0) / 3_600_000f).coerceIn(0f, 1f)
            } else null
            if (fraction != null) {
                ring.color = EYE
                val arc = RectF(ringCx - ringR, top + h / 2f - ringR, ringCx + ringR, top + h / 2f + ringR)
                canvas.drawArc(arc, -90f, 360f * fraction.coerceIn(0f, 1f), false, ring)
            }
            val text = if (live.until > 0) {
                val left = ((live.until - System.currentTimeMillis()) / 1000).coerceAtLeast(0)
                "%d:%02d".format(left / 60, left % 60)
            } else live.text
            val tw = small.measureText(text)
            canvas.drawText(text, ringCx - ringR - 5f * density - tw, top + h / 2f + small.textSize * 0.36f, small)
            postInvalidateDelayed(1000)
        } else if (speed != null && open < 0.2f) {
            val tw = small.measureText(speed)
            small.alpha = 200
            canvas.drawText(speed!!, rect.right - h * 0.5f - tw, top + h / 2f + small.textSize * 0.36f, small)
            small.alpha = 255
        }

        val said = message
        if (open > 0.4f && said != null) {
            label.alpha = ((open - 0.4f) / 0.6f * 255).toInt().coerceIn(0, 255)
            val baseline = top + h / 2f + label.textSize * 0.36f
            // Left of the camera hole when it fits, else the whole pill.
            val tw = label.measureText(said)
            val x = (rect.left + h * 0.55f).coerceAtMost(cx - hole * 1.6f - tw).coerceAtLeast(rect.left + h * 0.4f)
            canvas.drawText(said, x, baseline, label)
        } else if (open > 0.4f) {
            val alpha = ((open - 0.4f) / 0.6f * 255).toInt().coerceIn(0, 255)
            label.alpha = alpha
            val baseline = top + h / 2f + label.textSize * 0.36f
            val time = SimpleDateFormat("h:mm", Locale.getDefault()).format(Date())
            canvas.drawText(time, rect.left + h * 0.55f, baseline, label)
            if (battery >= 0) {
                val text = "$battery%"
                val tw = label.measureText(text)
                val right = rect.right - h * 0.55f
                canvas.drawText(text, right - tw, baseline, label)
                if (charging) {
                    bolt.alpha = alpha
                    val bx = right - tw - 10f * density
                    val by = top + h / 2f
                    val s = 5f * density
                    val path = Path().apply {
                        moveTo(bx + s * 0.2f, by - s)
                        lineTo(bx - s * 0.6f, by + s * 0.1f)
                        lineTo(bx, by + s * 0.1f)
                        lineTo(bx - s * 0.2f, by + s)
                        lineTo(bx + s * 0.6f, by - s * 0.1f)
                        lineTo(bx, by - s * 0.1f)
                        close()
                    }
                    canvas.drawPath(path, bolt)
                }
            }
        }

        // Keep the clock fresh while open.
        if (open > 0f) postInvalidateDelayed(1000)
    }

    private fun eyeH(h: Float) = h * 0.30f

    companion object {
        val EYE: Int = Color.rgb(124, 242, 192)
    }
}
