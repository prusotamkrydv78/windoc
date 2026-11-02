package com.notchy.app

import android.content.ClipboardManager
import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL

/**
 * Talks to the PC's Drop server (Notchy's Drop tab). A session link looks like
 * `http://192.168.1.5:51234/<token>/` and lives ten minutes; the server takes one file per
 * multipart POST to `<link>upload` and needs an exact Content-Length.
 */
object PcLink {
    private const val PREFS = "pc_link"
    private const val KEY_LINK = "link"
    private val LINK = Regex("""^http://[0-9.]+:[0-9]+/[A-Za-z0-9_-]+/$""")

    /** A Drop link in the form above, with a trailing slash added if missing. */
    fun normalise(text: String): String? {
        val trimmed = text.trim().let { if (it.endsWith("/")) it else "$it/" }
        return trimmed.takeIf { LINK.matches(it) }
    }

    /** The link on the clipboard, else the last one used. */
    fun guess(context: Context): String? {
        val clipboard = context.getSystemService(ClipboardManager::class.java)
        val clip = runCatching { clipboard.primaryClip?.getItemAt(0)?.text?.toString() }.getOrNull()
        return clip?.let(::normalise) ?: saved(context)
    }

    fun saved(context: Context): String? =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(KEY_LINK, null)

    fun remember(context: Context, link: String) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putString(KEY_LINK, link).apply()
    }

    data class FileInfo(val uri: Uri, val name: String, val size: Long)

    fun describe(context: Context, uri: Uri): FileInfo {
        var name = uri.lastPathSegment ?: "file"
        var size = -1L
        context.contentResolver.query(uri, null, null, null, null)?.use { cursor ->
            if (cursor.moveToFirst()) {
                val nameColumn = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                val sizeColumn = cursor.getColumnIndex(OpenableColumns.SIZE)
                if (nameColumn >= 0) cursor.getString(nameColumn)?.let { name = it }
                if (sizeColumn >= 0 && !cursor.isNull(sizeColumn)) size = cursor.getLong(sizeColumn)
            }
        }
        if (size < 0) {
            // Unknown size: count the bytes once, since the server needs Content-Length.
            size = context.contentResolver.openInputStream(uri)?.use { stream ->
                var total = 0L
                val buffer = ByteArray(64 * 1024)
                while (true) {
                    val read = stream.read(buffer)
                    if (read < 0) break
                    total += read
                }
                total
            } ?: 0L
        }
        return FileInfo(uri, name.replace("\"", "'"), size)
    }

    /**
     * Uploads one file. [progress] gets bytes sent so far. Throws on failure. Call off the
     * main thread.
     */
    fun upload(context: Context, link: String, file: FileInfo, progress: (Long) -> Unit) {
        val boundary = "NotchyBoundary" + System.nanoTime()
        val head = ("--$boundary\r\n" +
            "Content-Disposition: form-data; name=\"file\"; filename=\"${file.name}\"\r\n" +
            "Content-Type: application/octet-stream\r\n\r\n").toByteArray(Charsets.UTF_8)
        val tail = "\r\n--$boundary--\r\n".toByteArray(Charsets.UTF_8)
        val length = head.size + file.size + tail.size

        val connection = URL(link + "upload").openConnection() as HttpURLConnection
        try {
            connection.requestMethod = "POST"
            connection.doOutput = true
            connection.connectTimeout = 5000
            connection.readTimeout = 60000
            connection.setFixedLengthStreamingMode(length)
            connection.setRequestProperty("Content-Type", "multipart/form-data; boundary=$boundary")
            connection.outputStream.use { out ->
                out.write(head)
                val input = context.contentResolver.openInputStream(file.uri)
                    ?: throw IOException("Cannot open ${file.name}")
                input.use { stream ->
                    val buffer = ByteArray(64 * 1024)
                    var sent = 0L
                    var lastReport = 0L
                    while (sent < file.size) {
                        val read = stream.read(buffer, 0, minOf(buffer.size.toLong(), file.size - sent).toInt())
                        if (read < 0) throw IOException("${file.name} ended early")
                        out.write(buffer, 0, read)
                        sent += read
                        val now = System.currentTimeMillis()
                        if (now - lastReport > 400) {
                            progress(sent)
                            lastReport = now
                        }
                    }
                    progress(sent)
                }
                out.write(tail)
            }
            val code = connection.responseCode
            if (code == 404) throw IOException("The PC link has expired. Open Drop on the PC and copy the new link.")
            if (code != 200) throw IOException("The PC refused the file (HTTP $code).")
        } finally {
            connection.disconnect()
        }
    }
}
