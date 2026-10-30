//! Connects the screen time domain tracker to Windows foreground and idle signals and
//! persists it as `screen_time.json`.

use std::time::{Duration, Instant};

use winisland_core::screen_time::{
    ActivityEvent, AppIdentity, DaySummary, LocalDate, ScreenTimeSettings, ScreenTimeSnapshot,
    ScreenTimeTracker,
};
use winisland_platform_windows::{ForegroundTracker, input_idle_duration};

use super::json_store;

const FILE_NAME: &str = "screen_time.json";
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// No input for this long counts as away; a locked session also stops input.
const IDLE_THRESHOLD: Duration = Duration::from_secs(5 * 60);
const SAVE_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// Foreground and idle state are sampled at most this often.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

pub struct ScreenTimeService {
    tracker: ScreenTimeTracker,
    foreground: Option<ForegroundTracker>,
    idle: bool,
    today: LocalDate,
    next_poll: Instant,
    next_save: Instant,
    dirty: bool,
}

/// Screen time settings from the app configuration.
pub fn settings_for(config: &winisland_core::config::AppConfig) -> ScreenTimeSettings {
    ScreenTimeSettings {
        enabled: config.screen_time_enabled,
        store_titles: config.screen_time_titles,
        ..ScreenTimeSettings::default()
    }
}

fn local_today() -> LocalDate {
    let now = crate::platform::shell().local_datetime();
    LocalDate::new(
        i32::from(now.year),
        u8::try_from(now.month).unwrap_or(1),
        u8::try_from(now.day).unwrap_or(1),
    )
}

fn identity(executable: &str) -> AppIdentity {
    let name = executable
        .strip_suffix(".exe")
        .or_else(|| executable.strip_suffix(".EXE"))
        .unwrap_or(executable);
    AppIdentity::new(executable, name)
}

impl ScreenTimeService {
    /// Restores saved history. Tracking starts only when `settings.enabled` is true.
    pub fn load(settings: ScreenTimeSettings) -> Self {
        let today = local_today();
        let tracker = match json_store::read::<ScreenTimeSnapshot>(FILE_NAME, MAX_FILE_BYTES) {
            Ok(Some(snapshot)) => ScreenTimeTracker::restore(snapshot, settings, today)
                .unwrap_or_else(|error| {
                    log::warn!("Ignoring saved screen time: {error}");
                    ScreenTimeTracker::new(settings)
                }),
            Ok(None) => ScreenTimeTracker::new(settings),
            Err(error) => {
                log::warn!("Ignoring saved screen time: {error}");
                ScreenTimeTracker::new(settings)
            }
        };
        let now = Instant::now();
        let mut service = Self {
            tracker,
            foreground: None,
            idle: false,
            today,
            next_poll: now,
            next_save: now + SAVE_INTERVAL,
            dirty: false,
        };
        service.sync_hook();
        service
    }

    /// Applies changed settings. Disabling stops the Windows hook immediately.
    pub fn apply_settings(&mut self, settings: ScreenTimeSettings, now: Instant) {
        if settings == self.tracker.settings() {
            return;
        }
        let titles_changed = settings.store_titles != self.tracker.settings().store_titles;
        self.tracker.set_settings(settings, now, self.today);
        if titles_changed {
            // The hook decides whether titles are read, so restart it.
            self.foreground = None;
        }
        self.sync_hook();
        self.dirty = true;
        self.save(now);
    }

    fn sync_hook(&mut self) {
        let settings = self.tracker.settings();
        if !settings.enabled {
            self.foreground = None;
            return;
        }
        if self.foreground.is_none() {
            match ForegroundTracker::start(settings.store_titles) {
                Ok(tracker) => self.foreground = Some(tracker),
                Err(error) => log::warn!("Screen time tracking unavailable: {error}"),
            }
        }
    }

    /// Call from the frame loop. Cheap when called more often than once per second.
    pub fn poll(&mut self, now: Instant) {
        if now < self.next_poll {
            return;
        }
        self.next_poll = now + POLL_INTERVAL;
        if let Some(foreground) = self.foreground.as_ref() {
            let changes = foreground.drain();
            let idle = input_idle_duration().is_some_and(|idle| idle >= IDLE_THRESHOLD);
            if idle && !self.idle {
                self.tracker.record(ActivityEvent::Idle, now, self.today);
                self.dirty = true;
            } else if !idle && self.idle {
                self.tracker.record(ActivityEvent::Resumed, now, self.today);
            }
            self.idle = idle;
            if !idle {
                for change in changes {
                    self.tracker.record(
                        ActivityEvent::Foreground {
                            app: identity(&change.executable),
                            title: change.title,
                        },
                        now,
                        self.today,
                    );
                    self.dirty = true;
                }
            }
        }
        let today = local_today();
        if today != self.today {
            self.today = today;
            self.tracker.flush(now, today);
            self.dirty = true;
        }
        if now >= self.next_save {
            self.save(now);
        }
    }

    fn save(&mut self, now: Instant) {
        self.next_save = now + SAVE_INTERVAL;
        if !self.dirty {
            return;
        }
        let snapshot = self.tracker.snapshot(now, self.today);
        match json_store::write(FILE_NAME, &snapshot) {
            Ok(()) => self.dirty = false,
            Err(error) => log::warn!("Cannot save screen time: {error}"),
        }
    }

    /// Saves pending time. Call before the app exits.
    pub fn shutdown(&mut self, now: Instant) {
        self.foreground = None;
        self.dirty = true;
        self.save(now);
    }

    pub fn today(&self, now: Instant) -> Option<DaySummary> {
        self.tracker.day(self.today, now)
    }
}
