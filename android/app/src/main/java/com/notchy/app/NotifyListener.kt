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
        val playing = controller?.playbackState?.state == PlaybackState.STATE_PLAYING
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
        if (sbn.packageName == packageName || sbn.isOngoing) return
        val notification = sbn.notification
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
        val sent = Link.send("notif") {
            put("app", app)
            put("title", title)
            put("text", text)
        }
        if (sent) Stats.add(this, "alerts")
    }

    companion object {
        var instance: NotifyListener? = null
            private set

        fun allowed(context: Context): Boolean {
            val enabled = Settings.Secure.getString(context.contentResolver, "enabled_notification_listeners").orEmpty()
            return enabled.contains(ComponentName(context, NotifyListener::class.java).flattenToString())
        }
    }
}
