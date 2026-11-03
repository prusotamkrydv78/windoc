package com.notchy.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ServiceInfo
import android.graphics.drawable.Icon
import android.media.AudioAttributes
import android.media.AudioManager
import android.media.MediaPlayer
import android.media.RingtoneManager
import android.os.BatteryManager
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import org.json.JSONObject

/**
 * Keeps Notchy Link connected in the background and acts on what the PC sends: clipboard,
 * "find my phone", the PC's music as a notification with controls, and Focus sessions as a
 * Live Update countdown (which ColorOS shows in Fluid Cloud).
 */
class LinkService : Service() {
    private val main = Handler(Looper.getMainLooper())
    private var player: MediaPlayer? = null
    private var savedAlarmVolume = -1
    private var lastBattery = -1 to false
    private var wasAway = false
    private var wasConnected = false
    private var warnedPcLow = false

    private val batteryReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            val level = intent.getIntExtra(BatteryManager.EXTRA_LEVEL, -1)
            val scale = intent.getIntExtra(BatteryManager.EXTRA_SCALE, 100).coerceAtLeast(1)
            val status = intent.getIntExtra(BatteryManager.EXTRA_STATUS, -1)
            val charging = status == BatteryManager.BATTERY_STATUS_CHARGING ||
                status == BatteryManager.BATTERY_STATUS_FULL
            if (level >= 0) {
                lastBattery = level * 100 / scale to charging
                sendBattery()
            }
        }
    }

    private val linkListener: () -> Unit = {
        if (Link.connected && !wasConnected) {
            sendBattery()
            NotifyListener.instance?.publishMedia()
            val queued = PhoneMemory.queued(this)
            Thread { PhoneMemory.flush(this) }.start()
            if (queued > 0) RobotService.instance?.say("Gave PC $queued alerts")
            RobotService.instance?.say("Linked to ${Link.pcName ?: "PC"}")
        }
        if (!Link.connected && wasConnected) {
            // Lost the PC: if the phone also left its Wi-Fi in 90 s, you probably left the desk.
            main.removeCallbacks(leaveCheck)
            main.postDelayed(leaveCheck, 90_000)
        }
        if (Link.connected) main.removeCallbacks(leaveCheck)
        if (!Link.connected) {
            cancel(MEDIA_ID)
            cancel(MEETING_ID)
            cancel(FOCUS_ID)
            inMeeting = false
            focusing = false
            setSilent(false)
            RobotService.instance?.setFocused(false)
        }
        wasConnected = Link.connected
        if (Link.pcAway && !wasAway && Link.connected) {
            RobotService.instance?.say("I followed you here")
        }
        wasAway = Link.pcAway
        // Battery buddy: the PC's battery is low and it isn't charging.
        val pcLow = Link.connected && Link.pcBattery in 0..15 && !Link.pcCharging
        if (pcLow && !warnedPcLow) {
            RobotService.instance?.say("PC at ${Link.pcBattery}%")
            manager().notify(
                PC_LOW_ID,
                Notification.Builder(this, CHANNEL_MEDIA)
                    .setSmallIcon(R.drawable.ic_notchy)
                    .setContentTitle("${Link.pcName} is at ${Link.pcBattery}%")
                    .setContentText("Plug in the laptop before it sleeps")
                    .setAutoCancel(true)
                    .build(),
            )
        }
        if (Link.pcCharging || Link.pcBattery > 20) warnedPcLow = false
        if (pcLow) warnedPcLow = true
        showMedia()
        updateForeground()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        createChannels()
        startAsForeground()
        instance = this
        Link.onMessage = ::onMessage
        Link.addListener(linkListener)
        Link.start(this)
        UnlockWatcher.start(this)
        registerReceiver(batteryReceiver, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        main.postDelayed(screenTick, 60_000)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP_RING -> stopRing()
            ACTION_WANT -> {
                cancel(DOWNLOAD_ID)
                Link.send("want_file") { put("id", intent.getLongExtra("id", 0)) }
            }
            ACTION_MEDIA -> Link.send("media_cmd") { put("cmd", intent.getStringExtra("cmd") ?: "toggle") }
        }
        return START_STICKY
    }

    override fun onDestroy() {
        instance = null
        stopRing()
        Link.removeListener(linkListener)
        Link.onMessage = null
        Link.stop()
        UnlockWatcher.stop()
        unregisterReceiver(batteryReceiver)
        super.onDestroy()
    }

    private fun sendBattery() {
        val (percent, charging) = lastBattery
        if (percent >= 0) Link.send("battery") { put("pct", percent); put("charging", charging) }
    }

    private fun onMessage(message: JSONObject) {
        if (FileReceiver.handle(this, message)) return
        when (message.optString("t")) {
            "clip" -> {
                val text = message.optString("text")
                if (text.isNotEmpty()) {
                    getSystemService(ClipboardManager::class.java)
                        .setPrimaryClip(ClipData.newPlainText("From PC", text))
                    Stats.add(this, "clips")
                    val link = text.trim()
                    if (Regex("""^https?://\S+$""").matches(link)) {
                        offerLink(link)
                    } else {
                        RobotService.instance?.say("Copied from PC")
                    }
                }
            }
            "wave" -> {
                val name = message.optString("name", "A friend")
                RobotService.instance?.say("$name waved 👋")
                manager().notify(
                    WAVE_ID,
                    Notification.Builder(this, CHANNEL_MEDIA)
                        .setSmallIcon(R.drawable.ic_notchy)
                        .setContentTitle("$name waved at you")
                        .setContentText("From a Notchy nearby")
                        .setAutoCancel(true)
                        .build(),
                )
            }
            "ring" -> ring()
            "download" -> offerDownload(message)
            "meeting" -> {
                val on = message.optBoolean("on")
                inMeeting = on
                setSilent(on || focusing)
                RobotService.instance?.setFocused(on || focusing)
                if (on) RobotService.instance?.say("Shh… meeting on PC")
            }
            "focus" -> showFocus(message.optLong("until"))
            "phone_media" -> NotifyListener.instance?.command(message.optString("cmd"))
            "reply" -> {
                val ok = NotifyListener.instance?.reply(message.optString("key"), message.optString("text")) == true
                RobotService.instance?.say(if (ok) "Replied ✓" else "Can't reply to that")
            }
            "call_action" -> NotifyListener.instance?.callAction(message.optString("a"))
            "next_meeting" -> showMeeting(message)
            "need_internet" -> offerHotspot()
        }
    }

    // ---- Meeting countdown (Fluid Cloud) and the offline hotspot hint ---------------------

    private fun showMeeting(message: JSONObject) {
        val title = message.optString("title")
        if (title.isEmpty()) {
            cancel(MEETING_ID)
            return
        }
        val starts = message.optLong("starts")
        val link = message.optString("link").takeIf { it.startsWith("https://") }
        LiveUpdates.ensureChannel(this)
        val builder = Notification.Builder(this, "live")
            .setSmallIcon(R.drawable.ic_notchy)
            .setContentTitle(title)
            .setContentText(if (starts > System.currentTimeMillis()) "Meeting starts soon" else "Meeting started")
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setWhen(starts)
            .setShowWhen(true)
            .setUsesChronometer(true)
            .setChronometerCountDown(starts > System.currentTimeMillis())
            .setTimeoutAfter((starts - System.currentTimeMillis()).coerceAtLeast(0) + 15 * 60_000)
            .setCategory(Notification.CATEGORY_EVENT)
        if (link != null) {
            val join = PendingIntent.getActivity(
                this, 7, Intent(Intent.ACTION_VIEW, android.net.Uri.parse(link)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            builder.setContentIntent(join).addAction(Notification.Action.Builder(null as Icon?, "Join", join).build())
        }
        val notification = builder.build()
        notification.extras.putBoolean("android.requestPromotedOngoing", true)
        manager().notify(MEETING_ID, notification)
        RobotService.instance?.say("Meeting soon")
    }

    /** The PC lost its internet: Android won't let apps switch the hotspot on, so offer a tap. */
    private fun offerHotspot() {
        val tether = Intent().setClassName("com.android.settings", "com.android.settings.TetherSettings")
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        val target = if (tether.resolveActivity(packageManager) != null) tether
        else Intent(android.provider.Settings.ACTION_WIRELESS_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        val open = PendingIntent.getActivity(this, 8, target, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        manager().notify(
            HOTSPOT_ID,
            Notification.Builder(this, CHANNEL_ALERT)
                .setSmallIcon(R.drawable.ic_notchy)
                .setContentTitle("${Link.pcName ?: "Your PC"} lost internet")
                .setContentText("Tap to turn on your hotspot")
                .setContentIntent(open)
                .setAutoCancel(true)
                .setTimeoutAfter(10 * 60_000)
                .build(),
        )
        RobotService.instance?.say("PC is offline")
    }

    // ---- Screen time and the leave-home checklist ------------------------------------------

    private val screenTick = object : Runnable {
        override fun run() {
            ScreenTime.today(this@LinkService)?.let { (minutes, top) ->
                Link.send("screen_time") {
                    put("minutes", minutes)
                    put("top", org.json.JSONArray(top.map { org.json.JSONArray(listOf(it.first, it.second)) }))
                }
                ScreenTime.nudge(this@LinkService, minutes)
            }
            main.postDelayed(this, 10 * 60_000)
        }
    }

    private val leaveCheck = Runnable {
        if (!Link.connected && !onWifi()) {
            manager().notify(
                LEAVE_ID,
                Notification.Builder(this, CHANNEL_ALERT)
                    .setSmallIcon(R.drawable.ic_notchy)
                    .setContentTitle("Heading out?")
                    .setContentText("Charger · earbuds · wallet · keys")
                    .setStyle(Notification.BigTextStyle().bigText("Before you go: laptop charger, earbuds, wallet, keys. Your PC locks itself if you turned that on."))
                    .setAutoCancel(true)
                    .setTimeoutAfter(15 * 60_000)
                    .build(),
            )
            RobotService.instance?.say("Got your charger?")
        }
    }

    private fun onWifi(): Boolean {
        val connectivity = getSystemService(android.net.ConnectivityManager::class.java)
        val capabilities = connectivity.getNetworkCapabilities(connectivity.activeNetwork) ?: return false
        return capabilities.hasTransport(android.net.NetworkCapabilities.TRANSPORT_WIFI)
    }

    /** Handoff from the PC: a link copied there opens here with one tap. */
    private fun offerLink(url: String) {
        RobotService.instance?.say("Link from PC")
        val open = PendingIntent.getActivity(
            this, 2, Intent(Intent.ACTION_VIEW, android.net.Uri.parse(url)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        manager().notify(
            HANDOFF_ID,
            Notification.Builder(this, CHANNEL_ALERT)
                .setSmallIcon(R.drawable.ic_notchy)
                .setContentTitle("Continue on phone")
                .setContentText(url)
                .setContentIntent(open)
                .setAutoCancel(true)
                .setTimeoutAfter(120_000)
                .build(),
        )
    }

    /** A download finished on the PC: one tap brings it over. */
    private fun offerDownload(message: JSONObject) {
        val id = message.optLong("id")
        val name = message.optString("name")
        val size = message.optLong("size")
        val get = PendingIntent.getService(
            this, 6, Intent(this, LinkService::class.java).setAction(ACTION_WANT).putExtra("id", id),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        LiveUpdates.ensureChannel(this)
        manager().notify(
            DOWNLOAD_ID,
            Notification.Builder(this, "live")
                .setSmallIcon(R.drawable.ic_notchy)
                .setContentTitle("Downloaded on ${Link.pcName}")
                .setContentText("$name · ${android.text.format.Formatter.formatShortFileSize(this, size)}")
                .addAction(Notification.Action.Builder(null as Icon?, "Get it on phone", get).build())
                .setContentIntent(get)
                .setAutoCancel(true)
                .setTimeoutAfter(10 * 60_000)
                .build(),
        )
    }

    // ---- Find my phone -------------------------------------------------------------------

    private fun ring() {
        if (player != null) return
        val audio = getSystemService(AudioManager::class.java)
        savedAlarmVolume = audio.getStreamVolume(AudioManager.STREAM_ALARM)
        audio.setStreamVolume(AudioManager.STREAM_ALARM, audio.getStreamMaxVolume(AudioManager.STREAM_ALARM), 0)
        val uri = RingtoneManager.getDefaultUri(RingtoneManager.TYPE_ALARM)
            ?: RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE)
        player = runCatching {
            MediaPlayer().apply {
                setAudioAttributes(
                    AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_ALARM)
                        .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION).build(),
                )
                setDataSource(this@LinkService, uri)
                isLooping = true
                prepare()
                start()
            }
        }.getOrNull()
        val stop = PendingIntent.getService(
            this, 1, Intent(this, LinkService::class.java).setAction(ACTION_STOP_RING),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val notification = Notification.Builder(this, CHANNEL_ALERT)
            .setSmallIcon(R.drawable.ic_notchy)
            .setContentTitle("Your PC is looking for this phone")
            .setContentText("Tap to stop ringing")
            .setContentIntent(stop)
            .setDeleteIntent(stop)
            .addAction(Notification.Action.Builder(null as Icon?, "Found it", stop).build())
            .setCategory(Notification.CATEGORY_ALARM)
            .build()
        manager().notify(RING_ID, notification)
        RobotService.instance?.say("Here I am!")
        main.postDelayed({ stopRing() }, 45_000)
    }

    private fun stopRing() {
        player?.let { runCatching { it.stop(); it.release() } }
        player = null
        if (savedAlarmVolume >= 0) {
            getSystemService(AudioManager::class.java)
                .setStreamVolume(AudioManager.STREAM_ALARM, savedAlarmVolume, 0)
            savedAlarmVolume = -1
        }
        cancel(RING_ID)
    }

    // ---- PC music ------------------------------------------------------------------------

    private var shownMedia: Triple<String, String, Boolean>? = null

    private fun mediaAction(cmd: String, label: String, code: Int): Notification.Action {
        val intent = PendingIntent.getService(
            this, code, Intent(this, LinkService::class.java).setAction(ACTION_MEDIA).putExtra("cmd", cmd),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return Notification.Action.Builder(null as Icon?, label, intent).build()
    }

    private fun showMedia() {
        val media = Link.pcMedia
        if (media == shownMedia) return
        shownMedia = media
        if (media == null) {
            cancel(MEDIA_ID)
            return
        }
        val (title, artist, playing) = media
        val notification = Notification.Builder(this, CHANNEL_MEDIA)
            .setSmallIcon(R.drawable.ic_notchy)
            .setContentTitle(title)
            .setContentText(if (artist.isEmpty()) "Playing on ${Link.pcName}" else "$artist · on ${Link.pcName}")
            .setOnlyAlertOnce(true)
            .addAction(mediaAction("prev", "Back", 10))
            .addAction(mediaAction("toggle", if (playing) "Pause" else "Play", 11))
            .addAction(mediaAction("next", "Next", 12))
            .setStyle(Notification.MediaStyle().setShowActionsInCompactView(0, 1, 2))
            .build()
        manager().notify(MEDIA_ID, notification)
    }

    // ---- Focus as a Live Update ----------------------------------------------------------

    /** Whether Notchy turned Do Not Disturb on, so it only turns off what it turned on. */
    private var silenced = false

    private fun setSilent(on: Boolean) {
        val manager = manager()
        val wanted = getSharedPreferences("app", MODE_PRIVATE).getBoolean("focus_dnd", true)
        if (!manager.isNotificationPolicyAccessGranted) return
        if (on && wanted && !silenced &&
            manager.currentInterruptionFilter == NotificationManager.INTERRUPTION_FILTER_ALL
        ) {
            manager.setInterruptionFilter(NotificationManager.INTERRUPTION_FILTER_PRIORITY)
            silenced = true
        } else if (!on && silenced) {
            manager.setInterruptionFilter(NotificationManager.INTERRUPTION_FILTER_ALL)
            silenced = false
        }
    }

    private var inMeeting = false
    private var focusing = false

    private fun showFocus(until: Long) {
        if (until <= System.currentTimeMillis()) {
            focusing = false
            cancel(FOCUS_ID)
            setSilent(inMeeting)
            RobotService.instance?.setFocused(inMeeting)
            return
        }
        focusing = true
        setSilent(true)
        RobotService.instance?.setFocused(true)
        main.removeCallbacks(focusEnd)
        main.postDelayed(focusEnd, until - System.currentTimeMillis())
        LiveUpdates.ensureChannel(this)
        val builder = Notification.Builder(this, "live")
            .setSmallIcon(R.drawable.ic_notchy)
            .setContentTitle("Focus on ${Link.pcName ?: "PC"}")
            .setContentText("Stay with it — the robot is guarding your time")
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setWhen(until)
            .setShowWhen(true)
            .setUsesChronometer(true)
            .setChronometerCountDown(true)
            .setTimeoutAfter(until - System.currentTimeMillis() + 2000)
            .setCategory(Notification.CATEGORY_STOPWATCH)
        val notification = builder.build()
        notification.extras.putBoolean("android.requestPromotedOngoing", true)
        manager().notify(FOCUS_ID, notification)
    }

    private val focusEnd = Runnable {
        focusing = false
        setSilent(inMeeting)
        RobotService.instance?.setFocused(inMeeting)
    }

    // ---- Plumbing ------------------------------------------------------------------------

    private fun manager() = getSystemService(NotificationManager::class.java)
    private fun cancel(id: Int) = manager().cancel(id)

    private fun createChannels() {
        val manager = manager()
        manager.createNotificationChannel(NotificationChannel(CHANNEL_LINK, "Link status", NotificationManager.IMPORTANCE_MIN))
        manager.createNotificationChannel(NotificationChannel(CHANNEL_MEDIA, "PC music", NotificationManager.IMPORTANCE_LOW))
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL_ALERT, "Find my phone", NotificationManager.IMPORTANCE_HIGH).apply {
                setSound(null, null)
            },
        )
    }

    private fun foregroundNotification(): Notification {
        val open = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE,
        )
        val text = if (Link.connected) "Linked to ${Link.pcName}" else "Looking for your PC…"
        return Notification.Builder(this, CHANNEL_LINK)
            .setSmallIcon(R.drawable.ic_notchy)
            .setContentTitle("Notchy Link")
            .setContentText(text)
            .setContentIntent(open)
            .setOngoing(true)
            .build()
    }

    private fun startAsForeground() {
        if (Build.VERSION.SDK_INT >= 34) {
            startForeground(LINK_ID, foregroundNotification(), ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        } else {
            startForeground(LINK_ID, foregroundNotification())
        }
    }

    private fun updateForeground() = manager().notify(LINK_ID, foregroundNotification())

    companion object {
        private const val CHANNEL_LINK = "link"
        private const val CHANNEL_MEDIA = "pc_media"
        private const val CHANNEL_ALERT = "find_phone"
        private const val LINK_ID = 20
        private const val MEDIA_ID = 21
        private const val FOCUS_ID = 22
        private const val RING_ID = 23
        private const val HANDOFF_ID = 24
        private const val WAVE_ID = 25
        private const val PC_LOW_ID = 26
        private const val MEETING_ID = 28
        private const val HOTSPOT_ID = 29
        private const val LEAVE_ID = 31
        private const val ACTION_STOP_RING = "com.notchy.app.STOP_RING"
        private const val ACTION_MEDIA = "com.notchy.app.MEDIA"
        private const val ACTION_WANT = "com.notchy.app.WANT"
        private const val DOWNLOAD_ID = 27

        var instance: LinkService? = null
            private set

        fun start(context: Context) {
            if (Link.isPaired(context)) {
                context.startForegroundService(Intent(context, LinkService::class.java))
            }
        }
    }
}
