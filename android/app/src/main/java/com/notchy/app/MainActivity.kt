package com.notchy.app

import android.Manifest
import android.app.Activity
import android.app.AlertDialog
import android.content.ClipboardManager
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.WindowInsets
import android.view.inputmethod.EditorInfo
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.Toast
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning

/**
 * The phone app: four pages under a bottom bar.
 * - Home: the linked PC, quick actions, and anything still to set up.
 * - Remote: touchpad, media, volume and slide keys, and typing into the PC.
 * - Robot: the camera robot and OPPO's Fluid Cloud.
 * - Setup: every permission with its state, the link, and help.
 */
class MainActivity : Activity() {
    private enum class Page(val title: String, val glyph: String) {
        HOME("Home", "⌂"),
        REMOTE("Remote", "◎"),
        ROBOT("Robot", "◉"),
        SETUP("Setup", "⚙"),
    }

    private var page = Page.HOME
    private lateinit var content: FrameLayout
    private lateinit var tabs: LinearLayout
    private val linkListener: () -> Unit = { if (page != Page.REMOTE) render() }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.statusBarColor = Ui.BG
        window.navigationBarColor = Ui.CARD
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(Ui.BG)
        }
        content = FrameLayout(this)
        tabs = row().apply {
            setBackgroundColor(Ui.CARD)
            setPadding(dp(8), dp(6), dp(8), dp(6))
        }
        root.addView(content, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))
        root.addView(tabs, matchWidth())
        root.setOnApplyWindowInsetsListener { view, insets ->
            val bars = if (Build.VERSION.SDK_INT >= 30) {
                insets.getInsets(WindowInsets.Type.systemBars())
            } else {
                null
            }
            view.setPadding(0, bars?.top ?: 0, 0, bars?.bottom ?: 0)
            insets
        }
        setContentView(root)
        handleIntent(intent)
        LinkService.start(this)
        render()
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleIntent(intent)
    }

    override fun onResume() {
        super.onResume()
        Link.addListener(linkListener)
        render()
    }

    override fun onPause() {
        Link.removeListener(linkListener)
        super.onPause()
    }

    /** `notchy://pair?…` links (from the PC's QR) pair straight away. */
    private fun handleIntent(intent: Intent?) {
        val data = intent?.data ?: return
        pairWith(data.toString())
    }

    private fun pairWith(code: String) {
        if (Link.pair(this, code)) {
            LinkService.start(this)
            Toast.makeText(this, "Linked! Connecting to your PC…", Toast.LENGTH_SHORT).show()
            page = Page.HOME
            render()
        } else {
            Toast.makeText(this, "That is not a Notchy pairing code", Toast.LENGTH_LONG).show()
        }
    }

    // ---- Shell ---------------------------------------------------------------------------

    private fun render() {
        content.removeAllViews()
        val body = when (page) {
            Page.HOME -> homePage()
            Page.REMOTE -> remotePage()
            Page.ROBOT -> robotPage()
            Page.SETUP -> setupPage()
        }
        content.addView(body, fill())
        renderTabs()
    }

    private fun renderTabs() {
        tabs.removeAllViews()
        for (entry in Page.entries) {
            val selected = entry == page
            val item = column(2).apply {
                gravity = Gravity.CENTER
                setPadding(0, dp(6), 0, dp(6))
                background = if (selected) rounded(Ui.CARD2, dp(16).toFloat()) else null
                addView(label(entry.glyph, 18f, if (selected) Ui.ACCENT else Ui.DIM).apply { gravity = Gravity.CENTER })
                addView(label(entry.title, 12f, if (selected) Ui.TEXT else Ui.DIM, bold = selected).apply { gravity = Gravity.CENTER })
                setOnClickListener {
                    page = entry
                    render()
                }
            }
            tabs.addView(item, weighted())
        }
    }

    /** A scrolling page with a big title. */
    private fun page(title: String, subtitle: String, build: LinearLayout.() -> Unit): View {
        val column = column(12).apply {
            setPadding(dp(18), dp(20), dp(18), dp(28))
            addView(label(title, 30f, Ui.TEXT, bold = true))
            addView(label(subtitle, 14f, Ui.DIM).apply { setPadding(0, 0, 0, dp(6)) })
            build()
        }
        return ScrollView(this).apply {
            isFillViewport = true
            addView(column)
        }
    }

    // ---- Home ----------------------------------------------------------------------------

    private fun homePage() = page("Notchy", "Your PC's robot, on your phone") {
        val paired = Link.isPaired(this@MainActivity)
        addView(pcCard(paired), matchWidth())
        if (paired) {
            addView(sectionTitle("Quick actions"))
            val grid = column(10)
            val first = row(10)
            first.addView(tile("⇪", "Send clipboard") { sendClipboard() }, weighted())
            first.addView(tile("♪", "Ring PC") { act(Link.send("find_pc"), "Your PC is chiming") }, weighted())
            val second = row(10)
            second.addView(tile("⊘", "Lock PC") { act(Link.send("lock_pc"), "PC locked") }, weighted())
            second.addView(tile("↑", "Throw robot") { act(Link.send("throw"), "Caught by ${Link.pcName}") }, weighted())
            grid.addView(first, matchWidth())
            grid.addView(second, matchWidth())
            addView(grid, matchWidth())
            Link.pcMedia?.let { media -> addView(pcMediaCard(media), matchWidth()) }
        }
        val missing = permissions().filter { !it.ok }
        if (missing.isNotEmpty()) {
            addView(sectionTitle("Finish setting up"))
            val card = card(8)
            missing.forEach { card.addView(settingRow(it.title, it.detail, "Set up", false, it.fix)) }
            addView(card, matchWidth())
        }
    }

    private fun pcCard(paired: Boolean) = card(20).apply {
        if (!paired) {
            addView(label("Link your PC", 20f, Ui.TEXT, bold = true))
            addView(label("On the PC open Notchy → Phone and scan the code. Clipboard, alerts, music, a touchpad and the robot then flow between both.", 14f, Ui.DIM))
            addView(pill("Scan the PC's code", primary = true) { scan() }, matchWidth())
            addView(pill("Paste a code instead") { pasteCode() }, matchWidth())
            return@apply
        }
        val connected = Link.connected
        val top = row(12)
        val badge = label("▭", 22f, Ui.TEXT).apply {
            gravity = Gravity.CENTER
            background = rounded(Ui.CARD2, dp(14).toFloat())
        }
        top.addView(badge, LinearLayout.LayoutParams(dp(48), dp(48)))
        val names = column(4)
        names.addView(label(Link.pcName ?: "Your PC", 19f, Ui.TEXT, bold = true))
        val status = row(8)
        status.addView(dot(if (connected) Ui.MINT else Ui.FAINT))
        val line = when {
            !connected -> "Looking for it on Wi-Fi…"
            Link.pcBattery >= 0 && Link.pcCharging -> "Linked · PC ${Link.pcBattery}% charging"
            Link.pcBattery >= 0 -> "Linked · PC ${Link.pcBattery}%"
            else -> "Linked"
        }
        status.addView(label(line, 13f, Ui.DIM))
        names.addView(status)
        top.addView(names, weighted())
        addView(top)
        if (connected && Link.pcAway) {
            addView(label("You stepped away from the PC — the robot came with you.", 13f, Ui.MINT))
        }
        if (!connected) {
            addView(label("Make sure Notchy is running on the PC and both are on the same Wi-Fi.", 13f, Ui.DIM))
        }
    }

    private fun pcMediaCard(media: Triple<String, String, Boolean>) = card(16).apply {
        addView(label("Playing on ${Link.pcName}", 12f, Ui.DIM, bold = true))
        addView(label(media.first, 17f, Ui.TEXT, bold = true))
        if (media.second.isNotEmpty()) addView(label(media.second, 14f, Ui.DIM))
        val controls = row(10)
        controls.addView(pill("⏮") { Link.send("media_cmd") { put("cmd", "prev") } }, weighted())
        controls.addView(pill(if (media.third) "⏸" else "▶", primary = true) { Link.send("media_cmd") { put("cmd", "toggle") } }, weighted())
        controls.addView(pill("⏭") { Link.send("media_cmd") { put("cmd", "next") } }, weighted())
        addView(controls, matchWidth())
    }

    private fun act(sent: Boolean, done: String) {
        Toast.makeText(this, if (sent) done else "PC not connected", Toast.LENGTH_SHORT).show()
    }

    private fun sendClipboard() {
        val text = getSystemService(ClipboardManager::class.java).primaryClip
            ?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(this)?.toString().orEmpty()
        if (text.isEmpty()) {
            Toast.makeText(this, "Clipboard is empty", Toast.LENGTH_SHORT).show()
        } else {
            act(ClipSendActivity.sendText(text), "Sent to ${Link.pcName}")
        }
    }

    private fun scan() {
        val options = GmsBarcodeScannerOptions.Builder()
            .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
            .build()
        GmsBarcodeScanning.getClient(this, options).startScan()
            .addOnSuccessListener { barcode -> barcode.rawValue?.let(::pairWith) }
            .addOnFailureListener {
                Toast.makeText(this, "Scanner unavailable — use Paste a code", Toast.LENGTH_LONG).show()
            }
    }

    private fun pasteCode() {
        val field = EditText(this).apply {
            hint = "notchy://pair?…"
            setText(
                getSystemService(ClipboardManager::class.java).primaryClip
                    ?.getItemAt(0)?.text?.toString()?.takeIf { it.startsWith("notchy://") } ?: "",
            )
        }
        AlertDialog.Builder(this)
            .setTitle("Paste the pairing code")
            .setView(FrameLayout(this).apply {
                setPadding(dp(20), dp(8), dp(20), 0)
                addView(field)
            })
            .setPositiveButton("Link") { _, _ -> pairWith(field.text.toString()) }
            .setNegativeButton("Cancel", null)
            .show()
    }

    // ---- Remote --------------------------------------------------------------------------

    private fun remotePage(): View {
        if (!Link.connected) {
            return page("Remote", "Control the PC from here") {
                addView(card(20).apply {
                    addView(label("Not connected", 18f, Ui.TEXT, bold = true))
                    addView(label("The remote works once your PC is linked and nearby.", 14f, Ui.DIM))
                }, matchWidth())
            }
        }
        val column = column(12).apply {
            setPadding(dp(18), dp(20), dp(18), dp(18))
            addView(label("Remote", 30f, Ui.TEXT, bold = true))
            addView(label("Controlling ${Link.pcName}", 14f, Ui.DIM))
        }
        column.addView(TouchpadView(this), LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))
        val clicks = row(10)
        clicks.addView(pill("Left click") { Link.send("click") { put("b", "left") } }, weighted())
        clicks.addView(pill("Right click") { Link.send("click") { put("b", "right") } }, weighted())
        column.addView(clicks, matchWidth())
        column.addView(keyRow("⏮" to "prev", "⏯" to "play", "⏭" to "next", "🔉" to "vol_down", "🔇" to "mute", "🔊" to "vol_up"), matchWidth())
        column.addView(keyRow("◀" to "left", "▶" to "right", "␣" to "space", "⌫" to "back", "⏎" to "enter", "Esc" to "esc"), matchWidth())
        val typing = row(10)
        val field = EditText(this).apply {
            hint = "Type on the PC…"
            setTextColor(Ui.TEXT)
            setHintTextColor(Ui.FAINT)
            background = rounded(Ui.CARD2, dp(25).toFloat())
            setPadding(dp(18), dp(12), dp(18), dp(12))
            isSingleLine = true
            imeOptions = EditorInfo.IME_ACTION_SEND
        }
        val send = {
            val text = field.text.toString()
            if (text.isNotEmpty()) {
                Link.send("type") { put("text", text) }
                field.setText("")
            }
        }
        field.setOnEditorActionListener { _, _, _ -> send(); true }
        typing.addView(field, weighted())
        typing.addView(pill("Send", primary = true) { send() })
        column.addView(typing, matchWidth())
        return column
    }

    private fun keyRow(vararg keys: Pair<String, String>) = row(8).apply {
        for ((glyph, key) in keys) {
            addView(label(glyph, 17f, Ui.TEXT, bold = true).apply {
                gravity = Gravity.CENTER
                minHeight = dp(46)
                background = pressable(Ui.CARD2, dp(16).toFloat())
                setOnClickListener {
                    Link.send("key") { put("k", key) }
                    performHapticFeedback(android.view.HapticFeedbackConstants.KEYBOARD_TAP)
                }
            }, weighted())
        }
    }

    // ---- Robot ---------------------------------------------------------------------------

    private fun robotPage() = page("Robot", "Lives around your front camera") {
        val overlay = Settings.canDrawOverlays(this@MainActivity)
        addView(card(20).apply {
            addView(label(if (RobotService.running) "The robot is awake" else "The robot is asleep", 19f, Ui.TEXT, bold = true))
            addView(label("Tap it to see the time and battery. Long-press to throw it to your PC. It tells you when the PC copies something, rings, or when you walk away.", 14f, Ui.DIM))
            if (!overlay) {
                addView(pill("Allow drawing over apps", primary = true) {
                    startActivity(Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:$packageName")))
                }, matchWidth())
                addView(label("Greyed out? App info → ⋮ → Allow restricted settings, then try again.", 13f, Ui.FAINT))
            } else {
                addView(pill(if (RobotService.running) "Put it to sleep" else "Wake the robot", primary = !RobotService.running) {
                    if (RobotService.running) {
                        RobotService.stop(this@MainActivity)
                    } else {
                        startForegroundService(Intent(this@MainActivity, RobotService::class.java))
                    }
                    content.postDelayed({ render() }, 300)
                }, matchWidth())
            }
        }, matchWidth())
        addView(sectionTitle("OPPO Fluid Cloud"))
        addView(card(20).apply {
            addView(label("Live events in OPPO's notch", 17f, Ui.TEXT, bold = true))
            addView(label("Focus sessions from the PC and transfers show as Live Updates, which ColorOS puts in the Fluid Cloud bubble.", 14f, Ui.DIM))
            addView(pill("Send a test") { LiveUpdates.startDemo(this@MainActivity) }, matchWidth())
            addView(pill("Live update settings") {
                startActivity(Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, packageName))
            }, matchWidth())
        }, matchWidth())
    }

    // ---- Setup ---------------------------------------------------------------------------

    private class Permission(val title: String, val detail: String, val ok: Boolean, val fix: () -> Unit)

    private fun permissions(): List<Permission> {
        val notifications = Build.VERSION.SDK_INT < 33 ||
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED
        return listOf(
            Permission("Notifications", "Live updates, ringing, the PC's music", notifications) {
                if (Build.VERSION.SDK_INT >= 33) requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
            },
            Permission("Notification access", "Mirror phone alerts and music to the PC", NotifyListener.allowed(this)) {
                startActivity(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS))
            },
            Permission("Draw over apps", "The robot around the camera", Settings.canDrawOverlays(this)) {
                startActivity(Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:$packageName")))
            },
        )
    }

    private fun setupPage() = page("Setup", "Everything Notchy needs, in one place") {
        addView(sectionTitle("Permissions"))
        val perms = card(8)
        permissions().forEach { perms.addView(settingRow(it.title, it.detail, if (it.ok) "On" else "Set up", it.ok, it.fix)) }
        perms.addView(settingRow("Keep running", "ColorOS: allow background activity and auto launch", "Open", true) {
            startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:$packageName")))
        })
        addView(perms, matchWidth())

        addView(sectionTitle("Link"))
        val link = card(8)
        val paired = Link.isPaired(this@MainActivity)
        if (paired) {
            link.addView(settingRow(Link.pcName ?: "Your PC", if (Link.connected) "Connected now" else "Not connected", if (Link.connected) "Linked" else "Away", Link.connected) {})
            link.addView(settingRow("Link a different PC", "Scan a new code from Notchy → Phone", "Scan", true) { scan() })
            link.addView(settingRow("Unlink", "This phone forgets the PC", "Unlink", false) {
                AlertDialog.Builder(this@MainActivity)
                    .setTitle("Unlink ${Link.pcName}?")
                    .setPositiveButton("Unlink") { _, _ ->
                        Link.unpair(this@MainActivity)
                        stopService(Intent(this@MainActivity, LinkService::class.java))
                        render()
                    }
                    .setNegativeButton("Cancel", null)
                    .show()
            })
        } else {
            link.addView(settingRow("No PC linked", "Open Notchy → Phone on the PC", "Scan", false) { scan() })
        }
        addView(link, matchWidth())

        addView(sectionTitle("Tips"))
        addView(card(18).apply {
            addView(label("• Add the “Clipboard → PC” tile to Quick Settings for one-tap sending.", 14f, Ui.DIM))
            addView(label("• Share any link with Notchy and it opens on the PC.", 14f, Ui.DIM))
            addView(label("• Share photos with “Send to PC” for full-size transfers via Drop.", 14f, Ui.DIM))
        }, matchWidth())
        addView(label("Notchy ${packageManager.getPackageInfo(packageName, 0).versionName}", 12f, Ui.FAINT).apply {
            gravity = Gravity.CENTER
            setPadding(0, dp(12), 0, 0)
        }, matchWidth())
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        render()
    }
}
