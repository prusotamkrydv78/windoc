package com.notchy.app

import android.content.Context
import android.content.res.ColorStateList
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView

/**
 * The app's small view kit: colours, cards, buttons, rows. Plain views, no libraries, so
 * every screen shares one look.
 */
object Ui {
    val BG = Color.rgb(13, 14, 17)
    val CARD = Color.rgb(23, 25, 31)
    val CARD2 = Color.rgb(32, 35, 43)
    val LINE = Color.argb(24, 255, 255, 255)
    val TEXT = Color.rgb(241, 242, 245)
    val DIM = Color.rgb(154, 160, 172)
    val FAINT = Color.rgb(98, 104, 116)
    val ACCENT = Color.rgb(255, 122, 89)
    val MINT = Color.rgb(124, 242, 192)
    val RED = Color.rgb(240, 98, 90)
}

fun Context.dp(value: Float): Int = (value * resources.displayMetrics.density).toInt()
fun Context.dp(value: Int): Int = dp(value.toFloat())

fun rounded(color: Int, radius: Float, stroke: Int = 0, strokeWidth: Int = 0) = GradientDrawable().apply {
    setColor(color)
    cornerRadius = radius
    if (strokeWidth > 0) setStroke(strokeWidth, stroke)
}

fun Context.pressable(color: Int, radius: Float): RippleDrawable =
    RippleDrawable(ColorStateList.valueOf(Color.argb(40, 255, 255, 255)), rounded(color, radius), rounded(Color.WHITE, radius))

fun Context.label(text: String, size: Float = 15f, color: Int = Ui.TEXT, bold: Boolean = false) =
    TextView(this).apply {
        this.text = text
        textSize = size
        setTextColor(color)
        if (bold) typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
        includeFontPadding = false
        setLineSpacing(0f, 1.15f)
    }

fun Context.column(gap: Int = 0) = LinearLayout(this).apply {
    orientation = LinearLayout.VERTICAL
    if (gap > 0) {
        showDividers = LinearLayout.SHOW_DIVIDER_MIDDLE
        dividerDrawable = GradientDrawable().apply { setSize(1, dp(gap)); setColor(Color.TRANSPARENT) }
    }
}

fun Context.row(gap: Int = 0) = LinearLayout(this).apply {
    orientation = LinearLayout.HORIZONTAL
    gravity = Gravity.CENTER_VERTICAL
    if (gap > 0) {
        showDividers = LinearLayout.SHOW_DIVIDER_MIDDLE
        dividerDrawable = GradientDrawable().apply { setSize(dp(gap), 1); setColor(Color.TRANSPARENT) }
    }
}

/** A rounded card with padding. */
fun Context.card(padding: Int = 18, color: Int = Ui.CARD) = column(10).apply {
    background = rounded(color, dp(22).toFloat(), Ui.LINE, dp(1))
    val p = dp(padding)
    setPadding(p, p, p, p)
}

fun Context.sectionTitle(text: String) = label(text.uppercase(), 12f, Ui.DIM, bold = true).apply {
    letterSpacing = 0.08f
    setPadding(dp(6), dp(14), 0, dp(2))
}

/** A pill button; `primary` fills it with the accent colour. */
fun Context.pill(text: String, primary: Boolean = false, onClick: () -> Unit) = label(
    text, 15f, if (primary) Color.WHITE else Ui.TEXT, bold = true,
).apply {
    gravity = Gravity.CENTER
    minHeight = dp(50)
    setPadding(dp(18), 0, dp(18), 0)
    background = pressable(if (primary) Ui.ACCENT else Ui.CARD2, dp(25).toFloat())
    setOnClickListener { onClick() }
}

/** A square quick-action tile: a big glyph over a label. */
fun Context.tile(glyph: String, text: String, onClick: () -> Unit) = column(6).apply {
    gravity = Gravity.CENTER
    minimumHeight = dp(92)
    background = pressable(Ui.CARD2, dp(20).toFloat())
    setPadding(dp(8), dp(12), dp(8), dp(12))
    addView(label(glyph, 24f, Ui.TEXT).apply { gravity = Gravity.CENTER })
    addView(label(text, 13f, Ui.DIM, bold = true).apply { gravity = Gravity.CENTER })
    setOnClickListener { onClick() }
}

/** A settings row: title, detail, and a status on the right. Tapping runs [onClick]. */
fun Context.settingRow(title: String, detail: String, status: String, ok: Boolean, onClick: () -> Unit) =
    row(12).apply {
        setPadding(dp(4), dp(10), dp(4), dp(10))
        background = pressable(Color.TRANSPARENT, dp(14).toFloat())
        val text = column(4).apply {
            addView(label(title, 15f, Ui.TEXT, bold = true))
            addView(label(detail, 13f, Ui.DIM))
        }
        addView(text, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
        addView(label(status, 13f, if (ok) Ui.MINT else Ui.ACCENT, bold = true).apply {
            setPadding(dp(12), dp(6), dp(12), dp(6))
            background = rounded(if (ok) Color.argb(28, 124, 242, 192) else Color.argb(30, 255, 122, 89), dp(14).toFloat())
        })
        setOnClickListener { onClick() }
    }

/** A coloured status dot. */
fun Context.dot(color: Int) = View(this).apply {
    background = rounded(color, dp(5).toFloat())
    layoutParams = LinearLayout.LayoutParams(dp(10), dp(10))
}

fun matchWidth() = LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT)
fun weighted() = LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)
fun fill() = FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT)
