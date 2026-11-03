package com.notchy.app

import android.app.Activity
import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import android.graphics.Color
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import kotlin.concurrent.thread

/**
 * "Share → Notchy": sends photos and files to the PC's Drop tab. Progress shows as a Live
 * Update, so on ColorOS 16 it appears in the Fluid Cloud bubble around the camera.
 */
class ShareActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Shared text or links go straight over Notchy Link: links open on the PC.
        val sharedText = intent.getStringExtra(Intent.EXTRA_TEXT)
        if (intent.action == Intent.ACTION_SEND && sharedText != null && intent.getParcelableExtra<Uri>(Intent.EXTRA_STREAM) == null) {
            val sent = ClipSendActivity.sendText(sharedText)
            android.widget.Toast.makeText(
                this,
                if (sent) "Sent to ${Link.pcName ?: "PC"}" else "Link your PC in Notchy first",
                android.widget.Toast.LENGTH_SHORT,
            ).show()
            finish()
            return
        }
        val uris = sharedUris(intent)
        val pad = (20 * resources.displayMetrics.density).toInt()
        val column = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(pad, pad, pad, pad)
            setBackgroundColor(Color.rgb(16, 16, 20))
        }
        val title = TextView(this).apply {
            text = if (uris.size == 1) "Send 1 file to your PC" else "Send ${uris.size} files to your PC"
            textSize = 20f
            setTextColor(Color.WHITE)
        }
        val help = TextView(this).apply {
            text = "On the PC open Notchy → Drop, scan the QR with the phone camera and copy the link (or copy it on the PC and paste here)."
            textSize = 13f
            setTextColor(Color.rgb(170, 170, 180))
            setPadding(0, pad / 2, 0, pad / 2)
        }
        val field = EditText(this).apply {
            hint = "http://192.168.x.x:port/…/"
            setText(PcLink.guess(this@ShareActivity) ?: "")
            setTextColor(Color.WHITE)
            setHintTextColor(Color.GRAY)
            isSingleLine = true
        }
        val error = TextView(this).apply {
            setTextColor(Color.rgb(255, 120, 110))
            textSize = 13f
        }
        val send = Button(this).apply {
            text = "Send"
            isAllCaps = false
            setOnClickListener {
                val link = PcLink.normalise(field.text.toString())
                if (link == null) {
                    error.text = "That doesn't look like a Notchy Drop link."
                } else if (uris.isEmpty()) {
                    error.text = "Nothing to send."
                } else {
                    PcLink.remember(this@ShareActivity, link)
                    sendAll(applicationContext, link, uris)
                    finish()
                }
            }
        }
        listOf(title, help, field, error, send).forEach(column::addView)
        setContentView(column)
    }

    @Suppress("DEPRECATION")
    private fun sharedUris(intent: Intent): List<Uri> = when (intent.action) {
        Intent.ACTION_SEND -> listOfNotNull(
            if (Build.VERSION.SDK_INT >= 33) {
                intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
            } else {
                intent.getParcelableExtra(Intent.EXTRA_STREAM)
            },
        )
        Intent.ACTION_SEND_MULTIPLE -> if (Build.VERSION.SDK_INT >= 33) {
            intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java)
        } else {
            intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM)
        }.orEmpty()
        else -> emptyList()
    }

    companion object {
        private const val ID = 77

        /** Uploads in the background, one file after another, with a live progress update. */
        fun sendAll(context: Context, link: String, uris: List<Uri>) {
            thread(name = "notchy-send") {
                try {
                    val files = uris.map { PcLink.describe(context, it) }
                    val total = files.sumOf { it.size }.coerceAtLeast(1)
                    var before = 0L
                    files.forEachIndexed { index, file ->
                        PcLink.upload(context, link, file) { sent ->
                            val done = before + sent
                            val percent = (done * 100 / total).toInt()
                            val label = if (files.size == 1) file.name else "${index + 1} of ${files.size} · ${file.name}"
                            LiveUpdates.showProgress(context, ID, "Sending to PC", "$label · $percent%", percent, 100)
                        }
                        before += file.size
                    }
                    finish(context, if (files.size == 1) "${files[0].name} sent to PC" else "${files.size} files sent to PC", null)
                } catch (error: Exception) {
                    finish(context, "Couldn't send to PC", error.message)
                }
            }
        }

        private fun finish(context: Context, title: String, detail: String?) {
            LiveUpdates.cancel(context, ID)
            LiveUpdates.ensureChannel(context)
            val notification = Notification.Builder(context, "live")
                .setSmallIcon(R.drawable.ic_notchy)
                .setContentTitle(title)
                .setContentText(detail ?: "Find it in Notchy's Drop tab on the PC.")
                .setAutoCancel(true)
                .build()
            context.getSystemService(NotificationManager::class.java).notify(ID + 1, notification)
        }
    }
}
