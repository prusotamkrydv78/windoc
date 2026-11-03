package com.notchy.app

import android.app.Activity
import android.app.AlarmManager
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraManager
import android.os.Bundle
import android.view.Gravity
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.Toast

/**
 * Long-press the camera robot: a small panel with a timer (shown as a Live Update, so it
 * sits in Fluid Cloud), the flashlight, a quick note, and "Throw to PC".
 */
class QuickPanelActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val column = column(12).apply {
            setPadding(dp(20), dp(20), dp(20), dp(20))
            background = rounded(Ui.CARD, dp(24).toFloat())
        }
        column.addView(label("Notchy", 20f, Ui.TEXT, bold = true))

        column.addView(sectionTitle("Timer"))
        val timers = row(8)
        for (minutes in listOf(1, 5, 10, 25)) {
            timers.addView(pill("$minutes m") {
                Timers.start(this, minutes)
                finish()
            }, weighted())
        }
        column.addView(timers, matchWidth())
        if (Timers.running(this)) {
            column.addView(pill("Cancel timer") {
                Timers.cancel(this)
                finish()
            }, matchWidth())
        }

        column.addView(sectionTitle("Quick"))
        val quick = row(8)
        quick.addView(pill(if (Torch.on) "Light off" else "Flashlight") {
            Torch.toggle(this)
            finish()
        }, weighted())
        quick.addView(pill("Throw to PC", primary = Link.connected) {
            RobotService.instance?.throwToPc()
            finish()
        }, weighted())
        column.addView(quick, matchWidth())

        column.addView(sectionTitle("Note"))
        val note = EditText(this).apply {
            hint = "Jot something down…"
            setTextColor(Ui.TEXT)
            setHintTextColor(Ui.FAINT)
            background = rounded(Ui.CARD2, dp(16).toFloat())
            setPadding(dp(14), dp(12), dp(14), dp(12))
            minLines = 2
        }
        column.addView(note, matchWidth())
        column.addView(pill("Save note", primary = true) {
            val text = note.text.toString().trim()
            if (text.isNotEmpty()) {
                Notes.add(this, text)
                val toPc = Link.send("note") { put("text", text) }
                Toast.makeText(this, if (toPc) "Saved here and on ${Link.pcName}" else "Saved", Toast.LENGTH_SHORT).show()
                RobotService.instance?.say("Noted!")
            }
            finish()
        }, matchWidth())
        val recent = Notes.all(this).takeLast(3).reversed()
        if (recent.isNotEmpty()) {
            column.addView(sectionTitle("Recent notes"))
            recent.forEach { column.addView(label("• ${it.take(120)}", 13f, Ui.DIM)) }
        }

        setContentView(ScrollView(this).apply {
            setPadding(dp(16), dp(16), dp(16), dp(16))
            addView(column)
        }, LinearLayout.LayoutParams(dp(340), LinearLayout.LayoutParams.WRAP_CONTENT).apply { gravity = Gravity.CENTER })
    }
}

/** Phone timers: a Live Update countdown, and an alarm when it ends. */
object Timers {
    private const val LIVE_ID = 50
    private const val DONE_ID = 51
    private const val PREFS = "timers"

    fun running(context: Context) =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getLong("until", 0) > System.currentTimeMillis()

    fun start(context: Context, minutes: Int) {
        val until = System.currentTimeMillis() + minutes * 60_000L
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putLong("until", until).apply()
        LiveUpdates.ensureChannel(context)
        val notification = Notification.Builder(context, "live")
            .setSmallIcon(R.drawable.ic_notchy)
            .setContentTitle("$minutes minute timer")
            .setContentText("The robot will tell you when it's done")
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setWhen(until)
            .setShowWhen(true)
            .setUsesChronometer(true)
            .setChronometerCountDown(true)
            .setCategory(Notification.CATEGORY_STOPWATCH)
            .build()
        notification.extras.putBoolean("android.requestPromotedOngoing", true)
        context.getSystemService(NotificationManager::class.java).notify(LIVE_ID, notification)
        context.getSystemService(AlarmManager::class.java)
            .setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, until, done(context))
        RobotService.instance?.say("Timer: $minutes min")
    }

    fun cancel(context: Context) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().remove("until").apply()
        context.getSystemService(AlarmManager::class.java).cancel(done(context))
        context.getSystemService(NotificationManager::class.java).cancel(LIVE_ID)
    }

    private fun done(context: Context) = PendingIntent.getBroadcast(
        context, 9, Intent(context, TimerDoneReceiver::class.java), PendingIntent.FLAG_IMMUTABLE,
    )

    fun finished(context: Context) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().remove("until").apply()
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.cancel(LIVE_ID)
        manager.createNotificationChannel(NotificationChannel("timer_done", "Timer done", NotificationManager.IMPORTANCE_HIGH))
        manager.notify(
            DONE_ID,
            Notification.Builder(context, "timer_done")
                .setSmallIcon(R.drawable.ic_notchy)
                .setContentTitle("Time's up!")
                .setContentText("Your Notchy timer finished")
                .setAutoCancel(true)
                .setCategory(Notification.CATEGORY_ALARM)
                .build(),
        )
        RobotService.instance?.say("Time's up! ⏰")
    }
}

class TimerDoneReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) = Timers.finished(context)
}

/** The flashlight, for the quick panel. */
object Torch {
    var on = false
        private set

    fun toggle(context: Context) {
        val cameras = context.getSystemService(CameraManager::class.java)
        val id = cameras.cameraIdList.firstOrNull {
            cameras.getCameraCharacteristics(it).get(CameraCharacteristics.FLASH_INFO_AVAILABLE) == true
        } ?: return
        runCatching {
            cameras.setTorchMode(id, !on)
            on = !on
            RobotService.instance?.say(if (on) "Lights on 🔦" else "Lights off")
        }
    }
}

/** Quick notes kept on the phone (and copied to the PC's Memory when linked). */
object Notes {
    private const val PREFS = "notes"

    fun add(context: Context, text: String) {
        val all = all(context) + text
        val array = org.json.JSONArray()
        all.takeLast(200).forEach { array.put(it) }
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putString("all", array.toString()).apply()
    }

    fun all(context: Context): List<String> {
        val array = runCatching {
            org.json.JSONArray(context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString("all", "[]"))
        }.getOrDefault(org.json.JSONArray())
        return (0 until array.length()).map { array.getString(it) }
    }
}
