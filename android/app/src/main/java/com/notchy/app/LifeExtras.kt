package com.notchy.app

import android.Manifest
import android.app.Activity
import android.app.AlarmManager
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.ContentUris
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.net.ConnectivityManager
import android.net.Network
import android.net.wifi.WifiManager
import android.os.Bundle
import android.provider.MediaStore
import android.speech.RecognizerIntent
import android.widget.Toast
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.util.Calendar

// ---- Places: the robot knows home, office or out -----------------------------------------

/**
 * Wi-Fi networks the user named Home or Office (reading the network name needs location
 * permission). At the office the phone goes quiet (with Do Not Disturb access); at home it
 * comes back; out on mobile data the robot says so.
 */
object Places {
    private const val PREFS = "places"
    enum class Place(val label: String) { HOME("Home"), OFFICE("Office"), OUT("Out") }

    var current: Place? = null
        private set

    fun currentSsid(context: Context): String? {
        if (context.checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) != PackageManager.PERMISSION_GRANTED) return null
        @Suppress("DEPRECATION")
        val ssid = context.getSystemService(WifiManager::class.java).connectionInfo?.ssid ?: return null
        return ssid.trim('"').takeIf { it.isNotEmpty() && it != "<unknown ssid>" }
    }

    fun label(context: Context, place: Place) {
        val ssid = currentSsid(context) ?: return
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putString(ssid, place.name).apply()
        update(context)
    }

    fun labelled(context: Context): Map<String, String> =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).all.mapValues { it.value.toString() }

    private fun detect(context: Context): Place {
        val ssid = currentSsid(context) ?: return Place.OUT
        val saved = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(ssid, null)
        return saved?.let { runCatching { Place.valueOf(it) }.getOrNull() } ?: Place.OUT
    }

    fun update(context: Context) {
        val place = detect(context)
        if (place == current) return
        val before = current
        current = place
        if (before == null) return
        val manager = context.getSystemService(NotificationManager::class.java)
        when (place) {
            Place.OFFICE -> {
                if (manager.isNotificationPolicyAccessGranted &&
                    manager.currentInterruptionFilter == NotificationManager.INTERRUPTION_FILTER_ALL
                ) {
                    manager.setInterruptionFilter(NotificationManager.INTERRUPTION_FILTER_PRIORITY)
                }
                RobotService.instance?.say("At the office 💼")
            }
            Place.HOME -> {
                if (manager.isNotificationPolicyAccessGranted && before == Place.OFFICE) {
                    manager.setInterruptionFilter(NotificationManager.INTERRUPTION_FILTER_ALL)
                }
                RobotService.instance?.say("Home sweet home 🏠")
            }
            Place.OUT -> RobotService.instance?.say("Out and about")
        }
    }

    /** Watches network changes for as long as the link service runs. */
    fun watch(context: Context) {
        val app = context.applicationContext
        update(app)
        app.getSystemService(ConnectivityManager::class.java).registerDefaultNetworkCallback(
            object : ConnectivityManager.NetworkCallback() {
                override fun onAvailable(network: Network) = update(app)
                override fun onLost(network: Network) = update(app)
            },
        )
    }
}

// ---- Expenses: bank and UPI messages become a spending log ---------------------------------

object Expenses {
    private const val PREFS = "expenses"
    private val AMOUNT = Regex("""(?:rs\.?|inr|₹)\s?([\d,]+(?:\.\d{1,2})?)""", RegexOption.IGNORE_CASE)
    private val SPENT = Regex("""(?i)\b(debited|spent|paid|sent|purchase|withdrawn|txn of)\b""")
    private val MERCHANT = Regex("""(?i)\b(?:at|to|towards|on)\s+([A-Za-z0-9&.' -]{2,30}?)(?:\s+(?:on|via|ref|upi|for|from|avl|bal)\b|[.,]|$)""")

    data class Expense(val at: Long, val amount: Double, val merchant: String)

    /** A payment in [text], if it is one. */
    fun parse(text: String): Pair<Double, String>? {
        if (!SPENT.containsMatchIn(text)) return null
        if (Regex("(?i)\\b(credited|received|refund)\\b").containsMatchIn(text) && !Regex("(?i)debited").containsMatchIn(text)) return null
        val amount = AMOUNT.find(text)?.groupValues?.get(1)?.replace(",", "")?.toDoubleOrNull() ?: return null
        val merchant = MERCHANT.find(text)?.groupValues?.get(1)?.trim().orEmpty()
        return amount to merchant
    }

    fun record(context: Context, app: String, text: String) {
        val (amount, merchant) = parse(text) ?: return
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val all = JSONArray(prefs.getString("all", "[]"))
        all.put(JSONObject().put("at", System.currentTimeMillis()).put("amount", amount).put("merchant", merchant))
        val trimmed = JSONArray()
        for (index in maxOf(0, all.length() - 500) until all.length()) trimmed.put(all.get(index))
        prefs.edit().putString("all", trimmed.toString()).apply()
        Link.send("expense") {
            put("amount", amount)
            put("merchant", merchant)
            put("text", text.take(300))
            put("app", app)
        }
        RobotService.instance?.say("Spent ₹${amount.toInt()}")
    }

    fun since(context: Context, ms: Long): List<Expense> {
        val all = JSONArray(context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString("all", "[]"))
        return (0 until all.length()).map { all.getJSONObject(it) }
            .map { Expense(it.optLong("at"), it.optDouble("amount"), it.optString("merchant")) }
            .filter { it.at >= ms }
    }

    /** This week's total and top merchants. */
    fun weekly(context: Context): String {
        val week = since(context, System.currentTimeMillis() - 7 * 86_400_000L)
        if (week.isEmpty()) return "No payments spotted in the last 7 days.\n\nNotchy reads bank and UPI notifications (with notification access) — nothing is sent anywhere but your own PC."
        val top = week.groupBy { it.merchant.ifEmpty { "Other" } }
            .mapValues { entry -> entry.value.sumOf { it.amount } }
            .entries.sortedByDescending { it.value }.take(6)
            .joinToString("\n") { "• ${it.key}: ₹${"%.0f".format(it.value)}" }
        return "₹${"%.0f".format(week.sumOf { it.amount })} across ${week.size} payments\n\n$top"
    }
}

// ---- Photo memories: "on this day" from the gallery ----------------------------------------

object PhotoMemories {
    private const val PREFS = "photo_memories"

    fun allowed(context: Context) =
        context.checkSelfPermission(Manifest.permission.READ_MEDIA_IMAGES) == PackageManager.PERMISSION_GRANTED

    /** Once a day after 10 am, a photo from this date in an earlier year goes to the PC. */
    fun maybeSend(context: Context) {
        if (!allowed(context) || !Link.connected) return
        val now = Calendar.getInstance()
        if (now.get(Calendar.HOUR_OF_DAY) < 10) return
        val day = now.get(Calendar.YEAR) * 1000 + now.get(Calendar.DAY_OF_YEAR)
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        if (prefs.getInt("sent_day", 0) == day) return
        prefs.edit().putInt("sent_day", day).apply()
        Thread { find(context, now)?.let { (uri, years) -> send(context, uri, years) } }.start()
    }

    private fun find(context: Context, today: Calendar): Pair<android.net.Uri, Int>? {
        for (years in 1..8) {
            val start = (today.clone() as Calendar).apply {
                add(Calendar.YEAR, -years)
                set(Calendar.HOUR_OF_DAY, 0); set(Calendar.MINUTE, 0); set(Calendar.SECOND, 0)
            }.timeInMillis
            val end = start + 86_400_000L
            context.contentResolver.query(
                MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
                arrayOf(MediaStore.Images.Media._ID),
                "${MediaStore.Images.Media.DATE_TAKEN} >= ? AND ${MediaStore.Images.Media.DATE_TAKEN} < ?",
                arrayOf(start.toString(), end.toString()),
                "${MediaStore.Images.Media.DATE_TAKEN} ASC",
            )?.use { cursor ->
                if (cursor.count > 0) {
                    cursor.moveToPosition((0 until cursor.count).random())
                    val id = cursor.getLong(0)
                    return ContentUris.withAppendedId(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, id) to years
                }
            }
        }
        return null
    }

    /** Sends a 1600 px copy (small and quick) announced as a memory. */
    private fun send(context: Context, uri: android.net.Uri, years: Int) {
        val bitmap = runCatching {
            ImageDecoder.decodeBitmap(ImageDecoder.createSource(context.contentResolver, uri)) { decoder, info, _ ->
                val scale = 1600f / maxOf(info.size.width, info.size.height)
                if (scale < 1f) decoder.setTargetSize((info.size.width * scale).toInt(), (info.size.height * scale).toInt())
            }
        }.getOrNull() ?: return
        val year = Calendar.getInstance().get(Calendar.YEAR) - years
        val file = File(context.cacheDir, "On this day $year.jpg")
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.JPEG, 85, it) }
        val id = System.nanoTime()
        Link.send("photo_memory") { put("id", id); put("years", years) }
        runCatching {
            Link.sendFile(context, PcLink.FileInfo(android.net.Uri.fromFile(file), file.name, file.length()), id) {}
        }
        file.delete()
        RobotService.instance?.say("A memory from $year 📸")
    }
}

// ---- Voice: hold-to-talk commands -----------------------------------------------------------

/**
 * Speak to the robot (from the quick panel). Android's own speech recognition, no key needed.
 * Understands: "timer 10 minutes", "remind me at 5 to call mom", "remind me in 20 minutes to…",
 * "send … to PC", "note …", "flashlight", "find my PC", "lock PC"; anything else is a question.
 */
class VoiceActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val listen = Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH)
            .putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
            .putExtra(RecognizerIntent.EXTRA_PROMPT, "Tell the robot…")
        runCatching { startActivityForResult(listen, 1) }.onFailure {
            Toast.makeText(this, "Speech recognition isn't available", Toast.LENGTH_SHORT).show()
            finish()
        }
    }

    @Deprecated("Plain Activity result")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        val said = data?.getStringArrayListExtra(RecognizerIntent.EXTRA_RESULTS)?.firstOrNull()
        if (resultCode == RESULT_OK && !said.isNullOrBlank()) {
            val reply = Voice.handle(this, said)
            Toast.makeText(this, reply, Toast.LENGTH_LONG).show()
            RobotService.instance?.say(reply.take(22))
        }
        finish()
    }
}

object Voice {
    private val TIMER = Regex("""(?i)timer (?:for )?(\d+) ?(min|minute|minutes|hour|hours)""")
    private val REMIND_IN = Regex("""(?i)remind me in (\d+) ?(min|minute|minutes|hour|hours) (?:to )?(.+)""")
    private val REMIND_AT = Regex("""(?i)remind me at (\d{1,2})(?::(\d{2}))? ?(am|pm|a\.m\.|p\.m\.)? (?:to )?(.+)""")
    private val SEND = Regex("""(?i)^send (.+) to (?:the )?(?:pc|computer|laptop)$""")
    private val NOTE = Regex("""(?i)^(?:note|write down|remember) (.+)""")

    /** Does what [said] asks; returns what the robot answers. */
    fun handle(context: Context, said: String): String {
        val text = said.trim()
        TIMER.find(text)?.let {
            val amount = it.groupValues[1].toInt()
            val minutes = if (it.groupValues[2].startsWith("hour")) amount * 60 else amount
            Timers.start(context, minutes)
            return "Timer set for $minutes min"
        }
        REMIND_IN.find(text)?.let {
            val amount = it.groupValues[1].toLong()
            val minutes = if (it.groupValues[2].startsWith("hour")) amount * 60 else amount
            Reminders.at(context, System.currentTimeMillis() + minutes * 60_000, it.groupValues[3])
            return "I'll remind you in $minutes min"
        }
        REMIND_AT.find(text)?.let {
            var hour = it.groupValues[1].toInt()
            val minute = it.groupValues[2].toIntOrNull() ?: 0
            val half = it.groupValues[3].lowercase()
            if (half.startsWith("p") && hour < 12) hour += 12
            if (half.startsWith("a") && hour == 12) hour = 0
            // "at 5" without am/pm: the next 5 o'clock.
            val target = Calendar.getInstance().apply {
                set(Calendar.HOUR_OF_DAY, hour); set(Calendar.MINUTE, minute); set(Calendar.SECOND, 0)
                if (half.isEmpty() && timeInMillis < System.currentTimeMillis() && hour < 12) add(Calendar.HOUR_OF_DAY, 12)
                if (timeInMillis < System.currentTimeMillis()) add(Calendar.DAY_OF_YEAR, 1)
            }
            Reminders.at(context, target.timeInMillis, it.groupValues[4])
            return "Reminder at %d:%02d".format(target.get(Calendar.HOUR_OF_DAY), minute)
        }
        SEND.find(text)?.let {
            return if (ClipSendActivity.sendText(it.groupValues[1])) "Sent to ${Link.pcName}" else "PC not linked"
        }
        NOTE.find(text)?.let {
            Notes.add(context, it.groupValues[1])
            Link.send("note") { put("text", it.groupValues[1]) }
            return "Noted"
        }
        val lower = text.lowercase()
        return when {
            "flashlight" in lower || "torch" in lower -> { Torch.toggle(context); if (Torch.on) "Lights on" else "Lights off" }
            "find my pc" in lower || "ring my pc" in lower -> if (Link.send("find_pc")) "Ringing your PC" else "PC not linked"
            "lock" in lower && ("pc" in lower || "computer" in lower || "laptop" in lower) ->
                if (Link.send("lock_pc")) "PC locked" else "PC not linked"
            else -> {
                // A question: answer it like "Ask the robot".
                if (Link.send("ask") { put("q", text) }) "Asking ${Link.pcName}… check the notification"
                else PhoneMemory.ask(context, text).take(200)
            }
        }
    }
}

/** Voice reminders: an alarm that posts a notification and makes the robot speak. */
object Reminders {
    fun at(context: Context, time: Long, text: String) {
        val intent = Intent(context, ReminderReceiver::class.java).putExtra("text", text)
        val pending = PendingIntent.getBroadcast(
            context, (time / 1000).toInt(), intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        context.getSystemService(AlarmManager::class.java).setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, time, pending)
    }
}

class ReminderReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val text = intent.getStringExtra("text") ?: "Reminder"
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            android.app.NotificationChannel("reminders", "Reminders", NotificationManager.IMPORTANCE_HIGH),
        )
        manager.notify(
            (System.currentTimeMillis() % 100_000).toInt() + 1000,
            android.app.Notification.Builder(context, "reminders")
                .setSmallIcon(R.drawable.ic_notchy)
                .setContentTitle("Reminder")
                .setContentText(text)
                .setAutoCancel(true)
                .build(),
        )
        RobotService.instance?.say("⏰ ${text.take(18)}")
        Link.send("notif") { put("app", "Notchy"); put("title", "Reminder"); put("text", text) }
    }
}
