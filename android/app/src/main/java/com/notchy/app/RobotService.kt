package com.notchy.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ServiceInfo
import android.graphics.PixelFormat
import android.os.BatteryManager
import android.os.Build
import android.os.IBinder
import android.provider.Settings
import android.view.Gravity
import android.view.WindowManager

/**
 * Keeps the robot pill drawn over the front camera. A foreground service, so ColorOS is less
 * likely to stop it (the user still has to allow auto-launch and background activity).
 */
class RobotService : Service() {
    private var view: RobotView? = null
    private var senses: NotchSenses? = null
    private val windows by lazy { getSystemService(WindowManager::class.java) }

    private val batteryReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            val level = intent.getIntExtra(BatteryManager.EXTRA_LEVEL, -1)
            val scale = intent.getIntExtra(BatteryManager.EXTRA_SCALE, 100)
            val status = intent.getIntExtra(BatteryManager.EXTRA_STATUS, -1)
            val charging = status == BatteryManager.BATTERY_STATUS_CHARGING ||
                status == BatteryManager.BATTERY_STATUS_FULL
            if (level >= 0) {
                val percent = level * 100 / scale.coerceAtLeast(1)
                view?.setBattery(percent, charging)
                // Low battery, said once per discharge.
                if (!charging && percent <= 15 && !warnedLow) {
                    warnedLow = true
                    say("Feed me ⚡ $percent%")
                }
                if (charging || percent > 20) warnedLow = false
            }
        }
    }

    private var warnedLow = false
    private var lastChatter = 0L

    /** The phone's own moments: unlock, headphones. */
    private val phoneReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            when (intent.action) {
                Intent.ACTION_USER_PRESENT -> {
                    val hour = java.util.Calendar.getInstance().get(java.util.Calendar.HOUR_OF_DAY)
                    val missed = PhoneMemory.since(context, System.currentTimeMillis() - 30 * 60_000).size
                    // The morning brief from the PC, once, on the first unlock of the morning.
                    val prefs = context.getSharedPreferences("app", MODE_PRIVATE)
                    val brief = prefs.getString("brief", null)
                    val fresh = System.currentTimeMillis() - prefs.getLong("brief_at", 0) < 12 * 3_600_000L
                    if (hour in 5..11 && brief != null && fresh && !prefs.getBoolean("brief_shown", false)) {
                        prefs.edit().putBoolean("brief_shown", true).apply()
                        say("☀ $brief")
                        view?.yawn()
                        return
                    }
                    if (hour >= 12) prefs.edit().putBoolean("brief_shown", false).apply()
                    say(
                        when {
                            missed >= 3 -> "$missed new alerts"
                            hour in 5..10 -> "Good morning!"
                            hour >= 23 || hour < 5 -> "Bedtime soon?"
                            else -> "Hi again!"
                        },
                    )
                }
                Intent.ACTION_HEADSET_PLUG -> if (intent.getIntExtra("state", 0) == 1) say("Tunes time ♪")
            }
        }
    }

    /** A message arrived: a quick glance, at most every 20 seconds so it never nags. */
    fun noticeAlert(app: String) {
        val now = System.currentTimeMillis()
        if (now - lastChatter < 20_000) return
        lastChatter = now
        say(app.take(14))
    }

    /** The phone's player started a new song. */
    fun nowPlaying(title: String) {
        if (title.isNotEmpty()) say("♪ ${title.take(18)}")
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        startAsForeground()
        if (Settings.canDrawOverlays(this)) addRobot()
        registerReceiver(batteryReceiver, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        registerReceiver(phoneReceiver, IntentFilter().apply {
            addAction(Intent.ACTION_USER_PRESENT)
            addAction(Intent.ACTION_HEADSET_PLUG)
        })
        running = true
        instance = this
        getSharedPreferences("app", MODE_PRIVATE).edit().putBoolean("robot", true).apply()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int = START_STICKY

    /** Narrow, determined eyes while a Focus session runs on the PC. */
    fun setFocused(on: Boolean) {
        view?.focused = on
    }

    /** A live timer, ride or delivery in the pill, or `null` to clear it. */
    fun setActivity(activity: RobotView.LiveActivity?) {
        view?.post { view?.activity = activity }
    }

    fun setRinging(on: Boolean) {
        view?.post { view?.ringing = on }
    }

    /** Eyes glow the colour of the app that notified. */
    fun flash(packageName: String) {
        NotchSenses.colorFor(packageName)?.let { color -> view?.post { view?.flash(color) } }
    }

    fun setWear(wear: String) {
        getSharedPreferences("notch", MODE_PRIVATE).edit().putString("wear", wear).apply()
        view?.post { view?.wear = wear.ifEmpty { null } }
    }

    fun startGame() {
        view?.post { view?.startGame() }
    }

    fun setSpeedMeter(on: Boolean) {
        getSharedPreferences("notch", MODE_PRIVATE).edit().putBoolean("speed", on).apply()
        if (on) senses?.startSpeed() else senses?.stopSpeed()
    }

    /** The PC robot's unlocked accessories and Focus streak, shown as badges on the pill. */
    fun setBadges(unlocked: Int, streak: Int) {
        view?.post {
            view?.badges = unlocked
            view?.streak = streak
        }
    }

    /** Thrown over to the PC from the quick panel. */
    fun throwToPc() {
        view?.let { robot ->
            robot.throwNow()
            robot.postDelayed({
                robot.say(if (Link.send("throw")) "Caught by ${Link.pcName ?: "PC"}" else "PC not linked")
            }, 900)
        }
    }

    /** The robot says [text] in the pill for a few seconds. */
    fun say(text: String) {
        view?.say(text)
    }

    override fun onDestroy() {
        running = false
        instance = null
        unregisterReceiver(batteryReceiver)
        unregisterReceiver(phoneReceiver)
        senses?.stop()
        senses = null
        view?.let { windows.removeView(it) }
        view = null
        super.onDestroy()
    }

    private fun startAsForeground() {
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, "Robot", NotificationManager.IMPORTANCE_MIN),
        )
        val notification = Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_notchy)
            .setContentTitle("Notchy robot is awake")
            .setOngoing(true)
            .build()
        if (Build.VERSION.SDK_INT >= 34) {
            startForeground(ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        } else {
            startForeground(ID, notification)
        }
    }

    /** Places the pill centred on the camera cut-out, read from the display. */
    private fun addRobot() {
        val density = resources.displayMetrics.density
        var holeX = resources.displayMetrics.widthPixels / 2
        var holeY = (14 * density).toInt()
        var holeSize = 28f * density
        if (Build.VERSION.SDK_INT >= 30) {
            windows.currentWindowMetrics.windowInsets.displayCutout?.boundingRectTop?.let { cut ->
                if (!cut.isEmpty) {
                    holeX = cut.centerX()
                    holeY = cut.centerY()
                    holeSize = cut.height().toFloat().coerceIn(18f * density, 48f * density)
                }
            }
        }
        val robot = RobotView(this, holeSize)
        robot.onPet = { RobotLife.pet(this) }
        robot.onSwipe = { direction ->
            NotifyListener.instance?.command(if (direction > 0) "next" else "prev")
            robot.say(if (direction > 0) "Next ⏭" else "Back ⏮", 1200)
        }
        robot.onDoubleTap = {
            if (robot.ringing) {
                // Silence the ringer for this call.
                getSystemService(android.media.AudioManager::class.java)
                    .adjustStreamVolume(android.media.AudioManager.STREAM_RING, android.media.AudioManager.ADJUST_MUTE, 0)
                robot.ringing = false
                robot.say("Shh 🤫", 1500)
            } else {
                senses?.togglePrivacy()
            }
        }
        robot.onFeed = { senses?.feed() }
        robot.onScreenshotTap = { senses?.sendScreenshot() == true }
        getSharedPreferences("notch", MODE_PRIVATE).getString("wear", null)?.let { robot.wear = it.ifEmpty { null } }
        RobotLife.get(this).let { life ->
            robot.badges = life.unlocked
            robot.streak = life.streak
        }
        // Long-press: the quick panel (timer, flashlight, note, throw to PC).
        robot.onThrow = {
            startActivity(Intent(this, QuickPanelActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
        val width = robot.maxWidth.toInt()
        val height = (robot.pillH * 1.6f).toInt()
        val params = WindowManager.LayoutParams(
            width,
            height,
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or
                WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN or
                WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
            PixelFormat.TRANSLUCENT,
        ).apply {
            gravity = Gravity.TOP or Gravity.START
            x = holeX - width / 2
            y = holeY - height / 2
            if (Build.VERSION.SDK_INT >= 30) {
                layoutInDisplayCutoutMode =
                    WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS
            } else if (Build.VERSION.SDK_INT >= 28) {
                layoutInDisplayCutoutMode =
                    WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES
            }
        }
        windows.addView(robot, params)
        view = robot
        senses = NotchSenses(this, robot, windows).also { it.start() }
        // A morning yawn on the first wake of the day.
        val hour = java.util.Calendar.getInstance().get(java.util.Calendar.HOUR_OF_DAY)
        if (hour in 5..10) robot.postDelayed({ robot.yawn() }, 1500)
    }

    companion object {
        private const val CHANNEL = "robot"
        private const val ID = 7
        var running = false
            private set
        var instance: RobotService? = null
            private set

        /** Stopped by the user: don't bring it back after a restart. */
        fun stop(context: Context) {
            context.getSharedPreferences("app", MODE_PRIVATE).edit().putBoolean("robot", false).apply()
            context.stopService(Intent(context, RobotService::class.java))
        }
    }
}
