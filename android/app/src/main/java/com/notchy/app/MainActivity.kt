package com.notchy.app

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Color
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.view.Gravity
import android.view.View
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView

/** A setup screen: each permission Notchy needs, and the two experiments. */
class MainActivity : Activity() {
    private lateinit var status: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val pad = (20 * resources.displayMetrics.density).toInt()
        val column = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(pad, pad * 3, pad, pad)
            setBackgroundColor(Color.rgb(16, 16, 20))
        }
        column.addView(text("Notchy", 28f, true))
        column.addView(text("Your PC's notch robot, now on the phone.", 15f, false))
        status = text("", 14f, false).apply { setPadding(0, pad, 0, pad) }
        column.addView(status)

        column.addView(heading("1 · OPPO's notch (Fluid Cloud)"))
        column.addView(button("Allow notifications") { askNotifications() })
        column.addView(button("Test: send a live update") { LiveUpdates.startDemo(this) })
        column.addView(hint("A fake 30 s file transfer. Watch the bubble around the camera. If it only shows in the shade, open the next setting and turn on Live updates / Fluid Cloud for Notchy."))
        column.addView(button("Notchy notification settings") {
            startActivity(
                Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                    .putExtra(Settings.EXTRA_APP_PACKAGE, packageName),
            )
        })

        column.addView(heading("2 · The robot around the camera"))
        column.addView(button("Allow drawing over apps") {
            startActivity(
                Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:$packageName")),
            )
        })
        column.addView(button("Start / stop the robot") { toggleRobot() })
        column.addView(hint("Tap the robot: it hops and shows the time and battery. Plug in the charger and it shows a bolt."))

        column.addView(heading("3 · Keep it alive on ColorOS"))
        column.addView(hint("App info → Battery → allow background activity, and turn on Auto launch. Otherwise ColorOS stops the robot after a while."))
        column.addView(button("Open Notchy app info") {
            startActivity(
                Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:$packageName")),
            )
        })

        setContentView(ScrollView(this).apply { addView(column) })
    }

    override fun onResume() {
        super.onResume()
        refresh()
    }

    private fun refresh() {
        val notifications = Build.VERSION.SDK_INT < 33 ||
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED
        val overlay = Settings.canDrawOverlays(this)
        status.text = buildString {
            append("Android ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT})\n")
            append(if (Build.VERSION.SDK_INT >= 36) "✓ Live Updates supported\n" else "✗ Live Updates need Android 16\n")
            append(if (notifications) "✓ Notifications allowed\n" else "✗ Notifications not allowed\n")
            append(if (overlay) "✓ Can draw over apps\n" else "✗ Cannot draw over apps yet\n")
            append(if (RobotService.running) "● Robot running" else "○ Robot stopped")
        }
    }

    private fun askNotifications() {
        if (Build.VERSION.SDK_INT >= 33) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        refresh()
    }

    private fun toggleRobot() {
        val intent = Intent(this, RobotService::class.java)
        if (RobotService.running) {
            stopService(intent)
        } else if (!Settings.canDrawOverlays(this)) {
            status.text = "Allow drawing over apps first."
            return
        } else {
            startForegroundService(intent)
        }
        status.postDelayed({ refresh() }, 300)
    }

    private fun text(value: String, size: Float, bold: Boolean) = TextView(this).apply {
        text = value
        textSize = size
        setTextColor(Color.WHITE)
        if (bold) setTypeface(typeface, android.graphics.Typeface.BOLD)
    }

    private fun heading(value: String) = text(value, 17f, true).apply {
        setPadding(0, (18 * resources.displayMetrics.density).toInt(), 0, 6)
    }

    private fun hint(value: String) = text(value, 13f, false).apply {
        setTextColor(Color.rgb(170, 170, 180))
        setPadding(0, 4, 0, 8)
    }

    private fun button(label: String, action: () -> Unit) = Button(this).apply {
        text = label
        isAllCaps = false
        gravity = Gravity.CENTER
        setOnClickListener(View.OnClickListener { action() })
    }
}
