//! Smart Focus: when the user hops between apps a lot (25+ switches in five minutes) and no
//! Focus session is running, the robot suggests one. At most once an hour, never in the first
//! minutes after a Focus session ended.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(5 * 60);
const SWITCHES: usize = 25;
const QUIET: Duration = Duration::from_secs(60 * 60);

#[derive(Default)]
struct State {
    last_app: String,
    switches: VecDeque<Instant>,
    nudged: Option<Instant>,
    focus_ended: Option<Instant>,
    was_focusing: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Feed the foreground app every frame; returns a nudge once in a while.
pub fn poll(now: Instant, foreground_app: &str, focusing: bool) -> Option<String> {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if state.was_focusing && !focusing {
            state.focus_ended = Some(now);
        }
        state.was_focusing = focusing;
        if focusing {
            state.switches.clear();
            return None;
        }
        if !foreground_app.is_empty() && foreground_app != state.last_app {
            state.last_app = foreground_app.to_string();
            state.switches.push_back(now);
        }
        while state
            .switches
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) > WINDOW)
        {
            state.switches.pop_front();
        }
        let quiet = state.nudged.is_some_and(|at| now.saturating_duration_since(at) < QUIET)
            || state
                .focus_ended
                .is_some_and(|at| now.saturating_duration_since(at) < Duration::from_secs(10 * 60));
        if state.switches.len() < SWITCHES || quiet {
            return None;
        }
        state.nudged = Some(now);
        state.switches.clear();
        Some("You've jumped between apps a lot. Start a 25-minute Focus session from the notch?".to_string())
    })
}
