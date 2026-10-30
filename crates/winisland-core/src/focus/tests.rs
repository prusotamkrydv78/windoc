use super::*;

fn secs(value: u64) -> Duration {
    Duration::from_secs(value)
}

fn pomodoro() -> TimerKind {
    TimerKind::Pomodoro(PomodoroConfig {
        work: secs(100),
        short_break: secs(20),
        long_break: secs(50),
        long_break_interval: 2,
    })
}

/// Runs the current phase to completion and returns the event.
fn finish_phase(timer: &mut FocusTimer, now: &mut Instant) -> Option<FocusEvent> {
    timer.start(*now);
    *now += timer.phase_length().expect("timed phase");
    timer.tick(*now)
}

#[test]
fn countdown_finishes_once() {
    let base = Instant::now();
    let mut timer = FocusTimer::new(TimerKind::Countdown(secs(60)));
    assert!(timer.start(base));
    assert_eq!(timer.tick(base + secs(59)), None);
    assert_eq!(
        timer.tick(base + secs(60)),
        Some(FocusEvent::PhaseFinished {
            finished: Phase::Countdown,
            next: None,
        })
    );
    assert_eq!(timer.state(), TimerState::Finished);
    assert_eq!(timer.tick(base + secs(120)), None);
    let view = timer.view(base + secs(120));
    assert_eq!(view.remaining, Some(Duration::ZERO));
    assert_eq!(view.progress, Some(1.0));
}

#[test]
fn paused_time_is_not_counted() {
    let base = Instant::now();
    let mut timer = FocusTimer::new(TimerKind::Countdown(secs(60)));
    timer.start(base);
    assert!(timer.pause(base + secs(10)));
    assert!(timer.resume(base + secs(110)));
    assert_eq!(timer.elapsed(base + secs(120)), secs(20));
    assert_eq!(timer.tick(base + secs(150)), None);
    assert!(timer.tick(base + secs(160)).is_some());
}

#[test]
fn commands_reject_invalid_states() {
    let base = Instant::now();
    let mut timer = FocusTimer::new(TimerKind::Stopwatch);
    assert!(!timer.pause(base));
    assert!(!timer.resume(base));
    assert!(timer.start(base));
    assert!(!timer.start(base));
    assert!(!timer.skip());
}

#[test]
fn stopwatch_never_finishes() {
    let base = Instant::now();
    let mut timer = FocusTimer::new(TimerKind::Stopwatch);
    timer.start(base);
    assert_eq!(timer.tick(base + secs(100_000)), None);
    let view = timer.view(base + secs(90));
    assert_eq!(view.elapsed, secs(90));
    assert_eq!(view.remaining, None);
    assert_eq!(view.progress, None);
}

#[test]
fn pomodoro_cycle_includes_long_break() {
    let mut now = Instant::now();
    let mut timer = FocusTimer::new(pomodoro());
    let mut finished = Vec::new();
    for _ in 0..4 {
        if let Some(FocusEvent::PhaseFinished {
            finished: phase,
            next,
        }) = finish_phase(&mut timer, &mut now)
        {
            finished.push((phase, next));
        }
    }
    assert_eq!(
        finished,
        [
            (Phase::Work, Some(Phase::ShortBreak)),
            (Phase::ShortBreak, Some(Phase::Work)),
            (Phase::Work, Some(Phase::LongBreak)),
            (Phase::LongBreak, Some(Phase::Work)),
        ]
    );
    assert_eq!(timer.completed_work(), 2);
    assert_eq!(timer.state(), TimerState::Idle);
}

#[test]
fn zero_interval_disables_long_breaks() {
    let mut now = Instant::now();
    let mut timer = FocusTimer::new(TimerKind::Pomodoro(PomodoroConfig {
        long_break_interval: 0,
        ..PomodoroConfig::default()
    }));
    for _ in 0..8 {
        finish_phase(&mut timer, &mut now);
        assert_ne!(timer.phase(), Phase::LongBreak);
    }
}

#[test]
fn skip_moves_on_without_counting_work() {
    let base = Instant::now();
    let mut timer = FocusTimer::new(pomodoro());
    timer.start(base);
    assert!(timer.skip());
    assert_eq!(timer.phase(), Phase::ShortBreak);
    assert_eq!(timer.state(), TimerState::Idle);
    assert_eq!(timer.completed_work(), 0);
    assert!(timer.skip());
    assert_eq!(timer.phase(), Phase::Work);

    let mut countdown = FocusTimer::new(TimerKind::Countdown(secs(30)));
    assert!(countdown.skip());
    assert_eq!(countdown.state(), TimerState::Finished);
    assert!(!countdown.skip());
}

#[test]
fn reset_returns_to_first_phase() {
    let mut now = Instant::now();
    let mut timer = FocusTimer::new(pomodoro());
    finish_phase(&mut timer, &mut now);
    timer.start(now);
    timer.reset();
    assert_eq!(timer, FocusTimer::new(pomodoro()));
}

#[test]
fn backwards_clock_adds_no_time() {
    let base = Instant::now() + secs(1_000);
    let mut timer = FocusTimer::new(TimerKind::Countdown(secs(60)));
    timer.start(base);
    let earlier = base - secs(500);
    assert_eq!(timer.elapsed(earlier), Duration::ZERO);
    assert_eq!(timer.tick(earlier), None);
    assert!(timer.pause(earlier));
    assert_eq!(timer.elapsed(base), Duration::ZERO);
}

#[test]
fn snapshot_round_trips_through_json() {
    let base = Instant::now();
    let wall = SystemTime::UNIX_EPOCH + secs(1_700_000_000);
    let mut timer = FocusTimer::new(pomodoro());
    timer.start(base);
    timer.pause(base + secs(40));
    let snapshot = timer.snapshot(base + secs(40), wall);
    let json = serde_json::to_string(&snapshot).expect("serialize");
    let decoded: FocusSnapshot = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded, snapshot);
    assert_eq!(decoded.elapsed, secs(40));
}

#[test]
fn restore_paused_timer_ignores_downtime() {
    let base = Instant::now();
    let wall = SystemTime::UNIX_EPOCH + secs(1_700_000_000);
    let mut timer = FocusTimer::new(TimerKind::Countdown(secs(60)));
    timer.start(base);
    timer.pause(base + secs(10));
    let snapshot = timer.snapshot(base + secs(10), wall);
    let (restored, event) = FocusTimer::restore(&snapshot, base, wall + secs(3_600));
    assert_eq!(event, None);
    assert_eq!(restored.state(), TimerState::Paused);
    assert_eq!(restored.elapsed(base + secs(999)), secs(10));
}

#[test]
fn restore_running_timer_counts_downtime() {
    let base = Instant::now();
    let wall = SystemTime::UNIX_EPOCH + secs(1_700_000_000);
    let mut timer = FocusTimer::new(TimerKind::Countdown(secs(60)));
    timer.start(base);
    let snapshot = timer.snapshot(base + secs(10), wall);
    let later = base + secs(5);
    let (restored, event) = FocusTimer::restore(&snapshot, later, wall + secs(20));
    assert_eq!(event, None);
    assert_eq!(restored.state(), TimerState::Running);
    assert_eq!(restored.elapsed(later), secs(30));
    assert_eq!(restored.elapsed(later + secs(5)), secs(35));
}

#[test]
fn restore_expired_countdown_finishes() {
    let base = Instant::now();
    let wall = SystemTime::UNIX_EPOCH + secs(1_700_000_000);
    let mut timer = FocusTimer::new(TimerKind::Countdown(secs(60)));
    timer.start(base);
    let snapshot = timer.snapshot(base + secs(10), wall);
    let (restored, event) = FocusTimer::restore(&snapshot, base, wall + secs(3_600));
    assert_eq!(
        event,
        Some(FocusEvent::PhaseFinished {
            finished: Phase::Countdown,
            next: None,
        })
    );
    assert_eq!(restored.state(), TimerState::Finished);
}

#[test]
fn restore_with_wall_clock_moved_back_adds_nothing() {
    let base = Instant::now();
    let wall = SystemTime::UNIX_EPOCH + secs(1_700_000_000);
    let mut timer = FocusTimer::new(TimerKind::Countdown(secs(60)));
    timer.start(base);
    let snapshot = timer.snapshot(base + secs(10), wall);
    let (restored, _) = FocusTimer::restore(&snapshot, base, wall - secs(3_600));
    assert_eq!(restored.elapsed(base), secs(10));
}

#[test]
fn restore_rejects_inconsistent_snapshot() {
    let base = Instant::now();
    let snapshot = FocusSnapshot {
        kind: TimerKind::Stopwatch,
        phase: Phase::Work,
        state: TimerState::Running,
        elapsed: secs(10),
        completed_work: 3,
        saved_at: SystemTime::UNIX_EPOCH,
    };
    let (restored, event) = FocusTimer::restore(&snapshot, base, SystemTime::UNIX_EPOCH);
    assert_eq!(event, None);
    assert_eq!(restored, FocusTimer::new(TimerKind::Stopwatch));
}
