//! What the notch robot remembers between runs: today's breaks and Focus sessions, Focus
//! day streaks, focus minutes for the last couple of weeks, clipboard milestones, the first
//! screenshot of the day and how many days it has been used, kept in `mascot.json`.
//!
//! Days are counted from 1970-01-01 in local time; the caller passes today's number so the
//! logic stays pure and testable.

use serde::{Deserialize, Serialize};

use super::json_store;

const FILE_NAME: &str = "mascot.json";
const MAX_FILE_BYTES: u64 = 64 * 1024;
/// Days of focus minutes kept, enough for this week and the last.
const KEEP_DAYS: i32 = 14;
/// Clipboard copy counts that earn a little celebration.
pub const COPY_MILESTONES: [u64; 3] = [100, 500, 1000];
/// Active days needed for the scarf, the glasses and the cap.
pub const UNLOCK_DAYS: [u32; 3] = [3, 7, 14];
/// The weekly recap only shows for a week with at least this much focus.
const RECAP_MIN_SECONDS: f32 = 3600.0;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Memory {
    /// The day the daily counters belong to.
    pub day: i32,
    /// Breaks taken today: times the user came back after a while away.
    pub breaks: u32,
    /// Focus sessions finished today.
    pub focus_sessions: u32,
    /// Consecutive days with at least one finished Focus session, ending on
    /// `last_focus_day`.
    pub streak: u32,
    pub last_focus_day: Option<i32>,
    /// Seconds of running Focus timer per day, oldest first.
    pub focus_days: Vec<(i32, f32)>,
    /// The Monday-based week the last recap was shown for.
    pub last_recap_week: Option<i32>,
    /// Everything copied since the robot started counting.
    pub copies: u64,
    pub last_screenshot_day: Option<i32>,
    /// Days the app was in use, and the last of them.
    pub active_days: u32,
    pub last_active_day: Option<i32>,
}

/// Monday-based week number: 1970-01-01 was a Thursday.
pub fn week_of(day: i32) -> i32 {
    (day + 3).div_euclid(7)
}

impl Memory {
    pub fn load() -> Self {
        match json_store::read::<Self>(FILE_NAME, MAX_FILE_BYTES) {
            Ok(memory) => memory.unwrap_or_default(),
            Err(error) => {
                log::warn!("Ignoring robot memory: {error}");
                Self::default()
            }
        }
    }

    pub fn save(&self) {
        if let Err(error) = json_store::write(FILE_NAME, self) {
            log::warn!("Cannot save robot memory: {error}");
        }
    }

    /// Starts a fresh set of daily counters when the date changes.
    pub fn roll_day(&mut self, today: i32) -> bool {
        if self.day == today {
            return false;
        }
        self.day = today;
        self.breaks = 0;
        self.focus_sessions = 0;
        true
    }

    /// Counts today as a day of use. Returns true the first time each day.
    pub fn mark_active(&mut self, today: i32) -> bool {
        if self.last_active_day == Some(today) {
            return false;
        }
        self.last_active_day = Some(today);
        self.active_days = self.active_days.saturating_add(1);
        true
    }

    pub fn add_break(&mut self, today: i32) {
        self.roll_day(today);
        self.breaks += 1;
    }

    /// A Focus session finished. Returns today's session count and, for the first session of
    /// a day, the streak it extends to.
    pub fn focus_finished(&mut self, today: i32) -> (u32, Option<u32>) {
        self.roll_day(today);
        self.focus_sessions += 1;
        let streak = match self.last_focus_day {
            Some(day) if day == today => None,
            Some(day) if day == today - 1 => Some(self.streak + 1),
            _ => Some(1),
        };
        if let Some(streak) = streak {
            self.streak = streak;
            self.last_focus_day = Some(today);
        }
        (self.focus_sessions, streak)
    }

    pub fn add_focus(&mut self, today: i32, seconds: f32) {
        match self.focus_days.last_mut() {
            Some((day, total)) if *day == today => *total += seconds,
            _ => self.focus_days.push((today, seconds)),
        }
        self.focus_days.retain(|(day, _)| today - day < KEEP_DAYS);
    }

    /// Focus seconds over the days `from..to`.
    fn focus_between(&self, from: i32, to: i32) -> f32 {
        self.focus_days
            .iter()
            .filter(|(day, _)| (from..to).contains(day))
            .map(|(_, seconds)| seconds)
            .sum()
    }

    /// Once a week, the first time it is asked in a new week: last week's focus seconds, if
    /// there were enough to be worth a recap.
    pub fn recap_due(&mut self, today: i32) -> Option<f32> {
        let week = week_of(today);
        if self.last_recap_week == Some(week) {
            return None;
        }
        self.last_recap_week = Some(week);
        let monday = week * 7 - 3;
        let seconds = self.focus_between(monday - 7, monday);
        (seconds >= RECAP_MIN_SECONDS).then_some(seconds)
    }

    /// Counts a copy. Returns the milestone just reached, if any.
    pub fn copied(&mut self) -> Option<u64> {
        self.copies = self.copies.saturating_add(1);
        COPY_MILESTONES.contains(&self.copies).then_some(self.copies)
    }

    /// Whether this is the first screenshot today.
    pub fn screenshot(&mut self, today: i32) -> bool {
        if self.last_screenshot_day == Some(today) {
            return false;
        }
        self.last_screenshot_day = Some(today);
        true
    }
}

/// How many accessories `active_days` of use have unlocked, 0 to 3.
pub fn unlocked_count(active_days: u32) -> usize {
    UNLOCK_DAYS
        .iter()
        .filter(|&&days| active_days >= days)
        .count()
}

/// The robot's plant grows one stage per active day, up to a flowering stage 5.
pub fn plant_stage(active_days: u32) -> u8 {
    active_days.saturating_sub(1).min(5) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mascot_streak_counts_consecutive_days_and_resets_after_a_gap() {
        let mut memory = Memory::default();
        assert_eq!(memory.focus_finished(100), (1, Some(1)));
        // A second session the same day keeps the streak.
        assert_eq!(memory.focus_finished(100), (2, None));
        assert_eq!(memory.focus_finished(101), (1, Some(2)));
        assert_eq!(memory.focus_finished(102), (1, Some(3)));
        // Skipping a day starts over.
        assert_eq!(memory.focus_finished(104), (1, Some(1)));
    }

    #[test]
    fn mascot_daily_counters_reset_with_the_date() {
        let mut memory = Memory::default();
        memory.add_break(10);
        memory.add_break(10);
        memory.focus_finished(10);
        assert_eq!((memory.breaks, memory.focus_sessions), (2, 1));
        assert!(memory.roll_day(11));
        assert_eq!((memory.breaks, memory.focus_sessions), (0, 0));
        assert!(memory.mark_active(11));
        assert!(!memory.mark_active(11));
        assert!(memory.mark_active(12));
        assert_eq!(memory.active_days, 2);
    }

    #[test]
    fn mascot_weekly_recap_shows_once_per_week_for_last_week() {
        // Day 4 is Monday 1970-01-05; days 4..11 are one week.
        assert_eq!(week_of(3), 0);
        assert_eq!(week_of(4), 1);
        assert_eq!(week_of(10), 1);
        assert_eq!(week_of(11), 2);
        let mut memory = Memory::default();
        for day in 4..11 {
            memory.add_focus(day, 3600.0);
        }
        // Still in that week: nothing from the week before.
        assert_eq!(memory.recap_due(10), None);
        assert_eq!(memory.recap_due(11), Some(7.0 * 3600.0));
        assert_eq!(memory.recap_due(12), None);
        // Old days drop off.
        memory.add_focus(30, 60.0);
        assert!(memory.focus_days.iter().all(|(day, _)| 30 - day < KEEP_DAYS));
    }

    #[test]
    fn mascot_copy_milestones_and_first_screenshot() {
        let mut memory = Memory::default();
        let reached: Vec<u64> = (0..1200).filter_map(|_| memory.copied()).collect();
        assert_eq!(reached, COPY_MILESTONES);
        assert!(memory.screenshot(5));
        assert!(!memory.screenshot(5));
        assert!(memory.screenshot(6));
    }

    #[test]
    fn mascot_unlocks_follow_active_days() {
        assert_eq!(unlocked_count(0), 0);
        assert_eq!(unlocked_count(3), 1);
        assert_eq!(unlocked_count(6), 1);
        assert_eq!(unlocked_count(7), 2);
        assert_eq!(unlocked_count(14), 3);
        assert_eq!(plant_stage(0), 0);
        assert_eq!(plant_stage(1), 0);
        assert_eq!(plant_stage(3), 2);
        assert_eq!(plant_stage(40), 5);
    }
}
