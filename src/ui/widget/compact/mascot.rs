//! The island's robot: a cream body with a black screen for a face. In the idle notch it
//! goes about its day, sipping chai, typing, reading, watering a plant, stretching, looking
//! around, waving and napping, a scene at a time. It works on its laptop while a Focus
//! timer runs and sleeps under the moon at night. Greetings use its waving pose.

use std::cell::RefCell;
use std::f32::consts::{PI, TAU};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use winisland_render::text::{DrawTextCachedParams, FontManager};
use winisland_render::{Painter, Point, Radius, Rect, Rgba, StrokeCap, Vec2};

/// Room for the robot and the props of its current scene.
pub(super) const WIDTH: f32 = 132.0;
/// About 20 frames a second keeps the motion smooth without busy redraws.
pub(super) const FRAME: Duration = Duration::from_millis(50);
/// How long each daytime scene plays before the next one.
const SCENE_SECONDS: f32 = 14.0;
/// Props and captions fade in and out over this long at each scene change.
const FADE_SECONDS: f32 = 0.4;
const BREATH_SECONDS: f32 = 3.2;
/// Height of the robot from its feet to the top of its head, in robot units.
const ROBOT_HEIGHT: f32 = 22.0;
const ARM_LENGTH: f32 = 5.6;

const SHELL: (u8, u8, u8) = (242, 238, 228);
const SCREEN: (u8, u8, u8) = (14, 14, 16);
const FEET: (u8, u8, u8) = (178, 178, 182);
const WHITE: (u8, u8, u8) = (255, 255, 255);
const CHAI: (u8, u8, u8) = (196, 140, 84);
const RED: (u8, u8, u8) = (232, 104, 96);
const LEAF: (u8, u8, u8) = (110, 200, 120);
const POT: (u8, u8, u8) = (206, 120, 82);
const WATER: (u8, u8, u8) = (110, 180, 255);
const BOOK: (u8, u8, u8) = (88, 140, 230);
const LAPTOP: (u8, u8, u8) = (64, 64, 72);
const CODE: [(u8, u8, u8); 4] = [
    (130, 220, 140),
    (120, 180, 255),
    (240, 150, 200),
    (250, 210, 120),
];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Scene {
    Wave,
    Chai,
    Laptop,
    Read,
    Plant,
    Stretch,
    LookAround,
    Nap,
    Paint,
    Dance,
    Night,
}

/// The daytime order: a hello, then chai, then the rest of the day.
const DAY: [Scene; 9] = [
    Scene::Wave,
    Scene::Chai,
    Scene::Paint,
    Scene::Laptop,
    Scene::LookAround,
    Scene::Read,
    Scene::Plant,
    Scene::Stretch,
    Scene::Nap,
];

/// Without input for this long the user counts as away and the robot naps.
const AWAY_SECONDS: f32 = 300.0;

/// How long the robot dances when a song starts.
const DANCE_SECONDS: f32 = 8.0;

#[derive(Default)]
struct Context {
    name: String,
    focus_running: bool,
    /// The playing track, to notice when a new one starts.
    track: String,
    /// When the robot started dancing to the current track.
    dance_started: Option<f32>,
    /// No keyboard or mouse input for a while: the robot naps until the user is back.
    away: bool,
    /// When the current stretch of activity began, and when its next nudges are due.
    active_since: Option<f32>,
    next_water: f32,
    next_stretch: f32,
}

thread_local! {
    static CONTEXT: RefCell<Context> = RefCell::new(Context::default());
}

/// Tells the robot who it is greeting, whether a Focus timer is running and which track is
/// playing, if any. A new track gets a short dance. Call each tick.
pub(crate) fn set_context(
    name: &str,
    focus_running: bool,
    playing_track: Option<&str>,
    idle_seconds: f32,
) {
    let away = idle_seconds >= AWAY_SECONDS;
    let returned =
        CONTEXT.with(|context| std::mem::replace(&mut context.borrow_mut().away, away)) && !away;
    if returned {
        react(Reaction::WelcomeBack);
    }
    nudge(away, focus_running);
    CONTEXT.with(|context| {
        let mut context = context.borrow_mut();
        if context.name != name {
            context.name = name.to_string();
        }
        context.focus_running = focus_running;
        match playing_track {
            Some(track) if !track.is_empty() && context.track != track => {
                context.track = track.to_string();
                context.dance_started = Some(seconds());
            }
            Some(_) => {}
            None => context.dance_started = None,
        }
    });
}

/// A short animation for something that just happened, played over the robot's day.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Reaction {
    /// A mouse was plugged in: a real little mouse scares the robot off.
    MouseArrived,
    /// The charger went in: a lightning bolt drops into its hands.
    Charging,
    /// The charger came out: it jumps in surprise.
    Surprised,
    /// Something was copied: it holds up a note.
    Copied,
    /// A drive was plugged in: it catches a falling USB stick.
    Drive,
    /// A Focus session finished: confetti and a happy jump.
    Celebrate,
    /// The user is back after a while away: it wakes, stretches and waves.
    WelcomeBack,
    /// A water break: it sips from a glass.
    Water,
    /// A stretch break: arms up and a big stretch.
    StretchBreak,
    /// Around 1 pm: a steaming bowl of noodles.
    Lunch,
    /// Around 6 pm: it closes its laptop and waves.
    WrapUp,
}

impl Reaction {
    fn seconds(self) -> f32 {
        match self {
            Self::MouseArrived => 6.0,
            Self::Celebrate => 4.5,
            Self::WelcomeBack => 4.5,
            Self::Water | Self::StretchBreak | Self::Lunch | Self::WrapUp => 6.0,
            Self::Charging | Self::Drive => 4.0,
            Self::Copied => 3.0,
            Self::Surprised => 2.6,
        }
    }
}

thread_local! {
    /// The reaction waiting or playing, and when it started once it could be seen.
    static REACTION: std::cell::Cell<Option<(Reaction, Option<f32>)>> =
        const { std::cell::Cell::new(None) };
}

impl Reaction {
    /// The shortest gap between two plays of this reaction, so repeated events (copying
    /// again and again, a flaky charger) do not turn into a stream of animations.
    fn cooldown(self) -> f32 {
        match self {
            Self::Copied => 120.0,
            Self::MouseArrived | Self::Drive => 30.0,
            Self::Charging | Self::Surprised => 20.0,
            Self::WelcomeBack => 60.0,
            Self::Water | Self::StretchBreak => 600.0,
            Self::Lunch | Self::WrapUp => 3600.0,
            Self::Celebrate => 0.0,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

thread_local! {
    static LAST_PLAYED: std::cell::Cell<[Option<f32>; 11]> = const { std::cell::Cell::new([None; 11]) };
}

/// Whether `reaction` may play now; records it as played if so.
fn cooled_down(reaction: Reaction) -> bool {
    let now = seconds();
    LAST_PLAYED.with(|cell| {
        let mut last = cell.get();
        let ready = last[reaction.index()].is_none_or(|at| now - at >= reaction.cooldown());
        if ready {
            last[reaction.index()] = Some(now);
            cell.set(last);
        }
        ready
    })
}

/// Plays `reaction` the next time the robot is on screen, replacing any earlier one.
pub(crate) fn react(reaction: Reaction) {
    if !cooled_down(reaction) {
        return;
    }
    use crate::core::chime::{Sound, play};
    match reaction {
        Reaction::MouseArrived => play(Sound::Squeak),
        Reaction::Celebrate => play(Sound::Done),
        Reaction::WelcomeBack | Reaction::Charging => play(Sound::Hello),
        Reaction::Drive | Reaction::Surprised => play(Sound::Pop),
        Reaction::Copied
        | Reaction::Water
        | Reaction::StretchBreak
        | Reaction::Lunch
        | Reaction::WrapUp => {}
    }
    REACTION.with(|cell| cell.set(Some((reaction, None))));
}

/// The reaction to draw now and how far into it we are. The clock starts once the robot is
/// visible, so a reaction waits behind an activity card instead of playing unseen.
fn active_reaction(time: f32, alpha: u8) -> Option<(Reaction, f32)> {
    REACTION.with(|cell| {
        let (reaction, started) = cell.get()?;
        let started = match started {
            Some(started) => started,
            None if alpha > 200 => {
                cell.set(Some((reaction, Some(time))));
                time
            }
            None => return None,
        };
        let elapsed = time - started;
        if elapsed >= reaction.seconds() {
            cell.set(None);
            return None;
        }
        Some((reaction, elapsed))
    })
}

/// Water every hour of continuous activity (first after half an hour), a stretch every 55
/// minutes, lunch around 1 pm and a wrap-up around 6 pm. Never during a Focus session.
fn nudge(away: bool, focus_running: bool) {
    const WATER_EVERY: f32 = 3600.0;
    const STRETCH_EVERY: f32 = 3300.0;
    let now = seconds();
    let due = CONTEXT.with(|context| {
        let mut context = context.borrow_mut();
        if away {
            context.active_since = None;
            return None;
        }
        if context.active_since.is_none() {
            context.active_since = Some(now);
            context.next_water = now + WATER_EVERY / 2.0;
            context.next_stretch = now + STRETCH_EVERY;
        }
        if focus_running {
            return None;
        }
        if now >= context.next_water {
            context.next_water = now + WATER_EVERY;
            return Some(Reaction::Water);
        }
        if now >= context.next_stretch {
            context.next_stretch = now + STRETCH_EVERY;
            return Some(Reaction::StretchBreak);
        }
        None
    });
    let clock = crate::platform::shell().local_datetime();
    let daily = match (clock.hour, clock.minute) {
        (13, 0..30) => Some(Reaction::Lunch),
        (18, 0..30) => Some(Reaction::WrapUp),
        _ => None,
    };
    if let Some(reaction) = due.or(if focus_running || away { None } else { daily }) {
        react(reaction);
    }
}

thread_local! {
    /// Where the cursor is relative to the notch, as an eye offset, while it is near.
    static GAZE: std::cell::Cell<Option<(f32, f32)>> = const { std::cell::Cell::new(None) };
}

/// Lets open eyes follow the cursor; `None` when it is far from the notch.
pub(crate) fn set_gaze(gaze: Option<(f32, f32)>) {
    GAZE.with(|cell| cell.set(gaze));
}

/// Seconds into the current dance, while one is going.
fn dancing(time: f32) -> Option<f32> {
    CONTEXT.with(|context| {
        context
            .borrow()
            .dance_started
            .map(|start| time - start)
            .filter(|elapsed| (0.0..DANCE_SECONDS).contains(elapsed))
    })
}

fn seconds() -> f32 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f32()
}

fn color(alpha: u8, opacity: f32, (r, g, b): (u8, u8, u8)) -> Rgba {
    Rgba::from_argb((f32::from(alpha) * opacity.clamp(0.0, 1.0)) as u8, r, g, b)
}

fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// The scene to play now and how far into it we are, in seconds.
fn current_scene(time: f32, hour: u16, focus_running: bool) -> (Scene, f32) {
    if !(5..23).contains(&hour) {
        return (Scene::Night, time);
    }
    if focus_running {
        return (Scene::Laptop, time);
    }
    let index = (time / SCENE_SECONDS) as usize;
    (DAY[index % DAY.len()], time % SCENE_SECONDS)
}

fn greeting(name: &str) -> String {
    if name.is_empty() {
        "Hi!".to_string()
    } else {
        format!("Hi, {name}!")
    }
}

/// How visible the props are at `elapsed` seconds into a scene.
fn scene_fade(scene: Scene, elapsed: f32) -> f32 {
    if matches!(scene, Scene::Night) {
        return 1.0;
    }
    ease(elapsed / FADE_SECONDS) * ease((SCENE_SECONDS - elapsed) / FADE_SECONDS)
}

#[derive(Clone, Copy)]
enum Eyes {
    Closed,
    Happy,
    /// Open, looking by this offset in robot units.
    Open(f32, f32),
}

/// One frame of the robot: where its arms point (radians from pointing straight out to
/// its side, positive is down) and how its eyes look.
#[derive(Clone, Copy)]
struct Pose {
    eyes: Eyes,
    right_arm: f32,
    left_arm: f32,
    /// Extra rise of the body, in robot units.
    rise: f32,
    /// Lean in degrees around the feet.
    lean: f32,
}

impl Pose {
    const REST: f32 = 1.25;

    fn new(eyes: Eyes) -> Self {
        Self {
            eyes,
            right_arm: Self::REST,
            left_arm: Self::REST,
            rise: 0.0,
            lean: 0.0,
        }
    }
}

/// Where the robot's hands ended up, for props it holds.
struct Hands {
    right: Point,
    left: Point,
}

/// A blink every few seconds turns open eyes shut for a moment.
fn blink(eyes: Eyes, time: f32) -> Eyes {
    match eyes {
        Eyes::Open(..) if (time % 3.7) < 0.13 => Eyes::Closed,
        other => other,
    }
}

/// The idle notch: the robot and its current scene.
pub(super) fn draw(painter: Painter<'_>, rect: Rect, scale: f32, alpha: u8) {
    let time = seconds();
    let hour = crate::platform::shell().local_datetime().hour;
    let (name, focus_running) = CONTEXT.with(|context| {
        let context = context.borrow();
        (context.name.clone(), context.focus_running)
    });
    let away = CONTEXT.with(|context| context.borrow().away);
    let (scene, elapsed) = match dancing(time) {
        _ if away => (Scene::Nap, SCENE_SECONDS / 2.0),
        Some(elapsed) => (Scene::Dance, elapsed),
        None => current_scene(time, hour, focus_running),
    };
    let fade = if scene == Scene::Dance {
        ease(elapsed / FADE_SECONDS) * ease((DANCE_SECONDS - elapsed) / FADE_SECONDS)
    } else {
        scene_fade(scene, elapsed)
    };
    // The robot stands nearly the full height of the notch.
    let unit = (rect.height() / 25.0).clamp(0.7 * scale, 1.8 * scale);
    if let Some((reaction, elapsed)) = active_reaction(time, alpha) {
        let stage = Stage {
            painter,
            unit,
            alpha,
            x: rect.center_x() - 4.0 * unit,
            floor: rect.center_y() + 11.0 * unit,
            mid: rect.center_y(),
            time,
            elapsed,
            fade: ease(elapsed / FADE_SECONDS)
                * ease((reaction.seconds() - elapsed) / FADE_SECONDS),
        };
        stage.react(reaction, rect);
        return;
    }
    // Centre each scene by its own width: robot plus props, or the greeting text.
    let reach = match scene {
        Scene::Wave => {
            11.0 + FontManager::global().measure_text_cached(
                &greeting(&name),
                10.0 * unit,
                winisland_render::FontStyle::bold(),
            ) / unit
        }
        Scene::Chai => 30.0,
        Scene::Laptop => 26.0,
        Scene::Plant => 27.0,
        Scene::Read => 10.0,
        Scene::Stretch | Scene::LookAround => 13.0,
        Scene::Nap => 24.0,
        Scene::Night => 49.0,
        Scene::Paint => 31.0,
        Scene::Dance => 13.0,
    };
    let left_reach = 10.0;
    let offset = ((rect.width() / unit - (left_reach + reach)) / 2.0).max(1.0);
    let stage = Stage {
        painter,
        unit,
        alpha,
        x: rect.left + (offset + left_reach) * unit,
        floor: rect.center_y() + 11.0 * unit,
        mid: rect.center_y(),
        time,
        elapsed,
        fade,
    };
    match scene {
        Scene::Wave => stage.wave(&name),
        Scene::Chai => stage.chai(),
        Scene::Laptop => stage.laptop(),
        Scene::Read => stage.read(),
        Scene::Plant => stage.plant(),
        Scene::Stretch => stage.stretch(),
        Scene::LookAround => stage.look_around(),
        Scene::Nap => stage.sleep(false),
        Scene::Night => stage.sleep(true),
        Scene::Paint => stage.paint(),
        Scene::Dance => stage.dance(),
    }
}

/// What the robot does inside an activity card.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Gesture {
    /// Waves hello.
    Wave,
    /// Holds up a fresh picture, for a saved screenshot.
    Present,
    /// Slumped and sleepy, holding a nearly empty battery.
    Tired,
}

/// The robot awake in `gesture`, fitted to `rect`, for activity cards.
pub(crate) fn draw_gesture(painter: Painter<'_>, rect: Rect, alpha: u8, gesture: Gesture) {
    let unit = rect.height() / (ROBOT_HEIGHT + 2.0);
    let time = seconds();
    let stage = Stage {
        painter,
        unit,
        alpha,
        x: rect.center_x() - 1.5 * unit,
        floor: rect.bottom - unit,
        mid: rect.center_y(),
        time,
        elapsed: time,
        fade: 1.0,
    };
    match gesture {
        Gesture::Wave => {
            let mut pose = Pose::new(Eyes::Happy);
            pose.right_arm = -0.45 + (time / 0.9 * TAU).sin() * 0.45;
            stage.robot(pose);
        }
        Gesture::Present => stage.present(),
        Gesture::Tired => stage.tired(),
    }
}

#[derive(Clone, Copy)]
struct Stage<'a> {
    painter: Painter<'a>,
    unit: f32,
    alpha: u8,
    /// The robot's centre line.
    x: f32,
    /// Where its feet stand.
    floor: f32,
    /// The notch's vertical centre.
    mid: f32,
    time: f32,
    elapsed: f32,
    fade: f32,
}

impl Stage<'_> {
    fn color(&self, opacity: f32, rgb: (u8, u8, u8)) -> Rgba {
        color(self.alpha, opacity, rgb)
    }

    /// A prop colour, faded with the scene.
    fn prop(&self, opacity: f32, rgb: (u8, u8, u8)) -> Rgba {
        color(self.alpha, opacity * self.fade, rgb)
    }

    fn at(&self, dx: f32, dy: f32) -> Point {
        Point::new(self.x + dx * self.unit, self.floor + dy * self.unit)
    }

    fn rect(&self, dx: f32, dy: f32, width: f32, height: f32) -> Rect {
        Rect::from_xywh(
            self.x + dx * self.unit,
            self.floor + dy * self.unit,
            width * self.unit,
            height * self.unit,
        )
    }

    fn line(&self, from: Point, to: Point, width: f32, color: Rgba) {
        self.painter
            .stroke_line(from, to, width * self.unit, color, StrokeCap::Round);
    }

    fn text(&self, text: &str, x: f32, y: f32, size: f32, color: Rgba) {
        FontManager::global().draw_text_cached(DrawTextCachedParams {
            painter: self.painter,
            text,
            x,
            y,
            size: size * self.unit,
            bold: true,
            color,
            blur: None,
        });
    }

    /// Draws the robot in `pose` and reports where its hands are.
    fn robot(&self, pose: Pose) -> Hands {
        let unit = self.unit;
        let breath = (self.time / BREATH_SECONDS * TAU).sin();
        // The feet stay put; the rest of the body rises with each breath.
        let lift = ((breath * 0.5 + 0.5) * 0.8 + pose.rise) * unit;
        let painter = self.painter;
        let save = painter.save();
        if pose.lean != 0.0 {
            let pivot = self.at(0.0, 0.0);
            painter.translate(Vec2::new(pivot.x, pivot.y));
            painter.rotate_degrees(pose.lean);
            painter.translate(Vec2::new(-pivot.x, -pivot.y));
        }

        for side in [-1.0, 1.0] {
            painter.fill_oval(
                self.rect(side * 3.0 - 2.2, -2.4, 4.4, 2.4),
                self.color(1.0, FEET),
            );
        }
        let body = Rect::from_xywh(
            self.x - 4.6 * unit,
            self.floor - 8.2 * unit - lift * 0.5,
            9.2 * unit,
            6.4 * unit,
        );
        let shoulder_y = body.top + 1.6 * unit;
        let right_shoulder = Point::new(body.right - 0.6 * unit, shoulder_y);
        let left_shoulder = Point::new(body.left + 0.6 * unit, shoulder_y);
        let right = Point::new(
            right_shoulder.x + pose.right_arm.cos() * ARM_LENGTH * unit,
            right_shoulder.y + pose.right_arm.sin() * ARM_LENGTH * unit,
        );
        let left = Point::new(
            left_shoulder.x - pose.left_arm.cos() * ARM_LENGTH * unit,
            left_shoulder.y + pose.left_arm.sin() * ARM_LENGTH * unit,
        );
        self.line(right_shoulder, right, 2.4, self.color(1.0, SHELL));
        self.line(left_shoulder, left, 2.4, self.color(1.0, SHELL));
        painter.fill_round_rect(body, Radius::uniform(2.4 * unit), self.color(1.0, SHELL));

        // Head: a cream shell around a black screen, with a thicker chin like a monitor.
        let head = Rect::from_xywh(
            self.x - 8.0 * unit,
            body.top - 13.0 * unit - lift * 0.5,
            16.0 * unit,
            13.6 * unit,
        );
        painter.fill_round_rect(head, Radius::uniform(3.8 * unit), self.color(1.0, SHELL));
        let screen = Rect::from_xywh(
            head.left + 2.0 * unit,
            head.top + 2.0 * unit,
            head.width() - 4.0 * unit,
            head.height() - 5.2 * unit,
        );
        painter.fill_round_rect(screen, Radius::uniform(2.4 * unit), self.color(1.0, SCREEN));
        let eye_y = screen.center_y() + 0.6 * unit;
        for side in [-1.0, 1.0] {
            let eye_x = screen.center_x() + side * 2.8 * unit;
            match blink(pose.eyes, self.time) {
                Eyes::Closed => self.line(
                    Point::new(eye_x - 1.3 * unit, eye_y),
                    Point::new(eye_x + 1.3 * unit, eye_y),
                    1.4,
                    self.color(1.0, SHELL),
                ),
                Eyes::Happy => {
                    let peak = Point::new(eye_x, eye_y - 1.2 * unit);
                    for end in [-1.0, 1.0] {
                        self.line(
                            Point::new(eye_x + end * 1.5 * unit, eye_y + 0.4 * unit),
                            peak,
                            1.4,
                            self.color(1.0, SHELL),
                        );
                    }
                }
                Eyes::Open(dx, dy) => {
                    let (dx, dy) = GAZE.with(std::cell::Cell::get).unwrap_or((dx, dy));
                    painter.fill_round_rect(
                        Rect::from_xywh(
                            eye_x + (dx - 0.9) * unit,
                            eye_y + (dy - 1.5) * unit,
                            1.8 * unit,
                            3.0 * unit,
                        ),
                        Radius::uniform(0.9 * unit),
                        self.color(1.0, SHELL),
                    );
                }
            }
        }
        painter.restore_to(save);
        Hands { right, left }
    }

    fn wave(&self, name: &str) {
        let mut pose = Pose::new(Eyes::Happy);
        pose.right_arm = -0.45 + (self.time / 0.9 * TAU).sin() * 0.45;
        self.robot(pose);
        let greeting = greeting(name);
        let rise = (1.0 - ease(self.elapsed / 0.6)) * 3.0;
        self.text(
            &greeting,
            self.x + 11.0 * self.unit,
            self.mid + (3.0 + rise) * self.unit,
            10.0,
            self.prop(1.0, WHITE),
        );
    }

    fn chai(&self) {
        // Every few seconds the cup comes up for a sip, then goes back down.
        let cycle = (self.elapsed % 4.5) / 4.5;
        let sip = ease((cycle - 0.45) / 0.15) * ease((0.95 - cycle) / 0.15);
        let mut pose = Pose::new(if sip > 0.5 {
            Eyes::Happy
        } else {
            Eyes::Open(1.0, 0.0)
        });
        pose.right_arm = 0.55 + (-2.1 - 0.55) * sip;
        let hands = self.robot(pose);
        let cup = Rect::from_xywh(
            hands.right.x - 1.6 * self.unit,
            hands.right.y - 3.2 * self.unit,
            4.4 * self.unit,
            4.0 * self.unit,
        );
        self.painter
            .fill_round_rect(cup, Radius::uniform(1.0 * self.unit), self.prop(1.0, SHELL));
        self.painter.fill_rect(
            Rect::from_xywh(
                cup.left,
                cup.top + 1.2 * self.unit,
                cup.width(),
                1.1 * self.unit,
            ),
            self.prop(1.0, RED),
        );
        self.painter.stroke_circle(
            Point::new(cup.right + 0.4 * self.unit, cup.center_y()),
            1.1 * self.unit,
            0.8 * self.unit,
            self.prop(1.0, SHELL),
        );
        if sip < 0.2 {
            self.steam(Point::new(cup.center_x(), cup.top), 1.0);
        }

        // The teapot, steaming on its own.
        let pot = self.rect(17.0, -6.6, 9.0, 6.6);
        self.painter.fill_oval(pot, self.prop(1.0, SHELL));
        self.painter
            .fill_circle(self.at(21.5, -7.2), 1.2 * self.unit, self.prop(1.0, RED));
        self.line(
            self.at(17.6, -4.0),
            self.at(14.8, -6.0),
            1.4,
            self.prop(1.0, SHELL),
        );
        self.painter.stroke_circle(
            self.at(26.6, -3.6),
            1.6 * self.unit,
            0.9 * self.unit,
            self.prop(1.0, SHELL),
        );
        self.painter.fill_round_rect(
            self.rect(-6.0, -0.4, 36.0, 1.0),
            Radius::uniform(0.5 * self.unit),
            self.prop(0.9, CHAI),
        );
        self.steam(self.at(14.6, -6.6), 0.7);
    }

    /// Three soft wisps rising from `from`.
    fn steam(&self, from: Point, size: f32) {
        for index in 0..3 {
            let phase = (self.time / 1.6 + index as f32 / 3.0).fract();
            let sway = (phase * TAU + index as f32).sin() * 0.8 * self.unit;
            let x = from.x + (index as f32 - 1.0) * 1.3 * size * self.unit + sway;
            let y = from.y - (1.0 + phase * 5.0) * size * self.unit;
            let fade = (phase * PI).sin();
            self.line(
                Point::new(x, y),
                Point::new(x + sway * 0.5, y - 1.6 * size * self.unit),
                0.8,
                self.prop(0.55 * fade, WHITE),
            );
        }
    }

    fn laptop(&self) {
        let tap = (self.time * 9.0).sin() * 0.12;
        let mut pose = Pose::new(Eyes::Open(1.2, 0.6));
        pose.right_arm = 0.35 + tap;
        pose.left_arm = 1.0 - tap;
        self.robot(pose);

        // Screen, angled up from the back of the keyboard.
        let screen = self.rect(9.0, -14.0, 16.0, 11.0);
        self.painter.fill_round_rect(
            screen,
            Radius::uniform(1.4 * self.unit),
            self.prop(1.0, LAPTOP),
        );
        let inner = Rect::from_ltrb(
            screen.left + 1.2 * self.unit,
            screen.top + 1.2 * self.unit,
            screen.right - 1.2 * self.unit,
            screen.bottom - 1.0 * self.unit,
        );
        self.painter.fill_round_rect(
            inner,
            Radius::uniform(0.6 * self.unit),
            self.prop(1.0, (20, 22, 30)),
        );
        // Code scrolls up a line at a time.
        let save = self.painter.save();
        self.painter.clip_rect(inner);
        let line_height = 1.7 * self.unit;
        let scroll = (self.time * 1.4).fract() * line_height;
        let first = (self.time * 1.4) as usize;
        for row in 0..6 {
            let index = first + row;
            let indent = [0.0, 1.5, 3.0, 1.5, 0.0, 3.0][index % 6] * self.unit;
            let width = [6.0, 8.5, 5.0, 9.5, 7.0, 4.5][(index * 7) % 6] * self.unit;
            let y = inner.top + 1.0 * self.unit + row as f32 * line_height - scroll;
            self.line(
                Point::new(inner.left + 1.0 * self.unit + indent, y),
                Point::new(
                    (inner.left + 1.0 * self.unit + indent + width).min(inner.right - self.unit),
                    y,
                ),
                0.9,
                self.prop(0.95, CODE[index % CODE.len()]),
            );
        }
        self.painter.restore_to(save);
        // Keyboard base.
        self.painter.fill_round_rect(
            self.rect(5.0, -3.2, 21.0, 2.2),
            Radius::uniform(1.0 * self.unit),
            self.prop(1.0, (150, 150, 160)),
        );
    }

    fn read(&self) {
        // Eyes sweep along a line, then jump back for the next one.
        let sweep = (self.elapsed % 1.8) / 1.8;
        let mut pose = Pose::new(Eyes::Open(-1.0 + 2.0 * sweep, 1.2));
        pose.right_arm = 0.6;
        pose.left_arm = 0.6;
        let hands = self.robot(pose);
        let middle = Point::new(
            (hands.left.x + hands.right.x) / 2.0,
            hands.right.y - 3.5 * self.unit,
        );
        let half = 6.6 * self.unit;
        let height = 7.6 * self.unit;
        // Cover, then two pages.
        self.painter.fill_round_rect(
            Rect::from_xywh(
                middle.x - half - 0.5 * self.unit,
                middle.y - height / 2.0 - 0.4 * self.unit,
                2.0 * half + self.unit,
                height + 0.8 * self.unit,
            ),
            Radius::uniform(1.0 * self.unit),
            self.prop(1.0, BOOK),
        );
        for side in [-1.0, 1.0] {
            let page = Rect::from_xywh(
                if side < 0.0 {
                    middle.x - half
                } else {
                    middle.x + 0.2 * self.unit
                },
                middle.y - height / 2.0,
                half - 0.2 * self.unit,
                height,
            );
            self.painter
                .fill_rect(page, self.prop(1.0, (250, 248, 240)));
            for row in 0..3 {
                let y = page.top + (1.4 + row as f32 * 1.6) * self.unit;
                self.line(
                    Point::new(page.left + 0.8 * self.unit, y),
                    Point::new(page.right - 0.8 * self.unit, y),
                    0.5,
                    self.prop(0.6, (150, 150, 160)),
                );
            }
        }
        // Every few seconds a page turns over the spine.
        let turn = (self.elapsed % 5.0) / 0.6;
        if turn < 1.0 {
            let width = (half - 0.2 * self.unit) * (1.0 - 2.0 * turn).abs();
            let left = if turn < 0.5 {
                middle.x + 0.2 * self.unit
            } else {
                middle.x - width
            };
            self.painter.fill_rect(
                Rect::from_xywh(left, middle.y - height / 2.0, width, height),
                self.prop(1.0, (232, 230, 222)),
            );
        }
    }

    fn plant(&self) {
        // Pours for a while, then admires the plant.
        let pouring = (self.elapsed % 7.0) < 4.5;
        let mut pose = Pose::new(if pouring {
            Eyes::Open(1.2, 0.6)
        } else {
            Eyes::Happy
        });
        pose.right_arm = if pouring { -0.1 } else { 0.9 };
        let hands = self.robot(pose);
        // Watering can in the right hand, tipped while pouring.
        let can = Rect::from_xywh(
            hands.right.x - 1.0 * self.unit,
            hands.right.y - 1.6 * self.unit,
            5.0 * self.unit,
            3.6 * self.unit,
        );
        let save = self.painter.save();
        if pouring {
            let pivot = Point::new(can.center_x(), can.center_y());
            self.painter.translate(Vec2::new(pivot.x, pivot.y));
            self.painter.rotate_degrees(24.0);
            self.painter.translate(Vec2::new(-pivot.x, -pivot.y));
        }
        self.painter
            .fill_round_rect(can, Radius::uniform(0.9 * self.unit), self.prop(1.0, WATER));
        self.line(
            Point::new(can.right, can.top + 1.0 * self.unit),
            Point::new(can.right + 2.6 * self.unit, can.top - 0.6 * self.unit),
            1.0,
            self.prop(1.0, WATER),
        );
        self.painter.restore_to(save);
        if pouring {
            for index in 0..3 {
                let phase = (self.time * 1.8 + index as f32 / 3.0).fract();
                self.painter.fill_circle(
                    Point::new(
                        can.right + 2.8 * self.unit + index as f32 * 0.6 * self.unit,
                        can.top + phase * 7.0 * self.unit,
                    ),
                    0.55 * self.unit,
                    self.prop(0.9 * (1.0 - phase), WATER),
                );
            }
        }
        // The plant grows through the scene.
        let growth = 0.5 + 0.5 * ease(self.elapsed / (SCENE_SECONDS * 0.7));
        let stem_base = self.at(22.0, -4.4);
        let stem_top = self.at(22.0, -4.4 - 6.0 * growth);
        self.line(stem_base, stem_top, 0.9, self.prop(1.0, LEAF));
        for (side, height) in [(-1.0, 0.45), (1.0, 0.7), (-1.0, 0.95)] {
            let y = stem_base.y + (stem_top.y - stem_base.y) * height;
            self.painter.fill_oval(
                Rect::from_xywh(
                    stem_base.x
                        + if side < 0.0 {
                            -3.0 * growth * self.unit
                        } else {
                            0.0
                        },
                    y - 0.9 * self.unit,
                    3.0 * growth * self.unit,
                    1.8 * self.unit,
                ),
                self.prop(1.0, LEAF),
            );
        }
        self.painter.fill_round_rect(
            self.rect(19.0, -4.6, 6.0, 4.6),
            Radius::uniform(1.0 * self.unit),
            self.prop(1.0, POT),
        );
    }

    fn stretch(&self) {
        let reach = ease(((self.elapsed % 4.0) / 4.0 * TAU).sin() * 0.5 + 0.5);
        let mut pose = Pose::new(if reach > 0.6 {
            Eyes::Happy
        } else {
            Eyes::Closed
        });
        pose.right_arm = 0.9 + (-1.4 - 0.9) * reach;
        pose.left_arm = pose.right_arm;
        pose.rise = 1.4 * reach;
        self.robot(pose);
        if reach > 0.6 {
            for (dx, dy) in [(-11.0, -19.0), (11.5, -20.0)] {
                self.sparkle(self.at(dx, dy), (reach - 0.6) / 0.4);
            }
        }
    }

    /// A small four-pointed twinkle.
    fn sparkle(&self, at: Point, strength: f32) {
        let size = 1.2 * self.unit * strength.clamp(0.0, 1.0);
        let color = self.prop(strength, (255, 225, 140));
        self.line(
            Point::new(at.x - size, at.y),
            Point::new(at.x + size, at.y),
            0.7,
            color,
        );
        self.line(
            Point::new(at.x, at.y - size),
            Point::new(at.x, at.y + size),
            0.7,
            color,
        );
    }

    fn look_around(&self) {
        // Glance left, centre, right and back, holding each look.
        let looks = [-1.3, 0.0, 1.3, 0.0];
        let step = ((self.elapsed / 1.6) as usize) % looks.len();
        let mut pose = Pose::new(Eyes::Open(looks[step], -0.3));
        pose.lean = looks[step] * 3.0;
        self.robot(pose);
        // Now and then it wonders about something.
        let wonder = (self.elapsed % 6.0) / 6.0;
        if wonder > 0.55 {
            let strength = ease((wonder - 0.55) / 0.1) * ease((1.0 - wonder) / 0.1);
            self.text(
                "?",
                self.x + 11.0 * self.unit,
                self.mid - (2.0 + strength * 2.0) * self.unit,
                10.0,
                self.prop(strength, WHITE),
            );
        }
    }

    fn paint(&self) {
        // The brush dabs at the canvas while the palette waits in the other hand.
        let dab = (self.time * 2.2 * TAU).sin();
        let mut pose = Pose::new(Eyes::Open(1.3, -0.2));
        pose.right_arm = -0.2 + dab * 0.22;
        pose.left_arm = 0.75;
        let hands = self.robot(pose);

        // Easel: two front legs, one behind, and the canvas on top.
        let wood = (170, 120, 80);
        self.line(
            self.at(14.0, -9.0),
            self.at(12.4, 0.0),
            1.0,
            self.prop(1.0, wood),
        );
        self.line(
            self.at(24.0, -9.0),
            self.at(25.6, 0.0),
            1.0,
            self.prop(1.0, wood),
        );
        self.line(
            self.at(19.0, -18.0),
            self.at(19.0, 0.0),
            0.8,
            self.prop(0.7, wood),
        );
        let canvas = self.rect(13.0, -19.0, 12.0, 9.5);
        self.painter.fill_round_rect(
            canvas,
            Radius::uniform(0.6 * self.unit),
            self.prop(1.0, (250, 248, 240)),
        );
        // The picture appears a layer at a time: sky, sun, hills, then a little house.
        let progress = ease(self.elapsed / (SCENE_SECONDS * 0.8));
        let inner = Rect::from_ltrb(
            canvas.left + 0.8 * self.unit,
            canvas.top + 0.8 * self.unit,
            canvas.right - 0.8 * self.unit,
            canvas.bottom - 0.8 * self.unit,
        );
        let save = self.painter.save();
        self.painter.clip_rect(inner);
        let layer = |from: f32| ease((progress - from) / 0.2);
        self.painter.fill_rect(
            Rect::from_xywh(
                inner.left,
                inner.top,
                inner.width() * layer(0.0),
                inner.height(),
            ),
            self.prop(1.0, (150, 200, 245)),
        );
        self.painter.fill_circle(
            Point::new(inner.right - 2.4 * self.unit, inner.top + 2.2 * self.unit),
            1.6 * self.unit * layer(0.25),
            self.prop(1.0, (255, 205, 90)),
        );
        let hills = layer(0.5);
        self.painter.fill_oval(
            Rect::from_xywh(
                inner.left - 2.0 * self.unit,
                inner.bottom - 3.4 * self.unit * hills,
                inner.width() * 0.8,
                6.0 * self.unit,
            ),
            self.prop(1.0, (110, 190, 110)),
        );
        self.painter.fill_oval(
            Rect::from_xywh(
                inner.left + inner.width() * 0.45,
                inner.bottom - 2.6 * self.unit * hills,
                inner.width() * 0.8,
                6.0 * self.unit,
            ),
            self.prop(1.0, (80, 165, 95)),
        );
        let house = layer(0.75);
        if house > 0.0 {
            let base = Rect::from_xywh(
                inner.left + 3.0 * self.unit,
                inner.bottom - 4.6 * self.unit,
                2.6 * self.unit,
                2.2 * self.unit,
            );
            self.painter
                .fill_rect(base, self.prop(house, (240, 240, 235)));
            self.line(
                Point::new(base.left - 0.3 * self.unit, base.top),
                Point::new(base.center_x(), base.top - 1.4 * self.unit),
                0.7,
                self.prop(house, RED),
            );
            self.line(
                Point::new(base.center_x(), base.top - 1.4 * self.unit),
                Point::new(base.right + 0.3 * self.unit, base.top),
                0.7,
                self.prop(house, RED),
            );
        }
        self.painter.restore_to(save);

        // Brush in the right hand, its tip taking a colour from the palette.
        let tip_colors = [(150, 200, 245), (255, 205, 90), (110, 190, 110), RED];
        let tip_color = tip_colors[((progress * 4.0) as usize).min(3)];
        let tip = Point::new(
            hands.right.x + 3.4 * self.unit,
            hands.right.y - 1.6 * self.unit,
        );
        self.line(hands.right, tip, 0.7, self.prop(1.0, wood));
        self.painter
            .fill_circle(tip, 0.7 * self.unit, self.prop(1.0, tip_color));
        // Palette in the left hand.
        let palette = Rect::from_xywh(
            hands.left.x - 3.2 * self.unit,
            hands.left.y - 1.8 * self.unit,
            5.0 * self.unit,
            3.2 * self.unit,
        );
        self.painter
            .fill_oval(palette, self.prop(1.0, (236, 214, 176)));
        for (index, rgb) in tip_colors.iter().enumerate() {
            self.painter.fill_circle(
                Point::new(
                    palette.left + (1.1 + index as f32 * 0.95) * self.unit,
                    palette.center_y() + if index % 2 == 0 { -0.5 } else { 0.5 } * self.unit,
                ),
                0.45 * self.unit,
                self.prop(1.0, *rgb),
            );
        }
    }

    fn dance(&self) {
        // Two beats a second: arms swap, the body bounces and sways.
        let beat = (self.elapsed * 2.0 * TAU).sin();
        let mut pose = Pose::new(Eyes::Happy);
        pose.right_arm = -0.9 + beat * 0.8;
        pose.left_arm = -0.9 - beat * 0.8;
        pose.rise = beat.abs() * 1.4;
        pose.lean = (self.elapsed * TAU).sin() * 8.0;
        self.robot(pose);
        // Notes float up on either side.
        let colors = [(130, 220, 255), (255, 160, 210), (255, 220, 120)];
        for index in 0..3 {
            let phase = (self.elapsed / 1.6 + index as f32 / 3.0).fract();
            let side = if index % 2 == 0 { 1.0 } else { -1.0 };
            self.text(
                if index == 1 { "♫" } else { "♪" },
                self.x + side * (10.0 + phase * 2.0) * self.unit
                    - if side < 0.0 { 5.0 * self.unit } else { 0.0 },
                self.floor - (6.0 + phase * 12.0) * self.unit,
                7.5,
                self.prop((phase * PI).sin(), colors[index]),
            );
        }
    }

    /// The robot `dx` units to the side, for running about.
    fn shifted(&self, dx: f32) -> Self {
        Self {
            x: self.x + dx * self.unit,
            ..*self
        }
    }

    /// A short line of text beside the robot, fading in.
    fn caption(&self, key: &str, t: f32) {
        self.text(
            &winisland_core::i18n::tr(key),
            self.x + 11.0 * self.unit,
            self.mid + 3.0 * self.unit,
            9.0,
            self.prop(ease((t - 0.3) / 0.4), WHITE),
        );
    }

    fn exclaim(&self, at: Point, strength: f32) {
        self.text("!", at.x, at.y, 12.0, self.prop(strength, (255, 214, 90)));
    }

    fn react(&self, reaction: Reaction, rect: Rect) {
        let t = self.elapsed;
        let u = self.unit;
        match reaction {
            Reaction::Water => {
                // Sips from a glass every couple of seconds.
                let sip = ease(((t % 2.4) - 1.2) / 0.3) * ease((2.4 - (t % 2.4)) / 0.3);
                let mut pose = Pose::new(if sip > 0.5 {
                    Eyes::Happy
                } else {
                    Eyes::Open(1.0, 0.0)
                });
                pose.right_arm = 0.55 + (-2.1 - 0.55) * sip;
                let hands = self.robot(pose);
                let glass = Rect::from_xywh(
                    hands.right.x - 1.4 * u,
                    hands.right.y - 4.0 * u,
                    3.6 * u,
                    4.6 * u,
                );
                self.painter.fill_rect(
                    Rect::from_ltrb(glass.left, glass.top + 1.6 * u, glass.right, glass.bottom),
                    self.prop(0.9, WATER),
                );
                self.painter
                    .stroke_rect(glass, 0.5 * u, self.prop(1.0, WHITE));
                self.caption("nudge_water", t);
            }
            Reaction::StretchBreak => {
                self.stretch();
                self.caption("nudge_stretch", t);
            }
            Reaction::Lunch => {
                let mut pose = Pose::new(Eyes::Happy);
                pose.right_arm = 0.5 + (t * 3.0).sin() * 0.15;
                pose.left_arm = 0.6;
                let hands = self.robot(pose);
                let bowl = Rect::from_xywh(
                    (hands.left.x + hands.right.x) / 2.0 - 4.6 * u,
                    hands.right.y - 1.6 * u,
                    9.2 * u,
                    3.8 * u,
                );
                self.painter
                    .fill_round_rect(bowl, Radius::uniform(1.8 * u), self.prop(1.0, POT));
                self.painter.fill_oval(
                    Rect::from_xywh(bowl.left, bowl.top - 0.8 * u, bowl.width(), 1.8 * u),
                    self.prop(1.0, (250, 214, 150)),
                );
                let wood = self.prop(1.0, (200, 160, 110));
                self.line(
                    hands.right,
                    Point::new(bowl.center_x() + 1.0 * u, bowl.top),
                    0.5,
                    wood,
                );
                self.line(
                    Point::new(hands.right.x + 0.8 * u, hands.right.y),
                    Point::new(bowl.center_x() + 2.0 * u, bowl.top),
                    0.5,
                    wood,
                );
                self.steam(Point::new(bowl.center_x(), bowl.top - 0.6 * u), 1.0);
                self.caption("nudge_lunch", t);
            }
            Reaction::WrapUp => {
                // The laptop lid comes down, then a wave goodbye.
                let close = ease(t / 1.4);
                let mut pose = Pose::new(if close < 1.0 {
                    Eyes::Open(1.2, 0.6)
                } else {
                    Eyes::Happy
                });
                pose.right_arm = if t > 1.6 {
                    -0.45 + (t / 0.9 * TAU).sin() * 0.45
                } else {
                    0.4
                };
                self.robot(pose);
                let lid = (1.0 - close) * 11.0;
                self.painter.fill_round_rect(
                    self.rect(9.0, -3.0 - lid, 16.0, lid.max(0.6)),
                    Radius::uniform(1.0 * u),
                    self.prop(1.0, LAPTOP),
                );
                self.painter.fill_round_rect(
                    self.rect(5.0, -3.2, 21.0, 2.2),
                    Radius::uniform(1.0 * u),
                    self.prop(1.0, (150, 150, 160)),
                );
                self.caption("nudge_wrap_up", t);
            }
            Reaction::MouseArrived => self.mouse_chase(rect),
            Reaction::WelcomeBack => {
                // Eyes open, a big stretch, then a wave.
                let stretch = ease(t / 0.8) * ease((2.0 - t) / 0.6);
                let mut pose = Pose::new(if t < 0.5 { Eyes::Closed } else { Eyes::Happy });
                pose.rise = stretch * 1.4;
                pose.left_arm = 1.0 - 2.4 * stretch;
                pose.right_arm = if t > 2.0 {
                    -0.45 + (t / 0.9 * TAU).sin() * 0.45
                } else {
                    pose.left_arm
                };
                self.robot(pose);
                if t > 2.0 {
                    self.text(
                        &winisland_core::i18n::tr("activity_welcome_back"),
                        self.x + 11.0 * u,
                        self.mid + 3.0 * u,
                        9.0,
                        self.prop(ease((t - 2.0) / 0.4), WHITE),
                    );
                }
            }
            Reaction::Charging => {
                // A bolt drops from the top into raised hands, then glows there.
                let caught = t > 0.8;
                let mut pose = Pose::new(if caught {
                    Eyes::Happy
                } else {
                    Eyes::Open(0.0, -1.0)
                });
                pose.right_arm = -1.3;
                pose.left_arm = -1.3;
                pose.rise = if caught {
                    (t * 6.0).sin().abs() * 0.8
                } else {
                    0.0
                };
                let hands = self.robot(pose);
                let top = rect.top - 6.0 * u;
                let held = hands.right.y.min(hands.left.y) - 3.0 * u;
                self.bolt(Point::new(self.x, top + (held - top) * ease(t / 0.8)));
                if caught {
                    self.sparkle(self.at(-9.0, -18.0), 0.6 + 0.4 * (t * 5.0).sin());
                    self.sparkle(self.at(9.5, -17.0), 0.6 + 0.4 * (t * 4.0).cos());
                }
            }
            Reaction::Surprised => {
                let mut pose = Pose::new(Eyes::Open(0.0, -0.6));
                pose.rise = ((t / 0.5).min(1.0) * PI).sin() * 3.0;
                pose.right_arm = -1.1;
                pose.left_arm = -1.1;
                self.robot(pose);
                self.exclaim(self.at(9.0, -17.0), 1.0);
            }
            Reaction::Copied => {
                let mut pose = Pose::new(Eyes::Happy);
                pose.right_arm = 1.0 - 1.6 * ease(t / 0.4);
                let hands = self.robot(pose);
                let note = Rect::from_xywh(
                    hands.right.x - 1.0 * u,
                    hands.right.y - 6.0 * u,
                    9.2 * u,
                    6.0 * u,
                );
                self.painter.fill_round_rect(
                    note,
                    Radius::uniform(0.8 * u),
                    self.prop(1.0, (255, 250, 235)),
                );
                for row in 0..2 {
                    let y = note.top + (1.8 + row as f32 * 1.6) * u;
                    self.line(
                        Point::new(note.left + 1.2 * u, y),
                        Point::new(note.right - 3.4 * u, y),
                        0.5,
                        self.prop(0.7, (150, 150, 160)),
                    );
                }
                let check = Point::new(note.right - 2.2 * u, note.bottom - 2.0 * u);
                let bottom = Point::new(check.x - 0.2 * u, check.y + 0.8 * u);
                let green = self.prop(1.0, (60, 190, 100));
                self.line(Point::new(check.x - 1.0 * u, check.y), bottom, 0.7, green);
                self.line(
                    bottom,
                    Point::new(check.x + 1.2 * u, check.y - 1.0 * u),
                    0.7,
                    green,
                );
            }
            Reaction::Drive => {
                // A USB stick drops in and is caught.
                let caught = t > 0.9;
                let mut pose = Pose::new(if caught {
                    Eyes::Happy
                } else {
                    Eyes::Open(0.0, -1.2)
                });
                pose.right_arm = 0.1;
                pose.left_arm = 0.1;
                let hands = self.robot(pose);
                let start = rect.top - 6.0 * u;
                let y = start + (hands.right.y - 3.0 * u - start) * ease(t / 0.9);
                let stick = Rect::from_xywh(self.x - 1.4 * u, y, 2.8 * u, 4.2 * u);
                self.painter.fill_round_rect(
                    stick,
                    Radius::uniform(0.6 * u),
                    self.prop(1.0, (90, 120, 220)),
                );
                self.painter.fill_rect(
                    Rect::from_xywh(stick.left + 0.5 * u, stick.top - 1.4 * u, 1.8 * u, 1.6 * u),
                    self.prop(1.0, (200, 200, 210)),
                );
                if caught {
                    self.sparkle(self.at(8.0, -15.0), 0.5 + 0.5 * (t * 5.0).sin());
                }
            }
            Reaction::Celebrate => {
                let hop = (t * 1.5 * TAU).sin();
                let mut pose = Pose::new(Eyes::Happy);
                pose.rise = hop.abs() * 2.6;
                pose.right_arm = -1.5 + hop * 0.3;
                pose.left_arm = -1.5 - hop * 0.3;
                self.robot(pose);
                let colors = [
                    (255, 120, 120),
                    (255, 210, 90),
                    (120, 210, 140),
                    (120, 180, 255),
                    (220, 140, 255),
                ];
                for index in 0..18 {
                    let seed = index as f32 * 12.9898;
                    let column = (seed.sin() * 43758.547).fract().abs();
                    let speed = 0.6 + (seed.cos() * 1234.5).fract().abs() * 0.6;
                    let fall = ((t * speed + column) % 1.4) / 1.4;
                    let x = rect.left + column * rect.width();
                    let y = rect.top + fall * rect.height();
                    let spin = (t * 6.0 + seed).sin() * 0.8 * u;
                    self.line(
                        Point::new(x - spin, y),
                        Point::new(x + spin, y + 0.6 * u),
                        0.9,
                        self.prop(1.0, colors[index % colors.len()]),
                    );
                }
            }
        }
    }

    /// A lightning bolt centred on `at`.
    fn bolt(&self, at: Point) {
        let u = self.unit;
        let points = [(1.2, -3.0), (-1.0, 0.2), (0.6, 0.2), (-1.2, 3.2)];
        for pair in points.windows(2) {
            self.line(
                Point::new(at.x + pair[0].0 * u, at.y + pair[0].1 * u),
                Point::new(at.x + pair[1].0 * u, at.y + pair[1].1 * u),
                1.3,
                self.prop(1.0, (255, 214, 90)),
            );
        }
    }

    /// A little grey mouse standing at `at`, facing left when `left`, legs scurrying when
    /// `running`.
    fn critter(&self, at: Point, left: bool, running: bool) {
        let u = self.unit;
        let facing = if left { -1.0 } else { 1.0 };
        let grey = self.prop(1.0, (168, 168, 178));
        let pink = self.prop(1.0, (240, 160, 170));
        let tail_wave = (self.time * 10.0).sin() * 0.8 * u;
        self.line(
            Point::new(at.x - facing * 3.0 * u, at.y - 1.0 * u),
            Point::new(at.x - facing * 6.0 * u, at.y - 2.4 * u + tail_wave),
            0.5,
            pink,
        );
        let step = if running {
            (self.time * 30.0).sin() * 0.8 * u
        } else {
            0.0
        };
        for (dx, phase) in [(-1.6, 1.0), (1.4, -1.0)] {
            self.line(
                Point::new(at.x + dx * u, at.y - 0.8 * u),
                Point::new(at.x + dx * u + step * phase, at.y),
                0.5,
                grey,
            );
        }
        self.painter.fill_oval(
            Rect::from_xywh(at.x - 3.2 * u, at.y - 3.6 * u, 6.4 * u, 3.2 * u),
            grey,
        );
        let head = Point::new(at.x + facing * 3.2 * u, at.y - 2.4 * u);
        self.painter.fill_circle(head, 1.6 * u, grey);
        let ear = Point::new(head.x - facing * 0.6 * u, head.y - 1.6 * u);
        self.painter.fill_circle(ear, 1.0 * u, grey);
        self.painter.fill_circle(ear, 0.55 * u, pink);
        self.painter.fill_circle(
            Point::new(head.x + facing * 0.6 * u, head.y - 0.3 * u),
            0.3 * u,
            self.prop(1.0, (20, 20, 24)),
        );
        self.painter.fill_circle(
            Point::new(head.x + facing * 1.6 * u, head.y + 0.2 * u),
            0.35 * u,
            pink,
        );
    }

    /// A mouse wanders in; the robot spots it, panics and runs off; the mouse leaves; the
    /// robot creeps back, relieved.
    fn mouse_chase(&self, rect: Rect) {
        let t = self.elapsed;
        let u = self.unit;
        let off_left = (rect.left - self.x) / u - 14.0;
        let enter_x = (rect.right - self.x) / u + 8.0;
        let near = 15.0;
        let mouse_x = if t < 0.9 {
            enter_x + (near - enter_x) * ease(t / 0.9)
        } else if t < 3.0 {
            near - 6.0 * ease((t - 1.4) / 1.4)
        } else if t < 3.6 {
            near - 6.0
        } else {
            near - 6.0 + (enter_x - near + 6.0) * ease((t - 3.6) / 0.9)
        };
        let leaving = t >= 3.6;
        let mouse_running = !(0.9..1.4).contains(&t) && !(3.0..3.6).contains(&t);
        // The robot freezes, jumps, flees, then creeps back.
        let (dx, pose) = if t < 0.9 {
            (0.0, Pose::new(Eyes::Open(1.2, 0.4)))
        } else if t < 1.4 {
            let mut pose = Pose::new(Eyes::Open(1.6, -0.4));
            pose.rise = ((t - 0.9) / 0.5 * PI).sin() * 3.0;
            pose.right_arm = -1.4;
            pose.left_arm = -1.4;
            (0.0, pose)
        } else if t < 4.2 {
            let stride = (t * 22.0).sin();
            let mut pose = Pose::new(Eyes::Open(-1.4, -0.4));
            pose.lean = -12.0;
            pose.rise = stride.abs() * 1.2;
            pose.right_arm = -1.6 + stride * 0.3;
            pose.left_arm = -1.6 - stride * 0.3;
            (off_left * ease((t - 1.4) / 1.2), pose)
        } else {
            let back = ease((t - 4.2) / 1.4);
            let relieved = back > 0.9;
            let mut pose = Pose::new(if relieved {
                Eyes::Happy
            } else {
                Eyes::Open(1.0, 0.0)
            });
            pose.right_arm = if relieved { -2.0 } else { Pose::REST };
            (off_left * (1.0 - back), pose)
        };
        let robot = self.shifted(dx);
        robot.robot(pose);
        if (0.9..1.8).contains(&t) {
            robot.exclaim(robot.at(8.0, -18.0), 1.0);
        }
        if t > 5.2 {
            // A relieved bead of sweat by its head.
            let drop = robot.at(8.4, -15.0 + (t - 5.2) * 3.0);
            self.painter
                .fill_circle(drop, 0.8 * u, self.prop(0.9, (130, 190, 255)));
        }
        self.critter(
            Point::new(self.x + mouse_x * u, self.floor),
            !leaving,
            mouse_running,
        );
    }

    fn tired(&self) {
        let mut pose = Pose::new(Eyes::Closed);
        pose.right_arm = 0.2;
        pose.lean = 6.0 + (self.time * 0.8).sin() * 1.5;
        pose.rise = -0.6;
        let hands = self.robot(pose);
        // A battery with one red bar left, held out.
        let body = Rect::from_xywh(
            hands.right.x,
            hands.right.y - 2.0 * self.unit,
            6.0 * self.unit,
            3.4 * self.unit,
        );
        self.painter.stroke_round_rect(
            body,
            Radius::uniform(0.8 * self.unit),
            0.6 * self.unit,
            self.color(1.0, WHITE),
        );
        self.painter.fill_rect(
            Rect::from_xywh(
                body.right,
                body.center_y() - 0.7 * self.unit,
                0.7 * self.unit,
                1.4 * self.unit,
            ),
            self.color(1.0, WHITE),
        );
        let blink = 0.5 + 0.5 * (self.time * 4.0).sin();
        self.painter.fill_rect(
            Rect::from_xywh(
                body.left + 0.7 * self.unit,
                body.top + 0.7 * self.unit,
                1.3 * self.unit,
                body.height() - 1.4 * self.unit,
            ),
            self.color(blink, RED),
        );
    }

    /// Holds up a small framed picture, proud of it.
    fn present(&self) {
        let lift = ease((self.time % 2.4) / 0.4) * 0.4;
        let mut pose = Pose::new(Eyes::Happy);
        pose.right_arm = -0.6 - lift;
        pose.rise = 0.6;
        let hands = self.robot(pose);
        let frame = Rect::from_xywh(
            hands.right.x - 1.0 * self.unit,
            hands.right.y - 6.4 * self.unit,
            8.0 * self.unit,
            6.4 * self.unit,
        );
        self.painter.fill_round_rect(
            frame,
            Radius::uniform(0.8 * self.unit),
            self.color(1.0, WHITE),
        );
        let inner = Rect::from_ltrb(
            frame.left + 0.8 * self.unit,
            frame.top + 0.8 * self.unit,
            frame.right - 0.8 * self.unit,
            frame.bottom - 0.8 * self.unit,
        );
        self.painter
            .fill_rect(inner, self.color(1.0, (150, 200, 245)));
        let save = self.painter.save();
        self.painter.clip_rect(inner);
        self.painter.fill_oval(
            Rect::from_xywh(
                inner.left - self.unit,
                inner.bottom - 2.2 * self.unit,
                inner.width() + 2.0 * self.unit,
                4.0 * self.unit,
            ),
            self.color(1.0, (110, 190, 110)),
        );
        self.painter.fill_circle(
            Point::new(inner.right - 1.6 * self.unit, inner.top + 1.4 * self.unit),
            1.0 * self.unit,
            self.color(1.0, (255, 205, 90)),
        );
        self.painter.restore_to(save);
        self.sparkle(
            Point::new(frame.right + 0.6 * self.unit, frame.top - 0.4 * self.unit),
            0.6 + 0.4 * (self.time * 3.0).sin(),
        );
    }

    fn sleep(&self, night: bool) {
        let mut pose = Pose::new(Eyes::Closed);
        pose.lean = if night { 4.0 } else { 0.0 };
        self.robot(pose);
        // "z" and "Z" each keep their own lane, rising and fading half a cycle apart.
        for (offset, text, size, lane_x, lane_y) in
            [(0.0, "z", 7.0, 11.0, -12.0), (0.5, "Z", 9.5, 18.0, -17.0)]
        {
            let phase = (self.time / 2.4 + offset).fract();
            let fade = (phase * PI).sin();
            self.text(
                text,
                self.x + (lane_x + phase * 2.0) * self.unit,
                self.floor + (lane_y - phase * 4.0) * self.unit,
                size,
                self.prop(fade * 0.9, WHITE),
            );
        }
        if night {
            // A crescent moon and a couple of twinkling stars.
            let moon = self.at(32.0, -17.0);
            self.painter
                .fill_circle(moon, 3.6 * self.unit, self.color(1.0, (250, 232, 170)));
            self.painter.fill_circle(
                Point::new(moon.x + 1.6 * self.unit, moon.y - 1.0 * self.unit),
                3.2 * self.unit,
                self.color(1.0, (0, 0, 0)),
            );
            for (index, (dx, dy)) in [(40.0, -9.0), (26.0, -6.0), (46.0, -18.0)]
                .iter()
                .enumerate()
            {
                let twinkle =
                    ((self.time * 1.3 + index as f32 * 1.7) * TAU / 3.0).sin() * 0.5 + 0.5;
                self.sparkle(self.at(*dx, *dy), 0.4 + 0.6 * twinkle);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn night_and_focus_override_the_day() {
        assert_eq!(current_scene(100.0, 2, false).0, Scene::Night);
        assert_eq!(current_scene(100.0, 23, true).0, Scene::Night);
        assert_eq!(current_scene(100.0, 14, true).0, Scene::Laptop);
    }

    #[test]
    fn the_day_starts_with_a_wave_and_cycles_every_scene() {
        assert_eq!(current_scene(0.0, 14, false).0, Scene::Wave);
        let seen: Vec<Scene> = (0..DAY.len())
            .map(|index| current_scene(index as f32 * SCENE_SECONDS + 1.0, 14, false).0)
            .collect();
        assert_eq!(seen, DAY);
    }

    #[test]
    fn repeated_reactions_wait_for_their_cooldown() {
        assert!(cooled_down(Reaction::Copied));
        assert!(!cooled_down(Reaction::Copied));
        assert!(cooled_down(Reaction::Celebrate));
        assert!(cooled_down(Reaction::Celebrate));
    }

    #[test]
    fn props_fade_at_scene_edges() {
        assert!(scene_fade(Scene::Chai, 0.0) < 0.01);
        assert!(scene_fade(Scene::Chai, SCENE_SECONDS / 2.0) > 0.99);
        assert!(scene_fade(Scene::Chai, SCENE_SECONDS) < 0.01);
    }
}
