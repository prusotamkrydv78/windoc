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
    private val windows by lazy { getSystemService(WindowManager::class.java) }

    private val batteryReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            val level = intent.getIntExtra(BatteryManager.EXTRA_LEVEL, -1)
            val scale = intent.getIntExtra(BatteryManager.EXTRA_SCALE, 100)
            val status = intent.getIntExtra(BatteryManager.EXTRA_STATUS, -1)
            val charging = status == BatteryManager.BATTERY_STATUS_CHARGING ||
                status == BatteryManager.BATTERY_STATUS_FULL
            if (level >= 0) view?.setBattery(level * 100 / scale.coerceAtLeast(1), charging)
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        startAsForeground()
        if (Settings.canDrawOverlays(this)) addRobot()
        registerReceiver(batteryReceiver, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        running = true
        instance = this
        getSharedPreferences("app", MODE_PRIVATE).edit().putBoolean("robot", true).apply()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int = START_STICKY

    /** Narrow, determined eyes while a Focus session runs on the PC. */
    fun setFocused(on: Boolean) {
        view?.focused = on
    }

    /** The robot says [text] in the pill for a few seconds. */
    fun say(text: String) {
        view?.say(text)
    }

    override fun onDestroy() {
        running = false
        instance = null
        unregisterReceiver(batteryReceiver)
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
        robot.onThrow = {
            if (Link.send("throw")) {
                robot.postDelayed({ robot.say("Caught by ${Link.pcName ?: "PC"}") }, 900)
            } else {
                robot.postDelayed({ robot.say("PC not linked") }, 900)
            }
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
