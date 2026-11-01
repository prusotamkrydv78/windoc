//! Timers tab (Destination::Focus): a list of modes on the left, a progress ring in the
//! middle and the selected mode's controls on the right.
//!
//! Pomodoro drives the dashboard's shared Focus engine (`PAGE.focus`), so the notch's
//! Focus activity, the Home card and saved state keep working. Countdown, Stopwatch and
//! Water are self-contained here and can run alongside it. Settings and today's counters
//! are saved to `timers_tab.json`; running Countdown/Stopwatch/Water state is not kept
//! across restarts.

use std::cell::RefCell;
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};
use winisland_core::focus::{FocusTimer, FocusView, Phase, PomodoroConfig, TimerKind, TimerState};
use winisland_platform::Key;
use winisland_render::tokens::{SemanticColors, TextStyle, radius, space, typography};
use winisland_render::{Angle, Painter, PathBuilder, Point, Radius, Rect, StrokeCap};

use super::TabContext;
use super::clock::widgets::{
    TOGGLE_H, TOGGLE_W, draw_chip, draw_glyph, draw_round_button, draw_stepper, draw_toggle,
    stepper_halves, text_width,
};
use crate::ui::components::icon::{Icon, draw_icon};
use crate::ui::components::text::{Align, draw_text};
use crate::ui::dashboard::{DashboardHit, PAGE};

const FILE_NAME: &str = "timers_tab.json";
const MAX_FILE_BYTES: u64 = 8 * 1024;
const NOTIFY_TITLE: &str = "Timers";
/// Millilitres per logged glass, for the Water caption.
const GLASS_ML: u32 = 250;

// Control ids (private to this tab).
const ID_MODE: u16 = 200; // 200..=203
const ID_POMO_WORK: u16 = 300; // minus; plus is +1
const ID_POMO_BREAK: u16 = 302;
const ID_POMO_LONG: u16 = 304;
const ID_POMO_CYCLES: u16 = 306;
const ID_CD_HOURS: u16 = 310;
const ID_CD_MINUTES: u16 = 312;
const ID_CD_PRESET: u16 = 314; // 314..=317
const ID_WATER_GOAL: u16 = 320;
const ID_WATER_EVERY: u16 = 322;
const ID_WATER_GLASSES: u16 = 324;
const ID_AUTO_START: u16 = 330;
const ID_REPEAT: u16 = 331;
const ID_REMINDERS: u16 = 332;
const ID_RESET: u16 = 340;
const ID_PLAY: u16 = 341;
const ID_THIRD: u16 = 342;

const PRESETS: [u64; 4] = [5, 10, 25, 60];

const MODE_NAMES: [&str; 4] = ["Pomodoro", "Countdown", "Stopwatch", "Water"];
const HOURGLASS: &str = "M7 3 H17 M7 21 H17 M8 3 V6 L12 12 L8 18 V21 M16 3 V6 L12 12 L16 18 V21";
const STOPWATCH: &str = "M12 8 A7 7 0 1 1 11.99 8 M12 11.5 V15 M10 3 H14 M12 3 V8 M18.5 7 L20 5.5";
const DROP: &str = "M12 3 L17.2 11 A6 6 0 1 1 6.8 11 Z";
const RESET: &str = "M18.5 12 A6.5 6.5 0 1 1 16 7 M16.5 3.5 V7.5 H12.5";
const FLAG: &str = "M7 20 V4 M7 4 H16 L14 8 L16 12 H7";

const ROW_TITLE: TextStyle = TextStyle {
    size: 14.0,
    weight: 600,
    tabular: false,
};
const ROW_VALUE: TextStyle = TextStyle {
    size: 13.0,
    weight: 600,
    tabular: true,
};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
struct Prefs {
    /// Selected mode: 0 Pomodoro, 1 Countdown, 2 Stopwatch, 3 Water.
    mode: usize,
    /// Pomodoro lengths used when the shared engine is not running a Pomodoro.
    pomodoro: PomodoroConfig,
    auto_start: bool,
    countdown_secs: u64,
    countdown_repeat: bool,
    water_goal: u32,
    water_interval_min: u32,
    water_glasses: u32,
    completed_today: u32,
    /// The local date (year, month, day) today's counters belong to.
    day: (u16, u16, u16),
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            mode: 0,
            pomodoro: PomodoroConfig::default(),
            auto_start: false,
            countdown_secs: 15 * 60,
            countdown_repeat: false,
            water_goal: 8,
            water_interval_min: 45,
            water_glasses: 0,
            completed_today: 0,
            day: (0, 0, 0),
        }
    }
}

/// A pausable running clock.
#[derive(Clone, Copy, Debug, Default)]
struct Watch {
    accumulated: Duration,
    since: Option<Instant>,
}

impl Watch {
    fn elapsed(&self, now: Instant) -> Duration {
        self.accumulated
            + self
                .since
                .map_or(Duration::ZERO, |since| now.saturating_duration_since(since))
    }

    fn running(&self) -> bool {
        self.since.is_some()
    }

    fn start(&mut self, now: Instant) {
        if self.since.is_none() {
            self.since = Some(now);
        }
    }

    fn pause(&mut self, now: Instant) {
        if self.since.is_some() {
            self.accumulated = self.elapsed(now);
            self.since = None;
        }
    }

    fn toggle(&mut self, now: Instant) {
        if self.running() {
            self.pause(now);
        } else {
            self.start(now);
        }
    }

    /// Back to zero, keeping it running if it was.
    fn restart(&mut self, now: Instant) {
        let running = self.running();
        *self = Self::default();
        if running {
            self.start(now);
        }
    }
}

struct State {
    prefs: Prefs,
    countdown: Watch,
    countdown_done: bool,
    stopwatch: Watch,
    /// Lap lengths, oldest first.
    laps: Vec<Duration>,
    /// Time since the last water reminder; runs while reminders are on.
    water: Watch,
    /// The shared engine as seen last frame: phase, state, completed work.
    last_engine: Option<(bool, Phase, TimerState, u32)>,
    last_day_check: Option<Instant>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State {
        prefs: load_prefs(),
        countdown: Watch::default(),
        countdown_done: false,
        stopwatch: Watch::default(),
        laps: Vec::new(),
        water: Watch::default(),
        last_engine: None,
        last_day_check: None,
    });
}

fn load_prefs() -> Prefs {
    match crate::core::json_store::read::<Prefs>(FILE_NAME, MAX_FILE_BYTES) {
        Ok(Some(prefs)) => prefs,
        Ok(None) => Prefs::default(),
        Err(error) => {
            log::warn!("Ignoring saved Timers settings: {error}");
            Prefs::default()
        }
    }
}

fn save_prefs(prefs: &Prefs) {
    if let Err(error) = crate::core::json_store::write(FILE_NAME, prefs) {
        log::warn!("Cannot save Timers settings: {error}");
    }
}

fn notify(message: &str) {
    crate::platform::notify().show_toast(NOTIFY_TITLE, message);
}

// ---------------------------------------------------------------------------------------
// Formatting

/// "05:00", "1:02:03". Remaining time rounds up so 00:00 shows only at the end.
fn format_time(duration: Duration, round_up: bool) -> String {
    let millis = duration.as_millis();
    let seconds = if round_up {
        millis.div_ceil(1000)
    } else {
        millis / 1000
    };
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

/// "0:41.2" for laps.
fn format_lap(duration: Duration) -> String {
    let tenths = duration.as_millis() / 100;
    let (minutes, seconds, tenth) = (tenths / 600, tenths / 10 % 60, tenths % 10);
    format!("{minutes}:{seconds:02}.{tenth}")
}

/// "25m", "1h 30m", "45s".
fn format_length(duration: Duration) -> String {
    let secs = duration.as_secs();
    match (secs / 3600, secs / 60 % 60, secs % 60) {
        (0, 0, s) => format!("{s}s"),
        (0, m, _) => format!("{m}m"),
        (h, 0, _) => format!("{h}h"),
        (h, m, _) => format!("{h}h {m}m"),
    }
}

fn minutes(duration: Duration) -> u64 {
    duration.as_secs() / 60
}

// ---------------------------------------------------------------------------------------
// Pomodoro on the shared engine

struct Pomodoro {
    config: PomodoroConfig,
    view: FocusView,
    completed: u32,
    /// Whether the shared engine is running this Pomodoro (vs. an idle preview).
    live: bool,
}

fn pomodoro(prefs: &Prefs, now: Instant) -> Pomodoro {
    PAGE.with(|page| {
        let page = page.borrow();
        match page.focus.kind() {
            TimerKind::Pomodoro(config) => Pomodoro {
                config,
                view: page.focus.view(now),
                completed: page.focus.completed_work(),
                live: true,
            },
            _ => Pomodoro {
                config: prefs.pomodoro,
                view: FocusTimer::new(TimerKind::Pomodoro(prefs.pomodoro)).view(now),
                completed: 0,
                live: false,
            },
        }
    })
}

/// Changes the Pomodoro lengths, keeping a running session's phase and progress.
fn change_pomodoro(prefs: &mut Prefs, change: impl FnOnce(&mut PomodoroConfig)) {
    PAGE.with(|page| {
        let mut page = page.borrow_mut();
        let mut config = match page.focus.kind() {
            TimerKind::Pomodoro(config) => config,
            _ => prefs.pomodoro,
        };
        change(&mut config);
        prefs.pomodoro = config;
        if matches!(page.focus.kind(), TimerKind::Pomodoro(_)) {
            let (now, wall) = (Instant::now(), SystemTime::now());
            let mut snapshot = page.focus.snapshot(now, wall);
            snapshot.kind = TimerKind::Pomodoro(config);
            page.focus = FocusTimer::restore(&snapshot, now, wall).0;
            crate::core::focus_store::save(&page.focus);
        }
    });
}

fn pomodoro_button(prefs: &Prefs, id: u16) {
    PAGE.with(|page| {
        let mut page = page.borrow_mut();
        let now = Instant::now();
        let live = matches!(page.focus.kind(), TimerKind::Pomodoro(_));
        match id {
            ID_PLAY if live => super::super::toggle_focus(&mut page.focus, now),
            ID_PLAY => {
                page.focus = FocusTimer::new(TimerKind::Pomodoro(prefs.pomodoro));
                page.focus.start(now);
            }
            ID_RESET if live => page.focus.reset(),
            ID_THIRD if live => {
                page.focus.skip();
            }
            _ => return,
        }
        crate::core::focus_store::save(&page.focus);
    });
}

fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Work => "Focus",
        Phase::ShortBreak => "Break",
        Phase::LongBreak => "Long break",
        Phase::Countdown => "Countdown",
        Phase::Stopwatch => "Stopwatch",
    }
}

/// Cycle position: (cycles, done this cycle, 1-based current focus).
fn cycle(pomo: &Pomodoro) -> (u32, u32, u32) {
    let cycles = pomo.config.long_break_interval.max(1);
    let mut done = pomo.completed % cycles;
    if pomo.view.phase == Phase::LongBreak && done == 0 && pomo.completed > 0 {
        done = cycles;
    }
    (cycles, done, (done + 1).min(cycles))
}

// ---------------------------------------------------------------------------------------
// What each mode shows

enum Cell {
    /// A − label + pill; `minus` is the id of the minus half, plus is `minus + 1`.
    Stepper {
        label: String,
        minus: u16,
    },
    Chip {
        label: String,
        id: u16,
        active: bool,
    },
    Info {
        text: String,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Third {
    Skip,
    AddMinute,
    Lap,
    Glass,
}

struct Panel {
    title: String,
    /// Dots under the title: count, filled, and the one in progress.
    dots: (u32, u32, Option<u32>),
    subtitle: String,
    running: bool,
    third: Third,
    third_enabled: bool,
    cells: [Vec<Cell>; 2],
    toggle: Option<(&'static str, bool, u16)>,
    caption: String,
    /// Ring fill 0..=1, the big text in it and the label under that.
    ring: f32,
    big: String,
    label: String,
}

fn stepper(label: String, minus: u16) -> Cell {
    Cell::Stepper { label, minus }
}

fn panel(state: &State, now: Instant) -> Panel {
    let prefs = &state.prefs;
    match prefs.mode {
        1 => countdown_panel(state, now),
        2 => stopwatch_panel(state, now),
        3 => water_panel(state, now),
        _ => {
            let pomo = pomodoro(prefs, now);
            let (cycles, done, current) = cycle(&pomo);
            let paused = pomo.view.state == TimerState::Paused;
            let title = match pomo.view.phase {
                Phase::Work => format!("Focus {current} of {cycles}"),
                phase => format!("{} · {done} of {cycles} done", phase_name(phase)),
            };
            let next = match pomo.view.phase {
                Phase::Work => {
                    let interval = pomo.config.long_break_interval;
                    if interval > 0 && (pomo.completed + 1).is_multiple_of(interval) {
                        format!(
                            "Long break {} next",
                            format_time(pomo.config.long_break, false)
                        )
                    } else {
                        format!("Break {} next", format_time(pomo.config.short_break, false))
                    }
                }
                _ => format!("Focus {} next", format_time(pomo.config.work, false)),
            };
            let remaining = pomo.view.remaining.unwrap_or_default();
            Panel {
                title,
                dots: (
                    cycles.min(12),
                    done,
                    (pomo.view.phase == Phase::Work).then_some(done),
                ),
                subtitle: next,
                running: pomo.live && pomo.view.state == TimerState::Running,
                third: Third::Skip,
                third_enabled: pomo.live,
                cells: [
                    vec![
                        stepper(
                            format!("Focus {}", format_length(pomo.config.work)),
                            ID_POMO_WORK,
                        ),
                        stepper(
                            format!("Break {}", format_length(pomo.config.short_break)),
                            ID_POMO_BREAK,
                        ),
                    ],
                    vec![
                        stepper(
                            format!("Long {}", format_length(pomo.config.long_break)),
                            ID_POMO_LONG,
                        ),
                        stepper(
                            format!("Cycles {}", pomo.config.long_break_interval),
                            ID_POMO_CYCLES,
                        ),
                    ],
                ],
                toggle: Some(("Auto-start next", prefs.auto_start, ID_AUTO_START)),
                caption: match prefs.completed_today {
                    1 => "1 focus session completed today".to_string(),
                    n => format!("{n} focus sessions completed today"),
                },
                ring: pomo.view.progress.unwrap_or(0.0),
                big: format_time(remaining, true),
                label: if paused {
                    "Paused".to_string()
                } else {
                    phase_name(pomo.view.phase).to_string()
                },
            }
        }
    }
}

fn countdown_panel(state: &State, now: Instant) -> Panel {
    let prefs = &state.prefs;
    let length = Duration::from_secs(prefs.countdown_secs.max(1));
    let elapsed = state.countdown.elapsed(now).min(length);
    let remaining = length - elapsed;
    let running = state.countdown.running();
    let started = running || !state.countdown.accumulated.is_zero();
    let subtitle = if state.countdown_done {
        "Finished".to_string()
    } else if running {
        let local = crate::platform::shell().local_datetime();
        let end = u64::from(local.hour) * 3600
            + u64::from(local.minute) * 60
            + u64::from(local.second)
            + remaining.as_secs();
        format!("Ends at {:02}:{:02}", end / 3600 % 24, end / 60 % 60)
    } else if started {
        format!("Paused · {} left", format_time(remaining, true))
    } else {
        "Set a length, then start".to_string()
    };
    let total = prefs.countdown_secs;
    Panel {
        title: format!("Countdown {}", format_length(length)),
        dots: (0, 0, None),
        subtitle,
        running,
        third: Third::AddMinute,
        third_enabled: true,
        cells: [
            vec![
                stepper(format!("Hours {}", total / 3600), ID_CD_HOURS),
                stepper(format!("Minutes {}", total / 60 % 60), ID_CD_MINUTES),
            ],
            PRESETS
                .iter()
                .enumerate()
                .map(|(index, preset)| Cell::Chip {
                    label: format!("{preset}m"),
                    id: ID_CD_PRESET + index as u16,
                    active: total == preset * 60,
                })
                .collect(),
        ],
        toggle: Some(("Repeat when done", prefs.countdown_repeat, ID_REPEAT)),
        caption: "You get a notification when it ends".to_string(),
        ring: if state.countdown_done {
            1.0
        } else {
            elapsed.as_secs_f32() / length.as_secs_f32()
        },
        big: format_time(remaining, true),
        label: if state.countdown_done {
            "Done".to_string()
        } else if started && !running {
            "Paused".to_string()
        } else {
            "Countdown".to_string()
        },
    }
}

fn stopwatch_panel(state: &State, now: Instant) -> Panel {
    let elapsed = state.stopwatch.elapsed(now);
    let running = state.stopwatch.running();
    let laps = &state.laps;
    let mut recent: Vec<Cell> = laps
        .iter()
        .enumerate()
        .rev()
        .take(4)
        .map(|(index, lap)| Cell::Info {
            text: format!("Lap {}  {}", index + 1, format_lap(*lap)),
        })
        .collect();
    let second_row = if recent.len() > 2 {
        recent.split_off(2)
    } else {
        Vec::new()
    };
    if recent.is_empty() {
        recent.push(Cell::Info {
            text: "No laps yet".to_string(),
        });
    }
    let subtitle = match laps.iter().min() {
        Some(best) => format!("{} laps · best {}", laps.len(), format_lap(*best)),
        None if running => "The flag records a lap".to_string(),
        None => "Press play to start".to_string(),
    };
    Panel {
        title: "Stopwatch".to_string(),
        dots: (0, 0, None),
        subtitle,
        running,
        third: Third::Lap,
        third_enabled: running,
        cells: [recent, second_row],
        toggle: None,
        caption: String::new(),
        ring: (elapsed.as_secs_f32() % 60.0) / 60.0,
        big: format_time(elapsed, false),
        label: if !running && !elapsed.is_zero() {
            "Paused".to_string()
        } else {
            "Stopwatch".to_string()
        },
    }
}

fn water_panel(state: &State, now: Instant) -> Panel {
    let prefs = &state.prefs;
    let goal = prefs.water_goal.max(1);
    let glasses = prefs.water_glasses;
    let interval = Duration::from_secs(u64::from(prefs.water_interval_min.max(1)) * 60);
    let on = state.water.running();
    let subtitle = if on {
        let left = interval.saturating_sub(state.water.elapsed(now));
        format!(
            "Next reminder in {} min",
            minutes(left + Duration::from_secs(59))
        )
    } else {
        "Reminders are off".to_string()
    };
    Panel {
        title: if glasses >= goal {
            "Goal reached".to_string()
        } else {
            format!("Glass {} of {goal}", glasses + 1)
        },
        dots: (goal.min(12), glasses.min(goal.min(12)), None),
        subtitle,
        running: on,
        third: Third::Glass,
        third_enabled: true,
        cells: [
            vec![
                stepper(format!("Goal {goal}"), ID_WATER_GOAL),
                stepper(
                    format!("Every {}m", prefs.water_interval_min),
                    ID_WATER_EVERY,
                ),
            ],
            vec![stepper(
                format!("Glasses today {glasses}"),
                ID_WATER_GLASSES,
            )],
        ],
        toggle: Some(("Reminders", on, ID_REMINDERS)),
        caption: format!("{} ml of {} ml today", glasses * GLASS_ML, goal * GLASS_ML),
        ring: (glasses as f32 / goal as f32).min(1.0),
        big: format!("{glasses} / {goal}"),
        label: "glasses today".to_string(),
    }
}

/// The value line of a mode row: main text, detail, and whether it is running.
fn row_value(state: &State, mode: usize, now: Instant) -> (String, String, bool) {
    let prefs = &state.prefs;
    match mode {
        0 => {
            let pomo = pomodoro(prefs, now);
            let time = format_time(pomo.view.remaining.unwrap_or_default(), true);
            if !pomo.live {
                return (time, "Ready".to_string(), false);
            }
            let (cycles, _, current) = cycle(&pomo);
            let detail = match (pomo.view.state, pomo.view.phase) {
                (TimerState::Paused, _) => "Paused".to_string(),
                (_, Phase::Work) => format!("Focus · {current} of {cycles}"),
                (_, phase) => phase_name(phase).to_string(),
            };
            (time, detail, pomo.view.state == TimerState::Running)
        }
        1 => {
            let length = Duration::from_secs(prefs.countdown_secs.max(1));
            let remaining = length.saturating_sub(state.countdown.elapsed(now));
            let running = state.countdown.running();
            let detail = if state.countdown_done {
                "Done"
            } else if !running && !state.countdown.accumulated.is_zero() {
                "Paused"
            } else {
                ""
            };
            (format_time(remaining, true), detail.to_string(), running)
        }
        2 => {
            let detail = match state.laps.len() {
                0 => String::new(),
                1 => "1 lap".to_string(),
                n => format!("{n} laps"),
            };
            (
                format_time(state.stopwatch.elapsed(now), false),
                detail,
                state.stopwatch.running(),
            )
        }
        _ => (
            format!("{} / {}", prefs.water_glasses, prefs.water_goal.max(1)),
            "glasses".to_string(),
            state.water.running(),
        ),
    }
}

// ---------------------------------------------------------------------------------------
// Layout

/// Height of the right-hand control block.
const BLOCK_H: f32 = 214.0;

struct Layout {
    rows: [Rect; 4],
    ring: Rect,
    panel: Rect,
    title: Rect,
    dots_y: f32,
    subtitle: Rect,
    buttons: [Rect; 3],
    cell_rows: [Rect; 2],
    toggle_row: Rect,
    caption: Rect,
}

fn layout(grid: Rect) -> Layout {
    let list_w = (grid.width() * 0.27).clamp(150.0, 210.0);
    let panel_w = (grid.width() * 0.37).clamp(220.0, 290.0);
    let row_gap = space::S1;
    let row_h = (grid.height() - row_gap * 3.0) / 4.0;
    let rows = std::array::from_fn(|index| {
        Rect::from_xywh(
            grid.left,
            grid.top + index as f32 * (row_h + row_gap),
            list_w,
            row_h,
        )
    });
    let panel = Rect::from_ltrb(grid.right - panel_w, grid.top, grid.right, grid.bottom);
    let middle = Rect::from_ltrb(
        grid.left + list_w + space::S4,
        grid.top,
        panel.left - space::S4,
        grid.bottom,
    );
    let size = middle.width().min(grid.height() - 6.0).clamp(60.0, 220.0);
    let ring = Rect::from_xywh(
        middle.center_x() - size / 2.0,
        middle.center_y() - size / 2.0,
        size,
        size,
    );
    let top = grid.top + ((grid.height() - BLOCK_H) / 2.0).max(0.0);
    let center_x = panel.center_x();
    let (small, main, gap) = (34.0, 48.0, 16.0);
    let buttons_y = top + 74.0;
    let buttons = [
        Rect::from_xywh(
            center_x - main / 2.0 - gap - small,
            buttons_y - small / 2.0,
            small,
            small,
        ),
        Rect::from_xywh(center_x - main / 2.0, buttons_y - main / 2.0, main, main),
        Rect::from_xywh(
            center_x + main / 2.0 + gap,
            buttons_y - small / 2.0,
            small,
            small,
        ),
    ];
    let line = |y: f32, h: f32| Rect::from_xywh(panel.left, top + y, panel.width(), h);
    Layout {
        rows,
        ring,
        panel,
        title: line(0.0, 20.0),
        dots_y: top + 27.0,
        subtitle: line(33.0, 16.0),
        buttons,
        cell_rows: [line(106.0, 28.0), line(140.0, 28.0)],
        toggle_row: line(176.0, 22.0),
        caption: line(199.0, 15.0),
    }
}

/// The cells of a row, side by side with an 8 px gap.
fn cell_rects(row: Rect, count: usize) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let gap = space::S2;
    let width = (row.width() - gap * (count - 1) as f32) / count as f32;
    (0..count)
        .map(|index| {
            Rect::from_xywh(
                row.left + index as f32 * (width + gap),
                row.top,
                width,
                row.height(),
            )
        })
        .collect()
}

/// Label and switch centred in the toggle row; the whole pair is clickable.
fn toggle_parts(row: Rect, label: &str) -> (Rect, Rect, Rect) {
    let label_w = text_width(label, typography::BODY).ceil() + 2.0;
    let total = label_w + space::S3 + TOGGLE_W;
    let left = row.center_x() - total / 2.0;
    let label_rect = Rect::from_xywh(left, row.top, label_w, row.height());
    let toggle = Rect::from_xywh(
        left + label_w + space::S3,
        row.center_y() - TOGGLE_H / 2.0,
        TOGGLE_W,
        TOGGLE_H,
    );
    let hit = Rect::from_ltrb(
        label_rect.left - 6.0,
        row.top,
        toggle.right + 6.0,
        row.bottom,
    );
    (label_rect, toggle, hit)
}

// ---------------------------------------------------------------------------------------
// Drawing

/// Draws the tab. Return `false` to fall back to the legacy page.
pub fn draw(ctx: &TabContext<'_>) -> bool {
    let painter = ctx.painter;
    let colors = ctx.colors;
    let layout = layout(ctx.grid);
    let hovered = match ctx.hover {
        Some(DashboardHit::Tab(id)) => Some(id),
        _ => None,
    };
    let now = Instant::now();
    STATE.with(|cell| {
        let state = cell.borrow();
        draw_modes(painter, &layout, &state, hovered, now, colors);
        let panel = panel(&state, now);
        draw_ring(painter, layout.ring, &panel, colors);
        draw_panel(painter, &layout, &panel, hovered, colors);
    });
    true
}

fn draw_modes(
    painter: Painter<'_>,
    layout: &Layout,
    state: &State,
    hovered: Option<u16>,
    now: Instant,
    colors: &SemanticColors,
) {
    for (index, row) in layout.rows.iter().enumerate() {
        let selected = state.prefs.mode == index;
        let hot = hovered == Some(ID_MODE + index as u16);
        if selected {
            painter.fill_round_rect(*row, Radius::uniform(radius::CARD), colors.surface_raised);
        } else if hot {
            painter.fill_round_rect(
                *row,
                Radius::uniform(radius::CARD),
                colors.surface_raised.with_alpha(140),
            );
        }
        let icon_color = if selected || hot {
            colors.text_primary
        } else {
            colors.text_secondary
        };
        let icon = Rect::from_xywh(row.left + 12.0, row.center_y() - 11.0, 22.0, 22.0);
        match index {
            0 => draw_icon(painter, Icon::Focus, icon, icon_color),
            1 => draw_glyph(painter, HOURGLASS, icon, icon_color, 1.8),
            2 => draw_glyph(painter, STOPWATCH, icon, icon_color, 1.8),
            _ => draw_glyph(painter, DROP, icon, icon_color, 1.8),
        }
        let text_left = icon.right + 12.0;
        let text_right = row.right - 10.0;
        let mid = row.center_y();
        draw_text(
            painter,
            MODE_NAMES[index],
            ROW_TITLE,
            Rect::from_ltrb(text_left, mid - 18.0, text_right, mid),
            Align::Start,
            colors.text_primary,
        );
        let (value, detail, running) = row_value(state, index, now);
        let mut x = text_left;
        if running {
            painter.fill_circle(Point::new(x + 3.0, mid + 10.0), 3.0, colors.text_primary);
            x += 11.0;
        }
        let value_w = text_width(&value, ROW_VALUE) + 1.0;
        draw_text(
            painter,
            &value,
            ROW_VALUE,
            Rect::from_ltrb(x, mid + 1.0, text_right, mid + 19.0),
            Align::Start,
            colors
                .text_primary
                .with_alpha(if selected { 255 } else { 210 }),
        );
        if !detail.is_empty() && x + value_w + 8.0 < text_right {
            draw_text(
                painter,
                &detail,
                typography::CAPTION,
                Rect::from_ltrb(x + value_w + 6.0, mid + 1.0, text_right, mid + 19.0),
                Align::Start,
                colors.text_secondary,
            );
        }
    }
}

fn draw_ring(painter: Painter<'_>, rect: Rect, panel: &Panel, colors: &SemanticColors) {
    let stroke = (rect.width() * 0.035).clamp(5.0, 8.0);
    let ring = rect.inset(stroke / 2.0 + 1.0);
    painter.stroke_arc(
        ring,
        Angle::from_degrees(0.0),
        Angle::from_degrees(360.0),
        stroke,
        colors.surface_raised,
        StrokeCap::Round,
    );
    let sweep = 360.0 * panel.ring.clamp(0.0, 1.0);
    if sweep > 0.5 {
        painter.stroke_arc(
            ring,
            Angle::from_degrees(0.0),
            Angle::from_degrees(sweep),
            stroke,
            colors.text_primary,
            StrokeCap::Round,
        );
    }
    let big = TextStyle {
        size: (rect.width() * 0.2).clamp(24.0, 44.0),
        weight: 650,
        tabular: true,
    };
    let inner = ring.inset(stroke + 6.0);
    let center = rect.center_y();
    draw_text(
        painter,
        &panel.big,
        big,
        Rect::from_ltrb(
            inner.left,
            center - big.size * 0.75,
            inner.right,
            center + big.size * 0.35,
        ),
        Align::Center,
        colors.text_primary,
    );
    draw_text(
        painter,
        &panel.label,
        typography::LABEL,
        Rect::from_ltrb(
            inner.left,
            center + big.size * 0.45,
            inner.right,
            center + big.size * 0.45 + 18.0,
        ),
        Align::Center,
        colors.text_secondary,
    );
}

fn draw_panel(
    painter: Painter<'_>,
    layout: &Layout,
    panel: &Panel,
    hovered: Option<u16>,
    colors: &SemanticColors,
) {
    draw_text(
        painter,
        &panel.title,
        typography::SECTION_TITLE,
        layout.title,
        Align::Center,
        colors.text_primary,
    );
    let (count, filled, current) = panel.dots;
    if count > 0 {
        let spacing = 10.0;
        let start = layout.panel.center_x() - (count - 1) as f32 * spacing / 2.0;
        for index in 0..count {
            let center = Point::new(start + index as f32 * spacing, layout.dots_y);
            let color = if index < filled {
                colors.text_primary
            } else if current == Some(index) {
                colors.text_primary.with_alpha(130)
            } else {
                colors.surface_interactive
            };
            painter.fill_circle(center, 3.0, color);
        }
    }
    draw_text(
        painter,
        &panel.subtitle,
        typography::LABEL,
        layout.subtitle,
        Align::Center,
        colors.text_secondary,
    );

    // Reset, play/pause and the mode's third action.
    for (index, rect) in layout.buttons.iter().enumerate() {
        let id = ID_RESET + index as u16;
        let enabled = index != 2 || panel.third_enabled;
        let glyph = draw_round_button(
            painter,
            *rect,
            index == 1,
            hovered == Some(id),
            enabled,
            colors,
        );
        let inner = rect.inset(rect.width() * 0.28);
        match index {
            0 => draw_glyph(painter, RESET, inner, glyph, 2.0),
            1 => draw_play_pause(
                painter,
                rect.inset(rect.width() * 0.32),
                panel.running,
                glyph,
            ),
            _ => match panel.third {
                Third::Skip => draw_skip(painter, rect.inset(rect.width() * 0.32), glyph),
                Third::Lap => draw_glyph(painter, FLAG, inner, glyph, 2.0),
                Third::Glass => draw_icon(painter, Icon::Plus, inner, glyph),
                Third::AddMinute => draw_text(
                    painter,
                    "+1m",
                    TextStyle {
                        size: 11.0,
                        weight: 600,
                        tabular: false,
                    },
                    *rect,
                    Align::Center,
                    glyph,
                ),
            },
        }
    }

    for (row, cells) in layout.cell_rows.iter().zip(&panel.cells) {
        for (rect, cell) in cell_rects(*row, cells.len()).into_iter().zip(cells) {
            match cell {
                Cell::Stepper { label, minus } => {
                    let hot = match hovered {
                        Some(id) if id == *minus => Some(false),
                        Some(id) if id == *minus + 1 => Some(true),
                        _ => None,
                    };
                    draw_stepper(painter, rect, label, hot, colors);
                }
                Cell::Chip { label, id, active } => {
                    draw_chip(painter, rect, label, *active, hovered == Some(*id), colors);
                }
                Cell::Info { text } => {
                    painter.fill_round_rect(
                        rect,
                        Radius::uniform(rect.height() / 2.0),
                        colors.surface_raised.with_alpha(150),
                    );
                    draw_text(
                        painter,
                        text,
                        TextStyle {
                            size: 12.0,
                            weight: 500,
                            tabular: true,
                        },
                        rect,
                        Align::Center,
                        colors.text_secondary,
                    );
                }
            }
        }
    }

    if let Some((label, on, id)) = panel.toggle {
        let (label_rect, toggle, _) = toggle_parts(layout.toggle_row, label);
        let hot = hovered == Some(id);
        draw_text(
            painter,
            label,
            typography::BODY,
            label_rect,
            Align::Start,
            if hot {
                colors.text_primary
            } else {
                colors.text_secondary
            },
        );
        draw_toggle(painter, toggle, on, hot, colors);
    }
    draw_text(
        painter,
        &panel.caption,
        typography::CAPTION,
        layout.caption,
        Align::Center,
        colors.text_secondary,
    );
}

fn draw_play_pause(
    painter: Painter<'_>,
    inner: Rect,
    running: bool,
    color: winisland_render::Rgba,
) {
    if running {
        let bar = inner.width() * 0.26;
        for x in [
            inner.left + inner.width() * 0.16,
            inner.right - inner.width() * 0.16 - bar,
        ] {
            painter.fill_round_rect(
                Rect::from_xywh(x, inner.top, bar, inner.height()),
                Radius::uniform(bar / 2.0),
                color,
            );
        }
    } else {
        // A triangle nudged right for optical centring.
        let mut path = PathBuilder::new();
        path.move_to(Point::new(inner.left + inner.width() * 0.22, inner.top))
            .line_to(Point::new(inner.right, inner.center_y()))
            .line_to(Point::new(inner.left + inner.width() * 0.22, inner.bottom))
            .close();
        painter.fill_path(&path.detach(), color);
    }
}

fn draw_skip(painter: Painter<'_>, inner: Rect, color: winisland_render::Rgba) {
    let bar = (inner.width() * 0.18).max(2.0);
    let tip = inner.right - bar - 1.5;
    let mut path = PathBuilder::new();
    path.move_to(Point::new(inner.left + 1.0, inner.top))
        .line_to(Point::new(tip, inner.center_y()))
        .line_to(Point::new(inner.left + 1.0, inner.bottom))
        .close();
    painter.fill_path(&path.detach(), color);
    painter.fill_round_rect(
        Rect::from_xywh(inner.right - bar, inner.top, bar, inner.height()),
        Radius::uniform(bar / 2.0),
        color,
    );
}

/// `true` when this tab draws its own cards instead of sitting on the shared page card.
pub fn custom_layout() -> bool {
    true
}

// ---------------------------------------------------------------------------------------
// Input

/// Hit test in page coordinates. Use `DashboardHit::Tab(id)` with ids private to this tab.
pub fn hit(grid: Rect, point: Point) -> Option<DashboardHit> {
    let layout = layout(grid);
    if let Some(index) = layout.rows.iter().position(|row| row.contains(point)) {
        return Some(DashboardHit::Tab(ID_MODE + index as u16));
    }
    if let Some(index) = layout
        .buttons
        .iter()
        .position(|button| button.contains(point))
    {
        return Some(DashboardHit::Tab(ID_RESET + index as u16));
    }
    if !layout.panel.contains(point) {
        return None;
    }
    let panel = STATE.with(|cell| panel(&cell.borrow(), Instant::now()));
    for (row, cells) in layout.cell_rows.iter().zip(&panel.cells) {
        for (rect, cell) in cell_rects(*row, cells.len()).into_iter().zip(cells) {
            if !rect.contains(point) {
                continue;
            }
            match cell {
                Cell::Stepper { minus, .. } => {
                    let (left, right) = stepper_halves(rect);
                    if left.contains(point) {
                        return Some(DashboardHit::Tab(*minus));
                    }
                    if right.contains(point) {
                        return Some(DashboardHit::Tab(*minus + 1));
                    }
                }
                Cell::Chip { id, .. } => return Some(DashboardHit::Tab(*id)),
                Cell::Info { .. } => {}
            }
        }
    }
    if let Some((label, _, id)) = panel.toggle {
        let (_, _, hit) = toggle_parts(layout.toggle_row, label);
        if hit.contains(point) {
            return Some(DashboardHit::Tab(id));
        }
    }
    None
}

/// Applies a click from [`hit`]. `true` when handled.
pub fn apply(hit: DashboardHit) -> bool {
    let DashboardHit::Tab(id) = hit else {
        return false;
    };
    if !(ID_MODE..=ID_THIRD).contains(&id) {
        return false;
    }
    let now = Instant::now();
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let handled = state.apply(id, now);
        if handled {
            save_prefs(&state.prefs);
        }
        handled
    })
}

/// Steps `value` by `delta` within `min..=max`.
fn step(value: u64, delta: i64, min: u64, max: u64) -> u64 {
    (value as i64 + delta).clamp(min as i64, max as i64) as u64
}

fn step_duration(value: Duration, delta_minutes: i64, min: u64, max: u64) -> Duration {
    Duration::from_secs(step(minutes(value), delta_minutes, min, max) * 60)
}

impl State {
    fn apply(&mut self, id: u16, now: Instant) -> bool {
        // Minus halves have even offsets from their base id, plus halves odd.
        let sign = |base: u16| if id == base { -1 } else { 1 };
        match id {
            _ if (ID_MODE..ID_MODE + 4).contains(&id) => {
                self.prefs.mode = usize::from(id - ID_MODE);
            }
            ID_RESET | ID_PLAY | ID_THIRD => self.button(id, now),
            ID_POMO_WORK | 301 => change_pomodoro(&mut self.prefs, |config| {
                config.work = step_duration(config.work, 5 * sign(ID_POMO_WORK), 5, 180);
            }),
            ID_POMO_BREAK | 303 => change_pomodoro(&mut self.prefs, |config| {
                config.short_break = step_duration(config.short_break, sign(ID_POMO_BREAK), 1, 60);
            }),
            ID_POMO_LONG | 305 => change_pomodoro(&mut self.prefs, |config| {
                config.long_break = step_duration(config.long_break, 5 * sign(ID_POMO_LONG), 5, 90);
            }),
            ID_POMO_CYCLES | 307 => change_pomodoro(&mut self.prefs, |config| {
                config.long_break_interval = step(
                    u64::from(config.long_break_interval),
                    sign(ID_POMO_CYCLES),
                    1,
                    12,
                ) as u32;
            }),
            ID_CD_HOURS | 311 => {
                let hours = step(self.prefs.countdown_secs / 3600, sign(ID_CD_HOURS), 0, 23);
                self.set_countdown(hours * 3600 + self.prefs.countdown_secs / 60 % 60 * 60);
            }
            ID_CD_MINUTES | 313 => {
                let total = self.prefs.countdown_secs / 60;
                let minutes = step(total, sign(ID_CD_MINUTES), 1, 23 * 60 + 59);
                self.set_countdown(minutes * 60);
            }
            _ if (ID_CD_PRESET..ID_CD_PRESET + 4).contains(&id) => {
                self.set_countdown(PRESETS[usize::from(id - ID_CD_PRESET)] * 60);
            }
            ID_WATER_GOAL | 321 => {
                self.prefs.water_goal =
                    step(u64::from(self.prefs.water_goal), sign(ID_WATER_GOAL), 1, 20) as u32;
            }
            ID_WATER_EVERY | 323 => {
                self.prefs.water_interval_min = step(
                    u64::from(self.prefs.water_interval_min),
                    5 * sign(ID_WATER_EVERY),
                    10,
                    240,
                ) as u32;
            }
            ID_WATER_GLASSES | 325 => {
                self.prefs.water_glasses = step(
                    u64::from(self.prefs.water_glasses),
                    sign(ID_WATER_GLASSES),
                    0,
                    99,
                ) as u32;
            }
            ID_AUTO_START => self.prefs.auto_start = !self.prefs.auto_start,
            ID_REPEAT => self.prefs.countdown_repeat = !self.prefs.countdown_repeat,
            ID_REMINDERS => self.water.toggle(now),
            _ => return false,
        }
        true
    }

    /// A new countdown length. A countdown that is not running starts over.
    fn set_countdown(&mut self, seconds: u64) {
        self.prefs.countdown_secs = seconds.clamp(60, 24 * 3600 - 60);
        if !self.countdown.running() {
            self.countdown = Watch::default();
            self.countdown_done = false;
        }
    }

    fn button(&mut self, id: u16, now: Instant) {
        match (self.prefs.mode, id) {
            (0, _) => pomodoro_button(&self.prefs, id),
            (1, ID_RESET) => {
                self.countdown = Watch::default();
                self.countdown_done = false;
            }
            (1, ID_PLAY) => {
                if self.countdown_done {
                    self.countdown = Watch::default();
                    self.countdown_done = false;
                }
                self.countdown.toggle(now);
            }
            (1, _) => {
                self.prefs.countdown_secs = (self.prefs.countdown_secs + 60).min(24 * 3600 - 60);
                if self.countdown_done {
                    // One more minute on a finished countdown: run it again from there.
                    self.countdown_done = false;
                    self.countdown.accumulated =
                        Duration::from_secs(self.prefs.countdown_secs - 60);
                    self.countdown.start(now);
                }
            }
            (2, ID_RESET) => {
                self.stopwatch = Watch::default();
                self.laps.clear();
            }
            (2, ID_PLAY) => self.stopwatch.toggle(now),
            (2, _) => {
                if self.stopwatch.running() {
                    let total = self.stopwatch.elapsed(now);
                    let previous: Duration = self.laps.iter().sum();
                    self.laps.push(total.saturating_sub(previous));
                }
            }
            (_, ID_RESET) => self.water.restart(now),
            (_, ID_PLAY) => self.water.toggle(now),
            (_, _) => {
                self.prefs.water_glasses = (self.prefs.water_glasses + 1).min(99);
                // Drinking resets the reminder.
                self.water.restart(now);
            }
        }
    }

    fn poll(&mut self, now: Instant) {
        let mut dirty = false;

        // New day: clear today's counters.
        let due = self
            .last_day_check
            .is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(5));
        if due {
            self.last_day_check = Some(now);
            let local = crate::platform::shell().local_datetime();
            let today = (local.year, local.month, local.day);
            if self.prefs.day != today {
                if self.prefs.day != (0, 0, 0) {
                    self.prefs.completed_today = 0;
                    self.prefs.water_glasses = 0;
                }
                self.prefs.day = today;
                dirty = true;
            }
        }

        // Countdown end.
        let length = Duration::from_secs(self.prefs.countdown_secs.max(1));
        if self.countdown.running() && self.countdown.elapsed(now) >= length {
            if self.prefs.countdown_repeat {
                self.countdown = Watch::default();
                self.countdown.start(now);
                notify("Countdown finished. Starting again.");
            } else {
                self.countdown = Watch {
                    accumulated: length,
                    since: None,
                };
                self.countdown_done = true;
                notify("Countdown finished.");
            }
        }

        // Water reminder.
        let interval = Duration::from_secs(u64::from(self.prefs.water_interval_min.max(1)) * 60);
        if self.water.running() && self.water.elapsed(now) >= interval {
            self.water = Watch::default();
            self.water.start(now);
            notify("Time for a glass of water.");
        }

        // Pomodoro: count finished focus sessions and auto-start the next phase.
        let seen = PAGE.with(|page| {
            let page = page.try_borrow().ok()?;
            Some((
                matches!(page.focus.kind(), TimerKind::Pomodoro(_)),
                page.focus.phase(),
                page.focus.state(),
                page.focus.completed_work(),
            ))
        });
        if let Some(current) = seen {
            if let Some(last) = self.last_engine
                && current.0
                && last.0
            {
                if current.3 > last.3 {
                    self.prefs.completed_today += current.3 - last.3;
                    dirty = true;
                }
                let phase_ended = last.2 == TimerState::Running
                    && current.2 == TimerState::Idle
                    && current.1 != last.1;
                if phase_ended && self.prefs.auto_start {
                    PAGE.with(|page| {
                        if let Ok(mut page) = page.try_borrow_mut() {
                            page.focus.start(now);
                            crate::core::focus_store::save(&page.focus);
                        }
                    });
                }
            }
            self.last_engine = Some(current);
        }

        if dirty {
            save_prefs(&self.prefs);
        }
    }
}

pub fn wants_keyboard() -> bool {
    false
}

pub fn handle_key(_key: &Key) -> bool {
    false
}

/// Every frame, visible or not.
pub fn poll(now: Instant) {
    STATE.with(|cell| {
        if let Ok(mut state) = cell.try_borrow_mut() {
            state.poll(now);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_format_with_two_digit_minutes() {
        assert_eq!(format_time(Duration::from_secs(300), false), "05:00");
        assert_eq!(format_time(Duration::from_millis(1), true), "00:01");
        assert_eq!(format_time(Duration::from_secs(3723), false), "1:02:03");
        assert_eq!(format_lap(Duration::from_millis(41_250)), "0:41.2");
        assert_eq!(format_length(Duration::from_secs(5400)), "1h 30m");
    }

    #[test]
    fn steps_clamp() {
        assert_eq!(step(5, -5, 5, 180), 5);
        assert_eq!(step(25, 5, 5, 180), 30);
    }
}
