package com.notchy.app

import android.animation.ValueAnimator
import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.view.MotionEvent
import android.view.View
import android.view.animation.DecelerateInterpolator
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlin.math.max
import kotlin.math.sin

/**
 * The robot around the front camera: a black pill wrapping the punch hole, with two glowing
 * eyes either side of it. It blinks, looks around, hops when tapped, and widens to show the
 * time and battery (or a charging bolt) for a few seconds.
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
    private val bolt = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.rgb(255, 214, 61) }

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

    /** A Focus session runs on the PC: narrow, determined eyes. */
    var focused = false
        set(value) { field = value; invalidate() }

    /** Between 11 pm and 6 am the robot is sleepy: heavy lids, slow blinks. */
    private val sleepy: Boolean
        get() {
            val hour = java.util.Calendar.getInstance().get(java.util.Calendar.HOUR_OF_DAY)
            return hour >= 23 || hour < 6
        }

    /** Long-press: the robot is thrown over to the PC. */
    var onThrow: (() -> Unit)? = null

    private val restW get() = hole * 3.4f
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
        if (plugged) peek()
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

    private fun throwUp() {
        ValueAnimator.ofFloat(0f, 1f).apply {
            duration = 1400
            addUpdateListener { thrown = it.animatedValue as Float; invalidate() }
            start()
        }
        onThrow?.invoke()
    }

    private var pressedAt = 0L
    private val longPress = Runnable {
        pressedAt = 0L
        throwUp()
    }

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

    private fun doHop() {
        ValueAnimator.ofFloat(0f, 1f).apply {
            duration = 520
            addUpdateListener { hop = it.animatedValue as Float; invalidate() }
            start()
        }
    }

    @SuppressLint("ClickableViewAccessibility")
    override fun onTouchEvent(event: MotionEvent): Boolean {
        when (event.action) {
            MotionEvent.ACTION_DOWN -> {
                pressedAt = event.eventTime
                postDelayed(longPress, 550)
            }
            MotionEvent.ACTION_UP -> {
                removeCallbacks(longPress)
                if (pressedAt != 0L) {
                    doHop()
                    peek()
                }
                pressedAt = 0L
            }
            MotionEvent.ACTION_CANCEL -> {
                removeCallbacks(longPress)
                pressedAt = 0L
            }
        }
        return true
    }

    override fun onDraw(canvas: Canvas) {
        val w = restW + (openW - restW) * open
        val h = pillH
        val cx = width / 2f
        val top = (height - h) / 2f
        val rect = RectF(cx - w / 2f, top, cx + w / 2f, top + h)
        canvas.drawRoundRect(rect, h / 2f, h / 2f, pill)

        // Eyes sit either side of the camera hole and squash when blinking.
        // Thrown: the eyes shoot up out of the pill, then drop back in.
        val fly = if (thrown in 0.001f..0.999f) sin(thrown * Math.PI).toFloat() * h * 2.2f else 0f
        val bounce = sin(hop * Math.PI).toFloat() * h * 0.18f + fly
        val eyeW = hole * 0.42f
        val mood = when {
            focused -> 0.45f
            sleepy && open == 0f -> 0.35f
            else -> 1f
        }
        val eyeH = max(eyeH(h) * mood * (1f - blink * 0.85f), 1.5f * density)
        val gap = hole * 0.95f
        val shift = look * hole * 0.12f
        for (side in listOf(-1f, 1f)) {
            val ex = cx + side * gap + shift
            val ey = top + h / 2f - bounce
            val r = RectF(ex - eyeW / 2f, ey - eyeH / 2f, ex + eyeW / 2f, ey + eyeH / 2f)
            val g = RectF(r).apply { inset(-3f * density, -3f * density) }
            canvas.drawRoundRect(g, g.height() / 2f, g.height() / 2f, glow)
            canvas.drawRoundRect(r, r.height() / 2f, r.height() / 2f, eye)
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
                    val path = android.graphics.Path().apply {
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
