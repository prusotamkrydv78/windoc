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
    private companion object {
        const val SCAN_REQUEST = 7
    }

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
            val third = row(10)
            third.addView(tile("☰", "PC clipboard") { showPcClipboard() }, weighted())
            third.addView(tile("✋", "Wave friends") {
                act(Link.send("wave_friends"), "Waved at Notchy friends nearby")
            }, weighted())
            grid.addView(first, matchWidth())
            grid.addView(second, matchWidth())
            val fourth = row(10)
            fourth.addView(tile("▣", "Stand mode") {
                startActivity(Intent(this@MainActivity, StandActivity::class.java))
            }, weighted())
            fourth.addView(tile("⌗", "Scan to PC") { scanToPc() }, weighted())
            grid.addView(third, matchWidth())
            val fifth = row(10)
            fifth.addView(tile("✦", "What did I miss?") { askRecap() }, weighted())
            fifth.addView(tile("⌕", "Timeline") { showTimeline("") }, weighted())
            grid.addView(fourth, matchWidth())
            val sixth = row(10)
            sixth.addView(tile("?", "Ask the robot") { askRobot() }, weighted())
            sixth.addView(tile("◷", "Screen time") { showScreenTime() }, weighted())
            grid.addView(fifth, matchWidth())
            val seventh = row(10)
            seventh.addView(tile("✎", "Notes") { showNotes() }, weighted())
            seventh.addView(tile("₹", "Spending") {
                AlertDialog.Builder(this@MainActivity)
                    .setTitle("Last 7 days")
                    .setMessage(Expenses.weekly(this@MainActivity))
                    .setPositiveButton("OK", null)
                    .show()
            }, weighted())
            grid.addView(sixth, matchWidth())
            grid.addView(seventh, matchWidth())
            addView(grid, matchWidth())
            Link.pcMedia?.let { media -> addView(pcMediaCard(media), matchWidth()) }
            addView(sectionTitle("Today"))
            val today = row(10)
            for ((count, name) in listOf(
                Stats.get(this@MainActivity, "files") to "Files",
                Stats.get(this@MainActivity, "clips") to "Clips",
                Stats.get(this@MainActivity, "alerts") to "Alerts",
            )) {
                today.addView(card(14).apply {
                    gravity = Gravity.CENTER
                    addView(label(count.toString(), 24f, Ui.TEXT, bold = true).apply { gravity = Gravity.CENTER })
                    addView(label(name, 12f, Ui.DIM, bold = true).apply { gravity = Gravity.CENTER })
                }, weighted())
            }
            addView(today, matchWidth())
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

    private var scanUri: Uri? = null

    /** Takes a photo with the camera app and sends it straight to the PC. */
    private fun scanToPc() {
        if (!Link.connected) {
            act(false, "")
            return
        }
        val values = android.content.ContentValues().apply {
            put(android.provider.MediaStore.Images.Media.DISPLAY_NAME, "Scan ${java.text.SimpleDateFormat("yyyy-MM-dd HH.mm.ss", java.util.Locale.US).format(java.util.Date())}.jpg")
            put(android.provider.MediaStore.Images.Media.MIME_TYPE, "image/jpeg")
            put(android.provider.MediaStore.Images.Media.RELATIVE_PATH, "Pictures/Notchy")
        }
        val uri = contentResolver.insert(android.provider.MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values) ?: return
        scanUri = uri
        val capture = Intent(android.provider.MediaStore.ACTION_IMAGE_CAPTURE)
            .putExtra(android.provider.MediaStore.EXTRA_OUTPUT, uri)
            .addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
        runCatching { startActivityForResult(capture, SCAN_REQUEST) }
            .onFailure { Toast.makeText(this, "No camera app found", Toast.LENGTH_SHORT).show() }
    }

    @Deprecated("Activity result API needs androidx; plain Activity uses this.")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        val uri = scanUri ?: return
        scanUri = null
        if (requestCode != SCAN_REQUEST) return
        if (resultCode == RESULT_OK) {
            ShareActivity.sendOverLink(applicationContext, listOf(uri))
            Toast.makeText(this, "Sending scan to ${Link.pcName}…", Toast.LENGTH_SHORT).show()
        } else {
            contentResolver.delete(uri, null, null)
        }
    }

    /** Waits up to [seconds] for an answer from the PC, then runs [show] once. */
    private fun <T> await(read: () -> T?, seconds: Long, show: (T) -> Unit) {
        var done = false
        lateinit var listener: () -> Unit
        listener = {
            val value = read()
            if (value != null && !done) {
                done = true
                Link.removeListener(listener)
                show(value)
            }
        }
        Link.addListener(listener)
        content.postDelayed({
            Link.removeListener(listener)
            if (!done) Toast.makeText(this, "The PC didn't answer", Toast.LENGTH_SHORT).show()
        }, seconds * 1000)
    }

    /** The robot on the PC sums up the last three hours of phone alerts. */
    private fun askRecap() {
        Link.clearAnswers()
        if (!Link.send("recap") { put("hours", 3) }) {
            // No PC: the phone recaps from its own memory (and its own AI key, if set).
            Toast.makeText(this, "Thinking…", Toast.LENGTH_SHORT).show()
            Thread {
                val (text, model) = PhoneMemory.recap(this, 3)
                runOnUiThread { showRecap(text, model) }
            }.start()
            return
        }
        Toast.makeText(this, "Asking the robot…", Toast.LENGTH_SHORT).show()
        await({ Link.recap }, 60) { (text, model) -> showRecap(text, model) }
    }

    private fun showRecap(text: String, model: String?) {
        AlertDialog.Builder(this)
            .setTitle("What you missed")
            .setMessage(text + "\n\n" + (model?.let { "Written by $it" } ?: "Built-in summary"))
            .setPositiveButton("OK", null)
            .show()
    }

    /** Edits the phone's own AI settings (used when the PC is away). */
    private fun editAi() {
        val ai = PhoneMemory.ai(this)
        val url = EditText(this).apply { hint = PhoneMemory.OPENROUTER; setText(ai.baseUrl); isSingleLine = true }
        val key = EditText(this).apply {
            hint = if (ai.key.isEmpty()) "API key (sk-… or sk-or-…)" else "Key saved · type to replace"
            isSingleLine = true
            inputType = android.text.InputType.TYPE_CLASS_TEXT or android.text.InputType.TYPE_TEXT_VARIATION_PASSWORD
        }
        val model = EditText(this).apply { hint = "openai/gpt-4o-mini"; setText(ai.model); isSingleLine = true }
        val presets = row(8)
        presets.addView(pill("OpenRouter") { url.setText(PhoneMemory.OPENROUTER); model.setText("openai/gpt-4o-mini") }, weighted())
        presets.addView(pill("OpenAI") { url.setText(PhoneMemory.OPENAI); model.setText("gpt-4o-mini") }, weighted())
        val form = column(8).apply {
            setPadding(dp(20), dp(8), dp(20), 0)
            addView(presets, matchWidth())
            addView(label("Base URL", 12f, Ui.DIM))
            addView(url, matchWidth())
            addView(label("API key", 12f, Ui.DIM))
            addView(key, matchWidth())
            addView(label("Model", 12f, Ui.DIM))
            addView(model, matchWidth())
        }
        AlertDialog.Builder(this)
            .setTitle("AI on the phone")
            .setView(ScrollView(this).apply { addView(form) })
            .setPositiveButton("Save") { _, _ ->
                PhoneMemory.saveAi(
                    this,
                    PhoneMemory.Ai(url.text.toString(), key.text.toString().ifBlank { ai.key }, model.text.toString()),
                )
                render()
            }
            .setNeutralButton("Remove key") { _, _ ->
                PhoneMemory.saveAi(this, ai.copy(key = ""))
                render()
            }
            .setNegativeButton("Cancel", null)
            .show()
    }

    /** A question for the robot: the PC answers from Memory, or the phone from its own log. */
    private fun askRobot() {
        val field = EditText(this).apply { hint = "When did Ravi send that file?"; isSingleLine = true }
        AlertDialog.Builder(this)
            .setTitle("Ask the robot")
            .setView(FrameLayout(this).apply {
                setPadding(dp(20), dp(8), dp(20), 0)
                addView(field)
            })
            .setPositiveButton("Ask") { _, _ ->
                val question = field.text.toString().trim()
                if (question.isEmpty()) return@setPositiveButton
                Link.clearAnswers()
                if (Link.send("ask") { put("q", question) }) {
                    await({ Link.answer }, 60) { showAnswer(question, it) }
                } else {
                    Thread {
                        val answer = PhoneMemory.ask(this, question)
                        runOnUiThread { showAnswer(question, answer) }
                    }.start()
                }
            }
            .setNegativeButton("Cancel", null)
            .show()
    }

    private fun showAnswer(question: String, answer: String) {
        AlertDialog.Builder(this).setTitle(question).setMessage(answer).setPositiveButton("OK", null).show()
    }

    /** Shared notes: the PC's Notes and the phone's quick notes, with a box to add one. */
    private fun showNotes() {
        Link.clearAnswers()
        val linked = Link.send("notes_list")
        val build: (List<Pair<String, String>>) -> Unit = { pcNotes ->
            val list = column(8).apply { setPadding(dp(16), dp(8), dp(16), dp(8)) }
            val field = EditText(this).apply { hint = "New note (saved on both)"; minLines = 2 }
            list.addView(field, matchWidth())
            fun card(title: String, detail: String, where: String) = column(2).apply {
                setPadding(dp(14), dp(10), dp(14), dp(10))
                background = rounded(Ui.CARD2, dp(14).toFloat())
                addView(label(where, 11f, Ui.ACCENT, bold = true))
                addView(label(title, 14f, Ui.TEXT, bold = true))
                if (detail.isNotEmpty()) addView(label(detail, 13f, Ui.DIM))
            }
            pcNotes.forEach { (title, detail) -> list.addView(card(title, detail, "PC"), matchWidth()) }
            Notes.all(this).takeLast(20).reversed().forEach { list.addView(card(it.take(80), "", "Phone"), matchWidth()) }
            AlertDialog.Builder(this)
                .setTitle("Notes")
                .setView(ScrollView(this).apply { addView(list) })
                .setPositiveButton("Save") { _, _ ->
                    val text = field.text.toString().trim()
                    if (text.isNotEmpty()) {
                        Notes.add(this, text)
                        Link.send("note") { put("text", text) }
                    }
                }
                .setNegativeButton("Close", null)
                .show()
        }
        if (linked) await({ Link.pcNotes }, 8, build) else build(emptyList())
    }

    /** Phone vs PC screen time today. */
    private fun showScreenTime() {
        val phone = ScreenTime.today(this)
        if (phone == null) {
            AlertDialog.Builder(this)
                .setTitle("Screen time")
                .setMessage("Allow usage access so Notchy can count your phone's screen time.")
                .setPositiveButton("Allow") { _, _ -> startActivity(Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS)) }
                .setNegativeButton("Cancel", null)
                .show()
            return
        }
        val (minutes, top) = phone
        fun hm(value: Long) = "${value / 60}h ${value % 60}m"
        val pc = if (Link.pcScreenMinutes >= 0) hm(Link.pcScreenMinutes) else "not linked"
        val apps = top.joinToString("\n") { "• ${it.first}: ${hm(it.second)}" }
        AlertDialog.Builder(this)
            .setTitle("Today: phone ${hm(minutes)} · PC $pc")
            .setMessage(if (apps.isEmpty()) "No apps yet today" else apps)
            .setPositiveButton("OK", null)
            .show()
    }

    /** Notchy Memory on the PC, searchable from here. */
    private fun showTimeline(query: String) {
        Link.clearAnswers()
        if (!Link.send("memory_search") { put("q", query) }) {
            act(false, "")
            return
        }
        await({ Link.memory }, 10) { items ->
            val list = column(6).apply { setPadding(dp(16), dp(8), dp(16), dp(8)) }
            val search = EditText(this).apply {
                hint = "Search everything"
                isSingleLine = true
                setText(query)
                imeOptions = EditorInfo.IME_ACTION_SEARCH
            }
            list.addView(search, matchWidth())
            if (items.isEmpty()) list.addView(label("Nothing yet", 14f, Ui.DIM))
            val clock = java.text.SimpleDateFormat("d MMM, h:mm a", java.util.Locale.getDefault())
            for (item in items) {
                list.addView(column(2).apply {
                    setPadding(dp(14), dp(10), dp(14), dp(10))
                    background = rounded(Ui.CARD2, dp(14).toFloat())
                    addView(label("${item.kind} · ${clock.format(java.util.Date(item.at))}", 11f, Ui.ACCENT, bold = true))
                    addView(label(if (item.app.isEmpty()) item.title else "${item.app} · ${item.title}", 14f, Ui.TEXT, bold = true))
                    if (item.detail.isNotEmpty()) addView(label(item.detail, 13f, Ui.DIM))
                }, matchWidth())
            }
            lateinit var dialog: AlertDialog
            search.setOnEditorActionListener { _, _, _ ->
                dialog.dismiss()
                showTimeline(search.text.toString())
                true
            }
            dialog = AlertDialog.Builder(this)
                .setTitle("Timeline")
                .setView(ScrollView(this).apply { addView(list) })
                .setNegativeButton("Close", null)
                .show()
        }
    }

    /** Asks the PC for its clipboard history and shows it; tapping one copies it here. */
    private fun showPcClipboard() {
        if (!Link.send("clip_history")) {
            act(false, "")
            return
        }
        var shown = false
        lateinit var listener: () -> Unit
        listener = {
            val items = Link.pcHistory
            if (items != null && !shown) {
                shown = true
                Link.removeListener(listener)
                clipDialog(items)
            }
        }
        Link.addListener(listener)
        content.postDelayed({ Link.removeListener(listener) }, 5000)
    }

    private fun clipDialog(items: List<String>) {
        if (items.isEmpty()) {
            Toast.makeText(this, "The PC's clipboard history is empty (turn it on in Notchy)", Toast.LENGTH_LONG).show()
            return
        }
        val list = column(6).apply { setPadding(dp(16), dp(8), dp(16), dp(8)) }
        val search = EditText(this).apply { hint = "Search"; isSingleLine = true }
        list.addView(search, matchWidth())
        val rows = column(6)
        list.addView(rows, matchWidth())
        lateinit var dialog: AlertDialog
        fun fill(query: String) {
            rows.removeAllViews()
            items.filter { query.isBlank() || it.contains(query, ignoreCase = true) }.take(40).forEach { item ->
                rows.addView(label(item.replace('\n', ' ').take(140), 14f, Ui.TEXT).apply {
                    setPadding(dp(14), dp(12), dp(14), dp(12))
                    background = pressable(Ui.CARD2, dp(14).toFloat())
                    setOnClickListener {
                        getSystemService(ClipboardManager::class.java)
                            .setPrimaryClip(android.content.ClipData.newPlainText("From PC", item))
                        Toast.makeText(this@MainActivity, "Copied", Toast.LENGTH_SHORT).show()
                        dialog.dismiss()
                    }
                }, matchWidth())
            }
        }
        search.addTextChangedListener(object : android.text.TextWatcher {
            override fun afterTextChanged(text: android.text.Editable?) = fill(text?.toString().orEmpty())
            override fun beforeTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) {}
            override fun onTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) {}
        })
        fill("")
        dialog = AlertDialog.Builder(this)
            .setTitle("${Link.pcName} clipboard")
            .setView(ScrollView(this).apply { addView(list) })
            .setNegativeButton("Close", null)
            .show()
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
        addView(sectionTitle("Notch extras"))
        val notch = card(8)
        val speedOn = getSharedPreferences("notch", MODE_PRIVATE).getBoolean("speed", false)
        notch.addView(settingRow("Speed meter", "Download speed next to the camera", if (speedOn) "On" else "Off", speedOn) {
            RobotService.instance?.setSpeedMeter(!speedOn)
                ?: getSharedPreferences("notch", MODE_PRIVATE).edit().putBoolean("speed", !speedOn).apply()
            render()
        })
        val mic = checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED
        notch.addView(settingRow("Eyes dance to music", "Uses the audio visualizer (needs microphone permission; nothing is recorded)", if (mic) "On" else "Allow", mic) {
            if (!mic) requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), 4)
        })
        notch.addView(label("Swipe the pill ← → for songs · swipe down or long-press for the panel · double-tap to hide the screen (or silence a call) · hold while charging to feed it · shake for a dizzy robot", 13f, Ui.DIM).apply {
            setPadding(dp(4), dp(6), dp(4), dp(6))
        })
        addView(notch, matchWidth())

        addView(sectionTitle("Robot's life"))
        val life = RobotLife.get(this@MainActivity)
        addView(card(18).apply {
            if (RobotLife.synced(this@MainActivity) == 0L) {
                addView(label("Link your PC and the robot brings its memories over: days together, Focus streak, unlocked accessories and its plant.", 14f, Ui.DIM))
            } else {
                val stats = row(10)
                for ((value, name) in listOf(
                    life.days.toString() to "Days",
                    "${life.streak}" to "Streak",
                    life.pets.toString() to "Pets",
                )) {
                    stats.addView(column(2).apply {
                        gravity = Gravity.CENTER
                        addView(label(value, 22f, Ui.TEXT, bold = true).apply { gravity = Gravity.CENTER })
                        addView(label(name, 12f, Ui.DIM, bold = true).apply { gravity = Gravity.CENTER })
                    }, weighted())
                }
                addView(stats, matchWidth())
                addView(label("Today: ${life.focusToday} Focus sessions (${life.focusMinutes} min), ${life.breaks} breaks", 14f, Ui.DIM))
                addView(label("Copied ${life.copies} things together · plant: ${RobotLife.plantName(life.plant)}", 14f, Ui.DIM))
                val owned = RobotLife.ACCESSORIES.take(life.unlocked)
                addView(label(
                    if (owned.isEmpty()) "Accessories unlock after 3, 7 and 14 days together" else "Wearing: ${owned.joinToString(", ")}",
                    14f, Ui.MINT,
                ))
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
            Permission(
                "Photos",
                "\"On this day\" photos sent to the PC once a day",
                PhotoMemories.allowed(this),
            ) {
                requestPermissions(arrayOf(Manifest.permission.READ_MEDIA_IMAGES), 2)
            },
            Permission(
                "Location (for Wi-Fi names)",
                "Lets the robot tell home from office by your Wi-Fi",
                checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED,
            ) {
                requestPermissions(arrayOf(Manifest.permission.ACCESS_FINE_LOCATION), 3)
            },
            Permission(
                "Usage access",
                "Screen time on the phone, side by side with the PC",
                ScreenTime.allowed(this),
            ) {
                startActivity(Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS))
            },
            Permission(
                "Do Not Disturb access",
                "Silence the phone while you focus on the PC",
                getSystemService(android.app.NotificationManager::class.java).isNotificationPolicyAccessGranted,
            ) {
                startActivity(Intent(Settings.ACTION_NOTIFICATION_POLICY_ACCESS_SETTINGS))
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

        addView(sectionTitle("On the phone"))
        val phone = card(8)
        val ssid = Places.currentSsid(this@MainActivity)
        val saved = Places.labelled(this@MainActivity)
        phone.addView(settingRow(
            "Places",
            if (ssid == null) "Connect to Wi-Fi (and allow location) to name it Home or Office"
            else "This Wi-Fi: $ssid · ${saved[ssid]?.lowercase()?.replaceFirstChar { it.uppercase() } ?: "not named"}",
            "Name it",
            ssid != null && saved.containsKey(ssid),
        ) {
            if (ssid == null) return@settingRow
            AlertDialog.Builder(this@MainActivity)
                .setTitle("What is \"$ssid\"?")
                .setItems(arrayOf("Home", "Office")) { _, which ->
                    Places.label(this@MainActivity, if (which == 0) Places.Place.HOME else Places.Place.OFFICE)
                    render()
                }
                .show()
        })
        val ai = PhoneMemory.ai(this@MainActivity)
        phone.addView(settingRow(
            "AI recap",
            if (ai.key.isEmpty()) "Add an OpenRouter or OpenAI key for smart recaps without the PC" else "${ai.model} · used when the PC is away",
            if (ai.key.isEmpty()) "Add key" else "On",
            ai.key.isNotEmpty(),
        ) { editAi() })
        val queued = PhoneMemory.queued(this@MainActivity)
        phone.addView(settingRow(
            "Waiting for the PC",
            "Alerts kept while the PC is away, sent when it's back",
            "$queued",
            true,
        ) {})
        addView(phone, matchWidth())

        addView(sectionTitle("Tips"))
        addView(card(18).apply {
            addView(label("• Long-press the camera robot for a timer, the flashlight and quick notes.", 14f, Ui.DIM))
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
