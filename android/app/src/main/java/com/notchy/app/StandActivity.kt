package com.notchy.app

import android.animation.ValueAnimator
import android.app.Activity
import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.RectF
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.Gravity
import android.view.View
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.LinearLayout
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlin.math.sin

/**
 * Stand mode: the phone as a second screen beside the laptop. A big clock, the robot's face,
 * the PC's music with controls, a Focus countdown and the PC's battery. Keeps the screen on;
 * tap the clock to leave.
 */
class StandActivity : Activity() {
    private val handler = Handler(Looper.getMainLooper())
    private lateinit var clock: android.widget.TextView
    private lateinit var date: android.widget.TextView
    private lateinit var status: android.widget.TextView
    private lateinit var focus: android.widget.TextView
    private lateinit var media: LinearLayout
    private val listener: () -> Unit = { refresh() }

    private val tick = object : Runnable {
        override fun run() {
            refresh()
            handler.postDelayed(this, 1000)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        window.statusBarColor = android.graphics.Color.BLACK
        window.navigationBarColor = android.graphics.Color.BLACK
        val root = FrameLayout(this).apply { setBackgroundColor(android.graphics.Color.BLACK) }
        val column = column(14).apply {
            gravity = Gravity.CENTER
            setPadding(dp(24), dp(24), dp(24), dp(24))
        }
        column.addView(Face(this), LinearLayout.LayoutParams(dp(220), dp(90)).apply { gravity = Gravity.CENTER })
        clock = label("", 84f, Ui.TEXT, bold = true).apply {
            gravity = Gravity.CENTER
            setOnClickListener { finish() }
        }
        date = label("", 16f, Ui.DIM).apply { gravity = Gravity.CENTER }
        focus = label("", 20f, Ui.MINT, bold = true).apply { gravity = Gravity.CENTER }
        status = label("", 14f, Ui.DIM).apply { gravity = Gravity.CENTER }
        media = column(10).apply { gravity = Gravity.CENTER }
        listOf(clock, date, focus, media, status).forEach { column.addView(it, matchWidth()) }
        root.addView(column, fill())
        setContentView(root)
    }

    override fun onResume() {
        super.onResume()
        Link.addListener(listener)
        handler.post(tick)
    }

    override fun onPause() {
        Link.removeListener(listener)
        handler.removeCallbacks(tick)
        super.onPause()
    }

    private var shownMedia: Triple<String, String, Boolean>? = null

    private fun refresh() {
        val now = Date()
        clock.text = SimpleDateFormat("h:mm", Locale.getDefault()).format(now)
        date.text = SimpleDateFormat("EEEE, d MMMM", Locale.getDefault()).format(now)
        val left = Link.focusUntil - System.currentTimeMillis()
        focus.text = when {
            Link.pcMeeting -> "In a meeting"
            left > 0 -> "Focus · %d:%02d left".format(left / 60_000, left / 1000 % 60)
            else -> ""
        }
        focus.visibility = if (focus.text.isEmpty()) View.GONE else View.VISIBLE
        status.text = when {
            !Link.connected -> "PC not connected"
            Link.pcBattery >= 0 -> "${Link.pcName} · ${Link.pcBattery}%${if (Link.pcCharging) " charging" else ""}"
            else -> "${Link.pcName}"
        }
        if (Link.pcMedia != shownMedia) {
            shownMedia = Link.pcMedia
            media.removeAllViews()
            shownMedia?.let { (title, artist, playing) ->
                media.addView(label(title, 20f, Ui.TEXT, bold = true).apply { gravity = Gravity.CENTER }, matchWidth())
                if (artist.isNotEmpty()) media.addView(label(artist, 15f, Ui.DIM).apply { gravity = Gravity.CENTER }, matchWidth())
                val controls = row(12).apply { gravity = Gravity.CENTER }
                controls.addView(pill("⏮") { Link.send("media_cmd") { put("cmd", "prev") } })
                controls.addView(pill(if (playing) "⏸" else "▶", primary = true) { Link.send("media_cmd") { put("cmd", "toggle") } })
                controls.addView(pill("⏭") { Link.send("media_cmd") { put("cmd", "next") } })
                media.addView(controls, matchWidth())
            }
        }
    }

    /** The robot's face, big: two eyes that blink and drift. */
    private class Face(context: Context) : View(context) {
        private val eye = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = RobotView.EYE }
        private val glow = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = RobotView.EYE; alpha = 50 }
        private var phase = 0f
        private var blink = 0f

        init {
            ValueAnimator.ofFloat(0f, 1f).apply {
                duration = 6000
                repeatCount = ValueAnimator.INFINITE
                addUpdateListener {
                    phase = it.animatedValue as Float
                    // A blink near the end of every loop.
                    blink = if (phase > 0.93f) sin((phase - 0.93f) / 0.07f * Math.PI).toFloat() else 0f
                    invalidate()
                }
                start()
            }
        }

        override fun onDraw(canvas: Canvas) {
            val drift = sin(phase * 2 * Math.PI).toFloat() * width * 0.04f
            val h = height * 0.62f * (1f - blink * 0.9f).coerceAtLeast(0.06f)
            val w = width * 0.2f
            for (side in listOf(-1f, 1f)) {
                val cx = width / 2f + side * width * 0.22f + drift
                val r = RectF(cx - w / 2, height / 2f - h / 2, cx + w / 2, height / 2f + h / 2)
                val g = RectF(r).apply { inset(-w * 0.15f, -w * 0.15f) }
                canvas.drawRoundRect(g, w * 0.6f, w * 0.6f, glow)
                canvas.drawRoundRect(r, w / 2, w / 2, eye)
            }
        }
    }
}
