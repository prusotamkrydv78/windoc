package com.notchy.app

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.LinearGradient
import android.graphics.Paint
import android.graphics.Shader
import android.os.SystemClock
import android.view.View
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.io.File

/**
 * Draws every island state onto a phone-top backdrop and saves it to
 * `app/build/island-preview/`, so the design can be checked without a phone.
 *
 * Run: `gradle testDebugUnitTest --tests com.notchy.app.IslandPreview`
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], qualifiers = "w411dp-h915dp-xxhdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class IslandPreview {
    private val context get() = RuntimeEnvironment.getApplication()
    private val density get() = context.resources.displayMetrics.density
    private val out = File("build/island-preview").apply { mkdirs() }

    private fun picture(top: Int, bottom: Int): Bitmap {
        val size = 192
        val bitmap = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
        Canvas(bitmap).drawPaint(Paint().apply {
            shader = LinearGradient(0f, 0f, size.toFloat(), size.toFloat(), top, bottom, Shader.TileMode.CLAMP)
        })
        return bitmap
    }

    private fun render(name: String, expand: Boolean = false, setup: (RobotView) -> Unit) {
        val view = RobotView(context, 28f * density)
        setup(view)
        view.settle(expand)
        val (w, h) = view.previewSize()
        view.measure(View.MeasureSpec.makeMeasureSpec(w, View.MeasureSpec.EXACTLY), View.MeasureSpec.makeMeasureSpec(h, View.MeasureSpec.EXACTLY))
        view.layout(0, 0, w, h)
        val screenW = context.resources.displayMetrics.widthPixels
        val bitmap = Bitmap.createBitmap(screenW, h + (36 * density).toInt(), Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)
        canvas.drawPaint(Paint().apply {
            shader = LinearGradient(0f, 0f, 0f, bitmap.height.toFloat(), Color.rgb(120, 160, 210), Color.rgb(190, 205, 230), Shader.TileMode.CLAMP)
        })
        canvas.translate((screenW - w) / 2f, 8f * density)
        view.draw(canvas)
        // The real camera hole, to check nothing runs into it.
        val hole = 28f * density
        canvas.drawCircle(w / 2f, 4f * density + view.pillH / 2f, hole / 2f, Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.rgb(28, 28, 34) })
        canvas.drawCircle(w / 2f, 4f * density + view.pillH / 2f, hole / 5f, Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.rgb(20, 30, 60) })
        File(out, "$name.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
    }

    private val now get() = System.currentTimeMillis()

    @Test
    fun renderAll() {
        render("01-rest") {}
        render("02-peek") { it.setBattery(76, false); it.peek() }
        render("03-say") { it.say("Linked to Purshotam's laptop") }
        render("04-charging") { it.setBattery(40, false); it.setBattery(41, true) }
        render("05-low-battery") { it.showLowBattery(14) }
        render("06-silent") { it.showRinger(true) }
        render("07-ring") { it.showRinger(false) }
        render("08-notice") {
            it.showNotice(
                RobotView.Notice(
                    key = "k", app = "WhatsApp", icon = context.getDrawable(R.drawable.ic_notchy),
                    avatar = picture(Color.rgb(255, 180, 90), Color.rgb(200, 60, 120)),
                    title = "John Wick", text = "Hello!!! Can we meet today? Yes! Where do you want to meet? I'm free after six.",
                    actions = listOf("Reply", "Mark as read", "Mute"),
                ),
            )
        }
        render("09-call-ringing") {
            it.call = RobotView.Call("Daisy Williams", picture(Color.rgb(150, 110, 220), Color.rgb(60, 40, 120)), ringing = true, since = now)
        }
        render("10-call-compact") {
            it.call = RobotView.Call("Daisy Williams", picture(Color.rgb(150, 110, 220), Color.rgb(60, 40, 120)), ringing = false, since = now - 68_000)
        }
        render("11-call-card", expand = true) {
            it.call = RobotView.Call("Daisy Williams", picture(Color.rgb(150, 110, 220), Color.rgb(60, 40, 120)), ringing = false, since = now - 68_000)
        }
        val song = RobotView.Music(
            "Stranger Things Have Happened", "Hello Bunny", picture(Color.rgb(40, 200, 220), Color.rgb(20, 60, 140)),
            playing = true, position = 54_000, duration = 170_000, updatedAt = SystemClock.elapsedRealtime(), speed = 1f,
        )
        render("12-music-compact") { it.music = song }
        render("13-music-card", expand = true) { it.music = song }
        render("14-timer-compact") { it.activity = RobotView.LiveActivity("Timer", null, now + 285_000) }
        render("15-timer-card", expand = true) { it.activity = RobotView.LiveActivity("Timer", null, now + 285_000) }
        render("16-live-compact") { it.activity = RobotView.LiveActivity("Swiggy", 0.6f, detail = "Arriving in 12 min · Ravi is on the way") }
        render("17-live-card", expand = true) { it.activity = RobotView.LiveActivity("Swiggy order", 0.6f, detail = "Arriving in 12 min · Ravi is on the way") }
    }
}
