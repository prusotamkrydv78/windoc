//! The robot's dance moves and its newer everyday scenes, and the shuffle that picks them.
//!
//! Dancing: a move lasts a few seconds, the next is picked at random (never the same twice
//! in a row) and the robot blends from one pose into the next, so it never snaps.

use std::f32::consts::{PI, TAU};

use winisland_render::{Point, Radius, Rect};

use super::{Eyes, Pose, Stage, WHITE, ease};

/// How long each dance move lasts, and how long the blend into the next takes.
const MOVE_SECONDS: f32 = 3.2;
const BLEND_SECONDS: f32 = 0.45;
const MOVES: u32 = 13;

/// A cheap, well-mixed hash: the same `n` always gives the same pick, different ones look
/// random.
pub(super) fn mix(n: u32) -> u32 {
    let mut x = n.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^ (x >> 15)
}

thread_local! {
    /// Per shuffle (`salt`): the last slot picked, its pick and the pick before it.
    static PICKS: std::cell::RefCell<std::collections::HashMap<u32, (u32, u32, u32)>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// A random pick out of `count` for slot `index`, never the same as the slot before. Slots
/// are asked for in order as time passes; each pick steps a random distance from the last,
/// so it can never land on it.
pub(super) fn shuffled(index: u32, count: u32, salt: u32) -> u32 {
    if count <= 1 {
        return 0;
    }
    let fresh = |i: u32| mix(i ^ salt) % count;
    let step = |previous: u32, i: u32| (previous + 1 + mix(i ^ salt ^ 0xA5A5) % (count - 1)) % count;
    PICKS.with(|picks| {
        let mut picks = picks.borrow_mut();
        let base = match picks.get(&salt).copied() {
            Some((last, value, _)) if last == index => return value,
            Some((last, _, previous)) if last == index + 1 => return previous,
            Some((last, value, _)) if last + 1 == index => value,
            // A jump (first use, or the clock skipped ahead): start from a fresh pick.
            _ => fresh(index.wrapping_sub(1)),
        };
        let pick = step(base, index);
        picks.insert(salt, (index, pick, base));
        pick
    })
}

/// One dance move's pose at `t` seconds in, and how far to the side it steps.
fn dance_move(id: u32, t: f32) -> (Pose, f32) {
    let beat = (t * 2.0 * TAU).sin();
    let step = (t * TAU).sin();
    let mut pose = Pose::new(Eyes::Happy);
    let mut dx = 0.0;
    match id {
        // Arm swaps with a bounce.
        0 => {
            pose.right_arm = -0.9 + beat * 0.8;
            pose.left_arm = -0.9 - beat * 0.8;
            pose.rise = beat.abs() * 1.4;
            pose.lean = step * 8.0;
        }
        // Side-steps with claps.
        1 => {
            dx = step * 3.0;
            let clap = beat.abs();
            pose.right_arm = 0.2 + clap * 0.9;
            pose.left_arm = pose.right_arm;
            pose.lean = -step * 6.0;
            pose.rise = clap * 0.8;
        }
        // Arms up, swaying.
        2 => {
            pose.right_arm = -1.6 + beat * 0.25;
            pose.left_arm = -1.6 - beat * 0.25;
            pose.lean = step * 12.0;
            pose.rise = (beat * 0.5 + 0.5) * 1.6;
        }
        // The floss: both arms swing the same way, fast, hips the other way.
        3 => {
            let swing = (t * 3.0 * TAU).sin();
            pose.right_arm = 1.0 + swing * 0.9;
            pose.left_arm = 1.0 - swing * 0.9;
            pose.lean = -swing * 7.0;
            dx = swing * 1.2;
        }
        // A spin, then a little bow.
        4 => {
            let turn = (t / 1.2).min(1.0);
            pose.lean = 360.0 * ease(turn);
            pose.rise = (turn * PI).sin() * 2.5;
            pose.right_arm = -1.2;
            pose.left_arm = -1.2;
            if t > 1.4 {
                pose.lean = ((t - 1.4) / 0.6).min(1.0).sin() * 14.0;
                pose.right_arm = 0.8;
            }
        }
        // The robot: stiff arms that snap to a new angle on every beat.
        5 => {
            let tick = (t * 4.0) as u32;
            let angles = [-0.6, 0.2, 0.9, -1.2, 0.5, -0.2];
            pose.right_arm = angles[(mix(tick) % 6) as usize];
            pose.left_arm = angles[(mix(tick + 7) % 6) as usize];
            pose.eyes = Eyes::Open(0.0, 0.0);
            pose.lean = if tick % 2 == 0 { 4.0 } else { -4.0 };
        }
        // Jumping claps.
        6 => {
            let hop = (t * 1.6 * PI).sin().abs();
            pose.rise = hop * 4.0;
            pose.right_arm = if hop > 0.8 { -1.4 } else { 0.4 };
            pose.left_arm = pose.right_arm;
        }
        // The wave: arms ripple overhead.
        7 => {
            pose.right_arm = -1.3 + (t * 5.0).sin() * 0.5;
            pose.left_arm = -1.3 + (t * 5.0 + 1.6).sin() * 0.5;
            pose.lean = (t * 2.5).sin() * 9.0;
            pose.rise = (t * 5.0).sin().abs() * 0.8;
        }
        // A shuffle: quick little steps side to side.
        8 => {
            dx = (t * 2.0 * TAU).sin().signum() * ((t * 4.0 * TAU).sin().abs() * 1.6);
            pose.rise = (t * 4.0 * TAU).sin().abs() * 1.2;
            pose.right_arm = 0.9 + beat * 0.4;
            pose.left_arm = 0.9 - beat * 0.4;
        }
        // Head bopping to the beat.
        9 => {
            pose.lean = (t * 4.0 * TAU).sin() * 6.0;
            pose.right_arm = 1.25 + (t * 4.0 * TAU).sin() * 0.2;
            pose.left_arm = pose.right_arm;
            pose.eyes = Eyes::Closed;
        }
        // Airplane: arms out, banking left and right.
        10 => {
            pose.right_arm = -0.05;
            pose.left_arm = -0.05;
            pose.lean = (t * 1.6).sin() * 18.0;
            dx = (t * 1.6).sin() * 2.5;
        }
        // The twist.
        11 => {
            let twist = (t * 3.0 * TAU).sin();
            pose.lean = twist * 10.0;
            pose.right_arm = 0.6 - twist * 0.6;
            pose.left_arm = 0.6 + twist * 0.6;
            pose.rise = (twist.abs() - 0.5).max(0.0) * 1.2;
        }
        // The disco point: one arm up to the sky, then down across.
        _ => {
            let up = (t * 1.5 * TAU).sin() > 0.0;
            pose.right_arm = if up { -1.9 } else { 2.2 };
            pose.left_arm = 1.4;
            pose.lean = if up { -10.0 } else { 6.0 };
            pose.rise = 0.6;
        }
    }
    (pose, dx)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

impl Stage<'_> {
    /// Dancing to music: a random move every few seconds, blended into the next.
    pub(super) fn dance_mix(&self) {
        let slot = (self.time / MOVE_SECONDS) as u32;
        let into = self.time - slot as f32 * MOVE_SECONDS;
        let current = shuffled(slot, MOVES, 0x5eed);
        let (mut pose, mut dx) = dance_move(current, into);
        // Blend out of the previous move over the first moments of this one.
        if slot > 0 && into < BLEND_SECONDS {
            let previous = shuffled(slot - 1, MOVES, 0x5eed);
            let (from, from_dx) = dance_move(previous, into + MOVE_SECONDS);
            let k = ease(into / BLEND_SECONDS);
            pose.right_arm = lerp(from.right_arm, pose.right_arm, k);
            pose.left_arm = lerp(from.left_arm, pose.left_arm, k);
            pose.rise = lerp(from.rise, pose.rise, k);
            // A spin ends a full turn round, which is the same as upright.
            pose.lean = lerp(from.lean % 360.0, pose.lean, k);
            dx = lerp(from_dx, dx, k);
        }
        self.shifted(dx).robot(pose);
        self.notes();
    }

    /// Juggling three balls.
    pub(super) fn juggle(&self) {
        let u = self.unit;
        let mut pose = Pose::new(Eyes::Open(0.0, -1.4));
        let toss = (self.time * 3.0 * TAU).sin();
        pose.right_arm = 0.1 + toss * 0.35;
        pose.left_arm = 0.1 - toss * 0.35;
        let hands = self.robot(pose);
        let colours = [(255, 110, 110), (110, 200, 255), (255, 210, 90)];
        for (index, colour) in colours.into_iter().enumerate() {
            let phase = (self.time * 0.9 + index as f32 / 3.0).fract();
            // Each ball arcs from the left hand to the right and back underneath.
            let x = lerp(hands.left.x, hands.right.x, (phase * TAU).cos() * -0.5 + 0.5);
            let y = hands.right.y - (phase * TAU).sin().abs() * 11.0 * u;
            self.painter.fill_circle(Point::new(x, y), 1.4 * u, self.prop(1.0, colour));
        }
    }

    /// A yoga balance: one foot up, arms overhead, eyes closed, a gentle sway.
    pub(super) fn yoga(&self) {
        let mut pose = Pose::new(Eyes::Closed);
        let sway = (self.time * 0.8).sin();
        pose.right_arm = -1.55;
        pose.left_arm = -1.55;
        pose.lean = sway * 4.0;
        pose.rise = 1.6;
        self.robot(pose);
        // A yoga mat under its feet.
        self.painter.fill_round_rect(
            self.rect(-9.0, -1.0, 18.0, 1.4),
            Radius::uniform(0.7 * self.unit),
            self.prop(1.0, (150, 120, 230)),
        );
    }

    /// Blowing bubbles that float up and pop.
    pub(super) fn bubbles(&self) {
        let u = self.unit;
        let mut pose = Pose::new(Eyes::Happy);
        pose.right_arm = -0.4;
        let hands = self.robot(pose);
        // The wand: a stick and a ring.
        let ring = Point::new(hands.right.x + 1.6 * u, hands.right.y - 3.2 * u);
        self.line(hands.right, ring, 0.4, self.prop(1.0, (200, 200, 210)));
        self.painter
            .stroke_circle(ring, 1.4 * u, 0.4 * u, self.prop(1.0, (200, 200, 210)));
        for index in 0..5 {
            let age = (self.time * 0.35 + index as f32 / 5.0).fract();
            let x = ring.x + (age * 14.0 + (age * 9.0 + index as f32).sin() * 1.5) * u;
            let y = ring.y - age * 14.0 * u;
            let radius = (0.8 + age * 1.6) * u;
            let fade = 1.0 - (age - 0.75).max(0.0) / 0.25;
            self.painter.stroke_circle(
                Point::new(x, y),
                radius,
                0.25 * u,
                self.prop(0.8 * fade, (190, 225, 255)),
            );
            self.painter.fill_circle(
                Point::new(x - radius * 0.35, y - radius * 0.35),
                radius * 0.2,
                self.prop(0.9 * fade, WHITE),
            );
        }
    }

    /// Strumming a little guitar, notes rising.
    pub(super) fn guitar(&self) {
        let u = self.unit;
        let strum = (self.time * 4.0 * TAU).sin();
        let mut pose = Pose::new(Eyes::Closed);
        pose.right_arm = 0.7 + strum * 0.25;
        pose.left_arm = -0.2;
        pose.lean = (self.time * 2.0).sin() * 4.0;
        self.robot(pose);
        // The guitar across its body: a body, a sound hole and a neck to the left.
        let body = self.rect(-1.0, -7.4, 7.0, 5.4);
        self.painter
            .fill_oval(body, self.prop(1.0, (214, 140, 70)));
        self.painter.fill_circle(
            Point::new(body.center_x(), body.center_y()),
            1.0 * u,
            self.prop(1.0, (90, 50, 20)),
        );
        self.line(
            Point::new(body.left + 0.5 * u, body.center_y() - 0.4 * u),
            self.at(-12.0, -11.5),
            0.9,
            self.prop(1.0, (120, 80, 40)),
        );
        self.notes();
    }

    /// Rolling on a skateboard, side to side, with a little ollie now and then.
    pub(super) fn skate(&self) {
        let u = self.unit;
        let roll = (self.time * 0.9).sin();
        let ollie = {
            let at = self.time % 4.0;
            if (2.6..3.2).contains(&at) {
                ((at - 2.6) / 0.6 * PI).sin() * 3.0
            } else {
                0.0
            }
        };
        let rider = self.shifted(roll * 6.0);
        let mut pose = Pose::new(Eyes::Happy);
        pose.right_arm = -0.2 + roll * 0.3;
        pose.left_arm = 0.4 - roll * 0.3;
        pose.lean = -roll * 8.0;
        pose.rise = ollie + 1.2;
        rider.robot(pose);
        // The board and its wheels, lifted with the ollie.
        let board = Rect::from_xywh(
            rider.x - 7.0 * u,
            self.floor - (1.4 + ollie) * u,
            14.0 * u,
            1.2 * u,
        );
        self.painter
            .fill_round_rect(board, Radius::uniform(0.6 * u), self.prop(1.0, (230, 90, 90)));
        for side in [-1.0f32, 1.0] {
            self.painter.fill_circle(
                Point::new(rider.x + side * 4.6 * u, board.bottom + 0.4 * u),
                0.8 * u,
                self.prop(1.0, (60, 60, 70)),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shuffle_never_repeats_and_uses_every_choice() {
        let mut seen = [false; 13];
        for index in 0..200 {
            let here = shuffled(index, 13, 7);
            seen[here as usize] = true;
            if index > 0 {
                assert_ne!(here, shuffled(index - 1, 13, 7), "slot {index} repeats");
            }
        }
        assert!(seen.iter().all(|seen| *seen), "every choice comes up");
    }
}
