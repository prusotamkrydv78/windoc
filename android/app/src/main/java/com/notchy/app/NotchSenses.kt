package com.notchy.app

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.database.ContentObserver
import android.graphics.Color
import android.graphics.PixelFormat
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.hardware.camera2.CameraManager
import android.media.AudioManager
import android.media.AudioRecordingConfiguration
import android.media.audiofx.Visualizer
import android.net.TrafficStats
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import android.view.Gravity
import android.view.View
import android.view.WindowInsets
import android.view.WindowManager
import android.widget.TextView
import kotlin.math.sqrt

/**
 * Everything the camera robot notices on the phone, wired to [RobotView] by [RobotService]:
 * music loudness, camera and microphone use, screenshots, shakes, the phone in a pocket,
 * full-screen apps, network speed, plus the privacy screen and per-app colours.
 */
class NotchSenses(private val context: Context, private val robot: RobotView, private val windows: WindowManager) {
    private val main = Handler(Looper.getMainLooper())
    private val prefs = context.getSharedPreferences("notch", Context.MODE_PRIVATE)

    fun start() {
        startMusic()
        startPrivacyDot()
        startScreenshots()
        startMotion()
        startFullscreenWatch()
        if (prefs.getBoolean("speed", false)) startSpeed()
    }

    fun stop() {
        visualizer?.release()
        visualizer = null
        context.getSystemService(SensorManager::class.java).unregisterListener(motion)
        context.contentResolver.unregisterContentObserver(screenshots)
        camera?.let { context.getSystemService(CameraManager::class.java).unregisterAvailabilityCallback(it) }
        recording?.let { context.getSystemService(AudioManager::class.java).unregisterAudioRecordingCallback(it) }
        detector?.let { runCatching { windows.removeView(it) } }
        hidePrivacy()
        main.removeCallbacksAndMessages(null)
    }

    // ---- Music: eyes bounce to the beat ----------------------------------------------------

    private var visualizer: Visualizer? = null

    private fun startMusic() {
        if (context.checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) return
        visualizer = runCatching {
            Visualizer(0).apply {
                captureSize = Visualizer.getCaptureSizeRange()[0]
                setDataCaptureListener(object : Visualizer.OnDataCaptureListener {
                    override fun onWaveFormDataCapture(v: Visualizer, wave: ByteArray, rate: Int) {
                        var sum = 0.0
                        for (byte in wave) {
                            val value = (byte.toInt() and 0xff) - 128
                            sum += value * value
                        }
                        val level = (sqrt(sum / wave.size) / 40.0).toFloat().coerceIn(0f, 1f)
                        main.post { robot.beat = if (context.getSystemService(AudioManager::class.java).isMusicActive) level else 0f }
                    }

                    override fun onFftDataCapture(v: Visualizer, fft: ByteArray, rate: Int) {}
                }, Visualizer.getMaxCaptureRate() / 2, true, false)
                enabled = true
            }
        }.getOrNull()
    }

    // ---- Camera and microphone in use ------------------------------------------------------

    private var camera: CameraManager.AvailabilityCallback? = null
    private var recording: AudioManager.AudioRecordingCallback? = null
    private val busyCameras = mutableSetOf<String>()
    private var micBusy = false

    private fun updateDot() {
        robot.privacyDot = when {
            busyCameras.isNotEmpty() -> Color.rgb(255, 69, 58)
            micBusy -> Color.rgb(255, 159, 10)
            else -> 0
        }
    }

    private fun startPrivacyDot() {
        val cameras = context.getSystemService(CameraManager::class.java)
        camera = object : CameraManager.AvailabilityCallback() {
            override fun onCameraUnavailable(id: String) {
                // Our own torch does not count.
                if (Torch.on) return
                if (busyCameras.add(id) && busyCameras.size == 1) robot.say("Camera in use")
                updateDot()
            }

            override fun onCameraAvailable(id: String) {
                busyCameras.remove(id)
                updateDot()
            }
        }.also { cameras.registerAvailabilityCallback(it, main) }
        val audio = context.getSystemService(AudioManager::class.java)
        recording = object : AudioManager.AudioRecordingCallback() {
            override fun onRecordingConfigChanged(configs: MutableList<AudioRecordingConfiguration>) {
                val busy = configs.isNotEmpty()
                if (busy && !micBusy) robot.say("Mic in use")
                micBusy = busy
                updateDot()
            }
        }.also { audio.registerAudioRecordingCallback(it, main) }
    }

    // ---- Screenshots: caught and offered to the PC ------------------------------------------

    private var pending: Uri? = null
    private var lastShot = 0L

    private val screenshots = object : ContentObserver(main) {
        override fun onChange(selfChange: Boolean, uri: Uri?) {
            uri ?: return
            val path = runCatching {
                context.contentResolver.query(uri, arrayOf(MediaStore.Images.Media.RELATIVE_PATH, MediaStore.Images.Media.DISPLAY_NAME), null, null, null)
                    ?.use { if (it.moveToFirst()) "${it.getString(0)}${it.getString(1)}" else null }
            }.getOrNull() ?: return
            val now = System.currentTimeMillis()
            if (!path.contains("screenshot", ignoreCase = true) || now - lastShot < 3000) return
            lastShot = now
            pending = uri
            robot.say(if (Link.connected) "Tap me → send to PC" else "Nice screenshot!", 5000)
            main.postDelayed({ pending = null }, 6000)
        }
    }

    private fun startScreenshots() {
        if (!PhotoMemories.allowed(context)) return
        context.contentResolver.registerContentObserver(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, true, screenshots)
    }

    /** A tap right after a screenshot sends it. `true` when it did. */
    fun sendScreenshot(): Boolean {
        val uri = pending ?: return false
        pending = null
        if (!Link.connected) return false
        ShareActivity.sendOverLink(context.applicationContext, listOf(uri))
        robot.throwNow()
        robot.say("Sent to ${Link.pcName}")
        return true
    }

    // ---- Shake (dizzy) and pocket (sleep) --------------------------------------------------

    private var faceDown = false
    private var near = false
    private var lastShake = 0L

    private val motion = object : SensorEventListener {
        override fun onSensorChanged(event: SensorEvent) {
            when (event.sensor.type) {
                Sensor.TYPE_ACCELEROMETER -> {
                    val (x, y, z) = Triple(event.values[0], event.values[1], event.values[2])
                    val force = sqrt(x * x + y * y + z * z)
                    val now = System.currentTimeMillis()
                    if (force > 28f && now - lastShake > 2500) {
                        lastShake = now
                        robot.dizzy()
                        robot.say("Whoa, dizzy!", 1800)
                    }
                    faceDown = z < -8f
                }
                Sensor.TYPE_PROXIMITY -> near = event.values[0] < event.sensor.maximumRange
            }
            val pocket = faceDown && near
            if (pocket != robot.pocketed) robot.pocketed = pocket
        }

        override fun onAccuracyChanged(sensor: Sensor, accuracy: Int) {}
    }

    private fun startMotion() {
        val sensors = context.getSystemService(SensorManager::class.java)
        sensors.getDefaultSensor(Sensor.TYPE_ACCELEROMETER)?.let { sensors.registerListener(motion, it, SensorManager.SENSOR_DELAY_UI) }
        sensors.getDefaultSensor(Sensor.TYPE_PROXIMITY)?.let { sensors.registerListener(motion, it, SensorManager.SENSOR_DELAY_NORMAL) }
    }

    // ---- Full-screen apps: the pill fades away --------------------------------------------

    private var detector: View? = null

    /**
     * A 1-pixel-wide, full-height, untouchable window: when a game or video hides the status
     * bar, its insets say so, and the robot hides with it.
     */
    @SuppressLint("ClickableViewAccessibility")
    private fun startFullscreenWatch() {
        if (Build.VERSION.SDK_INT < 30) return
        val view = View(context)
        view.setOnApplyWindowInsetsListener { _, insets ->
            val full = !insets.isVisible(WindowInsets.Type.statusBars())
            robot.animate().alpha(if (full || robot.pocketed) 0f else 1f).setDuration(250).start()
            insets
        }
        val params = WindowManager.LayoutParams(
            1, WindowManager.LayoutParams.MATCH_PARENT,
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE,
            PixelFormat.TRANSPARENT,
        ).apply { gravity = Gravity.START or Gravity.TOP }
        runCatching { windows.addView(view, params) }.onSuccess { detector = view }
    }

    // ---- Network speed ---------------------------------------------------------------------

    private var lastBytes = -1L

    private val speedTick = object : Runnable {
        override fun run() {
            val bytes = TrafficStats.getTotalRxBytes()
            if (lastBytes >= 0 && bytes >= lastBytes) {
                val perSecond = (bytes - lastBytes) / 2
                robot.speed = when {
                    perSecond < 10_000 -> null
                    perSecond < 1_000_000 -> "${perSecond / 1000} KB/s"
                    else -> "%.1f MB/s".format(perSecond / 1_000_000f)
                }
            }
            lastBytes = bytes
            main.postDelayed(this, 2000)
        }
    }

    fun startSpeed() {
        main.removeCallbacks(speedTick)
        main.post(speedTick)
    }

    fun stopSpeed() {
        main.removeCallbacks(speedTick)
        robot.speed = null
    }

    // ---- Privacy screen --------------------------------------------------------------------

    private var privacy: View? = null

    fun togglePrivacy() {
        if (privacy != null) {
            hidePrivacy()
            return
        }
        val cover = TextView(context).apply {
            text = "Tap to show"
            setTextColor(Color.argb(120, 255, 255, 255))
            gravity = Gravity.CENTER
            setBackgroundColor(Color.argb(240, 0, 0, 0))
            setOnClickListener { hidePrivacy() }
        }
        val params = WindowManager.LayoutParams(
            WindowManager.LayoutParams.MATCH_PARENT, WindowManager.LayoutParams.MATCH_PARENT,
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN,
            PixelFormat.TRANSLUCENT,
        )
        runCatching { windows.addView(cover, params) }.onSuccess { privacy = cover }
    }

    private fun hidePrivacy() {
        privacy?.let { runCatching { windows.removeView(it) } }
        privacy = null
    }

    // ---- Feeding while charging ------------------------------------------------------------

    fun feed() {
        val fed = prefs.getInt("fed", 0) + 1
        prefs.edit().putInt("fed", fed).apply()
        val stage = listOf("tiny", "small", "growing", "big", "mighty")[(fed / 10).coerceAtMost(4)]
        robot.say("Yum! ×$fed · $stage bot")
    }

    companion object {
        /** Eye colours for apps that just notified. */
        fun colorFor(packageName: String): Int? = when {
            "whatsapp" in packageName -> Color.rgb(37, 211, 102)
            "telegram" in packageName -> Color.rgb(42, 171, 238)
            "instagram" in packageName -> Color.rgb(225, 48, 108)
            "gmail" in packageName || "android.gm" in packageName -> Color.rgb(234, 67, 53)
            "youtube" in packageName -> Color.rgb(255, 0, 0)
            "snapchat" in packageName -> Color.rgb(255, 252, 0)
            "discord" in packageName -> Color.rgb(88, 101, 242)
            "messaging" in packageName || "mms" in packageName -> Color.rgb(66, 133, 244)
            "phonepe" in packageName -> Color.rgb(95, 37, 159)
            "paytm" in packageName -> Color.rgb(0, 186, 242)
            "gpay" in packageName || "nbu.paisa" in packageName -> Color.rgb(66, 133, 244)
            "slack" in packageName -> Color.rgb(224, 30, 90)
            else -> null
        }
    }
}
