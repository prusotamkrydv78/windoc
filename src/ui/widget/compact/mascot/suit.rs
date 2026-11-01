//! The robot's red-and-gold flying suit for its trips to the taskbar: the pieces fly in and
//! snap on, the boots fire, it lifts off, swoops with a barrel roll and lands with a
//! shockwave; going home, the suit comes back on, it boosts up in an arc and zips into the
//! notch. Pure flight geometry ([`arrival`], [`departure`]) plus the drawing.

use std::f32::consts::PI;

use winisland_render::{Painter, Point, Radius, Rect, Rgba, Vec2};

use super::{Eyes, Pose, Stage, ease};

const RED: Rgba = Rgba::from_rgb(196, 30, 36);
const RED_DARK: Rgba = Rgba::from_rgb(140, 18, 24);
const GOLD: Rgba = Rgba::from_rgb(236, 186, 64);
const GLOW: Rgba = Rgba::from_rgb(150, 230, 255);

/// Seconds for each stage of arriving: suiting up under the notch, the boots firing and a
/// hover, the flight down, and the landing.
const SUIT_UP: f32 = 1.4;
const IGNITE: f32 = 0.7;
const FLY_DOWN: f32 = 2.4;
const LAND: f32 = 0.5;
pub(super) const ARRIVE_SECONDS: f32 = SUIT_UP + IGNITE + FLY_DOWN + LAND;
/// Seconds for going home: suiting up where it stands, a boost, and the flight up.
const BOOST: f32 = 0.6;
const FLY_UP: f32 = 2.2;
pub(super) const DEPART_SECONDS: f32 = SUIT_UP + BOOST + FLY_UP;
/// How long the suit pieces take to fly off after landing.
pub(super) const UNSUIT_SECONDS: f32 = 0.7;

/// Where the robot is in flight, in robot units with y up from the floor.
#[derive(Clone, Copy, Debug)]
pub(super) struct Flight {
    pub x: f32,
    pub y: f32,
    /// Tilt in degrees, leaning into the flight.
    pub tilt: f32,
    /// A barrel roll in radians (0 = facing the user).
    pub roll: f32,
    /// How much of the suit is on, 0 to 1.
    pub suited: f32,
    /// Thruster strength, 0 to 1.
    pub thrust: f32,
    /// Size, shrinking as it zips into the notch.
    pub scale: f32,
}

/// A point on a quadratic curve.
fn curve(from: (f32, f32), via: (f32, f32), to: (f32, f32), t: f32) -> (f32, f32) {
    let u = 1.0 - t;
    (
        u * u * from.0 + 2.0 * u * t * via.0 + t * t * to.0,
        u * u * from.1 + 2.0 * u * t * via.1 + t * t * to.1,
    )
}

/// Arriving `t` seconds in: from just under the notch (`notch_x`, `top_y`) down to `land_x`
/// on the floor. `None` once landed.
pub(super) fn arrival(t: f32, notch_x: f32, top_y: f32, land_x: f32) -> Option<Flight> {
    if t >= ARRIVE_SECONDS {
        return None;
    }
    let hover_y = top_y - 8.0;
    if t < SUIT_UP {
        // Drops out of the notch a little, then hangs there while the suit snaps on.
        let drop = ease(t / 0.4);
        return Some(Flight {
            x: notch_x,
            y: top_y + 6.0 - drop * 14.0,
            tilt: 0.0,
            roll: 0.0,
            suited: ((t - 0.2) / (SUIT_UP - 0.3)).clamp(0.0, 1.0),
            thrust: if t > SUIT_UP - 0.3 { 0.3 } else { 0.0 },
            scale: 1.0,
        });
    }
    let t = t - SUIT_UP;
    if t < IGNITE {
        // Boots fire; a wobbly hover.
        return Some(Flight {
            x: notch_x + (t * 9.0).sin() * 0.8,
            y: hover_y + (t * 7.0).sin() * 1.2,
            tilt: (t * 9.0).sin() * 3.0,
            roll: 0.0,
            suited: 1.0,
            thrust: 0.5 + 0.5 * ease(t / IGNITE),
            scale: 1.0,
        });
    }
    let t = t - IGNITE;
    if t < FLY_DOWN {
        // Out and round in a swoop, rolling once, then down to the floor.
        let p = ease(t / FLY_DOWN);
        let via = (notch_x + (land_x - notch_x) * 1.25, hover_y + 30.0);
        let (x, y) = curve((notch_x, hover_y), via, (land_x, 6.0), p);
        let (ahead_x, _) = curve((notch_x, hover_y), via, (land_x, 6.0), (p + 0.02).min(1.0));
        let heading = (ahead_x - x).signum();
        return Some(Flight {
            x,
            y,
            tilt: heading * 30.0 * (1.0 - p * 0.6),
            roll: if (0.3..0.7).contains(&p) {
                (p - 0.3) / 0.4 * 2.0 * PI
            } else {
                0.0
            },
            suited: 1.0,
            thrust: 1.0,
            scale: 1.0,
        });
    }
    let t = t - FLY_DOWN;
    // Touchdown: a crouch on landing, thrusters off.
    Some(Flight {
        x: land_x,
        y: 6.0 * (1.0 - ease(t / LAND)),
        tilt: 0.0,
        roll: 0.0,
        suited: 1.0,
        thrust: (1.0 - t / LAND).max(0.0) * 0.6,
        scale: 1.0,
    })
}

/// Going home `t` seconds in: from `launch_x` on the floor up into the notch. `None` once
/// it is in.
pub(super) fn departure(t: f32, launch_x: f32, notch_x: f32, top_y: f32) -> Option<Flight> {
    if t >= DEPART_SECONDS {
        return None;
    }
    if t < SUIT_UP {
        return Some(Flight {
            x: launch_x,
            y: 0.0,
            tilt: 0.0,
            roll: 0.0,
            suited: (t / (SUIT_UP - 0.2)).clamp(0.0, 1.0),
            thrust: if t > SUIT_UP - 0.3 { 0.4 } else { 0.0 },
            scale: 1.0,
        });
    }
    let t = t - SUIT_UP;
    if t < BOOST {
        // A crouch, then a blast off the floor.
        let p = t / BOOST;
        return Some(Flight {
            x: launch_x,
            y: if p < 0.5 { -1.5 * (p / 0.5) } else { (p - 0.5) * 2.0 * 10.0 },
            tilt: 0.0,
            roll: 0.0,
            suited: 1.0,
            thrust: if p < 0.5 { 0.6 } else { 1.0 },
            scale: 1.0,
        });
    }
    let t = t - BOOST;
    // Straight up first, then curving over to the notch, slowing as it arrives.
    let p = ease(t / FLY_UP);
    let via = (launch_x, top_y + 10.0);
    let (x, y) = curve((launch_x, 10.0), via, (notch_x, top_y + 4.0), p);
    let heading = (notch_x - launch_x).signum();
    Some(Flight {
        x,
        y,
        tilt: heading * 25.0 * (p * 4.0).min(1.0) * (1.0 - p),
        roll: 0.0,
        suited: 1.0,
        thrust: 1.0 - p * 0.5,
        // Shrinks into the notch over the last moments.
        scale: 1.0 - ease((p - 0.85) / 0.15) * 0.8,
    })
}

impl Stage<'_> {
    /// The robot in flight: the suit as far as it is on, thrusters below the boots.
    pub(super) fn fly(&self, flight: Flight, time: f32) {
        let painter = self.painter;
        let save = painter.save();
        let pivot = self.at(0.0, -10.0);
        painter.translate(Vec2::new(pivot.x, pivot.y));
        painter.rotate_degrees(flight.tilt);
        // The barrel roll squashes the robot sideways and back.
        painter.scale(Vec2::new(
            flight.scale * flight.roll.cos().abs().max(0.15),
            flight.scale,
        ));
        painter.translate(Vec2::new(-pivot.x, -pivot.y));
        self.thrusters(flight.thrust, time);
        let mut pose = Pose::new(if flight.thrust > 0.8 {
            Eyes::Happy
        } else {
            Eyes::Open(0.0, 0.4)
        });
        // Arms back like a flyer at full thrust, out to the sides while hovering.
        pose.right_arm = if flight.thrust > 0.8 { 1.9 } else { 0.4 };
        pose.left_arm = pose.right_arm;
        self.robot(pose);
        self.suit(flight.suited, time);
        painter.restore_to(save);
    }

    /// Flames under the boots, flickering, scaled by `thrust`.
    fn thrusters(&self, thrust: f32, time: f32) {
        if thrust <= 0.02 {
            return;
        }
        let u = self.unit;
        for (index, side) in [-3.0f32, 3.0].into_iter().enumerate() {
            let flicker = 1.0 + (time * 37.0 + index as f32 * 2.0).sin() * 0.15;
            let length = (4.0 + thrust * 7.0) * flicker;
            let foot = self.at(side, 0.0);
            self.painter.fill_oval(
                Rect::from_xywh(foot.x - 1.6 * u, foot.y - 0.4 * u, 3.2 * u, length * u),
                Rgba::from_argb((200.0 * thrust) as u8, 255, 150, 60),
            );
            self.painter.fill_oval(
                Rect::from_xywh(foot.x - 0.8 * u, foot.y, 1.6 * u, length * 0.6 * u),
                Rgba::from_argb((240.0 * thrust) as u8, 255, 240, 200),
            );
        }
    }

    /// The red-and-gold suit over the robot: boots, gauntlets, a chest plate with a glowing
    /// light and a helmet with a glowing visor. `suited` 0..1 brings the pieces in one by
    /// one from off to the sides.
    pub(super) fn suit(&self, suited: f32, time: f32) {
        if suited <= 0.0 {
            return;
        }
        let u = self.unit;
        // Each piece flies in from its own direction over its slice of `suited`.
        let piece = |index: f32, from: (f32, f32)| {
            let p = ease(((suited - index * 0.2) / 0.35).clamp(0.0, 1.0));
            ((1.0 - p) * from.0 * u, (1.0 - p) * from.1 * u, p)
        };
        let painter = self.painter;
        // Boots.
        let (dx, dy, p) = piece(0.0, (0.0, 18.0));
        if p > 0.0 {
            for side in [-3.0f32, 3.0] {
                let boot = self.rect(side - 2.6, -3.2, 5.2, 3.4);
                painter.fill_round_rect(
                    Rect::from_xywh(boot.left + dx, boot.top + dy, boot.width(), boot.height()),
                    Radius::uniform(1.2 * u),
                    RED.with_alpha((255.0 * p) as u8),
                );
                painter.fill_rect(
                    Rect::from_xywh(boot.left + dx, boot.top + dy, boot.width(), 0.8 * u),
                    GOLD.with_alpha((255.0 * p) as u8),
                );
            }
        }
        // Gauntlets on the shoulders, along the arms' start.
        let (dx, dy, p) = piece(1.0, (16.0, 0.0));
        if p > 0.0 {
            for side in [-1.0f32, 1.0] {
                painter.fill_round_rect(
                    Rect::from_xywh(
                        self.x + side * 5.6 * u - 1.6 * u + dx * side,
                        self.floor - 8.6 * u + dy,
                        3.2 * u,
                        3.0 * u,
                    ),
                    Radius::uniform(1.2 * u),
                    RED.with_alpha((255.0 * p) as u8),
                );
            }
        }
        // Chest plate with a glowing light.
        let (dx, dy, p) = piece(2.0, (-16.0, 6.0));
        if p > 0.0 {
            let chest = Rect::from_xywh(
                self.x - 4.4 * u + dx,
                self.floor - 8.4 * u + dy,
                8.8 * u,
                5.8 * u,
            );
            painter.fill_round_rect(chest, Radius::uniform(2.2 * u), RED.with_alpha((255.0 * p) as u8));
            painter.fill_round_rect(
                Rect::from_xywh(chest.left + 1.2 * u, chest.bottom - 1.6 * u, chest.width() - 2.4 * u, 1.0 * u),
                Radius::uniform(0.5 * u),
                GOLD.with_alpha((255.0 * p) as u8),
            );
            let pulse = 0.75 + 0.25 * (time * 4.0).sin();
            let core = Point::new(chest.center_x(), chest.top + 2.2 * u);
            painter.fill_circle(core, 1.9 * u, GLOW.with_alpha((70.0 * p * pulse) as u8));
            painter.fill_circle(core, 1.1 * u, GLOW.with_alpha((255.0 * p) as u8));
        }
        // Helmet over the head: red shell, gold face, a glowing visor.
        let (dx, dy, p) = piece(3.0, (0.0, -20.0));
        if p > 0.0 {
            let head = Rect::from_xywh(
                self.x - 8.4 * u + dx,
                self.floor - 22.0 * u + dy,
                16.8 * u,
                14.2 * u,
            );
            painter.fill_round_rect(head, Radius::uniform(4.0 * u), RED.with_alpha((255.0 * p) as u8));
            let face = Rect::from_xywh(
                head.left + 2.6 * u,
                head.top + 3.0 * u,
                head.width() - 5.2 * u,
                head.height() - 5.6 * u,
            );
            painter.fill_round_rect(face, Radius::uniform(2.6 * u), GOLD.with_alpha((255.0 * p) as u8));
            painter.fill_round_rect(
                Rect::from_xywh(face.left, face.bottom - 1.2 * u, face.width(), 1.2 * u),
                Radius::uniform(0.6 * u),
                RED_DARK.with_alpha((255.0 * p) as u8),
            );
            // The visor lights up once the helmet is fully on.
            let lit = ((suited - 0.95) / 0.05).clamp(0.0, 1.0) * p;
            for side in [-1.0f32, 1.0] {
                painter.fill_round_rect(
                    Rect::from_xywh(
                        face.center_x() + side * 2.8 * u - 1.6 * u,
                        face.top + 2.2 * u,
                        3.2 * u,
                        1.1 * u,
                    ),
                    Radius::uniform(0.5 * u),
                    GLOW.with_alpha((255.0 * lit) as u8),
                );
            }
        }
    }

    /// Suit pieces flying off in all directions just after landing, `t` 0..1.
    pub(super) fn unsuit(&self, t: f32) {
        let u = self.unit;
        let painter = self.painter;
        let fade = ((1.0 - t) * 255.0) as u8;
        for (index, (dx, dy)) in [(-1.0, -1.2), (1.0, -1.3), (-1.4, -0.4), (1.4, -0.5), (0.0, -1.6)]
            .into_iter()
            .enumerate()
        {
            let fly = ease(t) * 40.0;
            let spin = t * 360.0 * if index % 2 == 0 { 1.0 } else { -1.0 };
            let at = self.at(dx * fly, -10.0 + dy * fly);
            let save = painter.save();
            painter.translate(Vec2::new(at.x, at.y));
            painter.rotate_degrees(spin);
            painter.fill_round_rect(
                Rect::from_xywh(-2.4 * u, -1.6 * u, 4.8 * u, 3.2 * u),
                Radius::uniform(1.0 * u),
                if index == 4 { GOLD } else { RED }.with_alpha(fade),
            );
            painter.restore_to(save);
        }
    }
}

/// A ring of dust spreading out on the floor at `at`, `t` 0..1 after touching down.
pub(super) fn shockwave(painter: Painter<'_>, at: Point, unit: f32, t: f32) {
    if !(0.0..1.0).contains(&t) {
        return;
    }
    let width = (6.0 + ease(t) * 34.0) * unit;
    painter.stroke_round_rect(
        Rect::from_xywh(at.x - width / 2.0, at.y - width * 0.08, width, width * 0.16),
        Radius::uniform(width * 0.08),
        (1.6 * (1.0 - t)).max(0.2) * unit,
        Rgba::from_argb((200.0 * (1.0 - t)) as u8, 230, 230, 235),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrival_starts_under_the_notch_and_lands_on_the_floor() {
        let start = arrival(0.0, 500.0, 300.0, 700.0).unwrap();
        assert!((start.x - 500.0).abs() < 1.0 && start.y > 290.0);
        assert_eq!(start.suited, 0.0);
        let suited = arrival(SUIT_UP + 0.1, 500.0, 300.0, 700.0).unwrap();
        assert_eq!(suited.suited, 1.0);
        assert!(suited.thrust > 0.4);
        let landing = arrival(ARRIVE_SECONDS - 0.01, 500.0, 300.0, 700.0).unwrap();
        assert!((landing.x - 700.0).abs() < 1.0 && landing.y < 0.5);
        assert!(arrival(ARRIVE_SECONDS, 500.0, 300.0, 700.0).is_none());
    }

    #[test]
    fn departure_leaves_the_floor_and_shrinks_into_the_notch() {
        let start = departure(0.0, 200.0, 500.0, 300.0).unwrap();
        assert_eq!((start.x, start.y), (200.0, 0.0));
        let end = departure(DEPART_SECONDS - 0.01, 200.0, 500.0, 300.0).unwrap();
        assert!((end.x - 500.0).abs() < 2.0 && end.y > 295.0);
        assert!(end.scale < 0.4, "it shrinks into the notch");
        assert!(departure(DEPART_SECONDS, 200.0, 500.0, 300.0).is_none());
    }
}
