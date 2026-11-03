package com.notchy.app

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.RectF
import android.view.MotionEvent
import android.view.View
import kotlin.math.abs
import kotlin.math.hypot

/**
 * The phone as the PC's touchpad: one finger moves the pointer (with a little acceleration),
 * a tap clicks, a two-finger tap right-clicks, two fingers drag to scroll.
 */
class TouchpadView(context: Context) : View(context) {
    private val density = resources.displayMetrics.density
    private val surface = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Ui.CARD2 }
    private val hint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Ui.FAINT
        textSize = 14f * density
        textAlign = Paint.Align.CENTER
    }
    private val touch = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Ui.MINT; alpha = 70 }

    private var lastX = 0f
    private var lastY = 0f
    private var downAt = 0L
    private var travel = 0f
    private var fingers = 0
    private var scrollCarry = 0f
    private var pendingX = 0f
    private var pendingY = 0f
    private var lastSend = 0L
    private var touchX = -1f
    private var touchY = -1f

    @SuppressLint("ClickableViewAccessibility")
    override fun onTouchEvent(event: MotionEvent): Boolean {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                downAt = event.eventTime
                travel = 0f
                fingers = 1
                lastX = event.x
                lastY = event.y
            }
            MotionEvent.ACTION_POINTER_DOWN -> {
                fingers = maxOf(fingers, event.pointerCount)
                lastX = average(event, true)
                lastY = average(event, false)
            }
            MotionEvent.ACTION_MOVE -> {
                val x = average(event, true)
                val y = average(event, false)
                val dx = x - lastX
                val dy = y - lastY
                lastX = x
                lastY = y
                travel += hypot(dx, dy)
                if (event.pointerCount >= 2) {
                    scrollCarry += dy / (24f * density)
                    val lines = scrollCarry.toInt()
                    if (lines != 0) {
                        Link.send("scroll") { put("d", lines) }
                        scrollCarry -= lines
                    }
                } else {
                    val speed = hypot(dx, dy) / density
                    val gain = 1.4f + (speed / 8f).coerceAtMost(2.2f)
                    pendingX += dx * gain
                    pendingY += dy * gain
                    flush(event.eventTime)
                }
                touchX = event.x
                touchY = event.y
                invalidate()
            }
            MotionEvent.ACTION_UP -> {
                flush(event.eventTime, force = true)
                val quick = event.eventTime - downAt < 220
                if (quick && travel < 12 * density) {
                    Link.send("click") { put("b", if (fingers >= 2) "right" else "left") }
                    performHapticFeedback(android.view.HapticFeedbackConstants.CONTEXT_CLICK)
                }
                fingers = 0
                touchX = -1f
                invalidate()
            }
        }
        return true
    }

    /** Sends movement at most ~60 times a second. */
    private fun flush(now: Long, force: Boolean = false) {
        if (!force && now - lastSend < 16) return
        if (abs(pendingX) < 1 && abs(pendingY) < 1) return
        val x = pendingX.toInt()
        val y = pendingY.toInt()
        Link.send("mouse") { put("dx", x); put("dy", y) }
        pendingX -= x
        pendingY -= y
        lastSend = now
    }

    private fun average(event: MotionEvent, horizontal: Boolean): Float {
        var sum = 0f
        for (index in 0 until event.pointerCount) sum += if (horizontal) event.getX(index) else event.getY(index)
        return sum / event.pointerCount
    }

    override fun onDraw(canvas: Canvas) {
        val radius = 24f * density
        canvas.drawRoundRect(RectF(0f, 0f, width.toFloat(), height.toFloat()), radius, radius, surface)
        if (touchX >= 0) {
            canvas.drawCircle(touchX, touchY, 34f * density, touch)
        } else {
            canvas.drawText("Move with one finger · tap to click", width / 2f, height / 2f - 6 * density, hint)
            canvas.drawText("Two fingers: scroll · two-finger tap: right click", width / 2f, height / 2f + 16 * density, hint)
        }
    }
}
