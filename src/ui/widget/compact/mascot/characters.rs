//! Characters the user can pick for their notch friend. The robot is the original; the
//! others are creatures with bodies of their own: a round kitty with a curling tail, a bunny
//! with long ears and a puff tail, a bear, a panda, a chick that is one fluffy ball with
//! wings, and a squat frog with eyes on top.
//!
//! Every scene and reaction describes a [`Pose`] (where the arms point, how the eyes look,
//! lean and rise) and holds props at the hands; a creature draws that pose in its own body
//! and reports its hands in the same places as the robot's, so everything works unchanged.

use std::cell::Cell;
use std::f32::consts::TAU;

use winisland_render::{Point, Radius, Rect, Rgba, Vec2};

use super::{ARM_LENGTH, BREATH_SECONDS, Eyes, GAZE, Hands, Pose, Stage, blink};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) enum Character {
    #[default]
    Robot,
    Kitty,
    Bunny,
    Bear,
    Panda,
    Chick,
    Frog,
}

impl Character {
    fn from_index(index: u8) -> Self {
        match index {
            1 => Self::Kitty,
            2 => Self::Bunny,
            3 => Self::Bear,
            4 => Self::Panda,
            5 => Self::Chick,
            6 => Self::Frog,
            _ => Self::Robot,
        }
    }
}

thread_local! {
    static CHARACTER: Cell<Character> = const { Cell::new(Character::Robot) };
}

/// Applies the character chosen in Settings. Call each tick.
pub(crate) fn set_character(index: u8) {
    CHARACTER.with(|cell| cell.set(Character::from_index(index)));
}

fn current() -> Character {
    // Test hook: WINISLAND_MASCOT_DEMO_CHARACTERS=1 (with WINISLAND_MASCOT_DEMO=1) shows each
    // character for three seconds in turn.
    if super::demo() && std::env::var_os("WINISLAND_MASCOT_DEMO_CHARACTERS").is_some() {
        return Character::from_index(((super::seconds() / 3.0) as u8) % 7);
    }
    CHARACTER.with(Cell::get)
}

type Rgb = (u8, u8, u8);

#[derive(Clone, Copy, PartialEq)]
enum Ears {
    Pointy,
    Long,
    Round,
    Tuft,
    None,
}

#[derive(Clone, Copy, PartialEq)]
enum Tail {
    Curl,
    Puff,
    Stub,
    None,
}

/// How a creature looks.
struct Species {
    fur: Rgb,
    /// Belly and muzzle.
    light: Rgb,
    /// Inner ears, nose, cheeks.
    accent: Rgb,
    /// Ears and patches for the panda; otherwise the fur.
    marks: Rgb,
    ears: Ears,
    tail: Tail,
    /// Arms are wings (the chick).
    wings: bool,
    /// Head width and height in robot units.
    head: (f32, f32),
}

const INK: Rgb = (40, 32, 36);

fn species(character: Character) -> Option<Species> {
    Some(match character {
        Character::Robot => return None,
        Character::Kitty => Species {
            fur: (255, 196, 140),
            light: (255, 238, 214),
            accent: (255, 150, 170),
            marks: (240, 160, 100),
            ears: Ears::Pointy,
            tail: Tail::Curl,
            wings: false,
            head: (16.0, 13.6),
        },
        Character::Bunny => Species {
            fur: (250, 246, 250),
            light: (255, 255, 255),
            accent: (255, 160, 186),
            marks: (250, 246, 250),
            ears: Ears::Long,
            tail: Tail::Puff,
            wings: false,
            head: (15.0, 13.0),
        },
        Character::Bear => Species {
            fur: (176, 122, 82),
            light: (232, 196, 156),
            accent: (255, 150, 150),
            marks: (176, 122, 82),
            ears: Ears::Round,
            tail: Tail::Stub,
            wings: false,
            head: (16.4, 13.8),
        },
        Character::Panda => Species {
            fur: (250, 250, 250),
            light: (255, 255, 255),
            accent: (255, 160, 180),
            marks: (40, 40, 46),
            ears: Ears::Round,
            tail: Tail::Stub,
            wings: false,
            head: (16.4, 13.8),
        },
        Character::Chick => Species {
            fur: (255, 220, 80),
            light: (255, 238, 150),
            accent: (255, 150, 70),
            marks: (255, 200, 60),
            ears: Ears::Tuft,
            tail: Tail::None,
            wings: true,
            head: (17.0, 15.0),
        },
        Character::Frog => Species {
            fur: (120, 204, 110),
            light: (210, 240, 170),
            accent: (255, 150, 160),
            marks: (90, 170, 80),
            ears: Ears::None,
            tail: Tail::None,
            wings: false,
            head: (19.0, 11.0),
        },
    })
}

fn rgba(stage: &Stage<'_>, rgb: Rgb) -> Rgba {
    stage.color(1.0, rgb)
}

impl Stage<'_> {
    /// Draws the chosen creature in `pose`, or `None` for the robot, which draws itself.
    pub(super) fn creature(&self, pose: Pose) -> Option<Hands> {
        let character = current();
        let look = species(character)?;
        let u = self.unit;
        let painter = self.painter;
        let breath = (self.time / BREATH_SECONDS * TAU).sin();
        let lift = ((breath * 0.5 + 0.5) * 0.8 + pose.rise) * u;
        let save = painter.save();
        if pose.lean != 0.0 {
            let pivot = self.at(0.0, 0.0);
            painter.translate(Vec2::new(pivot.x, pivot.y));
            painter.rotate_degrees(pose.lean);
            painter.translate(Vec2::new(-pivot.x, -pivot.y));
        }
        let fur = rgba(self, look.fur);

        // The body: a soft round shape, its top where the robot's body is so the hands land
        // in the same places.
        let body = Rect::from_xywh(
            self.x - 5.6 * u,
            self.floor - 8.8 * u - lift * 0.5,
            11.2 * u,
            8.0 * u,
        );
        let shoulder_y = body.top + 1.6 * u;
        let right_shoulder = Point::new(self.x + 4.0 * u, shoulder_y);
        let left_shoulder = Point::new(self.x - 4.0 * u, shoulder_y);
        let right = Point::new(
            right_shoulder.x + pose.right_arm.cos() * ARM_LENGTH * u,
            right_shoulder.y + pose.right_arm.sin() * ARM_LENGTH * u,
        );
        let left = Point::new(
            left_shoulder.x - pose.left_arm.cos() * ARM_LENGTH * u,
            left_shoulder.y + pose.left_arm.sin() * ARM_LENGTH * u,
        );

        self.tail(look.tail, body, fur);
        // Feet: little paws (webbed and wide for the frog, orange for the chick).
        let foot = match character {
            Character::Chick => rgba(self, (255, 150, 60)),
            _ => rgba(self, look.marks),
        };
        for side in [-1.0f32, 1.0] {
            let width = if character == Character::Frog { 5.4 } else { 4.2 };
            painter.fill_oval(
                Rect::from_xywh(
                    self.x + side * 2.9 * u - width / 2.0 * u,
                    self.floor - 2.2 * u,
                    width * u,
                    2.2 * u,
                ),
                foot,
            );
        }
        painter.fill_oval(body, fur);
        painter.fill_oval(
            Rect::from_xywh(
                body.left + 2.6 * u,
                body.top + 2.0 * u,
                body.width() - 5.2 * u,
                body.height() - 2.6 * u,
            ),
            rgba(self, look.light),
        );
        // Arms: short round paws, or wings for the chick.
        for (shoulder, hand) in [(right_shoulder, right), (left_shoulder, left)] {
            if look.wings {
                let angle = (hand.y - shoulder.y).atan2(hand.x - shoulder.x);
                let save = painter.save();
                painter.translate(Vec2::new(shoulder.x, shoulder.y));
                painter.rotate_degrees(angle.to_degrees());
                painter.fill_oval(
                    Rect::from_xywh(-0.6 * u, -1.6 * u, ARM_LENGTH * u + 1.4 * u, 3.2 * u),
                    rgba(self, look.marks),
                );
                painter.restore_to(save);
            } else {
                self.line(shoulder, hand, 2.6, rgba(self, look.marks));
                painter.fill_circle(hand, 1.5 * u, rgba(self, look.marks));
            }
        }

        // The head, big and round, in the robot's head box so hats and props still fit.
        let (head_w, head_h) = look.head;
        let head = Rect::from_xywh(
            self.x - head_w / 2.0 * u,
            body.top - (head_h - 1.0) * u - lift * 0.5,
            head_w * u,
            head_h * u,
        );
        self.ears(look.ears, head, &look);
        let corner = if character == Character::Frog {
            5.0
        } else {
            head_h.min(head_w) * 0.47
        };
        painter.fill_round_rect(head, Radius::uniform(corner * u), fur);
        let eye_y = if character == Character::Frog {
            head.top + 0.6 * u
        } else {
            head.center_y() + 0.4 * u
        };
        if character == Character::Frog {
            // Eyes on bumps on top of the head.
            for side in [-1.0f32, 1.0] {
                painter.fill_circle(Point::new(self.x + side * 4.6 * u, eye_y), 3.2 * u, fur);
            }
        }
        if character == Character::Panda {
            for side in [-1.0f32, 1.0] {
                let save = painter.save();
                let centre = Point::new(self.x + side * 3.2 * u, eye_y + 0.2 * u);
                painter.translate(Vec2::new(centre.x, centre.y));
                painter.rotate_degrees(side * -24.0);
                painter.fill_oval(
                    Rect::from_xywh(-2.3 * u, -2.8 * u, 4.6 * u, 5.6 * u),
                    rgba(self, look.marks),
                );
                painter.restore_to(save);
            }
        }
        self.eyes(pose.eyes, eye_y, character, &look);
        self.muzzle(character, head, eye_y, &look);
        if pose.headset {
            let band = rgba(self, (60, 60, 70));
            for side in [-1.0f32, 1.0] {
                let ear = Point::new(self.x + side * (head_w / 2.0 + 0.4) * u, head.center_y());
                self.line(
                    Point::new(self.x, head.top - 0.8 * u),
                    Point::new(ear.x, ear.y - 2.0 * u),
                    0.8,
                    band,
                );
                painter.fill_round_rect(
                    Rect::from_xywh(ear.x - 1.4 * u, ear.y - 2.2 * u, 2.8 * u, 4.4 * u),
                    Radius::uniform(1.2 * u),
                    band,
                );
            }
        }
        // Accessories and weather gear sit on the head like the robot's.
        let face = head.inset(2.0 * u);
        self.wear(head, face, body, eye_y);
        painter.restore_to(save);
        Some(Hands { right, left })
    }

    fn tail(&self, tail: Tail, body: Rect, fur: Rgba) {
        let u = self.unit;
        let painter = self.painter;
        let wag = (self.time * 2.4).sin();
        match tail {
            Tail::Curl => {
                // A long tail curling up behind, swaying.
                let base = Point::new(body.left + 0.6 * u, body.bottom - 2.0 * u);
                let mid = Point::new(base.x - 4.0 * u, base.y - 2.0 * u + wag * 0.6 * u);
                let tip = Point::new(mid.x - 0.6 * u + wag * 1.2 * u, mid.y - 4.0 * u);
                self.line(base, mid, 1.8, fur);
                self.line(mid, tip, 1.6, fur);
                painter.fill_circle(tip, 0.9 * u, fur);
            }
            Tail::Puff => painter.fill_circle(
                Point::new(body.left + 0.4 * u, body.bottom - 2.6 * u + wag * 0.3 * u),
                2.0 * u,
                rgba(self, (255, 255, 255)),
            ),
            Tail::Stub => painter.fill_circle(
                Point::new(body.left + 0.6 * u, body.bottom - 2.4 * u),
                1.4 * u,
                fur,
            ),
            Tail::None => {}
        }
    }

    fn ears(&self, ears: Ears, head: Rect, look: &Species) {
        let u = self.unit;
        let painter = self.painter;
        let marks = rgba(self, look.marks);
        let inner = rgba(self, look.accent);
        match ears {
            Ears::Pointy => {
                for side in [-1.0f32, 1.0] {
                    let x = head.center_x() + side * 4.6 * u;
                    // A flick of one ear now and then.
                    let flick = if side > 0.0 && (self.time % 6.0) < 0.25 { 1.2 } else { 0.0 };
                    self.triangle(
                        Point::new(x - side * 2.8 * u, head.top + 2.2 * u),
                        Point::new(x + side * 2.6 * u, head.top + 1.4 * u),
                        Point::new(x + side * (0.8 + flick) * u, head.top - 3.8 * u),
                        rgba(self, look.fur),
                    );
                    self.triangle(
                        Point::new(x - side * 1.3 * u, head.top + 1.2 * u),
                        Point::new(x + side * 1.4 * u, head.top + 0.9 * u),
                        Point::new(x + side * (0.7 + flick) * u, head.top - 2.0 * u),
                        inner,
                    );
                }
            }
            Ears::Long => {
                for side in [-1.0f32, 1.0] {
                    let twitch = if side > 0.0 && (self.time % 5.0) < 0.3 { 14.0 } else { 0.0 };
                    let ear = Rect::from_xywh(
                        head.center_x() + side * 3.2 * u - 1.8 * u,
                        head.top - 9.0 * u,
                        3.6 * u,
                        11.0 * u,
                    );
                    let save = painter.save();
                    let pivot = Point::new(ear.center_x(), ear.bottom);
                    painter.translate(Vec2::new(pivot.x, pivot.y));
                    painter.rotate_degrees(side * (10.0 + twitch));
                    painter.translate(Vec2::new(-pivot.x, -pivot.y));
                    painter.fill_round_rect(ear, Radius::uniform(1.8 * u), rgba(self, look.fur));
                    painter.fill_round_rect(
                        Rect::from_xywh(ear.left + 0.9 * u, ear.top + 1.4 * u, 1.8 * u, 7.0 * u),
                        Radius::uniform(0.9 * u),
                        inner,
                    );
                    painter.restore_to(save);
                }
            }
            Ears::Round => {
                for side in [-1.0f32, 1.0] {
                    let centre = Point::new(head.center_x() + side * 5.8 * u, head.top + 1.4 * u);
                    painter.fill_circle(centre, 2.7 * u, marks);
                    if look.marks == look.fur {
                        painter.fill_circle(centre, 1.3 * u, rgba(self, look.light));
                    }
                }
            }
            Ears::Tuft => {
                for (dx, height) in [(-1.0, 2.8), (0.0, 3.8), (1.0, 2.8)] {
                    painter.fill_oval(
                        Rect::from_xywh(
                            head.center_x() + dx * 1.3 * u - 0.8 * u,
                            head.top - height * u,
                            1.6 * u,
                            (height + 1.4) * u,
                        ),
                        marks,
                    );
                }
            }
            Ears::None => {}
        }
    }

    /// Cartoon eyes: glossy dark ovals with a sparkle that follow the gaze, happy arcs or
    /// sleepy lines.
    fn eyes(&self, eyes: Eyes, eye_y: f32, character: Character, look: &Species) {
        let u = self.unit;
        let painter = self.painter;
        let spread = if character == Character::Frog { 4.6 } else { 3.2 };
        // Panda eyes sit in dark patches, so they are drawn light.
        let (ink, shine) = if character == Character::Panda {
            (rgba(self, (250, 250, 250)), rgba(self, look.marks))
        } else {
            (rgba(self, INK), rgba(self, (255, 255, 255)))
        };
        for side in [-1.0f32, 1.0] {
            let centre = Point::new(self.x + side * spread * u, eye_y);
            match blink(eyes, self.time) {
                Eyes::Closed => {
                    self.line(
                        Point::new(centre.x - 1.2 * u, centre.y + 0.2 * u),
                        Point::new(centre.x + 1.2 * u, centre.y + 0.2 * u),
                        0.7,
                        ink,
                    );
                }
                Eyes::Happy => {
                    let peak = Point::new(centre.x, centre.y - 1.0 * u);
                    for end in [-1.0f32, 1.0] {
                        self.line(
                            Point::new(centre.x + end * 1.3 * u, centre.y + 0.4 * u),
                            peak,
                            0.7,
                            ink,
                        );
                    }
                }
                Eyes::Open(dx, dy) => {
                    let (dx, dy) = GAZE.with(std::cell::Cell::get).unwrap_or((dx, dy));
                    let at = Point::new(centre.x + dx * 0.35 * u, centre.y + dy * 0.35 * u);
                    painter.fill_oval(
                        Rect::from_xywh(at.x - 1.1 * u, at.y - 1.4 * u, 2.2 * u, 2.8 * u),
                        ink,
                    );
                    painter.fill_circle(Point::new(at.x + 0.4 * u, at.y - 0.6 * u), 0.45 * u, shine);
                }
            }
        }
    }

    /// Nose, mouth and cheeks.
    fn muzzle(&self, character: Character, head: Rect, eye_y: f32, look: &Species) {
        let u = self.unit;
        let painter = self.painter;
        let ink = rgba(self, INK);
        let accent = rgba(self, look.accent);
        let cheeks = |dy: f32| {
            for side in [-1.0f32, 1.0] {
                painter.fill_oval(
                    Rect::from_xywh(
                        self.x + side * 5.4 * u - 1.3 * u,
                        eye_y + dy * u,
                        2.6 * u,
                        1.4 * u,
                    ),
                    self.color(0.7, look.accent),
                );
            }
        };
        let nose_y = eye_y + 2.2 * u;
        match character {
            Character::Kitty => {
                self.triangle(
                    Point::new(self.x - 0.8 * u, nose_y - 0.4 * u),
                    Point::new(self.x + 0.8 * u, nose_y - 0.4 * u),
                    Point::new(self.x, nose_y + 0.5 * u),
                    accent,
                );
                // A little "w" mouth.
                for side in [-1.0f32, 1.0] {
                    self.line(
                        Point::new(self.x, nose_y + 0.5 * u),
                        Point::new(self.x + side * 1.0 * u, nose_y + 1.2 * u),
                        0.45,
                        ink,
                    );
                }
                let whisker = self.color(0.7, (150, 110, 90));
                for side in [-1.0f32, 1.0] {
                    for tilt in [-0.5f32, 0.5] {
                        self.line(
                            Point::new(self.x + side * 4.6 * u, nose_y),
                            Point::new(head.center_x() + side * 9.6 * u, nose_y + tilt * 1.6 * u),
                            0.3,
                            whisker,
                        );
                    }
                }
                cheeks(1.4);
            }
            Character::Bunny => {
                painter.fill_oval(
                    Rect::from_xywh(self.x - 0.8 * u, nose_y - 0.5 * u, 1.6 * u, 1.1 * u),
                    accent,
                );
                // Two front teeth.
                painter.fill_round_rect(
                    Rect::from_xywh(self.x - 0.9 * u, nose_y + 0.9 * u, 1.8 * u, 1.4 * u),
                    Radius::uniform(0.3 * u),
                    rgba(self, (255, 255, 255)),
                );
                self.line(
                    Point::new(self.x, nose_y + 0.9 * u),
                    Point::new(self.x, nose_y + 2.2 * u),
                    0.2,
                    self.color(0.5, INK),
                );
                cheeks(1.2);
            }
            Character::Bear | Character::Panda => {
                if character == Character::Bear {
                    painter.fill_oval(
                        Rect::from_xywh(self.x - 2.6 * u, nose_y - 1.4 * u, 5.2 * u, 3.6 * u),
                        rgba(self, look.light),
                    );
                }
                painter.fill_oval(
                    Rect::from_xywh(self.x - 1.0 * u, nose_y - 0.7 * u, 2.0 * u, 1.3 * u),
                    ink,
                );
                for side in [-1.0f32, 1.0] {
                    self.line(
                        Point::new(self.x, nose_y + 0.6 * u),
                        Point::new(self.x + side * 0.9 * u, nose_y + 1.3 * u),
                        0.45,
                        ink,
                    );
                }
                cheeks(1.6);
            }
            Character::Chick => {
                let beak = rgba(self, look.accent);
                self.triangle(
                    Point::new(self.x - 1.5 * u, nose_y - 0.8 * u),
                    Point::new(self.x + 1.5 * u, nose_y - 0.8 * u),
                    Point::new(self.x, nose_y + 1.2 * u),
                    beak,
                );
                cheeks(1.0);
            }
            Character::Frog => {
                // A wide grin across the face.
                let save = painter.save();
                let mouth_y = head.center_y() + 1.0 * u;
                painter.clip_rect(Rect::from_ltrb(
                    head.left,
                    mouth_y,
                    head.right,
                    head.bottom,
                ));
                painter.stroke_circle(
                    Point::new(self.x, mouth_y - 2.4 * u),
                    5.0 * u,
                    0.6 * u,
                    rgba(self, look.marks),
                );
                painter.restore_to(save);
                for side in [-1.0f32, 1.0] {
                    painter.fill_oval(
                        Rect::from_xywh(
                            self.x + side * 6.6 * u - 1.3 * u,
                            mouth_y - 0.6 * u,
                            2.6 * u,
                            1.4 * u,
                        ),
                        self.color(0.7, look.accent),
                    );
                }
            }
            Character::Robot => {}
        }
    }

    /// The robot's antenna, behind its head, with a blinking light.
    pub(super) fn antenna(&self, head: Rect, shell: (u8, u8, u8)) {
        let u = self.unit;
        let base = Point::new(head.center_x(), head.top);
        self.line(base, Point::new(base.x, base.y - 2.6 * u), 0.6, self.color(1.0, shell));
        let blink = 0.5 + 0.5 * (self.time * 2.0).sin();
        self.painter.fill_circle(
            Point::new(base.x, base.y - 3.0 * u),
            0.9 * u,
            self.color(0.6 + 0.4 * blink, (255, 120, 110)),
        );
    }
}
