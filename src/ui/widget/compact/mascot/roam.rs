//! Now and then the robot leaves the notch and plays along the bottom of the screen: on top
//! of the taskbar when it shows, or along the screen's bottom edge when it is hidden.
//!
//! A trip: it drops out of the notch, its red-and-gold flying suit snaps on piece by piece,
//! the boots fire and it swoops down with a barrel roll to a landing on the taskbar (see
//! `suit`). The suit flies off and it plays: it chases and kicks a ball that rolls and
//! bounces off the screen edges, chases a wind-up mouse that turns round and chases it
//! back, then visits the taskbar: it sits on the app icons in the middle, peeks at the clock
//! and naps on the battery in the corner. It sits for a yawn and a wave, suits up again and
//! boosts back up into the notch. Meanwhile the notch stays empty.
//!
//! The app owns the see-through window it plays in (`window::app::roam_window`), the size of
//! the screen's work area, and calls [`tick`] and [`draw`].

use std::cell::RefCell;
use std::f32::consts::{PI, TAU};

use winisland_render::text::{DrawTextCachedParams, FontManager};
use winisland_render::{Painter, Point, Rect, Rgba, Vec2};

use super::life::{self, Activity};
use super::suit;
use super::{Eyes, Pose, Stage, demo, seconds};

/// Logical pixels per robot unit: about 48 px tall, a toy on the taskbar.
const UNIT: f32 = 1.9;

/// How long each part of the play lasts, in seconds.
const BALL_SECONDS: f32 = 22.0;
const MOUSE_SECONDS: f32 = 16.0;
const VISIT_SECONDS: f32 = 14.0;
const REST_SECONDS: f32 = 4.0;
/// Running speeds in robot units per second.
const RUN: f32 = 120.0;
const SPRINT: f32 = 170.0;
/// Room kept from the screen edges, in robot units.
const WALL: f32 = 12.0;
/// How far below the top of the screen the robot hangs under the notch, in robot units.
const UNDER_NOTCH: f32 = 26.0;
/// Smoke puffs drawn behind the flying robot.
const TRAIL_PUFFS: usize = 20;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    /// In the notch as usual.
    Home,
    /// A trip is due: the notch robot leaves at the end of its current scene.
    Leaving,
    /// Out: arriving, playing, then heading home. Started at this robot time.
    Out(f32),
}

/// One floating heart, puff of dust or sleepy "z".
#[derive(Clone, Copy)]
struct Bit {
    x: f32,
    y: f32,
    born: f32,
    kind: BitKind,
}

#[derive(Clone, Copy, PartialEq)]
enum BitKind {
    Heart,
    Dust,
}

struct Roam {
    phase: Phase,
    next_trip: Option<f32>,
    returned_at: Option<f32>,
    /// Playground size in robot units.
    width: f32,
    height: f32,
    last_step: Option<f32>,
    /// When the play on the floor began and when the robot set off home, robot time.
    play_from: Option<f32>,
    home_from: Option<f32>,
    robot_x: f32,
    robot_v: f32,
    facing: f32,
    ball_x: f32,
    ball_y: f32,
    ball_vx: f32,
    ball_vy: f32,
    ball_spin: f32,
    mouse_x: f32,
    mouse_v: f32,
    mouse_turn_at: f32,
    last_kick: f32,
    hop_at: Option<f32>,
    /// The app icon it sits on during the visit, in robot units.
    icon_x: f32,
    /// Where it took off for home.
    launch_x: f32,
    bits: Vec<Bit>,
    seed: u32,
}

impl Default for Roam {
    fn default() -> Self {
        Self {
            phase: Phase::Home,
            next_trip: None,
            returned_at: None,
            width: 1000.0,
            height: 560.0,
            last_step: None,
            play_from: None,
            home_from: None,
            robot_x: 0.0,
            robot_v: 0.0,
            facing: -1.0,
            ball_x: 0.0,
            ball_y: 0.0,
            ball_vx: 0.0,
            ball_vy: 0.0,
            ball_spin: 0.0,
            mouse_x: -20.0,
            mouse_v: 0.0,
            mouse_turn_at: 0.0,
            last_kick: -10.0,
            hop_at: None,
            icon_x: 0.0,
            launch_x: 0.0,
            bits: Vec::new(),
            seed: 0x9e37_79b9,
        }
    }
}

impl Roam {
    fn random(&mut self) -> f32 {
        // xorshift: plenty for a toy's whims.
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed % 10_000) as f32 / 10_000.0
    }

    /// Where the robot hangs under the notch, in robot units with y up from the floor.
    fn notch_spot(&self) -> (f32, f32) {
        (self.width / 2.0, self.height - UNDER_NOTCH)
    }

    /// Where it lands: to the right of the middle, clear of the corner.
    fn land_x(&self) -> f32 {
        self.width * 0.68
    }
}

thread_local! {
    static ROAM: RefCell<Roam> = RefCell::new(Roam::default());
    /// The notch scene's time at the last frame, to spot the moment a scene ends.
    static LAST_SCENE_TIME: std::cell::Cell<Option<f32>> = const { std::cell::Cell::new(None) };
}

/// Seconds between trips for the Settings choice: 0 never, 1 now and then, 2 often.
fn trip_gap(frequency: u8) -> Option<f32> {
    if demo() && std::env::var_os("WINISLAND_MASCOT_DEMO_ROAM").is_some() {
        return Some(15.0);
    }
    match frequency {
        1 => Some(25.0 * 60.0),
        2 => Some(8.0 * 60.0),
        3 => Some(60.0),
        _ => None,
    }
}

thread_local! {
    /// The gap between trips for the current setting, for scheduling the next one.
    static GAP: std::cell::Cell<f32> = const { std::cell::Cell::new(25.0 * 60.0) };
}

/// Whether the robot is away from the notch, so the notch draws no robot.
pub(crate) fn away_from_notch() -> bool {
    ROAM.with(|roam| matches!(roam.borrow().phase, Phase::Out(_)))
}

/// Whether a trip is due, so the notch robot leaves at the end of its scene.
pub(super) fn wants_to_leave() -> bool {
    ROAM.with(|roam| roam.borrow().phase == Phase::Leaving)
}

/// While a trip is due, the notch reports its scene time each frame. When it jumps back,
/// the old scene has ended with the robot walked fully off, and the trip begins.
pub(super) fn note_scene_time(elapsed: f32) {
    let last = LAST_SCENE_TIME.with(|cell| cell.replace(Some(elapsed)));
    if last.is_some_and(|last| elapsed < last) {
        LAST_SCENE_TIME.with(|cell| cell.set(None));
        left_notch();
    }
}

fn left_notch() {
    ROAM.with(|roam| {
        let mut roam = roam.borrow_mut();
        if roam.phase == Phase::Leaving {
            roam.phase = Phase::Out(seconds());
            roam.last_step = None;
            roam.play_from = None;
            roam.home_from = None;
            roam.bits.clear();
            // Fresh toys for every trip; they are placed when it lands.
            roam.robot_x = 0.0;
            roam.robot_v = 0.0;
            roam.ball_x = 0.0;
            roam.mouse_x = -20.0;
            roam.mouse_v = 0.0;
            roam.hop_at = None;
        }
    });
}

/// Seconds since the robot came back to the notch, while it walks in.
pub(super) fn since_return() -> Option<f32> {
    ROAM.with(|roam| roam.borrow().returned_at.map(|at| seconds() - at))
}

/// Whether the robot is out and its playground needs frames.
pub(crate) fn active() -> bool {
    away_from_notch()
}

/// Per tick: schedules trips. `allowed` is false while the user should not be disturbed
/// (fullscreen, Do Not Disturb, Focus, the panel open) or the robot is not shown.
pub(crate) fn tick(frequency: u8, allowed: bool) {
    let now = seconds();
    // Only while someone is there to see it.
    let away = super::CONTEXT.with(|context| context.borrow().away);
    let allowed = allowed
        && !away
        && !matches!(life::activity(), Activity::Meeting | Activity::Fullscreen);
    ROAM.with(|roam| {
        let mut roam = roam.borrow_mut();
        let Some(gap) = trip_gap(frequency) else {
            roam.next_trip = None;
            if roam.phase == Phase::Leaving {
                roam.phase = Phase::Home;
            }
            return;
        };
        // A changed setting reschedules the next trip at the new pace.
        if GAP.with(|cell| cell.replace(gap)) != gap && roam.phase == Phase::Home {
            roam.next_trip = None;
        }
        // The first trip comes a few minutes after starting.
        let next = *roam.next_trip.get_or_insert(now + gap.min(180.0));
        match roam.phase {
            Phase::Home if allowed && now >= next => roam.phase = Phase::Leaving,
            Phase::Leaving if !allowed => roam.phase = Phase::Home,
            _ => {}
        }
        if roam
            .returned_at
            .is_some_and(|at| now - at > super::ENTER_SECONDS * 2.0)
        {
            roam.returned_at = None;
        }
    });
}

/// Ends a trip early (the user needs the screen), sending the robot straight home.
pub(crate) fn come_home() {
    ROAM.with(|roam| {
        let mut roam = roam.borrow_mut();
        if matches!(roam.phase, Phase::Out(_)) {
            finish(&mut roam);
        }
    });
}

fn finish(roam: &mut Roam) {
    let now = seconds();
    roam.phase = Phase::Home;
    roam.returned_at = Some(now);
    // The next trip at the pace the setting asks for.
    roam.next_trip = Some(now + GAP.with(std::cell::Cell::get));
}

/// The robot in flight, if it is flying: arriving from the notch before the play begins,
/// or heading home after it.
fn flight(roam: &Roam, now: f32, started: f32) -> Option<suit::Flight> {
    let (notch_x, top_y) = roam.notch_spot();
    match (roam.play_from, roam.home_from) {
        (_, Some(home_from)) => suit::departure(now - home_from, roam.launch_x, notch_x, top_y),
        (None, None) => suit::arrival(now - started, notch_x, top_y, roam.land_x()),
        _ => None,
    }
}

/// The robot's spot in the playground in physical pixels, for hit testing.
pub(crate) fn robot_rect(scale: f32, height: f32) -> Option<Rect> {
    ROAM.with(|roam| {
        let roam = roam.borrow();
        matches!(roam.phase, Phase::Out(_)).then(|| {
            let unit = UNIT * scale;
            Rect::from_xywh(
                (roam.robot_x - 9.0) * unit,
                height - 26.0 * unit,
                18.0 * unit,
                26.0 * unit,
            )
        })
    })
}

/// The robot was poked: a happy hop with a heart.
pub(crate) fn poked() {
    ROAM.with(|roam| {
        let mut roam = roam.borrow_mut();
        let now = seconds();
        roam.hop_at = Some(now);
        let x = roam.robot_x;
        roam.bits.push(Bit {
            x,
            y: 30.0,
            born: now,
            kind: BitKind::Heart,
        });
    });
}

/// What the robot is doing on the floor, from seconds into the play.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Play {
    Ball,
    Mouse { chased: bool },
    /// Sitting on an app icon in the middle of the taskbar.
    Icon,
    /// Peeking down at the clock in the corner.
    Clock,
    /// Curled up on the battery.
    Battery,
    Rest,
    /// Suiting up to fly home.
    Leave,
}

fn play_at(t: f32) -> Play {
    let mouse_end = BALL_SECONDS + MOUSE_SECONDS;
    let visit_end = mouse_end + VISIT_SECONDS;
    if t < BALL_SECONDS {
        Play::Ball
    } else if t < mouse_end {
        Play::Mouse {
            chased: t > BALL_SECONDS + MOUSE_SECONDS * 0.55,
        }
    } else if t < mouse_end + VISIT_SECONDS * 0.35 {
        Play::Icon
    } else if t < mouse_end + VISIT_SECONDS * 0.6 {
        Play::Clock
    } else if t < visit_end {
        Play::Battery
    } else if t < visit_end + REST_SECONDS {
        Play::Rest
    } else {
        Play::Leave
    }
}

/// Where the clock and the battery sit along a Windows 11 taskbar, in robot units from the
/// right edge.
const CLOCK_FROM_RIGHT: f32 = 34.0;
const BATTERY_FROM_RIGHT: f32 = 62.0;

/// Advances the play on the floor to `now`.
fn step(roam: &mut Roam, now: f32, started: f32) {
    let dt = roam
        .last_step
        .map_or(0.0, |last| (now - last).clamp(0.0, 0.1));
    roam.last_step = Some(now);
    let width = roam.width;
    let margin = 12.0;

    // Flying in: nothing on the floor yet.
    if roam.home_from.is_none() && now - started < suit::ARRIVE_SECONDS {
        return;
    }
    let play_from = *roam.play_from.get_or_insert(now);
    if roam.robot_x == 0.0 && roam.ball_x == 0.0 {
        // Landed; the ball drops in beside it.
        roam.robot_x = roam.land_x();
        roam.ball_x = roam.land_x() - 14.0;
        roam.ball_y = 40.0;
        roam.ball_vx = -RUN * 0.6;
        roam.icon_x = width / 2.0 + (roam.random() - 0.5) * 60.0;
    }
    if roam.home_from.is_some() {
        // Flying home: drawn from time alone.
        if flight(roam, now, started).is_none() {
            finish(roam);
        }
        return;
    }
    let t = now - play_from;
    let play = play_at(t);

    // The ball: rolls with friction, bounces off the walls and the floor.
    roam.ball_vy += 520.0 * dt;
    roam.ball_y = (roam.ball_y - roam.ball_vy * dt).max(0.0);
    if roam.ball_y <= 0.0 && roam.ball_vy > 0.0 {
        roam.ball_vy = -roam.ball_vy * 0.45;
        if roam.ball_vy.abs() < 30.0 {
            roam.ball_vy = 0.0;
        }
    }
    roam.ball_vx *= (-0.55 * dt).exp();
    roam.ball_x += roam.ball_vx * dt;
    roam.ball_spin += roam.ball_vx * dt / 3.0;
    if roam.ball_x < margin {
        roam.ball_x = margin;
        roam.ball_vx = roam.ball_vx.abs() * 0.7;
    } else if roam.ball_x > width - margin {
        roam.ball_x = width - margin;
        roam.ball_vx = -roam.ball_vx.abs() * 0.7;
    }

    // Where the robot heads.
    let target = match play {
        Play::Ball => roam.ball_x,
        Play::Mouse { chased: false } => roam.mouse_x,
        Play::Mouse { chased: true } => {
            roam.robot_x + (roam.robot_x - roam.mouse_x).signum() * 60.0
        }
        Play::Icon => roam.icon_x,
        Play::Clock => width - CLOCK_FROM_RIGHT,
        Play::Battery => width - BATTERY_FROM_RIGHT,
        Play::Rest | Play::Leave => roam.robot_x,
    };
    let gap = target - roam.robot_x;
    let speed = if matches!(play, Play::Mouse { .. }) {
        SPRINT
    } else {
        RUN
    };
    let wanted = if gap.abs() < 4.0 {
        0.0
    } else {
        gap.signum() * speed.min(gap.abs() * 3.0)
    };
    // Ease into and out of a run rather than snapping to speed.
    roam.robot_v += (wanted - roam.robot_v) * (1.0 - (-6.0 * dt).exp());
    roam.robot_x = (roam.robot_x + roam.robot_v * dt).clamp(margin, width - WALL);
    if roam.robot_v.abs() > 8.0 {
        roam.facing = roam.robot_v.signum();
    }
    if play == Play::Leave {
        // Suits up where it stands and takes off for home.
        roam.home_from = Some(now);
        roam.launch_x = roam.robot_x;
        return;
    }

    // Kick the ball when it is close and not already flying off.
    if play == Play::Ball && (roam.ball_x - roam.robot_x).abs() < 9.0 && now - roam.last_kick > 0.8
    {
        let toward_middle = if roam.robot_x > width / 2.0 { -1.0 } else { 1.0 };
        let power = 150.0 + roam.random() * 160.0;
        roam.ball_vx = toward_middle * power;
        roam.ball_vy = -(90.0 + roam.random() * 80.0);
        roam.last_kick = now;
        roam.hop_at = Some(now);
        if roam.random() < 0.5 {
            let x = roam.robot_x;
            roam.bits.push(Bit {
                x,
                y: 30.0,
                born: now,
                kind: BitKind::Heart,
            });
        }
    }

    // The wind-up mouse zips about, turning now and then; later it chases the robot.
    match play {
        Play::Mouse { chased } => {
            if roam.mouse_x < 0.0 {
                roam.mouse_x = margin;
            }
            if chased {
                roam.mouse_v = (roam.robot_x - roam.mouse_x).signum() * SPRINT * 0.9;
            } else if now >= roam.mouse_turn_at || roam.mouse_v == 0.0 {
                let away = (roam.mouse_x - roam.robot_x).signum();
                let away = if away == 0.0 { 1.0 } else { away };
                roam.mouse_v = away * (SPRINT * 1.1);
                roam.mouse_turn_at = now + 1.2 + roam.random() * 1.6;
            }
            roam.mouse_x += roam.mouse_v * dt;
            if roam.mouse_x < margin || roam.mouse_x > width - margin {
                roam.mouse_x = roam.mouse_x.clamp(margin, width - margin);
                roam.mouse_v = -roam.mouse_v;
            }
        }
        _ if roam.mouse_x > -20.0 => {
            // It scurries off the left edge.
            roam.mouse_v = -SPRINT;
            roam.mouse_x += roam.mouse_v * dt;
        }
        _ => {}
    }

    // Dust behind running feet.
    if roam.robot_v.abs() > RUN * 0.7 && (now * 8.0).fract() < dt * 8.0 {
        let x = roam.robot_x - roam.facing * 5.0;
        roam.bits.push(Bit {
            x,
            y: 1.0,
            born: now,
            kind: BitKind::Dust,
        });
    }
    roam.bits.retain(|bit| now - bit.born < 1.4);
}

struct Frame {
    t: f32,
    /// Seconds into the play on the floor, if it has begun.
    play_t: Option<f32>,
    /// The flight now and a few moments ago, for the smoke trail.
    flight: Option<suit::Flight>,
    trail: Vec<suit::Flight>,
    /// Seconds since taking off for home, for the launch shockwave.
    home_t: Option<f32>,
    launch_x: f32,
    land_x: f32,
    robot_x: f32,
    robot_v: f32,
    facing: f32,
    ball: (f32, f32, f32),
    mouse_x: f32,
    mouse_v: f32,
    hop_at: Option<f32>,
    bits: Vec<Bit>,
}

/// Draws the playground: the robot, its ball, the mouse and the little hearts and dust.
/// `width` and `height` are the window size in physical pixels; the floor is its bottom.
pub(crate) fn draw(painter: Painter<'_>, width: f32, height: f32, scale: f32) {
    let now = seconds();
    let unit = UNIT * scale;
    let frame = ROAM.with(|roam| {
        let mut roam = roam.borrow_mut();
        roam.width = width / unit;
        roam.height = height / unit;
        let Phase::Out(started) = roam.phase else {
            return None;
        };
        step(&mut roam, now, started);
        if !matches!(roam.phase, Phase::Out(_)) {
            return None;
        }
        let flight_now = flight(&roam, now, started);
        // Close-set puffs over the last half second, while the thrusters burn hard, so they
        // merge into one plume.
        let trail = (1..=TRAIL_PUFFS)
            .filter_map(|back| flight(&roam, now - back as f32 * 0.025, started))
            .filter(|past| past.thrust > 0.7)
            .collect();
        Some(Frame {
            t: now - started,
            play_t: roam.play_from.map(|from| now - from),
            flight: flight_now,
            trail,
            home_t: roam.home_from.map(|from| now - from),
            launch_x: roam.launch_x,
            land_x: roam.land_x(),
            robot_x: roam.robot_x,
            robot_v: roam.robot_v,
            facing: roam.facing,
            ball: (roam.ball_x, roam.ball_y, roam.ball_spin),
            mouse_x: roam.mouse_x,
            mouse_v: roam.mouse_v,
            hop_at: roam.hop_at,
            bits: roam.bits.clone(),
        })
    });
    let Some(frame) = frame else {
        return;
    };
    let floor = height - 2.0 * scale;
    let stage_at = |x: f32, y: f32| Stage {
        painter,
        unit,
        alpha: 255,
        x: x * unit,
        floor: floor - y * unit,
        mid: floor - (y + 11.0) * unit,
        time: now,
        elapsed: frame.t,
        fade: 1.0,
    };

    // In the air: the smoke trail, then the suited robot.
    if let Some(flying) = frame.flight {
        // Oldest first, so newer, brighter smoke sits on top.
        for (index, past) in frame.trail.iter().enumerate().rev() {
            let age = (index + 1) as f32 / (TRAIL_PUFFS + 1) as f32;
            painter.fill_circle(
                Point::new(past.x * unit, floor - (past.y - 1.0) * unit),
                (1.4 + age * 3.4) * unit * past.scale,
                Rgba::from_argb((70.0 * (1.0 - age)) as u8, 225, 225, 232),
            );
        }
        // Blast-off from the floor on the way home.
        if let Some(home_t) = frame.home_t {
            let boost = home_t - suit::DEPART_SECONDS + 2.8;
            suit::shockwave(painter, Point::new(frame.launch_x * unit, floor), unit, boost / 0.8);
        }
        stage_at(flying.x, flying.y).fly(flying, now);
        return;
    }
    let Some(play_t) = frame.play_t else {
        return;
    };
    // Just landed: a ring of dust, and the suit pieces flying off.
    if play_t < 1.0 {
        suit::shockwave(painter, Point::new(frame.land_x * unit, floor), unit, play_t);
    }
    if play_t < suit::UNSUIT_SECONDS {
        stage_at(frame.robot_x, 0.0).unsuit(play_t / suit::UNSUIT_SECONDS);
    }
    let play = play_at(play_t);
    let (ball_x, ball_y, spin) = frame.ball;

    // A soft shadow under each toy, so they sit on the taskbar rather than float.
    for (x, size) in [
        (frame.robot_x, 8.0),
        (ball_x, 3.2 - ball_y.min(30.0) / 15.0),
        (frame.mouse_x, 3.0),
    ] {
        painter.fill_oval(
            Rect::from_xywh((x - size) * unit, floor - 0.9 * unit, size * 2.0 * unit, 1.8 * unit),
            Rgba::from_argb(60, 0, 0, 0),
        );
    }

    for bit in &frame.bits {
        let age = (now - bit.born) / 1.4;
        match bit.kind {
            BitKind::Heart => glyph(
                painter,
                "\u{2665}",
                Point::new((bit.x - 2.0) * unit, floor - (bit.y + age * 22.0) * unit),
                7.0 * unit,
                Rgba::from_argb((255.0 * (1.0 - age)) as u8, 255, 110, 150),
            ),
            BitKind::Dust => painter.fill_circle(
                Point::new(bit.x * unit, floor - (1.0 + age * 3.0) * unit),
                (1.0 + age * 2.0) * unit,
                Rgba::from_argb((110.0 * (1.0 - age)) as u8, 200, 200, 205),
            ),
        }
    }

    // The ball, red and white, spinning as it rolls; it waits by the wall during the visit.
    let ball = Point::new(ball_x * unit, floor - (3.0 + ball_y) * unit);
    painter.fill_circle(ball, 3.0 * unit, Rgba::from_rgb(235, 80, 80));
    let save = painter.save();
    painter.translate(Vec2::new(ball.x, ball.y));
    painter.rotate_degrees(spin.to_degrees());
    painter.fill_rect(
        Rect::from_xywh(-3.0 * unit, -0.6 * unit, 6.0 * unit, 1.2 * unit),
        Rgba::WHITE,
    );
    painter.restore_to(save);

    if frame.mouse_x > -15.0 && play_t > BALL_SECONDS - 2.0 {
        draw_mouse(painter, Point::new(frame.mouse_x * unit, floor), unit, frame.mouse_v, now);
    }

    // The robot.
    let stage = stage_at(frame.robot_x, 0.0);
    let running = frame.robot_v.abs() > 15.0;
    let hop = frame.hop_at.map_or(0.0, |at| {
        let since = now - at;
        if since < 0.45 {
            (since / 0.45 * PI).sin() * 3.5
        } else {
            0.0
        }
    });
    let mut pose = Pose::new(Eyes::Open(frame.facing * 1.2, 0.2));
    pose.rise = hop;
    if running {
        let swing = (now * 14.0).sin();
        pose.right_arm = 0.7 + swing * 0.7;
        pose.left_arm = 0.7 - swing * 0.7;
        pose.lean = frame.facing * 9.0;
        pose.rise = (now * 14.0).sin().abs() * 1.4 + hop;
    } else {
        match play {
            Play::Icon => {
                // Sitting on an app icon, swinging its legs (well, its whole self).
                pose.eyes = Eyes::Happy;
                pose.rise = -1.2 + (now * 3.0).sin().abs() * 0.6;
                pose.right_arm = 0.4;
                pose.left_arm = 0.4;
            }
            Play::Clock => {
                // Leaning over the edge to read the clock.
                pose.eyes = Eyes::Open(1.0, 1.4);
                pose.lean = 14.0;
                pose.right_arm = 1.5;
            }
            Play::Battery => {
                pose.eyes = Eyes::Closed;
                pose.rise = -2.6;
                pose.right_arm = 1.6;
                pose.left_arm = 1.6;
            }
            Play::Rest => {
                let rest = play_t - (BALL_SECONDS + MOUSE_SECONDS + VISIT_SECONDS);
                pose.eyes = if rest < 1.4 { Eyes::Closed } else { Eyes::Happy };
                pose.rise = -1.5;
                if rest >= 1.6 {
                    pose.right_arm = -0.45 + ((rest - 1.6) / 0.9 * TAU).sin() * 0.45;
                }
            }
            _ => {}
        }
    }
    let fleeing = matches!(play, Play::Mouse { chased: true });
    if fleeing {
        pose.eyes = Eyes::Happy;
        pose.right_arm = -1.2;
        pose.left_arm = -1.2;
    }
    if hop > 0.0 && !fleeing {
        pose.eyes = Eyes::Happy;
    }
    stage.robot(pose);
    if fleeing && (now * 3.0).fract() < 0.6 {
        stage.exclaim(stage.at(4.0, -24.0), 1.0);
    }
    if !running {
        match play {
            Play::Clock => glyph(
                painter,
                "?",
                stage.at(8.0, -24.0),
                9.0 * unit,
                Rgba::from_rgb(255, 214, 90),
            ),
            Play::Battery => {
                // "z z Z" drifting up while it naps on the battery.
                for index in 0..3 {
                    let phase = (now * 0.6 + index as f32 / 3.0).fract();
                    glyph(
                        painter,
                        if index == 2 { "Z" } else { "z" },
                        stage.at(6.0 + phase * 6.0, -20.0 - phase * 12.0),
                        (5.0 + index as f32 * 1.5) * unit,
                        Rgba::from_argb((230.0 * (1.0 - phase)) as u8, 255, 255, 255),
                    );
                }
            }
            Play::Icon if (now * 0.8).fract() < 0.5 => glyph(
                painter,
                "\u{2665}",
                stage.at(7.0, -24.0),
                6.0 * unit,
                Rgba::from_rgb(255, 120, 160),
            ),
            _ => {}
        }
    }
}

fn glyph(painter: Painter<'_>, text: &str, at: Point, size: f32, color: Rgba) {
    FontManager::global().draw_text_cached(DrawTextCachedParams {
        painter,
        text,
        x: at.x,
        y: at.y,
        size,
        bold: true,
        color,
        blur: None,
    });
}

/// A little grey wind-up mouse with a pink ear, a tail and a turning key.
fn draw_mouse(painter: Painter<'_>, at: Point, unit: f32, velocity: f32, now: f32) {
    let facing = if velocity < 0.0 { -1.0 } else { 1.0 };
    let body = Rect::from_xywh(at.x - 4.0 * unit, at.y - 4.2 * unit, 8.0 * unit, 4.2 * unit);
    let grey = Rgba::from_rgb(170, 170, 180);
    painter.fill_oval(body, grey);
    let head = Point::new(at.x + facing * 4.0 * unit, at.y - 2.4 * unit);
    painter.fill_circle(head, 1.8 * unit, grey);
    painter.fill_circle(
        Point::new(head.x - facing * 0.6 * unit, head.y - 1.8 * unit),
        1.0 * unit,
        Rgba::from_rgb(255, 170, 190),
    );
    painter.fill_circle(
        Point::new(head.x + facing * 0.8 * unit, head.y - 0.3 * unit),
        0.35 * unit,
        Rgba::from_rgb(20, 20, 24),
    );
    let tail = Point::new(at.x - facing * 4.0 * unit, at.y - 1.6 * unit);
    painter.stroke_line(
        tail,
        Point::new(tail.x - facing * 3.0 * unit, tail.y - (now * 10.0).sin() * 1.2 * unit),
        0.4 * unit,
        grey,
        winisland_render::StrokeCap::Round,
    );
    let key = Point::new(at.x, at.y - 4.6 * unit);
    let turn = (now * 6.0).sin();
    painter.stroke_line(
        Point::new(key.x - 1.4 * unit * turn, key.y - 0.8 * unit),
        Point::new(key.x + 1.4 * unit * turn, key.y - 0.8 * unit),
        0.6 * unit,
        Rgba::from_rgb(230, 190, 90),
        winisland_render::StrokeCap::Round,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trip_arrives_plays_visits_and_climbs_home() {
        let mut roam = Roam {
            width: 600.0,
            height: 300.0,
            phase: Phase::Out(0.0),
            ..Roam::default()
        };
        let mut now = 0.0;
        let mut kicked = false;
        let mut visited = (false, false);
        while now < 400.0 && matches!(roam.phase, Phase::Out(_)) {
            step(&mut roam, now, 0.0);
            kicked |= roam.last_kick > 0.0;
            if let (Some(from), None) = (roam.play_from, roam.home_from) {
                match play_at(now - from) {
                    Play::Clock if (roam.robot_x - (600.0 - CLOCK_FROM_RIGHT)).abs() < 6.0 => {
                        visited.0 = true;
                    }
                    Play::Battery
                        if (roam.robot_x - (600.0 - BATTERY_FROM_RIGHT)).abs() < 6.0 =>
                    {
                        visited.1 = true;
                    }
                    _ => {}
                }
                assert!(roam.ball_x >= 0.0 && roam.ball_x <= 600.0);
            }
            now += 1.0 / 30.0;
        }
        assert!(kicked, "the robot kicks the ball");
        assert!(visited.0 && visited.1, "it visits the clock and the battery");
        assert_eq!(roam.phase, Phase::Home, "it flies home");
    }

    #[test]
    fn it_lands_where_the_flight_ends_and_takes_off_where_it_stands() {
        let mut roam = Roam {
            width: 600.0,
            height: 300.0,
            phase: Phase::Out(0.0),
            ..Roam::default()
        };
        let landing = flight(&roam, suit::ARRIVE_SECONDS - 0.01, 0.0).unwrap();
        step(&mut roam, suit::ARRIVE_SECONDS + 0.01, 0.0);
        assert!((roam.robot_x - landing.x).abs() < 1.0, "play starts where it landed");
        roam.robot_x = 150.0;
        roam.launch_x = 150.0;
        roam.home_from = Some(100.0);
        let takeoff = flight(&roam, 100.0, 0.0).unwrap();
        assert_eq!(takeoff.x, 150.0);
    }

    #[test]
    fn trips_follow_the_setting() {
        assert_eq!(trip_gap(0), None);
        assert!(trip_gap(2).unwrap() < trip_gap(1).unwrap());
    }
}
