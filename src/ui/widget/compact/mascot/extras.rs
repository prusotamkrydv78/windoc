//! Drawing for the robot's newer moments: the meeting and popcorn scenes, the reactions
//! for streaks, milestones and play, and what it wears (accessories, weather gear).

use std::f32::consts::{PI, TAU};

use winisland_render::{Point, Radius, Rect};

use super::life::{self, Accessory, Weather};
use super::{Eyes, LAPTOP, Pose, Reaction, Stage, WHITE, ease};

const FLAME: (u8, u8, u8) = (255, 140, 50);
const FLAME_CORE: (u8, u8, u8) = (255, 214, 90);
const GOLD: (u8, u8, u8) = (255, 200, 60);
const SCARF: (u8, u8, u8) = (226, 86, 80);
const CAP: (u8, u8, u8) = (70, 130, 230);
const UMBRELLA: (u8, u8, u8) = (110, 160, 255);
const LENS: (u8, u8, u8) = (30, 30, 36);

impl Stage<'_> {
    /// A meeting: at the laptop with a headset on, listening, nodding now and then.
    pub(super) fn meeting(&self) {
        let mut pose = Pose::new(Eyes::Open(0.8, 0.2));
        pose.headset = true;
        pose.right_arm = 0.9;
        pose.left_arm = 1.1;
        pose.lean = (self.time * 0.9).sin().max(0.0) * 2.0;
        self.robot(pose);
        // The laptop lid open towards it, its screen a quiet call grid.
        let screen = self.rect(9.0, -14.0, 16.0, 11.0);
        self.painter
            .fill_round_rect(screen, Radius::uniform(1.4 * self.unit), self.prop(1.0, LAPTOP));
        let u = self.unit;
        for (index, (dx, dy)) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)]
            .into_iter()
            .enumerate()
        {
            let tile = Rect::from_xywh(
                screen.left + (1.2 + dx * 6.9) * u,
                screen.top + (1.2 + dy * 4.4) * u,
                6.3 * u,
                3.9 * u,
            );
            // The one speaking glows.
            let speaking = ((self.time / 2.5) as usize) % 4 == index;
            self.painter.fill_round_rect(
                tile,
                Radius::uniform(0.6 * u),
                self.prop(1.0, if speaking { (70, 120, 90) } else { (44, 46, 58) }),
            );
            self.painter.fill_circle(
                Point::new(tile.center_x(), tile.center_y()),
                1.0 * u,
                self.prop(0.8, (200, 200, 210)),
            );
        }
        self.painter.fill_round_rect(
            self.rect(5.0, -3.2, 21.0, 2.2),
            Radius::uniform(1.0 * u),
            self.prop(1.0, (150, 150, 160)),
        );
    }

    /// A fullscreen video: leaning back with a bucket of popcorn, munching.
    pub(super) fn popcorn(&self) {
        let munch = (self.time * 1.6).fract();
        let mut pose = Pose::new(Eyes::Open(0.4, -0.6));
        pose.lean = -5.0;
        pose.left_arm = 1.0;
        // The right hand goes from the bucket to the mouth and back.
        pose.right_arm = if munch < 0.5 {
            0.9 - ease(munch / 0.5) * 1.6
        } else {
            -0.7 + ease((munch - 0.5) / 0.5) * 1.6
        };
        let hands = self.robot(pose);
        let u = self.unit;
        let bucket = Rect::from_xywh(hands.left.x - 3.0 * u, hands.left.y - 2.0 * u, 5.4 * u, 5.0 * u);
        self.painter
            .fill_round_rect(bucket, Radius::uniform(0.6 * u), self.prop(1.0, (232, 90, 84)));
        for stripe in 0..2 {
            self.painter.fill_rect(
                Rect::from_xywh(
                    bucket.left + (1.0 + stripe as f32 * 2.2) * u,
                    bucket.top,
                    1.0 * u,
                    bucket.height(),
                ),
                self.prop(1.0, WHITE),
            );
        }
        for kernel in 0..4 {
            self.painter.fill_circle(
                Point::new(bucket.left + (0.9 + kernel as f32 * 1.2) * u, bucket.top - 0.4 * u),
                0.8 * u,
                self.prop(1.0, (255, 240, 200)),
            );
        }
        if munch < 0.5 {
            self.painter
                .fill_circle(hands.right, 0.7 * u, self.prop(1.0, (255, 240, 200)));
        }
    }

    /// Draws the reactions added for streaks, milestones and play. `false` for others.
    pub(super) fn react_more(&self, reaction: Reaction, rect: Rect) -> bool {
        let t = self.elapsed;
        let u = self.unit;
        match reaction {
            Reaction::Cheer => {
                let mut pose = Pose::new(Eyes::Happy);
                let pump = (t * 6.0).sin();
                pose.right_arm = -1.3 + pump * 0.25;
                pose.left_arm = -1.3 - pump * 0.25;
                pose.rise = pump.abs() * 1.2;
                self.robot(pose);
                self.sparkle(self.at(-8.0, -20.0), 0.6 + 0.4 * (t * 4.0).sin());
                self.sparkle(self.at(8.0, -21.0), 0.6 + 0.4 * (t * 4.0 + 1.5).sin());
                self.caption("reaction_cheer", t);
            }
            Reaction::Bedtime => {
                // A big yawn (eyes shut, leaning back), then a point up at the moon.
                let yawning = t < 1.6;
                let mut pose = Pose::new(Eyes::Closed);
                pose.lean = if yawning { -4.0 * (t / 1.6 * PI).sin() } else { 0.0 };
                pose.right_arm = if yawning { -0.3 } else { -1.1 };
                pose.left_arm = if yawning { -0.3 } else { 1.25 };
                self.robot(pose);
                let moon = self.at(10.0, -21.0);
                self.painter
                    .fill_circle(moon, 2.4 * u, self.prop(1.0, (255, 226, 140)));
                self.painter.fill_circle(
                    Point::new(moon.x + 1.1 * u, moon.y - 0.6 * u),
                    2.0 * u,
                    self.prop(1.0, (14, 14, 18)),
                );
                self.caption("nudge_bedtime", t);
            }
            Reaction::Trick => {
                // A full spin on the spot, ending with a little bow.
                let mut pose = Pose::new(Eyes::Happy);
                pose.lean = 360.0 * ease(t / 1.2);
                pose.rise = (t / 1.2 * PI).sin().max(0.0) * 3.0;
                pose.right_arm = -1.0;
                pose.left_arm = -1.0;
                self.robot(pose);
            }
            Reaction::Notice => {
                // Looks up at the cursor, then waves.
                let mut pose = Pose::new(if t < 0.8 {
                    Eyes::Open(0.0, -1.4)
                } else {
                    Eyes::Happy
                });
                if t >= 0.8 {
                    pose.right_arm = -0.45 + ((t - 0.8) / 0.9 * TAU).sin() * 0.45;
                }
                self.robot(pose);
                if t < 0.8 {
                    self.exclaim(self.at(6.0, -21.0), ease(t / 0.3));
                }
            }
            Reaction::Catch => {
                // A page floats down into its hands.
                let caught = t > 1.0;
                let mut pose = Pose::new(if caught {
                    Eyes::Happy
                } else {
                    Eyes::Open(0.0, -1.2)
                });
                pose.right_arm = 0.1;
                pose.left_arm = 0.1;
                let hands = self.robot(pose);
                let start = rect.top - 8.0 * u;
                let y = start + (hands.right.y - 6.0 * u - start) * ease(t / 1.0);
                let sway = if caught { 0.0 } else { (t * 6.0).sin() * 1.5 };
                let page = Rect::from_xywh(self.x - 2.6 * u + sway * u, y, 5.2 * u, 6.4 * u);
                self.painter
                    .fill_round_rect(page, Radius::uniform(0.5 * u), self.prop(1.0, WHITE));
                for line in 0..3 {
                    self.line(
                        Point::new(page.left + 1.0 * u, page.top + (1.6 + line as f32 * 1.4) * u),
                        Point::new(page.right - 1.0 * u, page.top + (1.6 + line as f32 * 1.4) * u),
                        0.4,
                        self.prop(1.0, (170, 175, 190)),
                    );
                }
                self.caption("reaction_caught", t);
            }
            Reaction::Streak => {
                let mut pose = Pose::new(Eyes::Happy);
                pose.right_arm = -1.2;
                pose.rise = (t * 3.0).sin().abs() * 0.8;
                let hands = self.robot(pose);
                self.flame(Point::new(hands.right.x, hands.right.y - 1.0 * u), t);
                self.caption("reaction_streak", t);
            }
            Reaction::Recap => {
                // Holds up a little card with a bar chart of the week.
                let mut pose = Pose::new(Eyes::Happy);
                pose.right_arm = -0.7;
                let hands = self.robot(pose);
                let card = Rect::from_xywh(
                    hands.right.x - 1.0 * u,
                    hands.right.y - 7.0 * u,
                    9.0 * u,
                    7.0 * u,
                );
                self.painter
                    .fill_round_rect(card, Radius::uniform(0.8 * u), self.prop(1.0, WHITE));
                let heights = [0.4, 0.7, 0.5, 0.9, 0.6, 0.3, 0.8];
                for (index, height) in heights.iter().enumerate() {
                    let grown = height * ease((t - index as f32 * 0.08) / 0.5);
                    let bar = 4.8 * u * grown;
                    self.painter.fill_rect(
                        Rect::from_xywh(
                            card.left + (0.9 + index as f32 * 1.1) * u,
                            card.bottom - 1.0 * u - bar,
                            0.7 * u,
                            bar,
                        ),
                        self.prop(1.0, (90, 150, 240)),
                    );
                }
                self.caption("reaction_recap", t);
            }
            Reaction::Milestone => {
                let mut pose = Pose::new(Eyes::Happy);
                pose.right_arm = -1.0;
                pose.rise = (t * 3.0).sin().abs() * 0.8;
                let hands = self.robot(pose);
                let note = Rect::from_xywh(
                    hands.right.x - 2.6 * u,
                    hands.right.y - 6.4 * u,
                    5.6 * u,
                    5.6 * u,
                );
                self.painter
                    .fill_round_rect(note, Radius::uniform(0.6 * u), self.prop(1.0, (255, 236, 150)));
                self.star(Point::new(note.center_x(), note.center_y()), 1.8 * u, GOLD);
                self.confetti(rect, t);
                self.caption("reaction_milestone", t);
            }
            Reaction::FirstShot => {
                // A white flash, then it holds up the picture.
                self.present();
                let flash = 1.0 - ease(t / 0.35);
                if flash > 0.0 {
                    self.painter.fill_circle(
                        self.at(0.0, -12.0),
                        14.0 * u,
                        self.prop(flash * 0.6, WHITE),
                    );
                }
                self.caption("reaction_first_shot", t);
            }
            _ => return false,
        }
        true
    }

    /// A flickering flame resting on `base`.
    fn flame(&self, base: Point, t: f32) {
        let u = self.unit;
        let flicker = 1.0 + (t * 11.0).sin() * 0.08 + (t * 17.0).sin() * 0.05;
        self.painter.fill_oval(
            Rect::from_xywh(
                base.x - 1.8 * u,
                base.y - 5.0 * u * flicker,
                3.6 * u,
                5.0 * u * flicker,
            ),
            self.prop(1.0, FLAME),
        );
        self.painter.fill_oval(
            Rect::from_xywh(
                base.x - 0.9 * u,
                base.y - 2.8 * u * flicker,
                1.8 * u,
                2.8 * u * flicker,
            ),
            self.prop(1.0, FLAME_CORE),
        );
    }

    /// A five-pointed star.
    fn star(&self, center: Point, radius: f32, rgb: (u8, u8, u8)) {
        for point in 0..5 {
            let angle = -PI / 2.0 + point as f32 * TAU / 5.0;
            self.line(
                center,
                Point::new(
                    center.x + angle.cos() * radius,
                    center.y + angle.sin() * radius,
                ),
                0.9,
                self.prop(1.0, rgb),
            );
        }
    }

    /// What the robot wears over its head and body: the chosen accessory and, when the
    /// weather calls for it, an umbrella or sunglasses. Called from `robot`, inside its lean.
    pub(super) fn wear(&self, head: Rect, screen: Rect, body: Rect, eye_y: f32) {
        let u = self.unit;
        let weather = life::weather();
        match life::accessory() {
            Some(Accessory::Scarf) => {
                let band = Rect::from_xywh(
                    body.left - 0.4 * u,
                    head.bottom - 0.8 * u,
                    body.width() + 0.8 * u,
                    2.0 * u,
                );
                self.painter
                    .fill_round_rect(band, Radius::uniform(1.0 * u), self.color(1.0, SCARF));
                self.painter.fill_round_rect(
                    Rect::from_xywh(band.right - 2.6 * u, band.bottom - 0.4 * u, 1.8 * u, 3.4 * u),
                    Radius::uniform(0.6 * u),
                    self.color(1.0, SCARF),
                );
            }
            Some(Accessory::Glasses) if weather != Weather::Sunny => {
                for side in [-1.0, 1.0] {
                    self.painter.stroke_circle(
                        Point::new(screen.center_x() + side * 2.8 * u, eye_y),
                        2.2 * u,
                        0.45 * u,
                        self.color(1.0, (230, 230, 236)),
                    );
                }
                self.line(
                    Point::new(screen.center_x() - 0.6 * u, eye_y),
                    Point::new(screen.center_x() + 0.6 * u, eye_y),
                    0.45,
                    self.color(1.0, (230, 230, 236)),
                );
            }
            Some(Accessory::Cap) if weather != Weather::Rain => {
                let crown = Rect::from_xywh(head.left + 2.0 * u, head.top - 2.6 * u, 12.0 * u, 4.4 * u);
                self.painter.fill_round_rect(
                    crown,
                    Radius::uniform(2.2 * u),
                    self.color(1.0, CAP),
                );
                self.painter.fill_round_rect(
                    Rect::from_xywh(crown.right - 2.0 * u, crown.bottom - 1.4 * u, 6.0 * u, 1.4 * u),
                    Radius::uniform(0.7 * u),
                    self.color(1.0, CAP),
                );
            }
            _ => {}
        }
        match weather {
            Weather::Sunny => {
                for side in [-1.0, 1.0] {
                    self.painter.fill_round_rect(
                        Rect::from_xywh(
                            screen.center_x() + side * 2.8 * u - 2.0 * u,
                            eye_y - 1.4 * u,
                            4.0 * u,
                            2.8 * u,
                        ),
                        Radius::uniform(1.2 * u),
                        self.color(1.0, LENS),
                    );
                }
                self.line(
                    Point::new(screen.left + 0.6 * u, eye_y - 0.6 * u),
                    Point::new(screen.right - 0.6 * u, eye_y - 0.6 * u),
                    0.4,
                    self.color(1.0, LENS),
                );
            }
            Weather::Rain => {
                // An umbrella hat: a canopy over the head on a short stem.
                let top = head.top - 6.0 * u;
                let canopy = Rect::from_xywh(head.center_x() - 11.0 * u, top, 22.0 * u, 8.0 * u);
                let save = self.painter.save();
                self.painter.clip_rect(Rect::from_ltrb(
                    canopy.left,
                    canopy.top,
                    canopy.right,
                    canopy.center_y(),
                ));
                self.painter.fill_oval(canopy, self.color(1.0, UMBRELLA));
                self.painter.restore_to(save);
                self.line(
                    Point::new(head.center_x(), canopy.center_y()),
                    Point::new(head.center_x(), head.top),
                    0.6,
                    self.color(1.0, (90, 90, 100)),
                );
                // A few drops sliding off the edges.
                for (index, side) in [-1.0, 1.0].into_iter().enumerate() {
                    let fall = (self.time * 1.3 + index as f32 * 0.5).fract();
                    self.painter.fill_circle(
                        Point::new(
                            head.center_x() + side * 10.5 * u,
                            canopy.center_y() + fall * 9.0 * u,
                        ),
                        0.45 * u,
                        self.color(1.0 - fall, (130, 190, 255)),
                    );
                }
            }
            Weather::None => {}
        }
    }
}
