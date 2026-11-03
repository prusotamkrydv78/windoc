package com.notchy.app

import android.content.Context
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.Base64
import org.json.JSONObject
import java.io.DataInputStream
import java.io.DataOutputStream
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.Socket
import java.security.MessageDigest
import java.security.SecureRandom
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import javax.crypto.Cipher
import javax.crypto.spec.IvParameterSpec
import javax.crypto.spec.SecretKeySpec

/**
 * Notchy Link, phone side: one encrypted TCP connection to the PC app, kept alive by
 * [LinkService]. Frames are `[u32 length][12-byte nonce][ChaCha20-Poly1305 ciphertext]` of
 * one JSON message, matching `src/core/phone_link.rs` on the PC.
 */
object Link {
    private const val PREFS = "link"
    private const val DISCOVERY_PORT = 47821
    private const val MAX_FRAME = 1024 * 1024

    data class Pairing(val host: String, val port: Int, val key: ByteArray, val pcName: String)

    /** What the UI shows. Read on the main thread. */
    var connected = false
        private set
    var pcName: String? = null
        private set
    var pcBattery: Int = -1
        private set
    var pcCharging = false
        private set
    var pcAway = false
        private set
    var pcMedia: Triple<String, String, Boolean>? = null
        private set
    /** When the PC's Focus session ends (epoch ms), or 0. */
    var focusUntil = 0L
        private set
    /** The PC's camera or mic is in use. */
    var pcMeeting = false
        private set

    /** The last recap from the PC: `(text, model or null)`. */
    var recap: Pair<String, String?>? = null
        private set
    /** The last Memory search results: `(kind, title, detail, app, at)`. */
    var memory: List<MemoryItem>? = null
        private set

    data class MemoryItem(val kind: String, val title: String, val detail: String, val app: String, val at: Long)

    /** Forgets old answers before asking again. */
    fun clearAnswers() {
        recap = null
        memory = null
        answer = null
    }

    /** The answer to the last question asked with `ask`. */
    var answer: String? = null
        private set
    /** The PC's screen time today, in minutes, or -1. */
    var pcScreenMinutes = -1L
        private set

    /** The PC's recent clipboard texts, after asking with `clip_history`. */
    var pcHistory: List<String>? = null
        private set

    private val main = Handler(Looper.getMainLooper())
    private val listeners = mutableListOf<() -> Unit>()
    private val outbox = LinkedBlockingQueue<String>()
    private val random = SecureRandom()

    @Volatile private var socket: Socket? = null
    @Volatile private var running = false
    private var worker: Thread? = null

    /** Messages from the PC, delivered on the main thread. */
    var onMessage: ((JSONObject) -> Unit)? = null

    fun addListener(listener: () -> Unit) { listeners += listener }
    fun removeListener(listener: () -> Unit) { listeners -= listener }
    private fun notifyListeners() = main.post { listeners.toList().forEach { it() } }

    // ---- Pairing -------------------------------------------------------------------------

    fun pairing(context: Context): Pairing? {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val host = prefs.getString("host", null) ?: return null
        val key = prefs.getString("key", null) ?: return null
        val bytes = runCatching { Base64.decode(key, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING) }
            .getOrNull()?.takeIf { it.size == 32 } ?: return null
        return Pairing(host, prefs.getInt("port", 47820), bytes, prefs.getString("name", "PC") ?: "PC")
    }

    fun isPaired(context: Context) = pairing(context) != null

    /** Saves a `notchy://pair?h=…&p=…&k=…&n=…` code. `false` when it is not one. */
    fun pair(context: Context, code: String): Boolean {
        val uri = runCatching { Uri.parse(code.trim()) }.getOrNull() ?: return false
        if (uri.scheme != "notchy" || uri.host != "pair") return false
        val host = uri.getQueryParameter("h") ?: return false
        val port = uri.getQueryParameter("p")?.toIntOrNull() ?: return false
        val key = uri.getQueryParameter("k") ?: return false
        val decoded = runCatching { Base64.decode(key, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING) }.getOrNull()
        if (decoded?.size != 32) return false
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
            .putString("host", host)
            .putInt("port", port)
            .putString("key", key)
            .putString("name", uri.getQueryParameter("n") ?: "PC")
            .apply()
        restart()
        return true
    }

    fun unpair(context: Context) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().clear().apply()
        stop()
    }

    private fun saveAddress(context: Context, host: String, port: Int) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
            .putString("host", host).putInt("port", port).apply()
    }

    fun deviceName(context: Context): String =
        Settings.Global.getString(context.contentResolver, "device_name")
            ?: "${Build.MANUFACTURER} ${Build.MODEL}"

    // ---- Sending -------------------------------------------------------------------------

    /** Queues a message; dropped when not connected. */
    fun send(type: String, build: JSONObject.() -> Unit = {}): Boolean {
        if (!connected) return false
        outbox.offer(JSONObject().apply { put("t", type); build() }.toString())
        return true
    }

    /** Messages waiting to go out, so big transfers can wait instead of filling memory. */
    fun backlog(): Int = outbox.size

    /**
     * Sends a file in 192 KB chunks. Call off the main thread. [progress] gets bytes sent.
     * Throws when the link drops.
     */
    fun sendFile(context: Context, file: PcLink.FileInfo, progress: (Long) -> Unit) {
        val id = System.nanoTime()
        if (!send("file_start") { put("id", id); put("name", file.name); put("size", file.size) }) {
            error("PC not connected")
        }
        val input = context.contentResolver.openInputStream(file.uri) ?: error("Cannot open ${file.name}")
        input.use { stream ->
            val buffer = ByteArray(192 * 1024)
            var sent = 0L
            while (sent < file.size) {
                val read = stream.read(buffer, 0, minOf(buffer.size.toLong(), file.size - sent).toInt())
                if (read < 0) break
                while (backlog() > 6) {
                    if (!connected) error("Link dropped")
                    Thread.sleep(20)
                }
                val data = Base64.encodeToString(buffer, 0, read, Base64.NO_WRAP)
                if (!send("file_chunk") { put("id", id); put("data", data) }) error("Link dropped")
                sent += read
                progress(sent)
            }
        }
        send("file_end") { put("id", id) }
    }

    // ---- Connection ----------------------------------------------------------------------

    private var appContext: Context? = null

    fun start(context: Context) {
        appContext = context.applicationContext
        if (running) return
        running = true
        worker = Thread({ loop() }, "notchy-link").apply { isDaemon = true; start() }
    }

    fun stop() {
        running = false
        runCatching { socket?.close() }
        worker?.interrupt()
        worker = null
        setConnected(false)
    }

    /** Drops the current connection so the loop reconnects with fresh settings. */
    private fun restart() {
        runCatching { socket?.close() }
    }

    private fun setConnected(value: Boolean) {
        connected = value
        if (!value) {
            pcMedia = null
            outbox.clear()
        }
        notifyListeners()
    }

    private fun loop() {
        var backoff = 1000L
        while (running) {
            val context = appContext ?: break
            val pairing = pairing(context)
            if (pairing == null) {
                sleep(3000)
                continue
            }
            val wasUp = runCatching { session(context, pairing.host, pairing.port, pairing.key) }
                .let { connected }
            setConnected(false)
            if (!running) break
            if (wasUp) backoff = 1000
            // The PC may have a new address: ask the network.
            discover(pairing.key)?.let { (host, port) ->
                if (host != pairing.host || port != pairing.port) {
                    saveAddress(context, host, port)
                    backoff = 500
                }
            }
            sleep(backoff)
            backoff = (backoff * 2).coerceAtMost(15_000)
        }
    }

    private fun sleep(millis: Long) {
        try { Thread.sleep(millis) } catch (_: InterruptedException) {}
    }

    private fun session(context: Context, host: String, port: Int, key: ByteArray) {
        val socket = Socket()
        this.socket = socket
        socket.use {
            socket.connect(InetSocketAddress(host, port), 4000)
            socket.tcpNoDelay = true
            socket.soTimeout = 70_000
            val input = DataInputStream(socket.getInputStream().buffered())
            val output = DataOutputStream(socket.getOutputStream().buffered())

            val challenge = read(input, key)
            if (challenge.optString("t") != "challenge") error("no challenge")
            pcName = challenge.optString("name", pcName ?: "PC")
            write(output, key, JSONObject().apply {
                put("t", "hello")
                put("c", challenge.getString("c"))
                put("name", deviceName(context))
            }.toString())
            outbox.clear()
            setConnected(true)

            val writer = Thread({
                try {
                    while (!socket.isClosed) {
                        val message = outbox.poll(25, TimeUnit.SECONDS)
                            ?: JSONObject().put("t", "ping").toString()
                        write(output, key, message)
                    }
                } catch (_: Exception) {
                    runCatching { socket.close() }
                }
            }, "notchy-link-writer").apply { isDaemon = true; start() }

            try {
                while (running) {
                    val message = read(input, key)
                    handle(message)
                }
            } finally {
                writer.interrupt()
            }
        }
    }

    private fun handle(message: JSONObject) {
        when (message.optString("t")) {
            "welcome" -> {
                pcName = message.optString("name", pcName ?: "PC")
                notifyListeners()
            }
            "pc" -> {
                pcAway = message.optBoolean("away")
                pcBattery = if (message.isNull("battery")) -1 else message.optInt("battery", -1)
                pcCharging = message.optBoolean("charging")
                notifyListeners()
            }
            "media" -> {
                val title = message.optString("title")
                pcMedia = if (title.isEmpty()) null
                else Triple(title, message.optString("artist"), message.optBoolean("playing"))
                notifyListeners()
            }
            "focus" -> {
                focusUntil = message.optLong("until")
                notifyListeners()
            }
            "meeting" -> {
                pcMeeting = message.optBoolean("on")
                notifyListeners()
            }
            "robot" -> {
                appContext?.let { RobotLife.save(it, message) }
                notifyListeners()
            }
            "answer" -> {
                answer = message.optString("text")
                notifyListeners()
            }
            "pc_screen_time" -> {
                pcScreenMinutes = if (message.isNull("minutes")) -1 else message.optLong("minutes", -1)
                notifyListeners()
            }
            "recap" -> {
                recap = message.optString("text") to message.optString("model").takeIf { it.isNotEmpty() && it != "null" }
                notifyListeners()
            }
            "memory" -> {
                val items = message.optJSONArray("items")
                memory = (0 until (items?.length() ?: 0)).map {
                    val item = items!!.getJSONObject(it)
                    MemoryItem(item.optString("kind"), item.optString("title"), item.optString("detail"), item.optString("app"), item.optLong("at"))
                }
                notifyListeners()
            }
            "clip_history" -> {
                val items = message.optJSONArray("items")
                pcHistory = (0 until (items?.length() ?: 0)).map { items!!.getString(it) }
                notifyListeners()
            }
            "pong" -> {}
        }
        main.post { onMessage?.invoke(message) }
    }

    // ---- Frames --------------------------------------------------------------------------

    private fun cipher(mode: Int, key: ByteArray, nonce: ByteArray): Cipher =
        Cipher.getInstance("ChaCha20-Poly1305").apply {
            init(mode, SecretKeySpec(key, "ChaCha20"), IvParameterSpec(nonce))
        }

    @Synchronized
    private fun write(output: DataOutputStream, key: ByteArray, text: String) {
        val nonce = ByteArray(12).also(random::nextBytes)
        val sealed = cipher(Cipher.ENCRYPT_MODE, key, nonce).doFinal(text.toByteArray(Charsets.UTF_8))
        output.writeInt(nonce.size + sealed.size)
        output.write(nonce)
        output.write(sealed)
        output.flush()
    }

    private fun read(input: DataInputStream, key: ByteArray): JSONObject {
        val length = input.readInt()
        if (length < 28 || length > MAX_FRAME) error("bad frame")
        val frame = ByteArray(length)
        input.readFully(frame)
        val plain = cipher(Cipher.DECRYPT_MODE, key, frame.copyOfRange(0, 12))
            .doFinal(frame, 12, length - 12)
        return JSONObject(String(plain, Charsets.UTF_8))
    }

    // ---- Discovery -----------------------------------------------------------------------

    private fun pairId(key: ByteArray): String =
        MessageDigest.getInstance("SHA-256").digest(key).take(8).joinToString("") { "%02x".format(it) }

    /** Broadcasts for the paired PC; returns its `(address, port)` if it answers. */
    private fun discover(key: ByteArray): Pair<String, Int>? = runCatching {
        DatagramSocket().use { socket ->
            socket.broadcast = true
            socket.soTimeout = 1500
            val ask = "NOTCHY?${pairId(key)}".toByteArray()
            socket.send(DatagramPacket(ask, ask.size, InetAddress.getByName("255.255.255.255"), DISCOVERY_PORT))
            val buffer = ByteArray(64)
            val reply = DatagramPacket(buffer, buffer.size)
            socket.receive(reply)
            val text = String(buffer, 0, reply.length)
            val port = text.removePrefix("NOTCHY!").trim().toIntOrNull() ?: return@use null
            reply.address.hostAddress?.let { it to port }
        }
    }.getOrNull()
}
