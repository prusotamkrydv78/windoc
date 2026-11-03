package com.notchy.app

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * The phone's own memory, so Notchy is useful without the PC:
 * - every mirrored alert is kept for a day (codes never), for "What did I miss?" on the phone;
 * - alerts that arrive while the PC is away are queued and handed over when it reconnects,
 *   so the PC's timeline and recap cover the gap;
 * - an optional AI key (OpenRouter, OpenAI or any OpenAI-compatible URL) for phone recaps,
 *   kept encrypted with an Android Keystore key.
 */
object PhoneMemory {
    private const val PREFS = "phone_memory"
    private const val KEEP_MS = 24 * 3_600_000L
    private const val MAX_LOG = 500
    private const val MAX_QUEUE = 300

    data class Alert(val at: Long, val app: String, val title: String, val text: String)

    private fun prefs(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    private fun read(context: Context, key: String): MutableList<Alert> {
        val array = runCatching { JSONArray(prefs(context).getString(key, "[]")) }.getOrDefault(JSONArray())
        return (0 until array.length()).mapNotNull { index ->
            array.optJSONObject(index)?.let {
                Alert(it.optLong("at"), it.optString("app"), it.optString("title"), it.optString("text"))
            }
        }.toMutableList()
    }

    private fun write(context: Context, key: String, alerts: List<Alert>) {
        val array = JSONArray()
        alerts.forEach {
            array.put(JSONObject().put("at", it.at).put("app", it.app).put("title", it.title).put("text", it.text))
        }
        prefs(context).edit().putString(key, array.toString()).apply()
    }

    /** Remembers an alert; queues it for the PC when [sent] is false. */
    @Synchronized
    fun record(context: Context, app: String, title: String, text: String, isCode: Boolean, sent: Boolean) {
        val now = System.currentTimeMillis()
        val alert = Alert(now, app, title.take(120), if (isCode) "A one-time code" else text.take(400))
        val log = read(context, "log").filter { it.at > now - KEEP_MS }.toMutableList()
        log += alert
        write(context, "log", log.takeLast(MAX_LOG))
        if (!sent && !isCode) {
            val queue = read(context, "queue")
            queue += alert
            write(context, "queue", queue.takeLast(MAX_QUEUE))
        }
    }

    /** Sends everything queued while the PC was away. Call once the link is up. */
    @Synchronized
    fun flush(context: Context) {
        val queue = read(context, "queue")
        if (queue.isEmpty()) return
        for (alert in queue) {
            Link.send("notif") {
                put("app", alert.app)
                put("title", alert.title)
                put("text", alert.text)
                put("at", alert.at)
                put("queued", true)
            }
        }
        write(context, "queue", emptyList())
    }

    fun queued(context: Context): Int = read(context, "queue").size

    fun since(context: Context, ms: Long): List<Alert> = read(context, "log").filter { it.at >= ms }

    // ---- Recap on the phone --------------------------------------------------------------

    /** Alerts grouped by app and sender, like the PC's built-in summary. */
    fun summarise(alerts: List<Alert>): String {
        if (alerts.isEmpty()) return "Nothing new in the last few hours."
        val apps = alerts.groupBy { it.app.ifEmpty { "Phone" } }.entries.sortedByDescending { it.value.size }
        val parts = apps.take(4).map { (app, items) ->
            val senders = items.filter { it.title.isNotEmpty() }.groupingBy { it.title }.eachCount()
                .entries.sortedByDescending { it.value }.take(3)
                .joinToString(", ") { if (it.value > 1) "${it.key} ×${it.value}" else it.key }
            if (senders.isEmpty()) "$app: ${items.size}" else "$app: ${items.size} ($senders)"
        }
        val more = if (apps.size > 4) " · +${apps.size - 4} more apps" else ""
        return "${alerts.size} alert${if (alerts.size == 1) "" else "s"} — ${parts.joinToString(" · ")}$more"
    }

    /** Recap of the last [hours] hours: the AI when a key is set, else the summary. Blocking. */
    fun recap(context: Context, hours: Int): Pair<String, String?> {
        val alerts = since(context, System.currentTimeMillis() - hours * 3_600_000L)
        val settings = ai(context)
        if (settings.key.isEmpty() || alerts.size < 2) return summarise(alerts) to null
        val lines = alerts.takeLast(60).joinToString("\n") { "- ${it.app} · ${it.title}: ${it.text.replace('\n', ' ')}" }
        val prompt = "Summarise these phone notifications in at most three short sentences. " +
            "Say who wants something and anything urgent. No lists, no greetings.\n\n$lines"
        return runCatching { chat(settings, prompt) to settings.model }
            .getOrElse { summarise(alerts) to null }
    }

    // ---- AI settings ---------------------------------------------------------------------

    data class Ai(val baseUrl: String, val key: String, val model: String)

    const val OPENROUTER = "https://openrouter.ai/api/v1"
    const val OPENAI = "https://api.openai.com/v1"

    fun ai(context: Context): Ai {
        val prefs = prefs(context)
        return Ai(
            prefs.getString("ai_url", OPENROUTER) ?: OPENROUTER,
            prefs.getString("ai_key", null)?.let(::decrypt).orEmpty(),
            prefs.getString("ai_model", "openai/gpt-4o-mini") ?: "openai/gpt-4o-mini",
        )
    }

    fun saveAi(context: Context, ai: Ai) {
        prefs(context).edit()
            .putString("ai_url", ai.baseUrl.trim().trimEnd('/'))
            .putString("ai_key", if (ai.key.isBlank()) null else encrypt(ai.key.trim()))
            .putString("ai_model", ai.model.trim())
            .apply()
    }

    /** One OpenAI-style chat completion. Throws with a short message. Blocking. */
    fun chat(ai: Ai, prompt: String): String {
        val connection = URL("${ai.baseUrl.trimEnd('/')}/chat/completions").openConnection() as HttpURLConnection
        try {
            connection.requestMethod = "POST"
            connection.doOutput = true
            connection.connectTimeout = 10_000
            connection.readTimeout = 45_000
            connection.setRequestProperty("Authorization", "Bearer ${ai.key}")
            connection.setRequestProperty("Content-Type", "application/json")
            connection.setRequestProperty("HTTP-Referer", "https://notchy.local")
            connection.setRequestProperty("X-Title", "Notchy")
            val body = JSONObject()
                .put("model", ai.model)
                .put("max_tokens", 200)
                .put("temperature", 0.3)
                .put("messages", JSONArray()
                    .put(JSONObject().put("role", "system").put("content", "You are Notchy, a friendly little robot living around a phone's camera. Be brief and warm."))
                    .put(JSONObject().put("role", "user").put("content", prompt)))
            connection.outputStream.use { it.write(body.toString().toByteArray()) }
            val code = connection.responseCode
            val stream = if (code in 200..299) connection.inputStream else connection.errorStream
            val reply = JSONObject(stream?.bufferedReader()?.readText() ?: "{}")
            if (code !in 200..299) error(reply.optJSONObject("error")?.optString("message") ?: "AI error $code")
            return reply.getJSONArray("choices").getJSONObject(0).getJSONObject("message").getString("content").trim()
        } finally {
            connection.disconnect()
        }
    }

    // ---- Keystore ------------------------------------------------------------------------

    private const val ALIAS = "notchy_ai"

    private fun key(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").apply {
            init(
                KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                    .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                    .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                    .build(),
            )
        }.generateKey()
    }

    private fun encrypt(plain: String): String {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, key()) }
        return Base64.encodeToString(cipher.iv + cipher.doFinal(plain.toByteArray()), Base64.NO_WRAP)
    }

    private fun decrypt(encoded: String): String? = runCatching {
        val bytes = Base64.decode(encoded, Base64.NO_WRAP)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply {
            init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, bytes, 0, 12))
        }
        String(cipher.doFinal(bytes, 12, bytes.size - 12))
    }.getOrNull()
}
