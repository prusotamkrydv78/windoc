package com.notchy.app

import android.app.Activity
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import android.widget.Toast

/** Quick Settings tile: "Send clipboard to PC". */
class ClipTile : TileService() {
    override fun onStartListening() {
        qsTile?.apply {
            state = if (Link.connected) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
            label = "Clipboard → PC"
            updateTile()
        }
    }

    override fun onClick() {
        // Android only lets an app with focus read the clipboard, so open a see-through screen.
        val intent = Intent(this, ClipSendActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        if (Build.VERSION.SDK_INT >= 34) {
            startActivityAndCollapse(PendingIntent.getActivity(this, 0, intent, PendingIntent.FLAG_IMMUTABLE))
        } else {
            @Suppress("DEPRECATION")
            startActivityAndCollapse(intent)
        }
    }
}

/** Invisible: reads the clipboard once it has focus, sends it to the PC and closes. */
class ClipSendActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        LinkService.start(this)
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus) return
        val text = getSystemService(ClipboardManager::class.java).primaryClip
            ?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(this)?.toString().orEmpty()
        val message = when {
            text.isEmpty() -> "Clipboard is empty"
            sendText(text) -> "Sent to ${Link.pcName ?: "PC"}"
            else -> "PC not connected"
        }
        Toast.makeText(this, message, Toast.LENGTH_SHORT).show()
        finish()
    }

    companion object {
        /** Links open in the PC's browser; anything else lands on its clipboard. */
        fun sendText(text: String): Boolean {
            val trimmed = text.trim()
            return if (Regex("""^https?://\S+$""").matches(trimmed)) {
                Link.send("url") { put("url", trimmed) }
            } else {
                Link.send("clip") { put("text", text) }
            }
        }
    }
}

/** Brings the link and the robot back after a restart. */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED) return
        LinkService.start(context)
        val prefs = context.getSharedPreferences("app", Context.MODE_PRIVATE)
        if (prefs.getBoolean("robot", false) && android.provider.Settings.canDrawOverlays(context)) {
            context.startForegroundService(Intent(context, RobotService::class.java))
        }
    }
}
