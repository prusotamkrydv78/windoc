package com.notchy.app

import android.content.Context
import org.json.JSONObject

/**
 * One robot, two bodies: what the PC robot remembers (days together, Focus streak, today's
 * sessions and breaks, copies, unlocked accessories, its plant) mirrored on the phone, plus
 * the phone's own count of pets. Pets reach the PC robot, which hops.
 */
object RobotLife {
    private const val PREFS = "robot_life"

    data class Life(
        val days: Int,
        val streak: Int,
        val focusToday: Int,
        val focusMinutes: Int,
        val breaks: Int,
        val copies: Long,
        val unlocked: Int,
        val plant: Int,
        val pets: Int,
    )

    fun save(context: Context, message: JSONObject) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
            .putInt("days", message.optInt("days"))
            .putInt("streak", message.optInt("streak"))
            .putInt("focus_today", message.optInt("focus_today"))
            .putInt("focus_minutes", message.optInt("focus_minutes"))
            .putInt("breaks", message.optInt("breaks"))
            .putLong("copies", message.optLong("copies"))
            .putInt("unlocked", message.optInt("unlocked"))
            .putInt("plant", message.optInt("plant"))
            .putLong("synced", System.currentTimeMillis())
            .apply()
        RobotService.instance?.setBadges(message.optInt("unlocked"), message.optInt("streak"))
    }

    fun get(context: Context): Life {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        return Life(
            prefs.getInt("days", 0),
            prefs.getInt("streak", 0),
            prefs.getInt("focus_today", 0),
            prefs.getInt("focus_minutes", 0),
            prefs.getInt("breaks", 0),
            prefs.getLong("copies", 0),
            prefs.getInt("unlocked", 0),
            prefs.getInt("plant", 0),
            prefs.getInt("pets", 0),
        )
    }

    fun synced(context: Context): Long =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getLong("synced", 0)

    private var lastPetSent = 0L

    /** A tap on the phone robot: counted here, and the PC robot hops (at most every 10 s). */
    fun pet(context: Context) {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        prefs.edit().putInt("pets", prefs.getInt("pets", 0) + 1).apply()
        val now = System.currentTimeMillis()
        if (now - lastPetSent > 10_000 && Link.send("pet")) lastPetSent = now
    }

    /** The plant's stage as a word, 0 (seed) to 5 (flowering). */
    fun plantName(stage: Int) =
        listOf("Seed", "Sprout", "Seedling", "Young plant", "Budding", "Flowering")[stage.coerceIn(0, 5)]

    val ACCESSORIES = listOf("Scarf", "Glasses", "Cap")
}
