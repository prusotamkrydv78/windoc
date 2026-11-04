package com.notchy.app

import android.animation.ValueAnimator
import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapShader
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.graphics.Shader
import android.graphics.Typeface
import android.graphics.drawable.Drawable
import android.os.SystemClock
import android.text.Layout
import android.text.StaticLayout
import android.text.TextPaint
import android.text.TextUtils
import android.view.MotionEvent
import android.view.View
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min
import kotlin.math.sin
import kotlin.math.sqrt

/**
 * The phone's Dynamic Island: one black shape around the front camera that springs between
 *
 * - **rest**: the robot's two eyes either side of the camera;
 * - **compact**: one running activity (call, timer, live progress, music) as a leading view
 *   left of the camera and a trailing view right of it;
 * - **cards**: an incoming call, a notification with its actions, the music player, a timer,
 *   low battery, and short banners (ring / silent, charging, the robot talking).
 *
 * Layout follows docs/design/NOTCH_DESIGN_REFERENCE.md: opaque black, content hugging the
 * camera, concentric margins, one key colour per activity, medium-weight type, springs.
 *
 * Gestures: on the robot, tap (pet, then time and battery), double-tap (silence a ringing
 * call, or the privacy screen), long-press or swipe down (quick panel), swipe left / right
 * (previous / next song), hold while charging (feed it). On an activity, tap to open its card;
 * on a card, tap a button, tap outside or swipe up to close.
 *
 * The overlay window is resized with the shape ([onResize]) so it never covers more of the
 * screen than the island itself.
 *
 * @param hole diameter of the camera cut-out in pixels; the island is sized from it.
 */
@SuppressLint("ViewConstructor")
class RobotView(context: Context, private val hole: Float) : View(context) {
    private val d = resources.displayMetrics.density
    private val sp = d * resources.configuration.fontScale.coerceIn(0.85f, 1.15f)

    // ---- Geometry -------------------------------------------------------------------------

    val pillH = max(hole + 8f * d, 32f * d)
    private val restW = pillH * 3.4f
    private val compactW = max(restW + 48f * d, hole + 2f * 62f * d)
    private val cardW = min(resources.displayMetrics.widthPixels - 22f * d, 420f * d)
    private val margin = 16f * d
    /** Window space above the island and around it (for the key line). */
    private val pad = 4f * d
    private val edge = 8f * d
    val maxWidth get() = cardW + 2f * edge

    // ---- Paints ---------------------------------------------------------------------------

    private val shapePaint = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.BLACK }
    private val keyLine = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.STROKE; strokeWidth = 1f * d }
    private val fill = Paint(Paint.ANTI_ALIAS_FLAG)
    private val stroke = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.STROKE
        strokeCap = Paint.Cap.ROUND
        strokeJoin = Paint.Join.ROUND
    }

    private fun font(size: Float, weight: Int, color: Int = Color.WHITE) = TextPaint(Paint.ANTI_ALIAS_FLAG).apply {
        textSize = size * sp
        typeface = Typeface.create(Typeface.DEFAULT, weight, false)
        this.color = color
        fontFeatureSettings = "tnum"
    }

    private val titleFont = font(15f, 600)
    private val bodyFont = font(13f, 400, DIM)
    private val captionFont = font(11f, 600, FAINT)
    private val valueFont = font(14f, 600)
    private val labelFont = font(13f, 500)
    private val bigFont = font(26f, 600)
    private val chipFont = font(12f, 600)

    // ---- Public state ---------------------------------------------------------------------

    /** A Focus session runs on the PC: the robot squints. */
    var focused = false
        set(value) { field = value; invalidate() }

    /** Kept for the PC link; not drawn (the island stays clean). */
    var badges = 0
    var streak = 0
    var wear: String? = null

    /** A running timer, ride, delivery or download. */
    data class LiveActivity(val text: String, val progress: Float?, val until: Long = 0L, val detail: String = "")
    var activity: LiveActivity? = null
        set(value) { field = value; refresh() }

    data class Call(val name: String, val avatar: Bitmap?, val ringing: Boolean, val since: Long)
    var call: Call? = null
        set(value) {
            if (value == null || field?.ringing != value.ringing) expandedByUser = false
            field = value
            refresh()
        }
    var callMuted = false
        set(value) { field = value; invalidate() }

    data class Music(
        val title: String,
        val artist: String,
        val art: Bitmap?,
        val playing: Boolean,
        val position: Long,
        val duration: Long,
        /** `SystemClock.elapsedRealtime()` of [position]. */
        val updatedAt: Long,
        val speed: Float,
    )
    var music: Music? = null
        set(value) {
            if (value?.art !== field?.art) accent = value?.art?.let(::accentOf) ?: Color.WHITE
            field = value
            refresh()
        }
    private var accent = Color.WHITE

    data class Notice(
        val key: String,
        val app: String,
        val icon: Drawable?,
        val avatar: Bitmap?,
        val title: String,
        val text: String,
        val actions: List<String>,
    )

    /** 0..1 loudness of the music playing. */
    var beat = 0f
        set(value) { field = value; if (current === rest) invalidate() }

    /** The phone is ringing (double-tap silences it). */
    var ringing = false

    /** Camera or microphone in use by some app (its colour), or 0. */
    var privacyDot = 0
        set(value) { field = value; invalidate() }

    /** A tiny download speed readout right of the camera, or null. */
    var speed: String? = null
        set(value) { field = value; refresh() }

    /** Face down in a pocket: the island fades out. */
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
    /** "answer", "decline", "end" or "mute". */
    var onCallAction: ((String) -> Unit)? = null
    /** "prev", "toggle" or "next". */
    var onMedia: ((String) -> Unit)? = null
    var onNoticeOpen: ((key: String) -> Unit)? = null
    var onNoticeAction: ((key: String, index: Int) -> Unit)? = null
    var onTimerCancel: (() -> Unit)? = null
    var onBatterySaver: (() -> Unit)? = null
    /** The overlay window should become this many pixels wide and tall. */
    var onResize: ((width: Int, height: Int) -> Unit)? = null

    // ---- Faces ----------------------------------------------------------------------------

    /** One layout of the island: its size and how it draws inside its own rect. */
    private abstract inner class Face {
        abstract val w: Float
        abstract val h: Float
        abstract fun draw(c: Canvas, r: RectF)
        /** The activity's key colour, for the outline; 0 for none. */
        open val key: Int get() = 0
        /** Redraw interval while shown (clocks, waveforms), or 0. */
        open val tick: Long get() = 0
    }

    private var current: Face
    private var previous: Face? = null
    private var switchedAt = 0L

    private var banner: Face? = null
    private var bannerUntil = 0L
    private val expire = Runnable { banner = null; refresh() }
    private var expandedByUser = false
    private val collapse = Runnable { expandedByUser = false; refresh() }

    /** Button areas of the face on screen, newest last; filled while drawing it. */
    private val hits = mutableListOf<Pair<RectF, () -> Unit>>()
    private var collecting = false

    private fun button(r: RectF, action: () -> Unit) {
        if (collecting) hits += RectF(r) to action
    }

    /** The face to show now, most important first. */
    private fun pick(): Face {
        call?.let { if (it.ringing) return callCard }
        if (banner != null && System.currentTimeMillis() < bannerUntil) return banner!!
        val ongoing = ongoing()
        if (expandedByUser && ongoing != null) return ongoing.second
        return ongoing?.first ?: rest
    }

    private fun ongoing(): Pair<Face, Face>? = when {
        call != null -> callCompact to callCard
        (activity?.until ?: 0L) > 0L -> timerCompact to timerCard
        activity != null -> liveCompact to liveCard
        music?.playing == true -> musicCompact to musicCard
        else -> null
    }

    private fun isCompact(face: Face) = face === callCompact || face === timerCompact || face === liveCompact || face === musicCompact

    private fun refresh() {
        val next = pick()
        if (next !== current) {
            previous = current
            current = next
            switchedAt = SystemClock.uptimeMillis()
            fitWindow(max(curW, next.w), max(curH, next.h))
        }
        lastFrame = SystemClock.uptimeMillis()
        postInvalidateOnAnimation()
    }

    private fun showBanner(face: Face, millis: Long) {
        banner = face
        bannerUntil = System.currentTimeMillis() + millis
        removeCallbacks(expire)
        postDelayed(expire, millis)
        refresh()
    }

    /** Closes a card or banner (a ringing call stays). */
    fun dismiss() {
        removeCallbacks(expire)
        banner = null
        expandedByUser = false
        refresh()
    }

    // ---- Shape spring ---------------------------------------------------------------------

    private var curW = restW
    private var curH = pillH
    private var velW = 0f
    private var velH = 0f
    private var lastFrame = 0L
    private val shapePath = Path()
    private val shapeRect = RectF()

    private fun step(x: Float, v: Float, target: Float, dt: Float): Pair<Float, Float> {
        // Opening overshoots a little; closing settles without bouncing.
        val opening = target > x
        val k = if (opening) 380f else 520f
        val c = 2f * (if (opening) 0.72f else 1f) * sqrt(k)
        var position = x
        var velocity = v
        val h = dt / 2f
        repeat(2) {
            velocity += (-k * (position - target) - c * velocity) * h
            position += velocity * h
        }
        if (abs(position - target) < 0.5f && abs(velocity) < 8f) return target to 0f
        return position to velocity
    }

    private var winW = 0
    private var winH = 0

    /** Asks for a window big enough for a [w] × [h] island. */
    private fun fitWindow(w: Float, h: Float) {
        val width = (w + 2f * edge).toInt()
        val height = (h + pad + edge).toInt()
        if (width != winW || height != winH) {
            winW = width
            winH = height
            post { onResize?.invoke(width, height) }
        }
    }

    fun windowWidth() = (restW + 2f * edge).toInt().also { winW = it }
    fun windowHeight() = (pillH + pad + edge).toInt().also { winH = it }

    // ---- Battery, banners, robot moments --------------------------------------------------

    private var battery = -1
    private var charging = false

    fun setBattery(percent: Int, isCharging: Boolean) {
        val plugged = isCharging && !charging && battery >= 0
        battery = percent
        charging = isCharging
        if (plugged) showBanner(chargeFace, 2600) else invalidate()
    }

    fun showLowBattery(percent: Int) {
        battery = percent
        showBanner(lowBatteryFace, 7000)
    }

    private var ringerSilent = false

    /** The ring / silent switch moved. */
    fun showRinger(silent: Boolean) {
        ringerSilent = silent
        showBanner(ringerFace, 2200)
    }

    private var notice: Notice? = null

    fun showNotice(value: Notice) {
        notice = value
        showBanner(noticeFace, 5500)
    }

    private var sayText = ""

    /** The robot says [text] for a while. */
    fun say(text: String, millis: Long = 3500) {
        sayText = text
        doHop()
        showBanner(sayFace, millis)
    }

    /** The time and battery for a few seconds. */
    fun peek(millis: Long = 3000) = showBanner(peekFace, millis)

    // ---- Robot animation state ------------------------------------------------------------

    private var blink = 0f
    private var hop = 0f
    private var look = 0f
    private var happy = 0f
    private var thrown = 0f
    private var dizzy = 0f
    private var yawn = 0f
    private var tint: Int? = null
    private val untint = Runnable { tint = null; invalidate() }

    /** The eyes fly up out of the island and drop back in. */
    fun throwNow() = animate01(1300) { thrown = it }

    /** The eyes glow [color] for a few seconds (the app that just notified). */
    fun flash(color: Int) {
        tint = color
        removeCallbacks(untint)
        postDelayed(untint, 2500)
        doHop()
    }

    fun dizzy() = animate01(1600) { dizzy = it }

    /** A morning yawn: the island stretches and the eyes close. */
    fun yawn() = animate01(1800) { yawn = it; if (current === rest) refreshSize() }

    private fun doHop() = animate01(480) { hop = it }

    private fun refreshSize() = postInvalidateOnAnimation()

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

    private fun scheduleBlink() {
        postDelayed({
            ValueAnimator.ofFloat(0f, 1f, 0f).apply {
                duration = 160
                addUpdateListener { blink = it.animatedValue as Float; if (current === rest) invalidate() }
                start()
            }
            scheduleBlink()
        }, (2800 + Math.random() * 3600).toLong())
    }

    private fun scheduleLook() {
        postDelayed({
            val target = listOf(-1f, 0f, 0f, 1f).random()
            ValueAnimator.ofFloat(look, target).apply {
                duration = 320
                addUpdateListener { look = it.animatedValue as Float; if (current === rest) invalidate() }
                start()
            }
            scheduleLook()
        }, (3200 + Math.random() * 5000).toLong())
    }

    private val sleepy: Boolean
        get() {
            val hour = java.util.Calendar.getInstance().get(java.util.Calendar.HOUR_OF_DAY)
            return hour >= 23 || hour < 6
        }

    // ---- The notch game: tap the dot as it appears ----------------------------------------

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

    // ---- Drawing helpers ------------------------------------------------------------------

    private val icons = HashMap<Int, Drawable>()

    private fun icon(c: Canvas, res: Int, cx: Float, cy: Float, size: Float, color: Int) {
        val drawable = icons.getOrPut(res) { context.getDrawable(res)!!.mutate() }
        drawable.setTint(color)
        val half = size / 2f
        drawable.setBounds((cx - half).toInt(), (cy - half).toInt(), (cx + half).toInt(), (cy + half).toInt())
        drawable.draw(c)
    }

    private fun baseline(cy: Float, paint: Paint) = cy - (paint.descent() + paint.ascent()) / 2f

    private fun fit(text: String, paint: TextPaint, width: Float) =
        TextUtils.ellipsize(text, paint, max(width, 0f), TextUtils.TruncateAt.END).toString()

    private fun text(c: Canvas, value: String, paint: TextPaint, x: Float, cy: Float, maxWidth: Float = Float.MAX_VALUE) {
        c.drawText(fit(value, paint, maxWidth), x, baseline(cy, paint), paint)
    }

    private fun textRight(c: Canvas, value: String, paint: TextPaint, right: Float, cy: Float) {
        c.drawText(value, right - paint.measureText(value), baseline(cy, paint), paint)
    }

    private fun circleButton(c: Canvas, cx: Float, cy: Float, size: Float, background: Int, res: Int, iconColor: Int, action: () -> Unit) {
        fill.color = background
        c.drawCircle(cx, cy, size / 2f, fill)
        icon(c, res, cx, cy, size * 0.5f, iconColor)
        button(RectF(cx - size / 2f - 4f * d, cy - size / 2f - 4f * d, cx + size / 2f + 4f * d, cy + size / 2f + 4f * d), action)
    }

    /** A picture clipped to a circle (radius = size / 2) or a rounded square. */
    private fun picture(c: Canvas, bitmap: Bitmap, r: RectF, radius: Float) {
        val shader = BitmapShader(bitmap, Shader.TileMode.CLAMP, Shader.TileMode.CLAMP)
        val scale = max(r.width() / bitmap.width, r.height() / bitmap.height)
        shader.setLocalMatrix(Matrix().apply {
            setScale(scale, scale)
            postTranslate(r.centerX() - bitmap.width * scale / 2f, r.centerY() - bitmap.height * scale / 2f)
        })
        fill.shader = shader
        fill.color = Color.WHITE
        c.drawRoundRect(r, radius, radius, fill)
        fill.shader = null
    }

    private fun avatar(c: Canvas, bitmap: Bitmap?, cx: Float, cy: Float, size: Float, fallback: Int, color: Int) {
        val r = RectF(cx - size / 2f, cy - size / 2f, cx + size / 2f, cy + size / 2f)
        if (bitmap != null) {
            picture(c, bitmap, r, size / 2f)
        } else {
            fill.color = withAlpha(color, 0.22f)
            c.drawCircle(cx, cy, size / 2f, fill)
            icon(c, fallback, cx, cy, size * 0.5f, color)
        }
    }

    /** A battery glyph whose right edge is at [right]. */
    private fun batteryGlyph(c: Canvas, right: Float, cy: Float, level: Int, color: Int) {
        val w = 24f * d
        val h = 11.5f * d
        val body = RectF(right - w, cy - h / 2f, right - 2.5f * d, cy + h / 2f)
        stroke.strokeWidth = 1f * d
        stroke.color = withAlpha(Color.WHITE, 0.4f)
        c.drawRoundRect(body, 3.2f * d, 3.2f * d, stroke)
        fill.color = withAlpha(Color.WHITE, 0.4f)
        c.drawRoundRect(RectF(right - 1.8f * d, cy - 2f * d, right, cy + 2f * d), 1f * d, 1f * d, fill)
        val inner = RectF(body).apply { inset(2f * d, 2f * d) }
        inner.right = inner.left + inner.width() * (level.coerceIn(0, 100) / 100f)
        fill.color = color
        c.drawRoundRect(inner, 1.6f * d, 1.6f * d, fill)
    }

    private fun clock(millis: Long): String {
        val seconds = (millis / 1000).coerceAtLeast(0)
        return if (seconds >= 3600) "%d:%02d:%02d".format(seconds / 3600, seconds / 60 % 60, seconds % 60)
        else "%d:%02d".format(seconds / 60, seconds % 60)
    }

    /** Left and right edges of the camera, with breathing room. */
    private val holeLeft get() = width / 2f - hole / 2f - 8f * d
    private val holeRight get() = width / 2f + hole / 2f + 8f * d

    // ---- The robot ------------------------------------------------------------------------

    private enum class Mood { NORMAL, HAPPY, SLEEPY, FOCUS, DIZZY }

    private fun mood(): Mood = when {
        dizzy > 0f -> Mood.DIZZY
        happy > 0f -> Mood.HAPPY
        yawn > 0.25f && yawn < 0.8f -> Mood.SLEEPY
        focused -> Mood.FOCUS
        sleepy && music?.playing != true -> Mood.SLEEPY
        else -> Mood.NORMAL
    }

    /** Two eyes either side of the camera in a [h]-tall row centred on [cy]. */
    private fun eyes(c: Canvas, cy: Float, h: Float, mood: Mood = mood()) {
        val cx = width / 2f
        val wing = (restW - hole) / 2f
        val spread = hole / 2f + wing * 0.46f
        val eyeW = h * 0.17f
        val eyeH = h * 0.40f
        val fly = if (thrown in 0.001f..0.999f) sin(thrown * Math.PI).toFloat() * h * 1.6f else 0f
        val y = cy - sin(hop * Math.PI).toFloat() * h * 0.12f - fly
        val color = tint ?: EYE
        fill.color = color
        stroke.color = color
        for (side in listOf(-1f, 1f)) {
            val x = cx + side * spread + look * 2.5f * d
            when (mood) {
                Mood.HAPPY -> {
                    // Smiling eyes: upside-down U arcs.
                    stroke.strokeWidth = eyeW * 0.62f
                    val arc = RectF(x - eyeW * 1.05f, y - eyeH * 0.18f, x + eyeW * 1.05f, y + eyeH * 0.62f)
                    c.drawArc(arc, 200f, 140f, false, stroke)
                }
                Mood.SLEEPY -> {
                    stroke.strokeWidth = 2.2f * d
                    c.drawLine(x - eyeW * 0.9f, y + eyeH * 0.12f, x + eyeW * 0.9f, y + eyeH * 0.12f, stroke)
                }
                Mood.FOCUS -> {
                    val r = RectF(x - eyeW * 0.65f, y - eyeH * 0.12f, x + eyeW * 0.65f, y + eyeH * 0.28f)
                    c.drawRoundRect(r, eyeW * 0.3f, eyeW * 0.3f, fill)
                }
                Mood.DIZZY -> {
                    stroke.strokeWidth = 1.8f * d
                    c.drawCircle(x, y, eyeW * 0.85f, stroke)
                    val angle = dizzy * 10f * side
                    c.drawCircle(x + kotlin.math.cos(angle) * eyeW * 0.4f, y + sin(angle) * eyeW * 0.4f, eyeW * 0.3f, fill)
                }
                Mood.NORMAL -> {
                    val open = (1f - blink * 0.88f) * (1f + beat * 0.35f)
                    val eh = max(eyeH * open, 2f * d)
                    val r = RectF(x - eyeW / 2f, y - eh / 2f, x + eyeW / 2f, y + eh / 2f)
                    c.drawRoundRect(r, eyeW / 2f, eyeW / 2f, fill)
                }
            }
        }
    }

    private val rest = object : Face() {
        override val w get() = restW + (if (speed != null) 46f * d else 0f) + sin(yawn * Math.PI).toFloat() * 10f * d
        override val h get() = pillH

        override fun draw(c: Canvas, r: RectF) {
            eyes(c, r.centerY(), r.height())
            speed?.let { value ->
                captionFont.color = DIM
                textRight(c, value, captionFont, r.right - pillH * 0.45f, r.centerY())
                captionFont.color = FAINT
            }
            if (privacyDot != 0) {
                fill.color = privacyDot
                c.drawCircle(r.right - pillH * 0.32f, r.top + pillH * 0.3f, 3f * d, fill)
            }
            if (gameRound >= 0 && dotSide != 0) {
                fill.color = Color.rgb(255, 214, 10)
                c.drawCircle(width / 2f + dotSide * (r.width() / 2f - pillH * 0.5f), r.centerY(), pillH * 0.2f, fill)
            }
        }
    }

    // ---- Banners ----------------------------------------------------------------------------

    private val sayFace = object : Face() {
        override val w get() = (labelFont.measureText(sayText) + 48f * d).coerceIn(restW + 40f * d, cardW)
        override val h get() = pillH + 30f * d

        override fun draw(c: Canvas, r: RectF) {
            eyes(c, r.top + pillH / 2f, pillH, if (dizzy > 0f) Mood.DIZZY else Mood.HAPPY)
            val textW = labelFont.measureText(fit(sayText, labelFont, r.width() - 32f * d))
            text(c, sayText, labelFont, r.centerX() - textW / 2f, r.top + pillH + 10f * d, r.width() - 32f * d)
        }
    }

    private val peekFace = object : Face() {
        override val w get() = restW + 84f * d
        override val h get() = pillH

        override fun draw(c: Canvas, r: RectF) {
            val time = SimpleDateFormat("h:mm", Locale.getDefault()).format(Date())
            text(c, time, valueFont, r.left + pillH * 0.55f, r.centerY())
            if (battery >= 0) {
                val color = when {
                    charging -> GREEN
                    battery <= 20 -> RED
                    else -> Color.WHITE
                }
                val right = r.right - pillH * 0.5f
                batteryGlyph(c, right, r.centerY(), battery, color)
                captionFont.color = Color.WHITE
                textRight(c, "$battery%", valueFont, right - 30f * d, r.centerY())
                captionFont.color = FAINT
            }
        }

        override val tick get() = 1000L
    }

    private val chargeFace = object : Face() {
        override val w get() = restW + 96f * d
        override val h get() = pillH
        override val key get() = GREEN

        override fun draw(c: Canvas, r: RectF) {
            text(c, if (battery >= 100) "Charged" else "Charging", labelFont, r.left + pillH * 0.55f, r.centerY(), holeLeft - r.left - pillH * 0.55f)
            val right = r.right - pillH * 0.5f
            batteryGlyph(c, right, r.centerY(), battery, GREEN)
            valueFont.color = GREEN
            textRight(c, "$battery%", valueFont, right - 30f * d, r.centerY())
            valueFont.color = Color.WHITE
        }
    }

    private val ringerFace = object : Face() {
        override val w get() = max(compactW + 70f * d, 236f * d)
        override val h get() = pillH + 4f * d
        override val key get() = if (ringerSilent) ORANGE else BLUE

        override fun draw(c: Canvas, r: RectF) {
            val label = if (ringerSilent) "Silent Mode" else "Ring Mode"
            text(c, label, valueFont, r.left + h * 0.55f, r.centerY(), holeLeft - r.left - h * 0.55f)
            val pillW = 48f * d
            val pillHt = 26f * d
            val right = r.right - (h - pillHt) / 2f - 2f * d
            val capsule = RectF(right - pillW, r.centerY() - pillHt / 2f, right, r.centerY() + pillHt / 2f)
            fill.color = key
            c.drawRoundRect(capsule, pillHt / 2f, pillHt / 2f, fill)
            icon(c, if (ringerSilent) R.drawable.is_bell_off else R.drawable.is_bell, capsule.centerX(), capsule.centerY(), 16f * d, Color.WHITE)
        }
    }

    private val lowBatteryFace = object : Face() {
        override val w get() = cardW
        override val h get() = pillH + 98f * d
        override val key get() = ORANGE

        override fun draw(c: Canvas, r: RectF) {
            val band = r.top + pillH / 2f + 4f * d
            text(c, "Low Battery", titleFont, r.left + margin + 4f * d, band, holeLeft - r.left - margin)
            val right = r.right - margin - 4f * d
            batteryGlyph(c, right, band, battery, ORANGE)
            valueFont.color = ORANGE
            textRight(c, "$battery%", valueFont, right - 30f * d, band)
            valueFont.color = Color.WHITE
            text(c, "Connect a charger or turn on Battery Saver", bodyFont, r.left + margin + 4f * d, r.top + pillH + 16f * d, r.width() - 2f * margin)
            val action = RectF(r.left + margin, r.bottom - margin - 40f * d, r.right - margin, r.bottom - margin)
            fill.color = withAlpha(ORANGE, 0.22f)
            c.drawRoundRect(action, 20f * d, 20f * d, fill)
            valueFont.color = ORANGE
            val label = "Turn on Battery Saver"
            text(c, label, valueFont, action.centerX() - valueFont.measureText(label) / 2f, action.centerY())
            valueFont.color = Color.WHITE
            button(action) { dismiss(); onBatterySaver?.invoke() }
        }
    }

    private val noticeFace = object : Face() {
        private val rowH get() = 50f * d
        override val w get() = cardW
        override val h get() = pillH + 8f * d + rowH + (if (notice?.actions.isNullOrEmpty()) 0f else 44f * d) + margin

        override fun draw(c: Canvas, r: RectF) {
            val n = notice ?: return
            button(r) { dismiss(); onNoticeOpen?.invoke(n.key) }
            // Top band, either side of the camera: the app, and an "open" arrow.
            val band = r.top + pillH / 2f + 2f * d
            var x = r.left + margin + 4f * d
            n.icon?.let { drawable ->
                val s = (16f * d).toInt()
                drawable.setBounds(x.toInt(), (band - s / 2f).toInt(), x.toInt() + s, (band + s / 2f).toInt())
                drawable.draw(c)
                x += 22f * d
            }
            text(c, "${n.app.uppercase(Locale.getDefault())} · NOW", captionFont, x, band, holeLeft - x)
            val arrow = r.right - margin - 8f * d
            icon(c, R.drawable.is_open, arrow, band, 16f * d, DIM)
            button(RectF(arrow - 20f * d, r.top, r.right, r.top + pillH)) { dismiss(); onNoticeOpen?.invoke(n.key) }

            // The message: avatar, title, two lines of text.
            val rowTop = r.top + pillH + 8f * d
            val size = 42f * d
            val avatarX = r.left + margin + size / 2f
            if (n.avatar != null) {
                avatar(c, n.avatar, avatarX, rowTop + size / 2f, size, R.drawable.is_bell, Color.WHITE)
            } else {
                fill.color = withAlpha(Color.WHITE, 0.12f)
                c.drawCircle(avatarX, rowTop + size / 2f, size / 2f, fill)
                n.icon?.let { drawable ->
                    val s = (24f * d).toInt()
                    val cy = rowTop + size / 2f
                    drawable.setBounds((avatarX - s / 2f).toInt(), (cy - s / 2f).toInt(), (avatarX + s / 2f).toInt(), (cy + s / 2f).toInt())
                    drawable.draw(c)
                }
            }
            val textX = r.left + margin + size + 12f * d
            val textW = r.right - margin - textX
            text(c, n.title, titleFont, textX, rowTop + 9f * d, textW)
            if (n.text.isNotEmpty()) {
                val layout = StaticLayout.Builder.obtain(n.text, 0, n.text.length, bodyFont, textW.toInt().coerceAtLeast(1))
                    .setMaxLines(2)
                    .setEllipsize(TextUtils.TruncateAt.END)
                    .setAlignment(Layout.Alignment.ALIGN_NORMAL)
                    .build()
                c.save()
                c.translate(textX, rowTop + 19f * d)
                layout.draw(c)
                c.restore()
            }

            // The notification's own buttons as chips.
            var chipX = r.left + margin
            val chipTop = rowTop + rowH + 10f * d
            n.actions.take(3).forEachIndexed { index, label ->
                val tw = chipFont.measureText(label)
                val chip = RectF(chipX, chipTop, chipX + tw + 26f * d, chipTop + 30f * d)
                if (chip.right > r.right - margin) return@forEachIndexed
                fill.color = withAlpha(Color.WHITE, 0.14f)
                c.drawRoundRect(chip, 15f * d, 15f * d, fill)
                text(c, label, chipFont, chip.left + 13f * d, chip.centerY())
                button(chip) { dismiss(); onNoticeAction?.invoke(n.key, index) }
                chipX = chip.right + 8f * d
            }
        }
    }

    // ---- Call -------------------------------------------------------------------------------

    private val callCompact = object : Face() {
        override val w get() = compactW
        override val h get() = pillH
        override val key get() = GREEN
        override val tick get() = 1000L

        override fun draw(c: Canvas, r: RectF) {
            val call = call ?: return
            val size = pillH - 12f * d
            avatar(c, call.avatar, r.left + pillH / 2f + 2f * d, r.centerY(), size, R.drawable.is_call, GREEN)
            valueFont.color = GREEN
            textRight(c, clock(System.currentTimeMillis() - call.since), valueFont, r.right - pillH * 0.5f, r.centerY())
            valueFont.color = Color.WHITE
        }
    }

    private val callCard = object : Face() {
        override val w get() = cardW
        override val h get() = max(pillH + 44f * d, 80f * d)
        override val key get() = GREEN
        override val tick get() = 1000L

        override fun draw(c: Canvas, r: RectF) {
            val call = call ?: return
            val size = 48f * d
            val cy = r.centerY() + 2f * d
            avatar(c, call.avatar, r.left + margin + size / 2f, cy, size, R.drawable.is_call, GREEN)
            val buttonSize = 44f * d
            val gap = 10f * d
            val secondX = r.right - margin - buttonSize / 2f
            val firstX = secondX - buttonSize - gap
            val textX = r.left + margin + size + 12f * d
            val textW = firstX - buttonSize / 2f - 10f * d - textX
            text(c, call.name, titleFont, textX, cy - 9f * d, textW)
            if (call.ringing) {
                text(c, "Incoming call", bodyFont, textX, cy + 10f * d, textW)
                circleButton(c, firstX, cy, buttonSize, RED, R.drawable.is_call_end, Color.WHITE) { onCallAction?.invoke("decline") }
                circleButton(c, secondX, cy, buttonSize, GREEN, R.drawable.is_call, Color.WHITE) { onCallAction?.invoke("answer") }
            } else {
                valueFont.color = GREEN
                text(c, clock(System.currentTimeMillis() - call.since), valueFont, textX, cy + 10f * d)
                valueFont.color = Color.WHITE
                val muteBg = if (callMuted) Color.WHITE else withAlpha(Color.WHITE, 0.16f)
                circleButton(c, firstX, cy, buttonSize, muteBg, R.drawable.is_mic_off, if (callMuted) Color.BLACK else Color.WHITE) {
                    callMuted = !callMuted
                    onCallAction?.invoke("mute")
                }
                circleButton(c, secondX, cy, buttonSize, RED, R.drawable.is_call_end, Color.WHITE) { onCallAction?.invoke("end") }
            }
        }
    }

    // ---- Timer ------------------------------------------------------------------------------

    private fun timerLeft() = ((activity?.until ?: 0L) - System.currentTimeMillis()).coerceAtLeast(0L)

    private val timerCompact = object : Face() {
        override val w get() = compactW
        override val h get() = pillH
        override val key get() = ORANGE
        override val tick get() = 500L

        override fun draw(c: Canvas, r: RectF) {
            icon(c, R.drawable.is_timer, r.left + pillH / 2f + 2f * d, r.centerY(), 20f * d, ORANGE)
            valueFont.color = ORANGE
            textRight(c, clock(timerLeft() + 999), valueFont, r.right - pillH * 0.5f, r.centerY())
            valueFont.color = Color.WHITE
        }
    }

    private val timerCard = object : Face() {
        override val w get() = cardW
        override val h get() = max(pillH + 50f * d, 86f * d)
        override val key get() = ORANGE
        override val tick get() = 500L

        override fun draw(c: Canvas, r: RectF) {
            val cy = r.centerY() + 4f * d
            val size = 46f * d
            fill.color = withAlpha(ORANGE, 0.2f)
            c.drawCircle(r.left + margin + size / 2f, cy, size / 2f, fill)
            icon(c, R.drawable.is_timer, r.left + margin + size / 2f, cy, 24f * d, ORANGE)
            val textX = r.left + margin + size + 12f * d
            text(c, activity?.text?.ifEmpty { "Timer" } ?: "Timer", captionFont, textX, cy - 14f * d)
            bigFont.color = ORANGE
            text(c, clock(timerLeft() + 999), bigFont, textX, cy + 8f * d)
            bigFont.color = Color.WHITE
            circleButton(c, r.right - margin - 22f * d, cy, 44f * d, withAlpha(Color.WHITE, 0.16f), R.drawable.is_close, Color.WHITE) {
                expandedByUser = false
                onTimerCancel?.invoke()
            }
        }
    }

    // ---- Live progress (ride, delivery, download) -------------------------------------------

    private fun ring(c: Canvas, cx: Float, cy: Float, radius: Float, progress: Float?, color: Int) {
        stroke.strokeWidth = 2.6f * d
        stroke.color = withAlpha(Color.WHITE, 0.2f)
        c.drawCircle(cx, cy, radius, stroke)
        if (progress != null) {
            stroke.color = color
            c.drawArc(RectF(cx - radius, cy - radius, cx + radius, cy + radius), -90f, 360f * progress.coerceIn(0f, 1f), false, stroke)
        }
    }

    private val liveCompact = object : Face() {
        override val w get() = compactW
        override val h get() = pillH
        override val key get() = GREEN

        override fun draw(c: Canvas, r: RectF) {
            val live = activity ?: return
            labelFont.color = Color.WHITE
            text(c, live.text, labelFont, r.left + pillH * 0.5f, r.centerY(), holeLeft - r.left - pillH * 0.5f)
            ring(c, r.right - pillH / 2f, r.centerY(), pillH * 0.24f, live.progress, GREEN)
        }
    }

    private val liveCard = object : Face() {
        override val w get() = cardW
        override val h get() = pillH + 64f * d

        override val key get() = GREEN

        override fun draw(c: Canvas, r: RectF) {
            val live = activity ?: return
            val x = r.left + margin + 4f * d
            val width = r.width() - 2f * margin - 8f * d
            text(c, live.text, titleFont, x, r.top + pillH + 6f * d, width - 50f * d)
            if (live.detail.isNotEmpty()) text(c, live.detail, bodyFont, x, r.top + pillH + 26f * d, width - 50f * d)
            ring(c, r.right - margin - 18f * d, r.top + pillH + 16f * d, 15f * d, live.progress, GREEN)
            live.progress?.let { progress ->
                val bar = RectF(x, r.bottom - margin - 5f * d, x + width, r.bottom - margin)
                fill.color = withAlpha(Color.WHITE, 0.2f)
                c.drawRoundRect(bar, 2.5f * d, 2.5f * d, fill)
                bar.right = bar.left + bar.width() * progress.coerceIn(0f, 1f)
                fill.color = GREEN
                c.drawRoundRect(bar, 2.5f * d, 2.5f * d, fill)
            }
        }
    }

    // ---- Music ------------------------------------------------------------------------------

    private fun musicPosition(m: Music): Long {
        val moved = if (m.playing) ((SystemClock.elapsedRealtime() - m.updatedAt) * m.speed).toLong() else 0L
        return (m.position + moved).coerceIn(0L, if (m.duration > 0) m.duration else Long.MAX_VALUE)
    }

    /** Four bars dancing while music plays, right-aligned at [right]. */
    private fun waveform(c: Canvas, right: Float, cy: Float, height: Float, playing: Boolean) {
        val bar = 3f * d
        val gap = 2.5f * d
        val t = SystemClock.uptimeMillis() / 1000f
        fill.color = accent
        for (index in 0 until 4) {
            val wave = if (playing) 0.35f + 0.65f * abs(sin(t * (5.1f + index * 1.7f) + index * 1.3f)) else 0.25f
            val bh = max(height * wave, bar)
            val x = right - (4 - index) * (bar + gap) + gap
            c.drawRoundRect(RectF(x, cy - bh / 2f, x + bar, cy + bh / 2f), bar / 2f, bar / 2f, fill)
        }
    }

    private fun art(c: Canvas, m: Music, r: RectF, radius: Float) {
        if (m.art != null) {
            picture(c, m.art, r, radius)
        } else {
            fill.color = withAlpha(Color.WHITE, 0.14f)
            c.drawRoundRect(r, radius, radius, fill)
            icon(c, R.drawable.is_play, r.centerX(), r.centerY(), r.width() * 0.5f, DIM)
        }
    }

    private val musicCompact = object : Face() {
        override val w get() = compactW
        override val h get() = pillH
        override val tick get() = 60L

        override fun draw(c: Canvas, r: RectF) {
            val m = music ?: return
            val size = pillH - 12f * d
            val left = r.left + pillH / 2f + 2f * d - size / 2f
            art(c, m, RectF(left, r.centerY() - size / 2f, left + size, r.centerY() + size / 2f), 6f * d)
            waveform(c, r.right - pillH * 0.42f, r.centerY(), pillH * 0.42f, m.playing)
        }
    }

    private val musicCard = object : Face() {
        override val w get() = cardW
        override val h get() = 166f * d
        override val tick get() = 60L

        override fun draw(c: Canvas, r: RectF) {
            val m = music ?: return
            val size = 54f * d
            val artRect = RectF(r.left + margin, r.top + 16f * d, r.left + margin + size, r.top + 16f * d + size)
            art(c, m, artRect, 12f * d)
            waveform(c, r.right - margin - 2f * d, r.top + pillH / 2f + 2f * d, 16f * d, m.playing)
            // Title and artist sit below the camera band so long names never run into it.
            val textX = artRect.right + 12f * d
            val textW = r.right - margin - textX
            text(c, m.title, titleFont, textX, artRect.centerY() + 6f * d, textW)
            text(c, m.artist, bodyFont, textX, artRect.centerY() + 24f * d, textW)

            // Scrubber with elapsed and total time.
            val scrubY = artRect.bottom + 22f * d
            val position = musicPosition(m)
            captionFont.color = DIM
            val elapsed = clock(position)
            val total = if (m.duration > 0) clock(m.duration) else "--:--"
            text(c, elapsed, captionFont, r.left + margin, scrubY)
            textRight(c, total, captionFont, r.right - margin, scrubY)
            captionFont.color = FAINT
            val bar = RectF(r.left + margin + 40f * d, scrubY - 2f * d, r.right - margin - 40f * d, scrubY + 2f * d)
            fill.color = withAlpha(Color.WHITE, 0.22f)
            c.drawRoundRect(bar, 2f * d, 2f * d, fill)
            if (m.duration > 0) {
                val done = RectF(bar).apply { right = left + width() * (position.toFloat() / m.duration).coerceIn(0f, 1f) }
                fill.color = Color.WHITE
                c.drawRoundRect(done, 2f * d, 2f * d, fill)
            }

            // Transport.
            val controlsY = scrubY + 34f * d
            val cx = r.centerX()
            transport(c, R.drawable.is_prev, cx - 70f * d, controlsY, 28f * d) { onMedia?.invoke("prev") }
            transport(c, if (m.playing) R.drawable.is_pause else R.drawable.is_play, cx, controlsY, 36f * d) { onMedia?.invoke("toggle") }
            transport(c, R.drawable.is_next, cx + 70f * d, controlsY, 28f * d) { onMedia?.invoke("next") }
        }
    }

    private fun transport(c: Canvas, res: Int, cx: Float, cy: Float, size: Float, action: () -> Unit) {
        icon(c, res, cx, cy, size, Color.WHITE)
        button(RectF(cx - 26f * d, cy - 24f * d, cx + 26f * d, cy + 24f * d), action)
    }

    // ---- Frame ------------------------------------------------------------------------------

    init {
        current = rest
        scheduleBlink()
        scheduleLook()
    }

    override fun onDraw(canvas: Canvas) {
        val now = SystemClock.uptimeMillis()
        val dt = ((now - lastFrame).coerceIn(1L, 32L)) / 1000f
        lastFrame = now

        // A banner that ran out while nothing redrew.
        if (banner != null && System.currentTimeMillis() >= bannerUntil) {
            banner = null
            val next = pick()
            if (next !== current) {
                previous = current
                current = next
                switchedAt = now
            }
        }

        val targetW = current.w
        val targetH = current.h
        step(curW, velW, targetW, dt).let { (x, v) -> curW = x; velW = v }
        step(curH, velH, targetH, dt).let { (x, v) -> curH = x; velH = v }
        val moving = velW != 0f || velH != 0f

        val cx = width / 2f
        shapeRect.set(cx - curW / 2f, pad, cx + curW / 2f, pad + curH)
        val radius = min(curH / 2f, 38f * d)
        shapePath.reset()
        shapePath.addRoundRect(shapeRect, radius, radius, Path.Direction.CW)
        canvas.drawPath(shapePath, shapePaint)

        // A thin key line in the activity's colour, like Apple's dark-mode outline.
        val key = current.key
        if (key != 0) {
            keyLine.color = withAlpha(key, 0.35f)
            canvas.drawPath(shapePath, keyLine)
        }

        canvas.save()
        canvas.clipPath(shapePath)
        val since = now - switchedAt
        val old = previous
        if (old != null) {
            val fade = 1f - since / 110f
            if (fade > 0f) {
                drawFace(canvas, old, fade, primary = false)
            } else {
                previous = null
            }
        }
        val alpha = ((since - 90f) / 190f).coerceIn(0f, 1f)
        drawFace(canvas, current, alpha, primary = true)
        canvas.restore()

        when {
            moving || previous != null || alpha < 1f -> postInvalidateOnAnimation()
            current.tick > 0 -> postInvalidateDelayed(current.tick)
            else -> fitWindow(curW, curH)
        }
    }

    private fun drawFace(canvas: Canvas, face: Face, alpha: Float, primary: Boolean) {
        val r = RectF(width / 2f - face.w / 2f, pad, width / 2f + face.w / 2f, pad + face.h)
        if (primary) hits.clear()
        collecting = primary && alpha > 0.5f
        val layered = alpha < 1f
        if (layered) canvas.saveLayerAlpha(r, (alpha * 255).toInt())
        face.draw(canvas, r)
        if (layered) canvas.restore()
        collecting = false
    }

    // ---- Touch ------------------------------------------------------------------------------

    private var downX = 0f
    private var downY = 0f
    private var pressedAt = 0L
    private var lastTap = 0L
    private val longPress = Runnable {
        pressedAt = 0L
        onThrow?.invoke()
    }
    private val feed = Runnable {
        pressedAt = 0L
        onFeed?.invoke()
    }
    private val singleTap = Runnable {
        if (onScreenshotTap?.invoke() == true) return@Runnable
        animate01(1300) { happy = it }
        doHop()
        peek()
        onPet?.invoke()
    }

    @SuppressLint("ClickableViewAccessibility")
    override fun onTouchEvent(event: MotionEvent): Boolean {
        when (event.actionMasked) {
            MotionEvent.ACTION_OUTSIDE -> {
                if (current !== rest && !isCompact(current) && call?.ringing != true) dismiss()
                return false
            }
            MotionEvent.ACTION_DOWN -> {
                if (!shapeRect.contains(event.x, event.y)) {
                    if (current !== rest && !isCompact(current)) dismiss()
                    return false
                }
                downX = event.x
                downY = event.y
                pressedAt = event.eventTime
                if (gameRound >= 0 && current === rest) {
                    gameTap(if (event.x < width / 2f) -1 else 1)
                    pressedAt = 0L
                    return true
                }
                if (current === rest || isCompact(current)) {
                    // Charging: a long hold feeds the robot; otherwise long-press opens the panel.
                    val feeding = charging && current === rest
                    postDelayed(if (feeding) feed else longPress, if (feeding) 2000 else 550)
                }
            }
            MotionEvent.ACTION_MOVE -> {
                val dx = event.x - downX
                val dy = event.y - downY
                if (pressedAt != 0L && (abs(dx) > 28 * d || abs(dy) > 24 * d)) {
                    removeCallbacks(longPress)
                    removeCallbacks(feed)
                    pressedAt = 0L
                    when {
                        abs(dx) > abs(dy) -> {
                            onSwipe?.invoke(if (dx > 0) 1 else -1)
                            look = if (dx > 0) 1f else -1f
                            doHop()
                        }
                        dy < 0 -> dismiss()
                        current === rest || isCompact(current) -> onThrow?.invoke()
                    }
                }
            }
            MotionEvent.ACTION_UP -> {
                removeCallbacks(longPress)
                removeCallbacks(feed)
                if (pressedAt != 0L) tap(event.x, event.y, event.eventTime)
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

    private fun tap(x: Float, y: Float, time: Long) {
        hits.lastOrNull { it.first.contains(x, y) }?.let { (_, action) ->
            performHapticFeedback(android.view.HapticFeedbackConstants.CONTEXT_CLICK)
            action()
            return
        }
        when {
            current === rest -> {
                if (time - lastTap < 300) {
                    removeCallbacks(singleTap)
                    lastTap = 0L
                    onDoubleTap?.invoke()
                } else {
                    lastTap = time
                    postDelayed(singleTap, 300)
                }
            }
            isCompact(current) -> {
                expandedByUser = true
                removeCallbacks(collapse)
                postDelayed(collapse, 8000)
                refresh()
            }
            current === sayFace -> if (onScreenshotTap?.invoke() != true) dismiss()
            else -> dismiss()
        }
    }

    companion object {
        /** Soft white eyes; the robot never shouts in neon. */
        val EYE: Int = Color.rgb(245, 245, 247)
        val GREEN: Int = Color.rgb(48, 209, 88)
        val ORANGE: Int = Color.rgb(255, 159, 10)
        val RED: Int = Color.rgb(255, 69, 58)
        val BLUE: Int = Color.rgb(10, 132, 255)
        val DIM: Int = Color.argb(166, 255, 255, 255)
        val FAINT: Int = Color.argb(140, 255, 255, 255)

        fun withAlpha(color: Int, alpha: Float): Int =
            Color.argb((alpha * 255).toInt(), Color.red(color), Color.green(color), Color.blue(color))

        /** A bright version of the artwork's average colour, for the waveform. */
        fun accentOf(art: Bitmap): Int {
            val pixel = Bitmap.createScaledBitmap(art, 1, 1, true).getPixel(0, 0)
            val hsv = FloatArray(3)
            Color.colorToHSV(pixel, hsv)
            if (hsv[1] < 0.12f) return Color.WHITE
            hsv[1] = max(hsv[1], 0.45f)
            hsv[2] = max(hsv[2], 0.9f)
            return Color.HSVToColor(hsv)
        }
    }
}
