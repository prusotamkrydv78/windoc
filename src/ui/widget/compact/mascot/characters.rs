//! Characters the user can pick for their notch friend. Each is a costume over the same
//! robot: its own shell colour and features (ears, whiskers, a beak…) drawn around the head
//! and on the face screen, so every scene, reaction and trip works unchanged.

use std::cell::Cell;

use winisland_render::{Point, Radius, Rect};

use super::Stage;

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

/// The character's own shell colour; `None` for the robot, which uses the colour setting.
pub(super) fn shell() -> Option<(u8, u8, u8)> {
    match current() {
        Character::Robot => None,
        Character::Kitty => Some((255, 214, 170)),
        Character::Bunny => Some((250, 246, 248)),
        Character::Bear => Some((196, 146, 104)),
        Character::Panda => Some((248, 248, 248)),
        Character::Chick => Some((255, 222, 92)),
        Character::Frog => Some((126, 206, 116)),
    }
}

const PINK: (u8, u8, u8) = (255, 160, 180);
const BLACK: (u8, u8, u8) = (34, 34, 38);

impl Stage<'_> {
    /// Features behind the head: ears, an antenna, a tuft, eyes on top for the frog.
    pub(super) fn character_back(&self, head: Rect, shell: (u8, u8, u8)) {
        let u = self.unit;
        let painter = self.painter;
        let fill = self.color(1.0, shell);
        match current() {
            Character::Robot => {
                // A little antenna with a blinking light.
                let base = Point::new(head.center_x(), head.top);
                self.line(base, Point::new(base.x, base.y - 2.6 * u), 0.6, fill);
                let blink = 0.5 + 0.5 * (self.time * 2.0).sin();
                painter.fill_circle(
                    Point::new(base.x, base.y - 3.0 * u),
                    0.9 * u,
                    self.color(0.6 + 0.4 * blink, (255, 120, 110)),
                );
            }
            Character::Kitty => {
                for side in [-1.0f32, 1.0] {
                    let x = head.center_x() + side * 4.8 * u;
                    self.triangle(
                        Point::new(x - side * 2.6 * u, head.top + 1.6 * u),
                        Point::new(x + side * 2.2 * u, head.top + 1.0 * u),
                        Point::new(x + side * 0.6 * u, head.top - 3.6 * u),
                        fill,
                    );
                    self.triangle(
                        Point::new(x - side * 1.2 * u, head.top + 0.8 * u),
                        Point::new(x + side * 1.2 * u, head.top + 0.6 * u),
                        Point::new(x + side * 0.5 * u, head.top - 2.0 * u),
                        self.color(1.0, PINK),
                    );
                }
            }
            Character::Bunny => {
                // Long ears that twitch now and then.
                for side in [-1.0f32, 1.0] {
                    let twitch = if (self.time % 5.0) < 0.3 && side > 0.0 {
                        12.0
                    } else {
                        0.0
                    };
                    let ear = Rect::from_xywh(
                        head.center_x() + side * 3.4 * u - 1.7 * u,
                        head.top - 8.5 * u,
                        3.4 * u,
                        10.0 * u,
                    );
                    let save = painter.save();
                    let pivot = Point::new(ear.center_x(), ear.bottom);
                    painter.translate(winisland_render::Vec2::new(pivot.x, pivot.y));
                    painter.rotate_degrees(side * (8.0 + twitch));
                    painter.translate(winisland_render::Vec2::new(-pivot.x, -pivot.y));
                    painter.fill_round_rect(ear, Radius::uniform(1.7 * u), fill);
                    painter.fill_round_rect(
                        Rect::from_xywh(
                            ear.left + 0.9 * u,
                            ear.top + 1.2 * u,
                            1.6 * u,
                            ear.height() - 3.0 * u,
                        ),
                        Radius::uniform(0.8 * u),
                        self.color(1.0, PINK),
                    );
                    painter.restore_to(save);
                }
            }
            Character::Bear | Character::Panda => {
                let ear_colour = if current() == Character::Panda {
                    self.color(1.0, BLACK)
                } else {
                    fill
                };
                for side in [-1.0f32, 1.0] {
                    let centre = Point::new(head.center_x() + side * 6.0 * u, head.top + 1.0 * u);
                    painter.fill_circle(centre, 2.6 * u, ear_colour);
                    if current() == Character::Bear {
                        painter.fill_circle(centre, 1.3 * u, self.color(1.0, (150, 104, 70)));
                    }
                }
            }
            Character::Chick => {
                // A little tuft of feathers.
                for (dx, height) in [(-1.0, 2.6), (0.0, 3.4), (1.0, 2.6)] {
                    painter.fill_oval(
                        Rect::from_xywh(
                            head.center_x() + dx * 1.2 * u - 0.7 * u,
                            head.top - height * u,
                            1.4 * u,
                            (height + 1.0) * u,
                        ),
                        fill,
                    );
                }
            }
            Character::Frog => {
                // Eyes on bumps on top of the head.
                for side in [-1.0f32, 1.0] {
                    let bump = Point::new(head.center_x() + side * 4.4 * u, head.top + 0.4 * u);
                    painter.fill_circle(bump, 3.0 * u, fill);
                    painter.fill_circle(bump, 1.8 * u, self.color(1.0, (255, 255, 255)));
                    painter.fill_circle(
                        Point::new(bump.x, bump.y + 0.3 * u),
                        0.9 * u,
                        self.color(1.0, BLACK),
                    );
                }
            }
        }
    }

    /// Features on the face screen: whiskers and a nose, a beak, eye patches, cheeks.
    pub(super) fn character_front(&self, head: Rect, screen: Rect, eye_y: f32) {
        let u = self.unit;
        let painter = self.painter;
        let cheeks = |colour: (u8, u8, u8)| {
            for side in [-1.0f32, 1.0] {
                painter.fill_oval(
                    Rect::from_xywh(
                        screen.center_x() + side * 4.6 * u - 1.2 * u,
                        eye_y + 1.6 * u,
                        2.4 * u,
                        1.2 * u,
                    ),
                    self.color(0.75, colour),
                );
            }
        };
        match current() {
            Character::Robot => {}
            Character::Kitty => {
                cheeks(PINK);
                let nose = Point::new(screen.center_x(), eye_y + 1.8 * u);
                painter.fill_circle(nose, 0.55 * u, self.color(1.0, PINK));
                let whisker = self.color(0.85, (240, 240, 245));
                for side in [-1.0f32, 1.0] {
                    for tilt in [-0.4f32, 0.4] {
                        self.line(
                            Point::new(head.center_x() + side * 6.5 * u, eye_y + 2.0 * u),
                            Point::new(
                                head.center_x() + side * 10.0 * u,
                                eye_y + (2.0 + tilt * 2.0) * u,
                            ),
                            0.3,
                            whisker,
                        );
                    }
                }
            }
            Character::Bunny => {
                cheeks(PINK);
                painter.fill_circle(
                    Point::new(screen.center_x(), eye_y + 1.8 * u),
                    0.5 * u,
                    self.color(1.0, PINK),
                );
            }
            Character::Bear => {
                painter.fill_oval(
                    Rect::from_xywh(screen.center_x() - 1.4 * u, eye_y + 1.2 * u, 2.8 * u, 1.8 * u),
                    self.color(1.0, (150, 104, 70)),
                );
            }
            Character::Panda => {
                // Soft patches round the eyes, under the glowing eyes on the screen.
                for side in [-1.0f32, 1.0] {
                    let save = painter.save();
                    let centre = Point::new(screen.center_x() + side * 2.8 * u, eye_y);
                    painter.translate(winisland_render::Vec2::new(centre.x, centre.y));
                    painter.rotate_degrees(side * 20.0);
                    painter.stroke_round_rect(
                        Rect::from_xywh(-2.0 * u, -2.6 * u, 4.0 * u, 5.2 * u),
                        Radius::uniform(2.0 * u),
                        0.6 * u,
                        self.color(0.5, (90, 90, 100)),
                    );
                    painter.restore_to(save);
                }
                cheeks(PINK);
            }
            Character::Chick => {
                self.triangle(
                    Point::new(screen.center_x() - 1.2 * u, eye_y + 1.4 * u),
                    Point::new(screen.center_x() + 1.2 * u, eye_y + 1.4 * u),
                    Point::new(screen.center_x(), eye_y + 3.0 * u),
                    self.color(1.0, (255, 150, 60)),
                );
                cheeks((255, 160, 120));
            }
            Character::Frog => {
                // A wide smile.
                let save = painter.save();
                painter.clip_rect(Rect::from_ltrb(
                    screen.left,
                    eye_y + 1.6 * u,
                    screen.right,
                    screen.bottom,
                ));
                painter.stroke_circle(
                    Point::new(screen.center_x(), eye_y + 0.6 * u),
                    2.6 * u,
                    0.5 * u,
                    self.color(1.0, (126, 206, 116)),
                );
                painter.restore_to(save);
                cheeks(PINK);
            }
        }
    }
}
