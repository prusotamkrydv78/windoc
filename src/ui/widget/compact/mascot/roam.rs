//! Now and then the robot leaves the notch and plays along the bottom of the screen: on top
//! of the taskbar when it shows, or along the screen's bottom edge when it is hidden.
//!
//! A trip: it runs in from the right, chases and kicks a ball that rolls and bounces off the
//! screen edges, then chases a wind-up mouse that turns round and chases it back, sits for a
//! yawn and a wave, and runs off with its ball. Meanwhile the notch stays empty; when it is
//! back it walks into the notch again. The app owns the see-through window it plays in
//! (`window::app::roam_window`) and calls [`tick`] and [`draw`].

use std::cell::RefCell;
use std::f32::consts::{PI, TAU};

use winisland_render::text::{DrawTextCachedParams, FontManager};
use winisland_render::{Painter, Point, Rect};

use super::life::{self, Activity};
use super::{Eyes, Pose, Stage, demo, seconds};

/// Height of the playground strip above its floor, in logical pixels.
pub(crate) const PLAYGROUND_HEIGHT: f32 = 120.0;
/// Logical pixels per robot unit: about 48 px tall, a toy on the taskbar.
const UNIT: f32 = 1.9;

/// How long each part of a trip lasts, in seconds.
const BALL_SECONDS: f32 = 24.0;
const MOUSE_SECONDS: f32 = 18.0;
const REST_SECONDS: f32 = 5.0;
/// Running speeds in robot units per second.
const RUN: f32 = 120.0;
const SPRINT: f32 = 170.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    /// In the notch as usual.
    Home,
    /// A trip is due: the notch robot leaves at the end of its current scene.
    Leaving,
    /// Out playing; the trip started at this robot time.
    Out(f32),
}

/// One floating heart or puff of dust.
#[derive(Clone, Copy)]
struct Bit {
    x: f32,
    y: f32,
    born: f32,
    heart: bool,
}

struct Roam {
    phase: Phase,
    /// When the next trip is due, in robot seconds.
    next_trip: Option<f32>,
    /// When the robot got back to the notch, so it can walk in.
    returned_at: Option<f32>,
    /// Playground width in robot units.
    width: f32,
    last_step: Option<f32>,
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
    /// The last kick and the last time the robot hopped (clicked, or for joy).
    last_kick: f32,
    hop_at: Option<f32>,
    bits: Vec<Bit>,
    /// A cheap random sequence for kicks and the mouse's turns.
    seed: u32,
}

impl Default for Roam {
    fn default() -> Self {
        Self {
            phase: Phase::Home,
            next_trip: None,
            returned_at: None,
            width: 800.0,
            last_step: None,
            robot_x: 0.0,
            robot_v: 0.0,
            facing: -1.0,
            ball_x: 0.0,
            ball_y: 0.0,
            ball_vx: 0.0,
            ball_vy: 0.0,
            ball_spin: 0.0,
            mouse_x: 0.0,
            mouse_v: 0.0,
            mouse_turn_at: 0.0,
            last_kick: -10.0,
            hop_at: None,
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
}

thread_local! {
    static ROAM: RefCell<Roam> = RefCell::new(Roam::default());
}

/// Seconds between trips for the Settings choice: 0 never, 1 now and then, 2 often.
fn trip_gap(frequency: u8) -> Option<f32> {
    if demo() && std::env::var_os("WINISLAND_MASCOT_DEMO_ROAM").is_some() {
        return Some(15.0);
    }
    match frequency {
        1 => Some(25.0 * 60.0),
        2 => Some(8.0 * 60.0),
        _ => None,
    }
}

/// Whether the robot is away from the notch, playing, so the notch draws no robot.
pub(crate) fn away_from_notch() -> bool {
    ROAM.with(|roam| matches!(roam.borrow().phase, Phase::Out(_)))
}

/// Whether a trip is due, so the notch robot leaves at the end of its scene.
pub(super) fn wants_to_leave() -> bool {
    ROAM.with(|roam| roam.borrow().phase == Phase::Leaving)
}

thread_local! {
    /// The notch scene's time at the last frame, to spot the moment a scene ends.
    static LAST_SCENE_TIME: std::cell::Cell<Option<f32>> = const { std::cell::Cell::new(None) };
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

/// The notch robot has walked off: the trip begins.
fn left_notch() {
    ROAM.with(|roam| {
        let mut roam = roam.borrow_mut();
        if roam.phase == Phase::Leaving {
            let now = seconds();
            roam.phase = Phase::Out(now);
            roam.last_step = None;
            // In from the right, the ball rolling ahead of it.
            let width = roam.width;
            roam.robot_x = width + 20.0;
            roam.robot_v = -RUN;
            roam.facing = -1.0;
            roam.ball_x = width + 4.0;
            roam.ball_y = 0.0;
            roam.ball_vx = -RUN * 1.4;
            roam.ball_vy = 0.0;
            roam.mouse_x = -20.0;
            roam.mouse_v = 0.0;
            roam.bits.clear();
        }
    });
}

/// Seconds since the robot came back to the notch, while it walks in.
pub(super) fn since_return() -> Option<f32> {
    ROAM.with(|roam| roam.borrow().returned_at.map(|at| seconds() - at))
}

/// Whether the robot is out playing and the playground needs frames.
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

/// Ends a trip early (the user needs the screen), sending the robot home.
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
    let gap = trip_gap(1).unwrap_or(25.0 * 60.0);
    roam.next_trip = Some(now + gap);
}

/// The robot's spot in the playground, in physical pixels, for hit testing; `None` when
/// it is not out.
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

/// The robot was poked in the playground: a happy hop with a heart.
pub(crate) fn poked() {
    ROAM.with(|roam| {
        let mut roam = roam.borrow_mut();
        let now = seconds();
        roam.hop_at = Some(now);
        let (x, born) = (roam.robot_x, now);
        roam.bits.push(Bit {
            x,
            y: 30.0,
            born,
            heart: true,
        });
    });
}

/// Advances the play to `now`, for a trip that started at `started`.
fn step(roam: &mut Roam, now: f32, started: f32) {
    let dt = roam
        .last_step
        .map_or(0.0, |last| (now - last).clamp(0.0, 0.1));
    roam.last_step = Some(now);
    let t = now - started;
    let width = roam.width;
    let margin = 12.0;

    // The ball: rolls with friction, bounces off the edges and the floor.
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
    if t > 2.0 {
        if roam.ball_x < margin {
            roam.ball_x = margin;
            roam.ball_vx = roam.ball_vx.abs() * 0.7;
        } else if roam.ball_x > width - margin {
            roam.ball_x = width - margin;
            roam.ball_vx = -roam.ball_vx.abs() * 0.7;
        }
    }

    // What the robot runs towards.
    let leaving = t > BALL_SECONDS + MOUSE_SECONDS + REST_SECONDS;
    let target = if leaving {
        width + 40.0
    } else if t < BALL_SECONDS {
        roam.ball_x
    } else if t < BALL_SECONDS + MOUSE_SECONDS {
        let chased = t > BALL_SECONDS + MOUSE_SECONDS * 0.55;
        // First it chases the mouse; then the mouse turns and it runs away, giggling.
        if chased {
            roam.robot_x + (roam.robot_x - roam.mouse_x).signum() * 60.0
        } else {
            roam.mouse_x
        }
    } else {
        roam.robot_x
    };
    let gap = target - roam.robot_x;
    let speed = if t >= BALL_SECONDS && t < BALL_SECONDS + MOUSE_SECONDS {
        SPRINT
    } else {
        RUN
    };
    let wanted = if gap.abs() < 6.0 {
        0.0
    } else {
        gap.signum() * speed.min(gap.abs() * 3.0)
    };
    // Ease into and out of a run rather than snapping to speed.
    roam.robot_v += (wanted - roam.robot_v) * (1.0 - (-6.0 * dt).exp());
    roam.robot_x += roam.robot_v * dt;
    if roam.robot_v.abs() > 8.0 {
        roam.facing = roam.robot_v.signum();
    }
    if !leaving {
        roam.robot_x = roam.robot_x.clamp(margin, width - margin);
    }

    // Kick the ball when it is close and not already flying off.
    if t < BALL_SECONDS && (roam.ball_x - roam.robot_x).abs() < 9.0 && now - roam.last_kick > 0.8
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
                heart: true,
            });
        }
    }

    // The wind-up mouse zips about, turning now and then; later it chases the robot.
    if t >= BALL_SECONDS - 1.0 && t < BALL_SECONDS + MOUSE_SECONDS {
        let chasing = t > BALL_SECONDS + MOUSE_SECONDS * 0.55;
        if chasing {
            let toward = (roam.robot_x - roam.mouse_x).signum();
            roam.mouse_v = toward * SPRINT * 0.9;
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
    } else if t >= BALL_SECONDS + MOUSE_SECONDS {
        // It scurries off the left edge.
        roam.mouse_v = -SPRINT;
        roam.mouse_x += roam.mouse_v * dt;
    }

    // Dust behind running feet.
    if roam.robot_v.abs() > RUN * 0.7 && (now * 8.0).fract() < dt * 8.0 {
        let x = roam.robot_x - roam.facing * 5.0;
        roam.bits.push(Bit {
            x,
            y: 1.0,
            born: now,
            heart: false,
        });
    }
    roam.bits.retain(|bit| now - bit.born < 1.4);

    if leaving && roam.robot_x > width + 30.0 {
        finish(roam);
    }
}

/// Draws the playground: the robot, its ball, the mouse and the little hearts and dust.
/// `width` and `height` are the window size in physical pixels.
pub(crate) fn draw(painter: Painter<'_>, width: f32, height: f32, scale: f32) {
    let now = seconds();
    let unit = UNIT * scale;
    let snapshot = ROAM.with(|roam| {
        let mut roam = roam.borrow_mut();
        roam.width = width / unit;
        let Phase::Out(started) = roam.phase else {
            return None;
        };
        step(&mut roam, now, started);
        Some((
            now - started,
            roam.robot_x,
            roam.robot_v,
            roam.facing,
            (roam.ball_x, roam.ball_y, roam.ball_spin),
            roam.mouse_x,
            roam.mouse_v,
            roam.hop_at,
            roam.bits.clone(),
        ))
    });
    let Some((t, robot_x, robot_v, facing, (ball_x, ball_y, spin), mouse_x, mouse_v, hop_at, bits)) =
        snapshot
    else {
        return;
    };
    let floor = height - 2.0 * scale;
    let stage = Stage {
        painter,
        unit,
        alpha: 255,
        x: robot_x * unit,
        floor,
        mid: floor - 11.0 * unit,
        time: now,
        elapsed: t,
        fade: 1.0,
    };

    // A soft shadow under each toy, so they sit on the taskbar rather than float.
    for (x, size) in [(robot_x, 8.0), (ball_x, 3.2 - ball_y.min(30.0) / 15.0), (mouse_x, 3.0)] {
        painter.fill_oval(
            Rect::from_xywh(
                (x - size) * unit,
                floor - 0.9 * unit,
                size * 2.0 * unit,
                1.8 * unit,
            ),
            winisland_render::Rgba::from_argb(60, 0, 0, 0),
        );
    }

    // Dust and hearts.
    for bit in &bits {
        let age = (now - bit.born) / 1.4;
        if bit.heart {
            FontManager::global().draw_text_cached(DrawTextCachedParams {
                painter,
                text: "\u{2665}",
                x: (bit.x - 2.0) * unit,
                y: floor - (bit.y + age * 22.0) * unit,
                size: 7.0 * unit,
                bold: false,
                color: winisland_render::Rgba::from_argb(
                    (255.0 * (1.0 - age)) as u8,
                    255,
                    110,
                    150,
                ),
                blur: None,
            });
        } else {
            painter.fill_circle(
                Point::new(bit.x * unit, floor - (1.0 + age * 3.0) * unit),
                (1.0 + age * 2.0) * unit,
                winisland_render::Rgba::from_argb((110.0 * (1.0 - age)) as u8, 200, 200, 205),
            );
        }
    }

    // The ball: red and white, spinning as it rolls.
    let ball = Point::new(ball_x * unit, floor - (3.0 + ball_y) * unit);
    painter.fill_circle(ball, 3.0 * unit, winisland_render::Rgba::from_rgb(235, 80, 80));
    let save = painter.save();
    painter.translate(winisland_render::Vec2::new(ball.x, ball.y));
    painter.rotate_degrees(spin.to_degrees());
    painter.fill_rect(
        Rect::from_xywh(-3.0 * unit, -0.6 * unit, 6.0 * unit, 1.2 * unit),
        winisland_render::Rgba::WHITE,
    );
    painter.restore_to(save);

    // The wind-up mouse, its key turning, facing where it runs.
    if t > BALL_SECONDS - 2.0 {
        draw_mouse(painter, Point::new(mouse_x * unit, floor), unit, mouse_v, now);
    }

    // The robot: running, kicking, fleeing, resting or waving.
    let rest_start = BALL_SECONDS + MOUSE_SECONDS;
    let running = robot_v.abs() > 15.0;
    let hop = hop_at.map_or(0.0, |at| {
        let since = now - at;
        if since < 0.45 {
            (since / 0.45 * PI).sin() * 3.5
        } else {
            0.0
        }
    });
    let mut pose = Pose::new(Eyes::Open(facing * 1.2, 0.2));
    if running {
        let swing = (now * 14.0).sin();
        pose.right_arm = 0.7 + swing * 0.7;
        pose.left_arm = 0.7 - swing * 0.7;
        pose.lean = facing * 9.0;
        pose.rise = (now * 14.0).sin().abs() * 1.4 + hop;
    } else {
        pose.rise = hop;
    }
    let fleeing = t > BALL_SECONDS + MOUSE_SECONDS * 0.55 && t < rest_start;
    if fleeing {
        pose.eyes = Eyes::Happy;
        pose.right_arm = -1.2;
        pose.left_arm = -1.2;
    }
    if (rest_start..rest_start + REST_SECONDS).contains(&t) && !running {
        // A sit, a yawn, then a wave at the user.
        let rest = t - rest_start;
        pose.eyes = if rest < 1.6 { Eyes::Closed } else { Eyes::Happy };
        pose.rise = -1.5;
        if rest >= 1.8 {
            pose.right_arm = -0.45 + ((rest - 1.8) / 0.9 * TAU).sin() * 0.45;
        }
    }
    if hop > 0.0 && !fleeing {
        pose.eyes = Eyes::Happy;
    }
    stage.robot(pose);
    if fleeing && (now * 3.0).fract() < 0.6 {
        stage.exclaim(stage.at(4.0, -24.0), 1.0);
    }
}

/// A little grey wind-up mouse with a pink ear, a tail and a turning key.
fn draw_mouse(painter: Painter<'_>, at: Point, unit: f32, velocity: f32, now: f32) {
    let facing = if velocity < 0.0 { -1.0 } else { 1.0 };
    let body = Rect::from_xywh(at.x - 4.0 * unit, at.y - 4.2 * unit, 8.0 * unit, 4.2 * unit);
    let grey = winisland_render::Rgba::from_rgb(170, 170, 180);
    painter.fill_oval(body, grey);
    let head = Point::new(at.x + facing * 4.0 * unit, at.y - 2.4 * unit);
    painter.fill_circle(head, 1.8 * unit, grey);
    painter.fill_circle(
        Point::new(head.x - facing * 0.6 * unit, head.y - 1.8 * unit),
        1.0 * unit,
        winisland_render::Rgba::from_rgb(255, 170, 190),
    );
    painter.fill_circle(
        Point::new(head.x + facing * 0.8 * unit, head.y - 0.3 * unit),
        0.35 * unit,
        winisland_render::Rgba::from_rgb(20, 20, 24),
    );
    // Tail.
    let tail = Point::new(at.x - facing * 4.0 * unit, at.y - 1.6 * unit);
    painter.stroke_line(
        tail,
        Point::new(tail.x - facing * 3.0 * unit, tail.y - (now * 10.0).sin() * 1.2 * unit),
        0.4 * unit,
        grey,
        winisland_render::StrokeCap::Round,
    );
    // The wind-up key on its back, turning.
    let key = Point::new(at.x, at.y - 4.6 * unit);
    let turn = (now * 6.0).sin();
    painter.stroke_line(
        Point::new(key.x - 1.4 * unit * turn, key.y - 0.8 * unit),
        Point::new(key.x + 1.4 * unit * turn, key.y - 0.8 * unit),
        0.6 * unit,
        winisland_render::Rgba::from_rgb(230, 190, 90),
        winisland_render::StrokeCap::Round,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trip_plays_and_ends_with_the_robot_home() {
        let mut roam = Roam {
            width: 600.0,
            phase: Phase::Out(0.0),
            robot_x: 620.0,
            ball_x: 604.0,
            ball_vx: -RUN * 1.4,
            ..Roam::default()
        };
        let total = BALL_SECONDS + MOUSE_SECONDS + REST_SECONDS + 12.0;
        let mut now = 0.0;
        let mut kicked = false;
        while now < total && matches!(roam.phase, Phase::Out(_)) {
            step(&mut roam, now, 0.0);
            kicked |= roam.last_kick > 0.0;
            if matches!(roam.phase, Phase::Out(_)) && now > 2.0 && now < BALL_SECONDS {
                // The ball stays on the playground.
                assert!(roam.ball_x >= 0.0 && roam.ball_x <= 600.0);
            }
            now += 1.0 / 30.0;
        }
        assert!(kicked, "the robot kicks the ball at least once");
        assert_eq!(roam.phase, Phase::Home, "the robot runs off and goes home");
        assert!(roam.returned_at.is_some());
    }

    #[test]
    fn trips_follow_the_setting() {
        assert_eq!(trip_gap(0), None);
        assert!(trip_gap(2).unwrap() < trip_gap(1).unwrap());
    }
}
