package com.notchy.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Handler
import android.os.Looper

/**
 * Android 16 "Live Updates": ongoing notifications the system may promote to the status bar
 * chip. ColorOS 16 shows promoted ones in Fluid Cloud, OPPO's own notch, so this is how
 * Notchy gets into the real notch without any hacks.
 *
 * Promotion rules (Android 16): the notification is ongoing, has a title, is not colorized,
 * uses a promotable style (ProgressStyle here) and asks for promotion. The user can turn it
 * off per app in the notification settings.
 */
object LiveUpdates {
    private const val CHANNEL = "live"
    private const val DEMO_ID = 42

    /** `Notification.EXTRA_REQUEST_PROMOTED_ONGOING`, spelled out so older SDKs compile. */
    private const val EXTRA_REQUEST_PROMOTED_ONGOING = "android.requestPromotedOngoing"
    private const val BAKLAVA = 36

    private val handler = Handler(Looper.getMainLooper())
    private var demoStep = 0

    fun ensureChannel(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java)
        // Live Updates need a channel that is not silent-minimum.
        val channel = NotificationChannel(CHANNEL, "Live activity", NotificationManager.IMPORTANCE_DEFAULT)
        channel.setSound(null, null)
        channel.enableVibration(false)
        manager.createNotificationChannel(channel)
    }

    /** Posts (or updates) one ongoing progress update. */
    fun showProgress(context: Context, id: Int, title: String, text: String, progress: Int, max: Int) {
        ensureChannel(context)
        val open = PendingIntent.getActivity(
            context, 0,
            Intent(context, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val builder = Notification.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notchy)
            .setContentTitle(title)
            .setContentText(text)
            .setContentIntent(open)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setCategory(Notification.CATEGORY_PROGRESS)
        if (Build.VERSION.SDK_INT >= BAKLAVA) {
            builder.setStyle(
                Notification.ProgressStyle()
                    .setProgress(progress)
                    .setProgressSegments(listOf(Notification.ProgressStyle.Segment(max)))
                    .setStyledByProgress(true),
            )
        } else {
            builder.setProgress(max, progress, false)
        }
        val notification = builder.build()
        notification.extras.putBoolean(EXTRA_REQUEST_PROMOTED_ONGOING, true)
        context.getSystemService(NotificationManager::class.java).notify(id, notification)
    }

    fun cancel(context: Context, id: Int) {
        context.getSystemService(NotificationManager::class.java).cancel(id)
    }

    /**
     * A 30-second fake "file from PC" transfer. If it shows in the Fluid Cloud bubble around
     * the camera, Notchy's live events can live in OPPO's notch.
     */
    fun startDemo(context: Context) {
        val app = context.applicationContext
        handler.removeCallbacksAndMessages(null)
        demoStep = 0
        val tick = object : Runnable {
            override fun run() {
                if (demoStep > 30) {
                    showDone(app)
                    return
                }
                showProgress(app, DEMO_ID, "Receiving from PC", "photo.jpg · ${demoStep * 100 / 30}%", demoStep, 30)
                demoStep++
                handler.postDelayed(this, 1000)
            }
        }
        handler.post(tick)
    }

    private fun showDone(context: Context) {
        cancel(context, DEMO_ID)
        val done = Notification.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notchy)
            .setContentTitle("photo.jpg received")
            .setContentText("Notchy test finished")
            .setAutoCancel(true)
            .build()
        context.getSystemService(NotificationManager::class.java).notify(DEMO_ID + 1, done)
    }
}
