package com.notchy.app

import android.app.Notification
import android.content.ComponentName
import android.content.Context
import android.media.MediaMetadata
import android.media.session.MediaController
import android.media.session.MediaSessionManager
import android.media.session.PlaybackState
import android.provider.Settings
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification

/**
 * Mirrors the phone's notifications to the PC (codes get copied there by themselves) and
 * shares the phone's now playing so the PC can show and control it. Needs the user to allow
 * notification access.
 */
class NotifyListener : NotificationListenerService() {
    private var controller: MediaController? = null
    private val recent = LinkedHashMap<String, Long>()

    private val callback = object : MediaController.Callback() {
        override fun onMetadataChanged(metadata: MediaMetadata?) = publishMedia()
        override fun onPlaybackStateChanged(state: PlaybackState?) = publishMedia()
    }

    private val sessions = MediaSessionManager.OnActiveSessionsChangedListener { list ->
        watch(list?.firstOrNull())
    }

    override fun onListenerConnected() {
        instance = this
        val manager = getSystemService(MediaSessionManager::class.java)
        val me = ComponentName(this, NotifyListener::class.java)
        runCatching {
            manager.addOnActiveSessionsChangedListener(sessions, me)
            watch(manager.getActiveSessions(me).firstOrNull())
        }
    }

    override fun onListenerDisconnected() {
        instance = null
        runCatching { getSystemService(MediaSessionManager::class.java).removeOnActiveSessionsChangedListener(sessions) }
        watch(null)
    }

    private fun watch(next: MediaController?) {
        if (next?.sessionToken == controller?.sessionToken) return
        controller?.unregisterCallback(callback)
        controller = next
        next?.registerCallback(callback)
        publishMedia()
    }

    fun publishMedia() {
        val metadata = controller?.metadata
        val title = metadata?.getString(MediaMetadata.METADATA_KEY_TITLE).orEmpty()
        val artist = metadata?.getString(MediaMetadata.METADATA_KEY_ARTIST).orEmpty()
        val state = controller?.playbackState
        val playing = state?.state == PlaybackState.STATE_PLAYING
        RobotService.instance?.setMusic(
            if (title.isEmpty()) null else RobotView.Music(
                title = title,
                artist = artist,
                art = metadata?.getBitmap(MediaMetadata.METADATA_KEY_ART)
                    ?: metadata?.getBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART)
                    ?: metadata?.getBitmap(MediaMetadata.METADATA_KEY_DISPLAY_ICON),
                playing = playing,
                position = state?.position ?: 0L,
                duration = metadata?.getLong(MediaMetadata.METADATA_KEY_DURATION) ?: 0L,
                updatedAt = state?.lastPositionUpdateTime ?: android.os.SystemClock.elapsedRealtime(),
                speed = state?.playbackSpeed?.takeIf { it > 0f } ?: 1f,
            ),
        )
        Link.send("phone_media") {
            put("title", title)
            put("artist", artist)
            put("playing", playing)
        }
    }

    fun command(cmd: String) {
        val controls = controller?.transportControls ?: return
        when (cmd) {
            "prev" -> controls.skipToPrevious()
            "next" -> controls.skipToNext()
            else -> if (controller?.playbackState?.state == PlaybackState.STATE_PLAYING) controls.pause() else controls.play()
        }
    }

    override fun onNotificationPosted(sbn: StatusBarNotification) {
        if (sbn.packageName == packageName) return
        val notification = sbn.notification
        // Calls and live progress (rides, deliveries) are ongoing: mirror them separately.
        if (notification.category == Notification.CATEGORY_CALL) {
            publishCall(sbn)
            return
        }
        if (isLive(sbn)) {
            publishLive(sbn)
            return
        }
        if (sbn.isOngoing) return
        if (notification.flags and Notification.FLAG_GROUP_SUMMARY != 0) return
        if (notification.category == Notification.CATEGORY_TRANSPORT) return
        val extras = notification.extras
        val title = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString().orEmpty()
        val text = (extras.getCharSequence(Notification.EXTRA_BIG_TEXT)
            ?: extras.getCharSequence(Notification.EXTRA_TEXT))?.toString().orEmpty()
        if (title.isEmpty() && text.isEmpty()) return
        // Apps re-post the same notification often; send each text once.
        val stamp = "${sbn.key}|$title|$text"
        val now = System.currentTimeMillis()
        if ((recent[stamp] ?: 0L) > now - 60_000) return
        recent[stamp] = now
        if (recent.size > 50) recent.remove(recent.keys.first())
        val app = runCatching {
            packageManager.getApplicationLabel(packageManager.getApplicationInfo(sbn.packageName, 0)).toString()
        }.getOrDefault(sbn.packageName)
        // Bank and UPI messages: a spending log.
        Expenses.record(this, app, "$title $text")
        val sent = Link.send("notif") {
            put("key", sbn.key)
            put("app", app)
            put("title", title)
            put("text", text)
            put("reply", replyAction(notification) != null)
        }
        if (sent) Stats.add(this, "alerts")
        val joined = "$title $text"
        val isCode = Regex("""\b\d{4,8}\b""").containsMatchIn(joined) &&
            Regex("(?i)otp|code|verif|password|passcode|pin").containsMatchIn(joined)
        PhoneMemory.record(this, app, title, text, isCode, sent)
        RobotService.instance?.showNotice(
            RobotView.Notice(
                key = sbn.key,
                app = app,
                icon = runCatching { packageManager.getApplicationIcon(sbn.packageName) }.getOrNull(),
                avatar = runCatching { notification.getLargeIcon()?.loadDrawable(this)?.let(::toBitmap) }.getOrNull(),
                title = title,
                text = text,
                actions = notification.actions?.mapNotNull { it.title?.toString() }.orEmpty(),
            ),
        )
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification) {
        val notification = sbn.notification
        if (notification.category == Notification.CATEGORY_CALL) {
            Link.send("call_end")
            RobotService.instance?.setRinging(false)
            RobotService.instance?.setCall(null)
        } else if (sbn.key in liveKeys) {
            liveKeys -= sbn.key
            Link.send("live_end") { put("key", sbn.key) }
            if (liveKeys.isEmpty()) RobotService.instance?.setActivity(null)
        }
    }

    // ---- Replies from the PC --------------------------------------------------------------

    private fun replyAction(notification: Notification): Notification.Action? =
        notification.actions?.firstOrNull { action -> action.remoteInputs?.isNotEmpty() == true }

    /** Sends [text] through the notification's own reply box, as if typed on the phone. */
    fun reply(key: String, text: String): Boolean {
        val sbn = runCatching { getActiveNotifications(arrayOf(key)) }.getOrNull()?.firstOrNull() ?: return false
        val action = replyAction(sbn.notification) ?: return false
        val input = action.remoteInputs.first()
        val intent = android.content.Intent()
        val results = android.os.Bundle().apply { putCharSequence(input.resultKey, text) }
        android.app.RemoteInput.addResultsToIntent(action.remoteInputs, intent, results)
        return runCatching { action.actionIntent.send(this, 0, intent); true }.getOrDefault(false)
    }

    // ---- Calls ---------------------------------------------------------------------------

    private fun publishCall(sbn: StatusBarNotification) {
        val notification = sbn.notification
        val extras = notification.extras
        val name = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString().orEmpty().ifEmpty { "Unknown caller" }
        // A running call shows a chronometer; a ringing one does not.
        val ongoing = extras.getBoolean(Notification.EXTRA_SHOW_CHRONOMETER) || notification.`when` < System.currentTimeMillis() - 5_000 && sbn.isOngoing && notification.fullScreenIntent == null
        val actions = notification.actions?.mapNotNull { it.title?.toString() }.orEmpty()
        Link.send("call") {
            put("name", name)
            put("state", if (ongoing) "ongoing" else "ringing")
            put("actions", org.json.JSONArray(actions.take(3) + if (ongoing) listOf("Mute") else emptyList()))
        }
        RobotService.instance?.setRinging(!ongoing)
        RobotService.instance?.setCall(
            RobotView.Call(
                name = name,
                avatar = runCatching { notification.getLargeIcon()?.loadDrawable(this)?.let(::toBitmap) }.getOrNull(),
                ringing = !ongoing,
                since = if (ongoing) notification.`when`.takeIf { it > 0 } ?: System.currentTimeMillis() else System.currentTimeMillis(),
            ),
        )
    }

    /** Presses the call notification's button named [action] (or mutes the mic). */
    fun callAction(action: String) {
        if (action == "Mute") {
            val audio = getSystemService(android.media.AudioManager::class.java)
            audio.isMicrophoneMute = !audio.isMicrophoneMute
            return
        }
        val call = runCatching { activeNotifications }.getOrNull()
            ?.firstOrNull { it.notification.category == Notification.CATEGORY_CALL } ?: return
        call.notification.actions?.firstOrNull { it.title?.toString() == action }?.actionIntent?.let {
            runCatching { it.send() }
        }
    }

    /** The island's call buttons: "answer", "decline", "end" or "mute". */
    fun callButton(kind: String) {
        if (kind == "mute") {
            callAction("Mute")
            return
        }
        val call = runCatching { activeNotifications }.getOrNull()
            ?.firstOrNull { it.notification.category == Notification.CATEGORY_CALL } ?: return
        val actions = call.notification.actions ?: return
        val words = if (kind == "answer") listOf("answer", "accept", "pick") else listOf("decline", "reject", "hang", "end", "dismiss")
        val action = actions.firstOrNull { action -> words.any { action.title?.toString()?.lowercase()?.contains(it) == true } }
            ?: if (kind == "answer") actions.lastOrNull() else actions.firstOrNull()
        runCatching { action?.actionIntent?.send() }
    }

    // ---- Notification card buttons -------------------------------------------------------

    private fun active(key: String) = runCatching { getActiveNotifications(arrayOf(key)) }.getOrNull()?.firstOrNull()

    /** Opens the app behind the notification. */
    fun open(key: String) {
        val sbn = active(key) ?: return
        runCatching { sbn.notification.contentIntent?.send() }
    }

    /** Presses the notification's button [index]; a reply button opens the app to type in. */
    fun press(key: String, index: Int) {
        val sbn = active(key) ?: return
        val action = sbn.notification.actions?.getOrNull(index) ?: return
        if (action.remoteInputs?.isNotEmpty() == true) {
            runCatching { sbn.notification.contentIntent?.send() }
        } else {
            runCatching { action.actionIntent.send() }
        }
    }

    private fun toBitmap(drawable: android.graphics.drawable.Drawable): android.graphics.Bitmap {
        if (drawable is android.graphics.drawable.BitmapDrawable && drawable.bitmap != null) return drawable.bitmap
        val size = 128
        val bitmap = android.graphics.Bitmap.createBitmap(size, size, android.graphics.Bitmap.Config.ARGB_8888)
        drawable.setBounds(0, 0, size, size)
        drawable.draw(android.graphics.Canvas(bitmap))
        return bitmap
    }

    // ---- Live progress (rides, deliveries) -------------------------------------------------

    private val liveKeys = mutableSetOf<String>()

    private fun isLive(sbn: StatusBarNotification): Boolean {
        val extras = sbn.notification.extras
        val hasBar = extras.getInt(Notification.EXTRA_PROGRESS_MAX, 0) > 0
        val trackingApp = TRACKING_APPS.any { sbn.packageName.contains(it) }
        return sbn.isOngoing && (hasBar || trackingApp) && sbn.notification.category != Notification.CATEGORY_TRANSPORT
    }

    private fun publishLive(sbn: StatusBarNotification) {
        val extras = sbn.notification.extras
        val max = extras.getInt(Notification.EXTRA_PROGRESS_MAX, 0)
        val progress = extras.getInt(Notification.EXTRA_PROGRESS, 0)
        val app = runCatching {
            packageManager.getApplicationLabel(packageManager.getApplicationInfo(sbn.packageName, 0)).toString()
        }.getOrDefault(sbn.packageName)
        liveKeys += sbn.key
        val shortTitle = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString().orEmpty().take(10).ifEmpty { app.take(10) }
        RobotService.instance?.setActivity(
            RobotView.LiveActivity(
                shortTitle,
                if (max > 0) progress.toFloat() / max else null,
                detail = extras.getCharSequence(Notification.EXTRA_TEXT)?.toString().orEmpty(),
            ),
        )
        Link.send("live") {
            put("key", sbn.key)
            put("app", app)
            put("title", extras.getCharSequence(Notification.EXTRA_TITLE)?.toString().orEmpty())
            put("text", extras.getCharSequence(Notification.EXTRA_TEXT)?.toString().orEmpty())
            if (max > 0) put("progress", progress * 100 / max)
        }
    }

    companion object {
        /** Ride and delivery apps whose ongoing notifications are worth a live pill on the PC. */
        private val TRACKING_APPS = listOf(
            "ubercab", "olacabs", "rapido", "swiggy", "zomato", "amazon", "flipkart", "blinkit",
            "zepto", "bigbasket", "dunzo", "porter", "irctc", "makemytrip",
        )

        var instance: NotifyListener? = null
            private set

        fun allowed(context: Context): Boolean {
            val enabled = Settings.Secure.getString(context.contentResolver, "enabled_notification_listeners").orEmpty()
            return enabled.contains(ComponentName(context, NotifyListener::class.java).flattenToString())
        }
    }
}
