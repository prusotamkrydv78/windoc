package com.notchy.app

import android.app.AppOpsManager
import android.app.usage.UsageStatsManager
import android.content.Context
import android.os.Process
import java.util.Calendar

/**
 * The phone's screen time today (needs "Usage access"), for the duel with the PC and a kind
 * nudge from the robot past four hours.
 */
object ScreenTime {
    private const val NUDGE_AT = 4 * 60L

    fun allowed(context: Context): Boolean {
        val ops = context.getSystemService(AppOpsManager::class.java)
        return ops.unsafeCheckOpNoThrow(AppOpsManager.OPSTR_GET_USAGE_STATS, Process.myUid(), context.packageName) ==
            AppOpsManager.MODE_ALLOWED
    }

    /** `(minutes today, top five apps with minutes)`, or `null` without usage access. */
    fun today(context: Context): Pair<Long, List<Pair<String, Long>>>? {
        if (!allowed(context)) return null
        val start = Calendar.getInstance().apply {
            set(Calendar.HOUR_OF_DAY, 0)
            set(Calendar.MINUTE, 0)
            set(Calendar.SECOND, 0)
            set(Calendar.MILLISECOND, 0)
        }.timeInMillis
        val usage = context.getSystemService(UsageStatsManager::class.java)
            .queryAndAggregateUsageStats(start, System.currentTimeMillis())
        val packages = context.packageManager
        val apps = usage.values
            .filter { it.totalTimeInForeground > 60_000 && it.packageName != context.packageName }
            .map { stats ->
                val name = runCatching {
                    packages.getApplicationLabel(packages.getApplicationInfo(stats.packageName, 0)).toString()
                }.getOrDefault(stats.packageName)
                name to stats.totalTimeInForeground / 60_000
            }
            .sortedByDescending { it.second }
        return apps.sumOf { it.second } to apps.take(5)
    }

    /** Once a day past four hours: the robot suggests a break. */
    fun nudge(context: Context, minutes: Long) {
        if (minutes < NUDGE_AT) return
        val prefs = context.getSharedPreferences("screen_time", Context.MODE_PRIVATE)
        val day = Calendar.getInstance().get(Calendar.DAY_OF_YEAR)
        if (prefs.getInt("nudged", -1) == day) return
        prefs.edit().putInt("nudged", day).apply()
        RobotService.instance?.say("${minutes / 60}h on the phone · break?")
    }
}
