//! Screen time: per-day active duration by application.
//!
//! The platform layer reports foreground changes and idle, lock and resume events; this
//! module turns them into daily totals. It never reads a clock itself: callers pass a
//! monotonic `Instant` and the current local date, so behaviour is deterministic.
//!
//! Privacy defaults (workflow §14): only the executable identity and a friendly name are
//! stored. Window titles are dropped unless `store_titles` is enabled, and turning it off
//! deletes titles already stored. Retention defaults to seven days.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub const SNAPSHOT_VERSION: u32 = 1;
pub const DEFAULT_RETENTION_DAYS: u32 = 7;
/// Longest stretch credited to one app without a fresh event. Guards against sleep or a
/// missed idle event inflating totals.
pub const MAX_SEGMENT: Duration = Duration::from_secs(2 * 60 * 60);
pub const MAX_TITLES_PER_APP: usize = 50;
pub const MAX_TITLE_CHARS: usize = 200;

/// A calendar date in the user's local time zone, supplied by the caller.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalDate {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

impl LocalDate {
    pub const fn new(year: i32, month: u8, day: u8) -> Self {
        Self { year, month, day }
    }

    /// Days since 1970-01-01 in the proleptic Gregorian calendar.
    pub fn day_number(self) -> i64 {
        // Howard Hinnant's days_from_civil.
        let month = i64::from(self.month);
        let day = i64::from(self.day);
        let year = i64::from(self.year) - i64::from(month <= 2);
        let era = year.div_euclid(400);
        let year_of_era = year - era * 400;
        let shifted_month = (month + 9) % 12;
        let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146_097 + day_of_era - 719_468
    }
}

/// Identifies an application without recording anything it displays.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AppIdentity {
    /// Executable file name, lowercased, such as `code.exe`.
    pub exe: String,
    /// Friendly name for display, such as `Visual Studio Code`.
    pub name: String,
}

impl AppIdentity {
    pub fn new(exe: &str, name: &str) -> Self {
        Self {
            exe: exe.trim().to_lowercase(),
            name: name.trim().to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActivityEvent {
    /// A window of `app` became the foreground window.
    Foreground {
        app: AppIdentity,
        title: Option<String>,
    },
    /// No input for the idle threshold. Time stops accruing.
    Idle,
    /// The session was locked or the system is going to sleep.
    Locked,
    /// Input resumed or the session was unlocked. Accrual restarts at the next
    /// foreground event.
    Resumed,
}

/// Maps an executable to a category. Rules are local, user-editable and explainable.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CategoryRule {
    pub exe: String,
    pub category: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenTimeSettings {
    /// Tracking is opt-in: the module explains its access before it is enabled.
    pub enabled: bool,
    pub store_titles: bool,
    pub retention_days: u32,
}

impl Default for ScreenTimeSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            store_titles: false,
            retention_days: DEFAULT_RETENTION_DAYS,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AppUsage {
    pub app: AppIdentity,
    pub active: Duration,
    /// Empty unless title collection is enabled.
    pub titles: BTreeMap<String, Duration>,
}

/// One day's usage, apps sorted by active time (longest first).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DaySummary {
    pub date: LocalDate,
    pub total: Duration,
    pub apps: Vec<AppUsage>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DayRecord {
    pub date: LocalDate,
    pub apps: Vec<AppUsage>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ScreenTimeSnapshot {
    pub version: u32,
    pub rules: Vec<CategoryRule>,
    pub days: Vec<DayRecord>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    UnsupportedVersion(u32),
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported screen time snapshot version {version}")
            }
        }
    }
}

impl std::error::Error for SnapshotError {}

#[derive(Clone, Debug)]
struct Segment {
    app: AppIdentity,
    title: Option<String>,
    since: Instant,
    date: LocalDate,
}

#[derive(Clone, Debug, Default)]
pub struct ScreenTimeTracker {
    settings: ScreenTimeSettings,
    rules: Vec<CategoryRule>,
    current: Option<Segment>,
    days: BTreeMap<LocalDate, BTreeMap<String, AppUsage>>,
}

impl ScreenTimeTracker {
    pub fn new(settings: ScreenTimeSettings) -> Self {
        Self {
            settings,
            ..Self::default()
        }
    }

    pub fn settings(&self) -> ScreenTimeSettings {
        self.settings
    }

    /// Applies new settings. Disabling stops the current segment (crediting time so far);
    /// turning titles off deletes every stored title.
    pub fn set_settings(&mut self, settings: ScreenTimeSettings, now: Instant, today: LocalDate) {
        if !settings.enabled {
            self.close(now);
        }
        if !settings.store_titles {
            for apps in self.days.values_mut() {
                for usage in apps.values_mut() {
                    usage.titles.clear();
                }
            }
            if let Some(segment) = self.current.as_mut() {
                segment.title = None;
            }
        }
        self.settings = settings;
        self.prune(today);
    }

    /// Records an activity event. Does nothing while tracking is disabled.
    pub fn record(&mut self, event: ActivityEvent, now: Instant, today: LocalDate) {
        if !self.settings.enabled {
            return;
        }
        self.close(now);
        if let ActivityEvent::Foreground { app, title } = event {
            let title = title
                .filter(|_| self.settings.store_titles)
                .map(|title| {
                    title
                        .trim()
                        .chars()
                        .take(MAX_TITLE_CHARS)
                        .collect::<String>()
                })
                .filter(|title| !title.is_empty());
            self.current = Some(Segment {
                app,
                title,
                since: now,
                date: today,
            });
        }
    }

    /// Credits time so far without changing the foreground app. Call at least when the
    /// local date changes, so time after midnight lands on the new day.
    pub fn flush(&mut self, now: Instant, today: LocalDate) {
        if let Some(segment) = self.current.take() {
            let (app, title) = (segment.app.clone(), segment.title.clone());
            self.credit(segment, now);
            self.current = Some(Segment {
                app,
                title,
                since: now,
                date: today,
            });
        }
        self.prune(today);
    }

    fn close(&mut self, now: Instant) {
        if let Some(segment) = self.current.take() {
            self.credit(segment, now);
        }
    }

    fn credit(&mut self, segment: Segment, now: Instant) {
        let elapsed = now
            .saturating_duration_since(segment.since)
            .min(MAX_SEGMENT);
        if elapsed.is_zero() {
            return;
        }
        let usage = self
            .days
            .entry(segment.date)
            .or_default()
            .entry(segment.app.exe.clone())
            .or_insert_with(|| AppUsage {
                app: segment.app.clone(),
                active: Duration::ZERO,
                titles: BTreeMap::new(),
            });
        usage.active += elapsed;
        // Keep the latest friendly name the platform reported.
        usage.app.name = segment.app.name;
        if let Some(title) = segment.title {
            if let Some(existing) = usage.titles.get_mut(&title) {
                *existing += elapsed;
            } else if usage.titles.len() < MAX_TITLES_PER_APP {
                usage.titles.insert(title, elapsed);
            }
        }
    }

    /// Drops days older than the retention window ending at `today`.
    pub fn prune(&mut self, today: LocalDate) {
        let retention = i64::from(self.settings.retention_days.max(1));
        let oldest_kept = today.day_number() - retention + 1;
        self.days.retain(|date, _| date.day_number() >= oldest_kept);
    }

    /// The app currently accruing time, if any.
    pub fn current_app(&self) -> Option<&AppIdentity> {
        self.current.as_ref().map(|segment| &segment.app)
    }

    /// Usage for `date`, including the open segment when it belongs to that day.
    pub fn day(&self, date: LocalDate, now: Instant) -> Option<DaySummary> {
        let mut apps: BTreeMap<String, AppUsage> =
            self.days.get(&date).cloned().unwrap_or_default();
        if let Some(segment) = self.current.as_ref().filter(|segment| segment.date == date) {
            let elapsed = now
                .saturating_duration_since(segment.since)
                .min(MAX_SEGMENT);
            if !elapsed.is_zero() {
                apps.entry(segment.app.exe.clone())
                    .or_insert_with(|| AppUsage {
                        app: segment.app.clone(),
                        active: Duration::ZERO,
                        titles: BTreeMap::new(),
                    })
                    .active += elapsed;
            }
        }
        if apps.is_empty() {
            return None;
        }
        let mut apps: Vec<AppUsage> = apps.into_values().collect();
        apps.sort_by(|a, b| {
            b.active
                .cmp(&a.active)
                .then_with(|| a.app.exe.cmp(&b.app.exe))
        });
        let total = apps.iter().map(|usage| usage.active).sum();
        Some(DaySummary { date, total, apps })
    }

    pub fn dates(&self) -> Vec<LocalDate> {
        self.days.keys().copied().collect()
    }

    pub fn rules(&self) -> &[CategoryRule] {
        &self.rules
    }

    /// Replaces the rules. Executable names are normalized to lowercase; later rules for
    /// the same executable replace earlier ones.
    pub fn set_rules(&mut self, rules: Vec<CategoryRule>) {
        let mut normalized: Vec<CategoryRule> = Vec::with_capacity(rules.len());
        for rule in rules {
            let exe = rule.exe.trim().to_lowercase();
            let category = rule.category.trim().to_string();
            if exe.is_empty() || category.is_empty() {
                continue;
            }
            normalized.retain(|existing| existing.exe != exe);
            normalized.push(CategoryRule { exe, category });
        }
        self.rules = normalized;
    }

    /// The category for `exe` and the rule that produced it, so the UI can explain it.
    pub fn category_for(&self, exe: &str) -> Option<&CategoryRule> {
        let exe = exe.trim().to_lowercase();
        self.rules.iter().find(|rule| rule.exe == exe)
    }

    /// Active time per category for `date`. Apps without a rule are omitted.
    pub fn category_totals(&self, date: LocalDate, now: Instant) -> Vec<(String, Duration)> {
        let mut totals: BTreeMap<String, Duration> = BTreeMap::new();
        if let Some(day) = self.day(date, now) {
            for usage in day.apps {
                if let Some(rule) = self.category_for(&usage.app.exe) {
                    *totals.entry(rule.category.clone()).or_default() += usage.active;
                }
            }
        }
        let mut totals: Vec<(String, Duration)> = totals.into_iter().collect();
        totals.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        totals
    }

    /// Deletes every recorded day. Rules and settings are kept.
    pub fn clear(&mut self) {
        self.days.clear();
        self.current = None;
    }

    /// Saves recorded days and rules. Time in the open segment is credited first.
    pub fn snapshot(&mut self, now: Instant, today: LocalDate) -> ScreenTimeSnapshot {
        self.flush(now, today);
        ScreenTimeSnapshot {
            version: SNAPSHOT_VERSION,
            rules: self.rules.clone(),
            days: self
                .days
                .iter()
                .map(|(date, apps)| DayRecord {
                    date: *date,
                    apps: apps.values().cloned().collect(),
                })
                .collect(),
        }
    }

    pub fn restore(
        snapshot: ScreenTimeSnapshot,
        settings: ScreenTimeSettings,
        today: LocalDate,
    ) -> Result<Self, SnapshotError> {
        if snapshot.version != SNAPSHOT_VERSION {
            return Err(SnapshotError::UnsupportedVersion(snapshot.version));
        }
        let mut tracker = Self::new(settings);
        tracker.set_rules(snapshot.rules);
        for record in snapshot.days {
            let day = tracker.days.entry(record.date).or_default();
            for mut usage in record.apps {
                if !settings.store_titles {
                    usage.titles.clear();
                }
                let key = usage.app.exe.clone();
                match day.get_mut(&key) {
                    Some(existing) => existing.active += usage.active,
                    None => {
                        day.insert(key, usage);
                    }
                }
            }
        }
        tracker.prune(today);
        Ok(tracker)
    }
}

#[cfg(test)]
mod tests;
