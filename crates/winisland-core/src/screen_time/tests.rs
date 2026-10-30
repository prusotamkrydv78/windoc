use super::*;

const DAY: LocalDate = LocalDate::new(2026, 10, 30);
const NEXT_DAY: LocalDate = LocalDate::new(2026, 10, 31);

fn secs(value: u64) -> Duration {
    Duration::from_secs(value)
}

fn enabled() -> ScreenTimeSettings {
    ScreenTimeSettings {
        enabled: true,
        ..ScreenTimeSettings::default()
    }
}

fn app(exe: &str) -> AppIdentity {
    AppIdentity::new(exe, exe)
}

fn foreground(exe: &str, title: Option<&str>) -> ActivityEvent {
    ActivityEvent::Foreground {
        app: app(exe),
        title: title.map(str::to_string),
    }
}

fn active(tracker: &ScreenTimeTracker, date: LocalDate, now: Instant, exe: &str) -> Duration {
    tracker
        .day(date, now)
        .and_then(|day| day.apps.into_iter().find(|usage| usage.app.exe == exe))
        .map_or(Duration::ZERO, |usage| usage.active)
}

#[test]
fn day_number_matches_known_dates() {
    assert_eq!(LocalDate::new(1970, 1, 1).day_number(), 0);
    assert_eq!(LocalDate::new(1970, 1, 2).day_number(), 1);
    assert_eq!(LocalDate::new(2000, 3, 1).day_number(), 11_017);
    assert_eq!(NEXT_DAY.day_number() - DAY.day_number(), 1);
    // Across a year boundary and a leap day.
    assert_eq!(
        LocalDate::new(2025, 1, 1).day_number() - LocalDate::new(2024, 12, 31).day_number(),
        1
    );
    assert_eq!(
        LocalDate::new(2024, 3, 1).day_number() - LocalDate::new(2024, 2, 28).day_number(),
        2
    );
}

#[test]
fn disabled_by_default_and_records_nothing() {
    let mut tracker = ScreenTimeTracker::new(ScreenTimeSettings::default());
    let start = Instant::now();
    tracker.record(foreground("code.exe", None), start, DAY);
    assert!(tracker.current_app().is_none());
    assert!(tracker.day(DAY, start + secs(60)).is_none());
}

#[test]
fn switching_apps_credits_each() {
    let mut tracker = ScreenTimeTracker::new(enabled());
    let start = Instant::now();
    tracker.record(foreground("code.exe", None), start, DAY);
    tracker.record(foreground("firefox.exe", None), start + secs(60), DAY);
    tracker.record(foreground("code.exe", None), start + secs(90), DAY);
    let now = start + secs(100);
    assert_eq!(active(&tracker, DAY, now, "code.exe"), secs(70));
    assert_eq!(active(&tracker, DAY, now, "firefox.exe"), secs(30));
    let day = tracker.day(DAY, now).expect("day");
    assert_eq!(day.total, secs(100));
    assert_eq!(day.apps[0].app.exe, "code.exe");
}

#[test]
fn idle_and_lock_stop_accrual_until_next_foreground() {
    let mut tracker = ScreenTimeTracker::new(enabled());
    let start = Instant::now();
    tracker.record(foreground("code.exe", None), start, DAY);
    tracker.record(ActivityEvent::Idle, start + secs(10), DAY);
    tracker.record(ActivityEvent::Resumed, start + secs(500), DAY);
    assert_eq!(
        active(&tracker, DAY, start + secs(600), "code.exe"),
        secs(10)
    );
    tracker.record(foreground("code.exe", None), start + secs(600), DAY);
    tracker.record(ActivityEvent::Locked, start + secs(605), DAY);
    assert_eq!(
        active(&tracker, DAY, start + secs(900), "code.exe"),
        secs(15)
    );
}

#[test]
fn titles_are_dropped_unless_enabled() {
    let mut tracker = ScreenTimeTracker::new(enabled());
    let start = Instant::now();
    tracker.record(foreground("code.exe", Some("secret.txt")), start, DAY);
    tracker.record(ActivityEvent::Idle, start + secs(5), DAY);
    let day = tracker.day(DAY, start + secs(5)).expect("day");
    assert!(day.apps[0].titles.is_empty());
}

#[test]
fn titles_are_stored_when_enabled_and_purged_when_disabled() {
    let settings = ScreenTimeSettings {
        store_titles: true,
        ..enabled()
    };
    let mut tracker = ScreenTimeTracker::new(settings);
    let start = Instant::now();
    tracker.record(foreground("code.exe", Some("  main.rs  ")), start, DAY);
    tracker.record(ActivityEvent::Idle, start + secs(5), DAY);
    let day = tracker.day(DAY, start + secs(5)).expect("day");
    assert_eq!(day.apps[0].titles.get("main.rs"), Some(&secs(5)));

    tracker.set_settings(enabled(), start + secs(5), DAY);
    let day = tracker.day(DAY, start + secs(5)).expect("day");
    assert!(day.apps[0].titles.is_empty());
}

#[test]
fn flush_moves_time_after_midnight_to_the_new_day() {
    let mut tracker = ScreenTimeTracker::new(enabled());
    let start = Instant::now();
    tracker.record(foreground("code.exe", None), start, DAY);
    tracker.flush(start + secs(30), NEXT_DAY);
    let now = start + secs(50);
    assert_eq!(active(&tracker, DAY, now, "code.exe"), secs(30));
    assert_eq!(active(&tracker, NEXT_DAY, now, "code.exe"), secs(20));
}

#[test]
fn long_gaps_are_capped() {
    let mut tracker = ScreenTimeTracker::new(enabled());
    let start = Instant::now();
    tracker.record(foreground("code.exe", None), start, DAY);
    tracker.record(ActivityEvent::Idle, start + secs(10 * 60 * 60), DAY);
    assert_eq!(active(&tracker, DAY, start, "code.exe"), MAX_SEGMENT);
}

#[test]
fn clock_going_backwards_credits_nothing() {
    let mut tracker = ScreenTimeTracker::new(enabled());
    let later = Instant::now() + secs(100);
    tracker.record(foreground("code.exe", None), later, DAY);
    tracker.record(ActivityEvent::Idle, later - secs(50), DAY);
    assert!(tracker.day(DAY, later).is_none());
}

#[test]
fn retention_drops_old_days() {
    let settings = ScreenTimeSettings {
        retention_days: 2,
        ..enabled()
    };
    let mut tracker = ScreenTimeTracker::new(settings);
    let start = Instant::now();
    let old = LocalDate::new(2026, 10, 28);
    tracker.record(foreground("code.exe", None), start, old);
    tracker.record(ActivityEvent::Idle, start + secs(10), old);
    tracker.record(foreground("code.exe", None), start + secs(10), DAY);
    tracker.record(ActivityEvent::Idle, start + secs(20), DAY);
    tracker.prune(DAY);
    // Two-day window ending on the 30th keeps the 29th and 30th only.
    assert_eq!(tracker.dates(), vec![DAY]);
}

#[test]
fn rules_categorize_and_explain() {
    let mut tracker = ScreenTimeTracker::new(enabled());
    tracker.set_rules(vec![
        CategoryRule {
            exe: "CODE.EXE".into(),
            category: "Work".into(),
        },
        CategoryRule {
            exe: "steam.exe".into(),
            category: "Games".into(),
        },
        CategoryRule {
            exe: "code.exe".into(),
            category: "Development".into(),
        },
        CategoryRule {
            exe: " ".into(),
            category: "Ignored".into(),
        },
    ]);
    assert_eq!(tracker.rules().len(), 2);
    let rule = tracker.category_for("Code.exe").expect("rule");
    assert_eq!(rule.category, "Development");

    let start = Instant::now();
    tracker.record(foreground("code.exe", None), start, DAY);
    tracker.record(foreground("steam.exe", None), start + secs(40), DAY);
    tracker.record(foreground("notepad.exe", None), start + secs(50), DAY);
    let totals = tracker.category_totals(DAY, start + secs(60));
    assert_eq!(
        totals,
        vec![
            ("Development".to_string(), secs(40)),
            ("Games".to_string(), secs(10)),
        ]
    );
}

#[test]
fn snapshot_round_trip_and_version_check() {
    let mut tracker = ScreenTimeTracker::new(enabled());
    let start = Instant::now();
    tracker.record(foreground("code.exe", None), start, DAY);
    let snapshot = tracker.snapshot(start + secs(30), DAY);
    let json = serde_json::to_string(&snapshot).expect("serialize");
    let decoded: ScreenTimeSnapshot = serde_json::from_str(&json).expect("deserialize");
    let restored = ScreenTimeTracker::restore(decoded.clone(), enabled(), DAY).expect("restore");
    assert_eq!(active(&restored, DAY, start, "code.exe"), secs(30));
    assert!(restored.current_app().is_none());

    let mut wrong = decoded;
    wrong.version = 99;
    assert_eq!(
        ScreenTimeTracker::restore(wrong, enabled(), DAY).err(),
        Some(SnapshotError::UnsupportedVersion(99))
    );
}

#[test]
fn clear_removes_history() {
    let mut tracker = ScreenTimeTracker::new(enabled());
    let start = Instant::now();
    tracker.record(foreground("code.exe", None), start, DAY);
    tracker.clear();
    assert!(tracker.day(DAY, start + secs(10)).is_none());
}
