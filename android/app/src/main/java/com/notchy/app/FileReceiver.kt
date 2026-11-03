package com.notchy.app

import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Environment
import android.provider.MediaStore
import android.util.Base64
import android.webkit.MimeTypeMap
import org.json.JSONObject
import java.io.OutputStream

/**
 * Files from the PC (Phone tab → File). Saved to Downloads/Notchy through MediaStore, with
 * a Live Update while they arrive and a tap-to-open notification when done.
 */
object FileReceiver {
    private const val ID = 30

    private class Incoming(val id: Long, val name: String, val size: Long, val uri: Uri, val out: OutputStream) {
        var written = 0L
        var lastPost = 0L
    }

    private var current: Incoming? = null

    /** `true` when [message] was a file message. Runs on the main thread. */
    fun handle(context: Context, message: JSONObject): Boolean {
        when (message.optString("t")) {
            "file_start" -> start(context, message)
            "file_chunk" -> chunk(context, message)
            "file_end" -> end(context, message)
            else -> return false
        }
        return true
    }

    private fun start(context: Context, message: JSONObject) {
        abort(context)
        val name = message.optString("name", "file").replace(Regex("""[\\/:*?"<>|]"""), "_")
        val extension = name.substringAfterLast('.', "").lowercase()
        val mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension) ?: "application/octet-stream"
        val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, name)
            put(MediaStore.Downloads.MIME_TYPE, mime)
            put(MediaStore.Downloads.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS + "/Notchy")
            put(MediaStore.Downloads.IS_PENDING, 1)
        }
        val resolver = context.contentResolver
        val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values) ?: return
        val out = resolver.openOutputStream(uri) ?: return
        current = Incoming(message.optLong("id"), name, message.optLong("size"), uri, out.buffered(256 * 1024))
        RobotService.instance?.say("Catching a file")
    }

    private fun chunk(context: Context, message: JSONObject) {
        val file = current ?: return
        if (message.optLong("id") != file.id) return
        val bytes = runCatching { Base64.decode(message.optString("data"), Base64.NO_WRAP) }.getOrNull() ?: return
        runCatching { file.out.write(bytes) }.onFailure { abort(context); return }
        file.written += bytes.size
        val now = System.currentTimeMillis()
        if (now - file.lastPost > 400) {
            file.lastPost = now
            val percent = (file.written * 100 / file.size.coerceAtLeast(1)).toInt()
            LiveUpdates.showProgress(context, ID, "From ${Link.pcName ?: "PC"}", "${file.name} · $percent%", percent, 100)
        }
    }

    private fun end(context: Context, message: JSONObject) {
        val file = current ?: return
        if (message.optLong("id") != file.id) return
        current = null
        runCatching { file.out.close() }
        LiveUpdates.cancel(context, ID)
        if (file.written != file.size) {
            context.contentResolver.delete(file.uri, null, null)
            return
        }
        context.contentResolver.update(file.uri, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null)
        val open = PendingIntent.getActivity(
            context, 3,
            Intent(Intent.ACTION_VIEW).setDataAndType(file.uri, context.contentResolver.getType(file.uri))
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        LiveUpdates.ensureChannel(context)
        val done = Notification.Builder(context, "live")
            .setSmallIcon(R.drawable.ic_notchy)
            .setContentTitle("${file.name} received")
            .setContentText("Saved in Downloads/Notchy · tap to open")
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()
        context.getSystemService(NotificationManager::class.java).notify(ID + 1, done)
        RobotService.instance?.say("Got it!")
        Stats.add(context, "files")
    }

    private fun abort(context: Context) {
        val file = current ?: return
        current = null
        runCatching { file.out.close() }
        context.contentResolver.delete(file.uri, null, null)
        LiveUpdates.cancel(context, ID)
    }
}

/** Today's numbers for the Home page: files, clips and alerts that crossed the link. */
object Stats {
    private fun key() = java.text.SimpleDateFormat("yyyy-MM-dd", java.util.Locale.US).format(java.util.Date())

    fun add(context: Context, what: String) {
        val prefs = context.getSharedPreferences("stats", Context.MODE_PRIVATE)
        val day = key()
        if (prefs.getString("day", null) != day) prefs.edit().clear().putString("day", day).apply()
        prefs.edit().putInt(what, prefs.getInt(what, 0) + 1).apply()
    }

    fun get(context: Context, what: String): Int {
        val prefs = context.getSharedPreferences("stats", Context.MODE_PRIVATE)
        return if (prefs.getString("day", null) == key()) prefs.getInt(what, 0) else 0
    }
}
