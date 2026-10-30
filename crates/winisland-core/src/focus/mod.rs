//! Focus timers: countdown, stopwatch and Pomodoro cycles.
//!
//! Active timing uses a caller-supplied monotonic [`Instant`], never the wall clock, so
//! tests are deterministic and clock changes cannot distort a running session. The wall
//! clock is used only to reconcile time that passed while the app was closed or asleep
//! (see [`FocusSnapshot`]).

use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PomodoroConfig {
    pub work: Duration,
    pub short_break: Duration,
    pub long_break: Duration,
    /// A long break follows every this many completed work phases. Zero disables long breaks.
    pub long_break_interval: u32,
}

impl Default for PomodoroConfig {
    fn default() -> Self {
        Self {
            work: Duration::from_secs(25 * 60),
            short_break: Duration::from_secs(5 * 60),
            long_break: Duration::from_secs(15 * 60),
            long_break_interval: 4,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimerKind {
    Countdown(Duration),
    Stopwatch,
    Pomodoro(PomodoroConfig),
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Countdown,
    Stopwatch,
    Work,
    ShortBreak,
    LongBreak,
}

impl Phase {
    pub const fn label_key(self) -> &'static str {
        match self {
            Self::Countdown => "focus_phase_countdown",
            Self::Stopwatch => "focus_phase_stopwatch",
            Self::Work => "focus_phase_work",
            Self::ShortBreak => "focus_phase_short_break",
            Self::LongBreak => "focus_phase_long_break",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimerState {
    Idle,
    Running,
    Paused,
    Finished,
}

/// Emitted when a timed phase completes, so the app can notify the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusEvent {
    PhaseFinished {
        finished: Phase,
        /// The phase that is now waiting to start; `None` when the timer is finished.
        next: Option<Phase>,
    },
}

/// Read-only state for drawing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FocusView {
    pub phase: Phase,
    pub label_key: &'static str,
    pub state: TimerState,
    pub elapsed: Duration,
    /// `None` for the stopwatch, which has no end.
    pub remaining: Option<Duration>,
    /// 0.0 to 1.0; `None` for the stopwatch.
    pub progress: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FocusTimer {
    kind: TimerKind,
    phase: Phase,
    state: TimerState,
    /// Active time in the current phase up to `running_since`.
    accumulated: Duration,
    running_since: Option<Instant>,
    completed_work: u32,
}

impl FocusTimer {
    pub fn new(kind: TimerKind) -> Self {
        Self {
            kind,
            phase: initial_phase(kind),
            state: TimerState::Idle,
            accumulated: Duration::ZERO,
            running_since: None,
            completed_work: 0,
        }
    }

    pub fn kind(&self) -> TimerKind {
        self.kind
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn state(&self) -> TimerState {
        self.state
    }

    pub fn completed_work(&self) -> u32 {
        self.completed_work
    }

    /// Length of the current phase; `None` for the stopwatch.
    pub fn phase_length(&self) -> Option<Duration> {
        match (self.kind, self.phase) {
            (TimerKind::Countdown(duration), _) => Some(duration),
            (TimerKind::Stopwatch, _) => None,
            (TimerKind::Pomodoro(config), Phase::ShortBreak) => Some(config.short_break),
            (TimerKind::Pomodoro(config), Phase::LongBreak) => Some(config.long_break),
            (TimerKind::Pomodoro(config), _) => Some(config.work),
        }
    }

    /// Active time in the current phase. A clock that moves backwards adds nothing.
    pub fn elapsed(&self, now: Instant) -> Duration {
        let running = self
            .running_since
            .map_or(Duration::ZERO, |since| now.saturating_duration_since(since));
        self.accumulated.saturating_add(running)
    }

    /// Starts an idle or finished timer. Returns `false` when nothing changed.
    pub fn start(&mut self, now: Instant) -> bool {
        match self.state {
            TimerState::Idle | TimerState::Finished => {
                self.accumulated = Duration::ZERO;
                self.running_since = Some(now);
                self.state = TimerState::Running;
                true
            }
            TimerState::Running | TimerState::Paused => false,
        }
    }

    pub fn pause(&mut self, now: Instant) -> bool {
        if self.state != TimerState::Running {
            return false;
        }
        self.accumulated = self.elapsed(now);
        self.running_since = None;
        self.state = TimerState::Paused;
        true
    }

    pub fn resume(&mut self, now: Instant) -> bool {
        if self.state != TimerState::Paused {
            return false;
        }
        self.running_since = Some(now);
        self.state = TimerState::Running;
        true
    }

    /// Returns to the first phase and clears the Pomodoro cycle count.
    pub fn reset(&mut self) {
        *self = Self::new(self.kind);
    }

    /// Ends the current phase early. Skipped work phases do not count as completed.
    pub fn skip(&mut self) -> bool {
        match self.kind {
            TimerKind::Stopwatch => false,
            TimerKind::Countdown(duration) => {
                if self.state == TimerState::Finished {
                    return false;
                }
                self.accumulated = duration;
                self.running_since = None;
                self.state = TimerState::Finished;
                true
            }
            TimerKind::Pomodoro(_) => {
                // A skipped work phase earns a short break, never a long one.
                self.phase = if self.phase == Phase::Work {
                    Phase::ShortBreak
                } else {
                    Phase::Work
                };
                self.enter_waiting_phase();
                true
            }
        }
    }

    /// Advances time. Returns an event when the current phase completes.
    ///
    /// A finished Pomodoro phase moves to the next phase in the idle state, so breaks and
    /// work sessions start only when the user asks. Time beyond the end is discarded.
    pub fn tick(&mut self, now: Instant) -> Option<FocusEvent> {
        if self.state != TimerState::Running {
            return None;
        }
        let length = self.phase_length()?;
        if self.elapsed(now) < length {
            return None;
        }
        let finished = self.phase;
        match self.kind {
            TimerKind::Pomodoro(config) => {
                if finished == Phase::Work {
                    self.completed_work = self.completed_work.saturating_add(1);
                }
                self.phase = next_pomodoro_phase(finished, self.completed_work, config);
                self.enter_waiting_phase();
                Some(FocusEvent::PhaseFinished {
                    finished,
                    next: Some(self.phase),
                })
            }
            TimerKind::Countdown(_) | TimerKind::Stopwatch => {
                self.accumulated = length;
                self.running_since = None;
                self.state = TimerState::Finished;
                Some(FocusEvent::PhaseFinished {
                    finished,
                    next: None,
                })
            }
        }
    }

    pub fn view(&self, now: Instant) -> FocusView {
        let elapsed = self.elapsed(now);
        let (elapsed, remaining, progress) = match self.phase_length() {
            Some(length) => {
                let elapsed = elapsed.min(length);
                let progress = if length.is_zero() {
                    1.0
                } else {
                    (elapsed.as_secs_f32() / length.as_secs_f32()).clamp(0.0, 1.0)
                };
                (elapsed, Some(length - elapsed), Some(progress))
            }
            None => (elapsed, None, None),
        };
        FocusView {
            phase: self.phase,
            label_key: self.phase.label_key(),
            state: self.state,
            elapsed,
            remaining,
            progress,
        }
    }

    pub fn snapshot(&self, now: Instant, now_wall: SystemTime) -> FocusSnapshot {
        FocusSnapshot {
            kind: self.kind,
            phase: self.phase,
            state: self.state,
            elapsed: self.elapsed(now),
            completed_work: self.completed_work,
            saved_at: now_wall,
        }
    }

    /// Rebuilds a timer saved by [`FocusTimer::snapshot`].
    ///
    /// A running timer counts the wall-clock time that passed since it was saved; if the
    /// wall clock moved backwards, no time is added. A phase that ended while the app was
    /// closed completes immediately and its event is returned. An inconsistent snapshot
    /// falls back to a fresh timer of the same kind.
    pub fn restore(
        snapshot: &FocusSnapshot,
        now: Instant,
        now_wall: SystemTime,
    ) -> (Self, Option<FocusEvent>) {
        if !phase_matches_kind(snapshot.kind, snapshot.phase) {
            return (Self::new(snapshot.kind), None);
        }
        let mut timer = Self {
            kind: snapshot.kind,
            phase: snapshot.phase,
            state: snapshot.state,
            accumulated: snapshot.elapsed,
            running_since: None,
            completed_work: snapshot.completed_work,
        };
        if snapshot.state == TimerState::Running {
            let downtime = now_wall
                .duration_since(snapshot.saved_at)
                .unwrap_or(Duration::ZERO);
            timer.accumulated = snapshot.elapsed.saturating_add(downtime);
            timer.running_since = Some(now);
        }
        let event = timer.tick(now);
        (timer, event)
    }

    fn enter_waiting_phase(&mut self) {
        self.accumulated = Duration::ZERO;
        self.running_since = None;
        self.state = TimerState::Idle;
    }
}

/// Persisted timer state. Store it on pause, phase change and shutdown.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FocusSnapshot {
    pub kind: TimerKind,
    pub phase: Phase,
    pub state: TimerState,
    pub elapsed: Duration,
    pub completed_work: u32,
    pub saved_at: SystemTime,
}

fn initial_phase(kind: TimerKind) -> Phase {
    match kind {
        TimerKind::Countdown(_) => Phase::Countdown,
        TimerKind::Stopwatch => Phase::Stopwatch,
        TimerKind::Pomodoro(_) => Phase::Work,
    }
}

fn phase_matches_kind(kind: TimerKind, phase: Phase) -> bool {
    match kind {
        TimerKind::Countdown(_) => phase == Phase::Countdown,
        TimerKind::Stopwatch => phase == Phase::Stopwatch,
        TimerKind::Pomodoro(_) => {
            matches!(phase, Phase::Work | Phase::ShortBreak | Phase::LongBreak)
        }
    }
}

/// `completed_work` already includes a work phase that just finished.
fn next_pomodoro_phase(current: Phase, completed_work: u32, config: PomodoroConfig) -> Phase {
    match current {
        Phase::Work => {
            let interval = config.long_break_interval;
            if interval > 0 && completed_work > 0 && completed_work.is_multiple_of(interval) {
                Phase::LongBreak
            } else {
                Phase::ShortBreak
            }
        }
        _ => Phase::Work,
    }
}

#[cfg(test)]
mod tests;
