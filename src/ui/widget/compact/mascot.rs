//! The island's robot: a cream body with a black screen for a face. In the idle notch it
//! goes about its day, sipping chai, typing, reading, watering a plant, stretching, looking
//! around, waving and napping, a scene at a time. It works on its laptop while a Focus
//! timer runs and sleeps under the moon at night. Greetings use its waving pose.
//!
//! It also notices what the user is doing (typing along in a code editor, a headset on in a
//! meeting, popcorn for a fullscreen video), has a mood that builds over the day, remembers
//! Focus streaks and milestones (see `core::mascot_memory`), and slowly unlocks accessories.

mod characters;
mod extras;

pub(crate) use characters::set_character;
pub(crate) mod life;
pub(crate) mod roam;
mod suit;

use std::cell::RefCell;
use std::f32::consts::{PI, TAU};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use winisland_render::text::{DrawTextCachedParams, FontManager};
use winisland_render::{Painter, Point, Radius, Rect, Rgba, StrokeCap, Vec2};

/// Room for the robot and the props of its current scene.
pub(super) const WIDTH: f32 = 132.0;
/// About 20 frames a second keeps the motion smooth without busy redraws.
pub(super) const FRAME: Duration = Duration::from_millis(66);
/// How long each daytime scene plays before the next one.
const SCENE_SECONDS: f32 = 14.0;
const BREATH_SECONDS: f32 = 3.2;
/// Height of the robot from its feet to the top of its head, in robot units.
const ROBOT_HEIGHT: f32 = 22.0;
const ARM_LENGTH: f32 = 5.6;

const SHELL: (u8, u8, u8) = (242, 238, 228);
/// Shell colours to choose from in Settings: cream, mint, sky, blush and lilac.
const SHELLS: [(u8, u8, u8); 5] = [
    SHELL,
    (208, 238, 218),
    (206, 224, 248),
    (248, 214, 218),
    (226, 216, 248),
];
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
    /// A sip of water between dances.
    SipWater,
    /// Headphones on, nodding along.
    Groove,
    Night,
    /// The microphone or camera is in use: a headset on, listening quietly at the laptop.
    Meeting,
    /// A fullscreen video or game is in front: leaning back with popcorn.
    Popcorn,
}

/// A prop a scene sets out, so neighbouring scenes that share one skip the walk off and on.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Prop {
    Laptop,
    Cushion,
}

impl Scene {
    fn prop(self) -> Option<Prop> {
        match self {
            Self::Laptop | Self::Meeting => Some(Prop::Laptop),
            Self::Nap | Self::Night => Some(Prop::Cushion),
            _ => None,
        }
    }
}

/// Whether `next` can follow `scene` in place, without walking off and back on.
fn shares_props(scene: Scene, next: Scene) -> bool {
    scene == next || scene.prop().is_some_and(|prop| next.prop() == Some(prop))
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

/// Caption text size, in robot units.
const CAPTION_SIZE: f32 = 8.5;

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
    /// When the next Diwali moment is due during the festive days.
    next_diwali: Option<f32>,
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
    // The playground test hook keeps the robot "with" the user so trips can be watched.
    let away = idle_seconds >= AWAY_SECONDS
        && !(demo() && std::env::var_os("WINISLAND_MASCOT_DEMO_ROAM").is_some());
    let returned =
        CONTEXT.with(|context| std::mem::replace(&mut context.borrow_mut().away, away)) && !away;
    if returned {
        react(Reaction::WelcomeBack);
    }
    nudge(away, focus_running);
    let active_seconds = CONTEXT.with(|context| context.borrow().active_since)
        .map(|since| seconds() - since);
    life::tick(focus_running, away, active_seconds);
    life::demo_reactions();
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
    /// Clicked: a happy hop.
    Jump,
    /// The internet dropped: it holds an unplugged cable, glum.
    Offline,
    /// The internet is back: Wi-Fi waves and a cheer.
    Online,
    /// The PC has been working hard for a while: it fans itself.
    HotCpu,
    /// A download finished: a parcel drops into its arms.
    Downloaded,
    /// The third Focus session of the day: it lifts a trophy.
    Trophy,
    /// The festive days around Diwali: diyas, sparklers and fireworks.
    Diwali,
    /// Holi: clouds of coloured powder.
    Holi,
    /// Christmas: a Santa hat, snow and a little tree.
    Christmas,
    /// New Year: fireworks and confetti.
    NewYear,
    /// The user's birthday: a party hat and a cake with candles.
    Birthday,
    /// A long stretch of steady work: arms up, "On a roll!".
    Cheer,
    /// Still up after 1 am: a yawn and a point at the moon.
    Bedtime,
    /// Double-clicked: a spin.
    Trick,
    /// The cursor rested on it: it looks up and waves.
    Notice,
    /// A file was parked on the Shelf: it catches a falling page.
    Catch,
    /// Focus sessions on consecutive days: a little flame.
    Streak,
    /// Once a week: last week's focus hours on a little chart.
    Recap,
    /// A clipboard milestone (100, 500, 1000 copies): a starred note and confetti.
    Milestone,
    /// The first screenshot of the day: a camera flash.
    FirstShot,
}

impl Reaction {
    /// The caption shown beside the robot, if any.
    fn caption_key(self) -> Option<&'static str> {
        match self {
            Self::Water => Some("nudge_water"),
            Self::StretchBreak => Some("nudge_stretch"),
            Self::Lunch => Some("nudge_lunch"),
            Self::WrapUp => Some("nudge_wrap_up"),
            Self::Offline => Some("reaction_offline"),
            Self::Online => Some("reaction_online"),
            Self::HotCpu => Some("reaction_hot_cpu"),
            Self::Downloaded => Some("reaction_downloaded"),
            Self::Trophy => Some("reaction_trophy"),
            Self::WelcomeBack => Some("activity_welcome_back"),
            Self::Diwali if diwali_today().is_some_and(|offset| offset > 1) => {
                Some("reaction_diwali_coming")
            }
            Self::Diwali => Some("reaction_diwali"),
            Self::Holi => Some("reaction_holi"),
            Self::Christmas => Some("reaction_christmas"),
            Self::NewYear => Some("reaction_new_year"),
            Self::Birthday => Some("reaction_birthday"),
            Self::Cheer => Some("reaction_cheer"),
            Self::Bedtime => Some("nudge_bedtime"),
            Self::Catch => Some("reaction_caught"),
            Self::Streak => Some("reaction_streak"),
            Self::Recap => Some("reaction_recap"),
            Self::Milestone => Some("reaction_milestone"),
            Self::FirstShot => Some("reaction_first_shot"),
            _ => None,
        }
    }

    fn seconds(self) -> f32 {
        match self {
            Self::MouseArrived => 6.0,
            Self::Celebrate => 4.5,
            Self::WelcomeBack => 4.5,
            Self::Water | Self::StretchBreak | Self::Lunch | Self::WrapUp => 6.0,
            Self::Jump => 1.0,
            Self::Offline | Self::Downloaded => 4.0,
            Self::Online => 3.5,
            Self::HotCpu | Self::Trophy => 5.0,
            Self::Diwali | Self::Holi | Self::Christmas | Self::NewYear | Self::Birthday => 9.0,
            Self::Charging | Self::Drive => 4.0,
            Self::Copied => 3.0,
            Self::Surprised => 2.6,
            Self::Cheer => 4.0,
            Self::Bedtime => 5.0,
            Self::Trick => 1.6,
            Self::Notice => 2.4,
            Self::Catch => 3.0,
            Self::Streak | Self::Recap | Self::Milestone => 5.0,
            Self::FirstShot => 3.5,
        }
    }
}

/// The caption for `key`, with the current reaction's number (a streak, hours, a count)
/// filled in where the text has a `{0}`.
fn caption_text(key: &str) -> String {
    winisland_core::i18n::tr_args(key, &[&life::caption_arg()])
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
            Self::Jump => 1.2,
            Self::Offline | Self::Online => 60.0,
            Self::HotCpu => 600.0,
            Self::Downloaded => 20.0,
            Self::Trophy
            | Self::Diwali
            | Self::Holi
            | Self::Christmas
            | Self::NewYear
            | Self::Birthday => 0.0,
            Self::Celebrate => 0.0,
            Self::Cheer => 1800.0,
            Self::Bedtime => 3600.0,
            Self::Trick => 1.5,
            Self::Notice => 30.0,
            Self::Catch => 3.0,
            Self::Streak | Self::Recap | Self::Milestone | Self::FirstShot => 0.0,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

thread_local! {
    static LAST_PLAYED: std::cell::Cell<[Option<f32>; ALL_REACTIONS.len()]> =
        const { std::cell::Cell::new([None; ALL_REACTIONS.len()]) };
    /// A reaction to play once the current one ends, such as a streak after a celebration.
    static QUEUED: std::cell::Cell<Option<Reaction>> = const { std::cell::Cell::new(None) };
}

/// Plays `reaction` right after the reaction that is about to start or is playing.
fn queue_after(reaction: Reaction) {
    QUEUED.with(|cell| cell.set(Some(reaction)));
}

/// Plays `reaction` after the current one, or now if nothing is playing.
fn queue(reaction: Reaction) {
    if REACTION.with(std::cell::Cell::get).is_some() {
        QUEUED.with(|cell| cell.set(Some(reaction)));
    } else {
        react(reaction);
    }
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
    // Remembering (breaks, Focus sessions, copies) happens whether or not it shows; a
    // milestone can take the reaction's place.
    let reaction = life::on_reaction(reaction);
    if !allowed(reaction) {
        return;
    }
    // In a meeting the robot keeps quiet: only things the user should know about show.
    if life::activity() == life::Activity::Meeting
        && matches!(
            reaction,
            Reaction::Cheer
                | Reaction::Bedtime
                | Reaction::Water
                | Reaction::StretchBreak
                | Reaction::Lunch
                | Reaction::WrapUp
                | Reaction::Diwali
                | Reaction::Holi
                | Reaction::Christmas
                | Reaction::NewYear
                | Reaction::Birthday
        )
    {
        return;
    }
    if !cooled_down(reaction) {
        return;
    }
    // Things the user should notice each get their own sound; the robot's own moments
    // (being clicked, greetings, festivals) stay silent.
    use crate::core::chime::{Sound, play};
    let sound = match reaction {
        Reaction::MouseArrived | Reaction::Drive => Some(Sound::DeviceIn),
        Reaction::Charging => Some(Sound::PowerOn),
        Reaction::Surprised => Some(Sound::PowerOff),
        Reaction::Copied => Some(Sound::Copy),
        Reaction::Downloaded => Some(Sound::Download),
        Reaction::Offline => Some(Sound::Offline),
        Reaction::Online => Some(Sound::Online),
        Reaction::HotCpu => Some(Sound::Warning),
        Reaction::Celebrate => Some(Sound::Done),
        Reaction::Trophy => Some(Sound::Fanfare),
        Reaction::Water | Reaction::StretchBreak | Reaction::Lunch | Reaction::WrapUp => {
            Some(Sound::Reminder)
        }
        Reaction::Bedtime => Some(Sound::Reminder),
        Reaction::Streak | Reaction::Milestone | Reaction::Recap => Some(Sound::Done),
        Reaction::Catch => Some(Sound::Copy),
        Reaction::Jump
        | Reaction::Trick
        | Reaction::Notice
        | Reaction::Cheer
        | Reaction::FirstShot
        | Reaction::WelcomeBack
        | Reaction::Diwali
        | Reaction::Holi
        | Reaction::Christmas
        | Reaction::NewYear
        | Reaction::Birthday => None,
    };
    if let Some(sound) = sound {
        play(sound);
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
            // The next queued reaction, if any, starts as this one ends.
            cell.set(QUEUED.with(std::cell::Cell::take).map(|next| (next, None)));
            return None;
        }
        Some((reaction, elapsed))
    })
}

thread_local! {
    /// The last seen connection state, when CPU load first ran high, and today's Focus
    /// sessions as `(day, count)`.
    static SYSTEM: std::cell::Cell<(Option<bool>, Option<f32>, (u16, u32))> =
        const { std::cell::Cell::new((None, None, (0, 0))) };
}

/// Reacts to the connection dropping or returning and to the CPU staying above 85% for
/// 20 seconds. Call each tick with the latest system sample.
pub(crate) fn set_system(online: bool, cpu_percent: f32) {
    let now = seconds();
    let (last_online, hot_since, streak) = SYSTEM.with(std::cell::Cell::get);
    match last_online {
        Some(true) if !online => react(Reaction::Offline),
        Some(false) if online => react(Reaction::Online),
        _ => {}
    }
    let hot_since = if cpu_percent > 85.0 {
        let since = hot_since.unwrap_or(now);
        if now - since >= 20.0 {
            react(Reaction::HotCpu);
        }
        Some(since)
    } else if cpu_percent < 70.0 {
        None
    } else {
        hot_since
    };
    SYSTEM.with(|cell| cell.set((Some(online), hot_since, streak)));
}

/// A Focus session finished: confetti, or a trophy for the third one today.
pub(crate) fn focus_finished() {
    let today = crate::platform::shell().local_datetime().day;
    let count = SYSTEM.with(|cell| {
        let (online, hot, (day, count)) = cell.get();
        let count = if day == today { count + 1 } else { 1 };
        cell.set((online, hot, (today, count)));
        count
    });
    react(if count == 3 {
        Reaction::Trophy
    } else {
        Reaction::Celebrate
    });
}

/// Days from 1970-01-01 to a Gregorian date.
fn days_from_civil(year: i32, month: u32, day: u32) -> i32 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = month as i32;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day as i32 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Lakshmi Puja dates.
const DIWALI: [(i32, u32, u32); 3] = [(2026, 11, 8), (2027, 10, 29), (2028, 10, 17)];

/// Days until Diwali (negative after it) during the festive window, 10 days before to 3
/// days after.
fn diwali_offset(year: i32, month: u32, day: u32) -> Option<i32> {
    let today = days_from_civil(year, month, day);
    DIWALI
        .iter()
        .map(|&(y, m, d)| days_from_civil(y, m, d) - today)
        .find(|offset| (-3..=10).contains(offset))
}

fn diwali_today() -> Option<i32> {
    let clock = crate::platform::shell().local_datetime();
    diwali_offset(
        i32::from(clock.year),
        u32::from(clock.month),
        u32::from(clock.day),
    )
}

#[derive(Clone, Copy)]
struct Preferences {
    /// 0 calm, 1 normal, 2 playful.
    liveliness: u8,
    festive: bool,
    nudges: bool,
    pc_reactions: bool,
    /// Month and day.
    birthday: Option<(u32, u32)>,
}

thread_local! {
    static PREFERENCES: std::cell::Cell<Preferences> = const {
        std::cell::Cell::new(Preferences {
            liveliness: 1,
            festive: true,
            nudges: true,
            pc_reactions: true,
            birthday: None,
        })
    };
}

/// Applies the Robot settings. `birthday` is `MM-DD` or empty. Call each tick.
pub(crate) fn set_preferences(
    liveliness: u8,
    festive: bool,
    nudges: bool,
    pc_reactions: bool,
    birthday: &str,
) {
    let birthday = birthday
        .split_once('-')
        .and_then(|(month, day)| Some((month.parse().ok()?, day.parse().ok()?)));
    PREFERENCES.with(|cell| {
        cell.set(Preferences {
            liveliness: liveliness.min(2),
            festive,
            nudges,
            pc_reactions,
            birthday,
        });
    });
}

fn preferences() -> Preferences {
    PREFERENCES.with(std::cell::Cell::get)
}

/// How long each scene plays: longer when calm, shorter when playful.
fn scene_seconds() -> f32 {
    if demo() {
        return 5.0;
    }
    [22.0, SCENE_SECONDS, 9.0][usize::from(preferences().liveliness)]
}

/// Stretches or shortens the gaps between nudges and festive moments with liveliness.
fn pace() -> f32 {
    [1.5, 1.0, 0.75][usize::from(preferences().liveliness)]
}

/// Whether the Robot settings allow `reaction`'s group.
fn allowed(reaction: Reaction) -> bool {
    let preferences = preferences();
    match reaction {
        Reaction::Water | Reaction::StretchBreak | Reaction::Lunch | Reaction::WrapUp => {
            preferences.nudges
        }
        Reaction::MouseArrived
        | Reaction::Charging
        | Reaction::Surprised
        | Reaction::Copied
        | Reaction::Drive
        | Reaction::Offline
        | Reaction::Online
        | Reaction::HotCpu
        | Reaction::Downloaded => preferences.pc_reactions,
        Reaction::Diwali
        | Reaction::Holi
        | Reaction::Christmas
        | Reaction::NewYear
        | Reaction::Birthday => preferences.festive,
        Reaction::Cheer | Reaction::Bedtime => preferences.nudges,
        Reaction::Catch | Reaction::FirstShot | Reaction::Milestone => preferences.pc_reactions,
        Reaction::Celebrate
        | Reaction::Trophy
        | Reaction::WelcomeBack
        | Reaction::Jump
        | Reaction::Trick
        | Reaction::Notice
        | Reaction::Streak
        | Reaction::Recap => true,
    }
}

/// Every reaction, in the order the Settings preview plays them.
const ALL_REACTIONS: [Reaction; 31] = [
    Reaction::MouseArrived,
    Reaction::Charging,
    Reaction::Surprised,
    Reaction::Copied,
    Reaction::Drive,
    Reaction::Celebrate,
    Reaction::WelcomeBack,
    Reaction::Water,
    Reaction::StretchBreak,
    Reaction::Lunch,
    Reaction::WrapUp,
    Reaction::Jump,
    Reaction::Offline,
    Reaction::Online,
    Reaction::HotCpu,
    Reaction::Downloaded,
    Reaction::Trophy,
    Reaction::Diwali,
    Reaction::Holi,
    Reaction::Christmas,
    Reaction::NewYear,
    Reaction::Birthday,
    Reaction::Cheer,
    Reaction::Bedtime,
    Reaction::Trick,
    Reaction::Notice,
    Reaction::Catch,
    Reaction::Streak,
    Reaction::Recap,
    Reaction::Milestone,
    Reaction::FirstShot,
];

/// Plays the next reaction in turn right away, ignoring settings and cooldowns.
pub(crate) fn preview_next() {
    thread_local! {
        static NEXT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }
    let index = NEXT.with(|next| {
        let index = next.get();
        next.set((index + 1) % ALL_REACTIONS.len());
        index
    });
    REACTION.with(|cell| cell.set(Some((ALL_REACTIONS[index], None))));
}

/// Holi dates.
const HOLI: [(i32, u32, u32); 2] = [(2027, 3, 22), (2028, 3, 11)];

/// The festival being celebrated today, if any, and days until its main day (negative
/// after). A birthday wins on the day itself.
fn festival_today() -> Option<(Reaction, i32)> {
    let clock = crate::platform::shell().local_datetime();
    let (year, month, day) = (
        i32::from(clock.year),
        u32::from(clock.month),
        u32::from(clock.day),
    );
    let today = days_from_civil(year, month, day);
    if preferences().birthday == Some((month, day)) {
        return Some((Reaction::Birthday, 0));
    }
    if let Some(offset) = diwali_offset(year, month, day) {
        return Some((Reaction::Diwali, offset));
    }
    let around = |dates: &[(i32, u32, u32)], before: i32, after: i32| {
        dates
            .iter()
            .map(|&(y, m, d)| days_from_civil(y, m, d) - today)
            .find(|offset| (-after..=before).contains(offset))
    };
    if let Some(offset) = around(&HOLI, 2, 1) {
        return Some((Reaction::Holi, offset));
    }
    if let Some(offset) = around(&[(year, 12, 25)], 5, 1) {
        return Some((Reaction::Christmas, offset));
    }
    if let Some(offset) = around(&[(year, 12, 31), (year - 1, 12, 31)], 0, 1) {
        return Some((Reaction::NewYear, offset));
    }
    None
}

thread_local! {
    /// When the robot last came back into view after a card, for its walk back in, the
    /// last frame's opacity, and whether the current scene is slow enough for fewer frames.
    static VIEW: std::cell::Cell<(Option<f32>, u8, bool)> =
        const { std::cell::Cell::new((None, 255, false)) };
}

/// The robot walks back in from the side for the first moments after reappearing.
const WALK_IN_SECONDS: f32 = 0.9;

/// Notes this frame's opacity and returns the walk-in offset in robot units, 0 once in place.
fn walk_in(time: f32, alpha: u8) -> f32 {
    VIEW.with(|cell| {
        let (mut since, last_alpha, slow) = cell.get();
        if last_alpha < 80 && alpha > 200 {
            since = Some(time);
        }
        cell.set((since, alpha, slow));
        since.map_or(0.0, |start| {
            -(1.0 - ease((time - start) / WALK_IN_SECONDS)) * 34.0
        })
    })
}

fn set_slow(slow: bool) {
    VIEW.with(|cell| {
        let (since, alpha, _) = cell.get();
        cell.set((since, alpha, slow));
    });
}

/// How soon to draw the robot again: fewer frames for slow, quiet scenes.
pub(super) fn frame_interval() -> Duration {
    if VIEW.with(std::cell::Cell::get).2 {
        Duration::from_millis(140)
    } else {
        FRAME
    }
}

/// Water every hour of continuous activity (first after half an hour), a stretch every 55
/// minutes, lunch around 1 pm and a wrap-up around 6 pm. Never during a Focus session.
fn nudge(away: bool, focus_running: bool) {
    let water_every = 3600.0 * pace();
    let stretch_every = 3300.0 * pace();
    let now = seconds();
    let due = CONTEXT.with(|context| {
        let mut context = context.borrow_mut();
        if away {
            context.active_since = None;
            return None;
        }
        if context.active_since.is_none() {
            context.active_since = Some(now);
            context.next_water = now + water_every / 2.0;
            context.next_stretch = now + stretch_every;
        }
        if focus_running {
            return None;
        }
        if now >= context.next_water {
            context.next_water = now + water_every;
            return Some(Reaction::Water);
        }
        if now >= context.next_stretch {
            context.next_stretch = now + stretch_every;
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
    // Festive moments every 20 minutes, every 10 on Diwali itself and the day either side,
    // the first a minute after starting.
    if let Some((festival, offset)) = festival_today()
        && !away
        && !focus_running
    {
        let interval = if offset.abs() <= 1 { 600.0 } else { 1200.0 } * pace();
        let fire = CONTEXT.with(|context| {
            let mut context = context.borrow_mut();
            let next = *context.next_diwali.get_or_insert(now + 60.0);
            let fire = now >= next;
            if fire {
                context.next_diwali = Some(now + interval);
            }
            fire
        });
        if fire {
            react(festival);
        }
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

/// While music plays, dancing is the main thing: dance, a sip of water, dance again, then
/// groove with headphones, round and round. Returns the scene, seconds into it and its
/// length.
fn music_scene(elapsed: f32) -> (Scene, f32, f32) {
    const ROUTINE: [(Scene, f32); 4] = [
        (Scene::Dance, 18.0),
        (Scene::SipWater, 6.0),
        (Scene::Dance, 18.0),
        (Scene::Groove, 10.0),
    ];
    let total: f32 = ROUTINE.iter().map(|(_, length)| length).sum();
    let mut at = elapsed % total;
    for (scene, length) in ROUTINE {
        if at < length {
            return (scene, at, length);
        }
        at -= length;
    }
    (Scene::Dance, 0.0, 18.0)
}

/// How long the robot dances, with the waveform beside it, when a song starts. After
/// that the music steps aside and the robot goes back to its day.
const MUSIC_DANCE_SECONDS: f32 = 18.0;

/// Whether the robot is dancing to a song that just started, so the waveform shows.
pub(crate) fn music_spotlight() -> bool {
    dancing(seconds()).is_some()
}

/// Seconds since the current track started, during its opening dance.
fn dancing(time: f32) -> Option<f32> {
    CONTEXT.with(|context| {
        context
            .borrow()
            .dance_started
            .map(|start| (time - start).max(0.0))
            .filter(|elapsed| *elapsed < MUSIC_DANCE_SECONDS)
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
    if !(5..23).contains(&hour) && !demo() {
        return (Scene::Night, time);
    }
    if focus_running {
        return (Scene::Laptop, time);
    }
    let index = (time / scene_seconds()) as usize;
    (DAY[index % DAY.len()], time % scene_seconds())
}

/// The scene for `time`, after what the user is doing and the robot's mood have had their
/// say: a meeting or a fullscreen video takes over, an editor means typing along, and the
/// mood swaps some daytime scenes for others.
fn day_scene(time: f32, hour: u16, focus_running: bool) -> (Scene, f32) {
    let (scene, elapsed) = current_scene(time, hour, focus_running);
    let scene = match life::activity() {
        life::Activity::Meeting => Scene::Meeting,
        life::Activity::Fullscreen => Scene::Popcorn,
        life::Activity::Editor => Scene::Laptop,
        life::Activity::Idle if scene == Scene::Night => scene,
        life::Activity::Idle => life::mood_scene(scene, life::mood()),
    };
    (scene, elapsed)
}

fn greeting(name: &str) -> String {
    if name.is_empty() {
        "Hi".to_string()
    } else {
        format!("Hi, {name}")
    }
}

/// How long the robot takes to walk on with a new scene, and off at its end.
const ENTER_SECONDS: f32 = 1.3;
const EXIT_SECONDS: f32 = 1.1;
/// Walk distance in robot units when the notch's edges are unknown.
const STROLL: f32 = 75.0;
/// Extra room past the notch edge so nothing peeks in, in robot units.
const CLEARANCE: f32 = 4.0;

thread_local! {
    /// The notch's left and right edges, set each frame before the robot draws.
    static NOTCH_SPAN: std::cell::Cell<Option<(f32, f32)>> = const { std::cell::Cell::new(None) };
}

/// Records the notch's current left and right edges, so the robot walks fully past them.
pub(crate) fn set_notch_span(left: f32, right: f32) {
    NOTCH_SPAN.with(|cell| cell.set(Some((left, right))));
}

/// How far, in robot units, the robot standing at `x` must walk left (to enter from) and
/// right (to leave by) so it and its props are wholly outside the notch. Props reach
/// `left_reach` units to its left and `right_reach` to its right.
fn walk_distances(x: f32, unit: f32, left_reach: f32, right_reach: f32) -> (f32, f32) {
    NOTCH_SPAN.with(std::cell::Cell::get).map_or((STROLL, STROLL), |(left, right)| {
        let unit = unit.max(0.01);
        (
            ((x - left) / unit + right_reach + CLEARANCE).max(0.0),
            ((right - x) / unit + left_reach + CLEARANCE).max(0.0),
        )
    })
}

/// Test hook: with `WINISLAND_MASCOT_DEMO=1` scenes change every few seconds, day or night,
/// so transitions can be watched without waiting.
fn demo() -> bool {
    static DEMO: OnceLock<bool> = OnceLock::new();
    *DEMO.get_or_init(|| std::env::var_os("WINISLAND_MASCOT_DEMO").is_some_and(|v| v == "1"))
}

/// A little hop per step while the robot is walking on or off, in robot units.
fn step_bob(shift: f32, time: f32) -> f32 {
    if shift.abs() < 0.5 {
        return 0.0;
    }
    (time * 2.6 * TAU).sin().abs() * 1.2
}

/// Sideways offset in robot units at `elapsed` seconds into a scene: the robot carries the
/// scene's props in from beyond the notch's left edge (`from` units away), then out past
/// its right edge (`to` units), so it is wholly outside the notch at every scene change.
fn scene_shift(scene: Scene, elapsed: f32, length: f32, (from, to): (f32, f32)) -> f32 {
    if matches!(scene, Scene::Night) {
        return 0.0;
    }
    // Sleepy walks are slow, happy ones brisk.
    let pace = life::walk_pace(life::mood());
    let enter = 1.0 - ease(elapsed / (ENTER_SECONDS * pace));
    let exit = 1.0 - ease((length - elapsed) / (EXIT_SECONDS * pace));
    // Ease in harder on the way out so it sets off gently.
    -enter * from + exit * exit * to
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
    /// A headset over the head, for meetings.
    headset: bool,
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
            headset: false,
        }
    }
}

/// Where the robot's hands ended up, for props it holds.
struct Hands {
    right: Point,
    left: Point,
}

/// A blink every few seconds turns open eyes shut for a moment; sleepy eyes blink slower
/// and more often.
fn blink(eyes: Eyes, time: f32) -> Eyes {
    let (period, length) = life::blink_timing(life::mood());
    match eyes {
        Eyes::Open(..) if (time % period) < length => Eyes::Closed,
        other => other,
    }
}

/// The idle notch: the robot and its current scene.
pub(super) fn draw(painter: Painter<'_>, rect: Rect, scale: f32, alpha: u8) {
    let time = seconds();
    let walk = walk_in(time, alpha);
    let hour = crate::platform::shell().local_datetime().hour;
    let (name, focus_running) = CONTEXT.with(|context| {
        let context = context.borrow();
        (context.name.clone(), context.focus_running)
    });
    let away = CONTEXT.with(|context| context.borrow().away);
    // Neighbouring scenes, to skip the walk when they share the props (or are the same
    // scene, as when the user stays in a meeting or an editor).
    let (mut before, mut after) = (None, None);
    let (scene, elapsed, length) = match dancing(time) {
        _ if away => (Scene::Nap, scene_seconds() / 2.0, scene_seconds()),
        Some(elapsed) => music_scene(elapsed),
        None => {
            let (scene, elapsed) = day_scene(time, hour, focus_running);
            before = Some(day_scene(time - elapsed - 0.01, hour, focus_running).0);
            after = Some(day_scene(time - elapsed + scene_seconds() + 0.01, hour, focus_running).0);
            (scene, elapsed, scene_seconds())
        }
    };
    let evening = !(5..18).contains(&hour);
    // The robot stands the full height of the notch, so it grows with a taller notch.
    let unit = (rect.height() / 23.0).clamp(0.7 * scale, 3.0 * scale);
    // Out playing on the taskbar: the notch waits, and so do reactions.
    if roam::away_from_notch() {
        set_slow(true);
        return;
    }
    if let Some((reaction, elapsed)) = active_reaction(time, alpha) {
        set_slow(false);
        // Centre the robot and its caption together.
        let caption = reaction.caption_key().map_or(0.0, |key| {
            11.0 * unit
                + FontManager::global().measure_text_cached(
                    &caption_text(key),
                    CAPTION_SIZE * unit,
                    winisland_render::FontStyle::bold(),
                )
        });
        let x = if caption > 0.0 {
            (rect.center_x() - (caption - 9.0 * unit) / 2.0).max(rect.left + 10.0 * unit)
        } else {
            rect.center_x() - 4.0 * unit
        };
        // It walks on with the reaction and off again, like a scene.
        // Reactions reach about 9 units left of the robot and up to their caption right.
        let distances = walk_distances(x, unit, 9.0, caption / unit + 12.0);
        let shift = scene_shift(Scene::Wave, elapsed, reaction.seconds(), distances);
        let stage = Stage {
            painter,
            unit,
            alpha,
            x: x + shift * unit,
            floor: rect.center_y() + (11.0 - step_bob(shift, time)) * unit,
            mid: rect.center_y(),
            time,
            elapsed,
            // Props are carried on and off, so they stay solid.
            fade: 1.0,
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
        Scene::Nap => 26.0,
        Scene::Night => 38.0,
        Scene::Paint => 31.0,
        Scene::Dance => 13.0,
        Scene::SipWater | Scene::Groove => 13.0,
        Scene::Meeting => 26.0,
        Scene::Popcorn => 14.0,
    };
    let left_reach = 10.0;
    // Wide scenes shrink just enough to fit the slot; the rest use the full height.
    let unit = unit.min(rect.width() / (left_reach + reach + 2.0)).max(0.5 * scale);
    let offset = ((rect.width() / unit - (left_reach + reach)) / 2.0).max(1.0);
    let home = rect.left + (offset + left_reach) * unit;
    let shift = if walk == 0.0 {
        let (from, to) = walk_distances(home, unit, left_reach, reach);
        // A neighbour that shares the props takes over in place, without a walk.
        let from = if before.is_some_and(|before| shares_props(before, scene)) {
            0.0
        } else {
            from
        };
        // Off to play: it always walks out at the end of this scene.
        let leaving = roam::wants_to_leave();
        let to = if !leaving && after.is_some_and(|after| shares_props(scene, after)) {
            0.0
        } else {
            to
        };
        if leaving {
            roam::note_scene_time(elapsed);
        }
        match roam::since_return() {
            // Back from playing along the top edge: it walks in from the right.
            Some(since) if since < ENTER_SECONDS => {
                (1.0 - ease(since / ENTER_SECONDS)) * walk_distances(home, unit, left_reach, reach).1
            }
            _ => scene_shift(scene, elapsed, length, (from, to)),
        }
    } else {
        0.0
    };
    let stage = Stage {
        painter,
        unit,
        alpha,
        // After a card it walks back in from the side, carrying its scene; at each scene
        // change it carries the old props out and the new ones in.
        x: home + (walk + shift) * unit,
        floor: rect.center_y() + (11.0 - step_bob(shift, time)) * unit,
        mid: rect.center_y(),
        time,
        elapsed,
        fade: 1.0,
    };
    set_slow(
        walk == 0.0
            && shift == 0.0
            && matches!(
                scene,
                Scene::Nap | Scene::Night | Scene::Read | Scene::LookAround
            ),
    );
    match scene {
        Scene::Wave => stage.wave(&name),
        Scene::Chai => stage.chai(),
        Scene::Laptop => stage.laptop(),
        Scene::Read => stage.read(),
        Scene::Plant => stage.plant(),
        Scene::Stretch => stage.stretch(),
        Scene::LookAround => stage.look_around(),
        Scene::Nap => stage.sleep(evening),
        Scene::Night => stage.sleep(true),
        Scene::Meeting => stage.meeting(),
        Scene::Popcorn => stage.popcorn(),
        Scene::Paint => stage.paint(),
        Scene::Dance => stage.dance(),
        Scene::SipWater => stage.sip_water(),
        Scene::Groove => stage.groove(),
    }
}

/// Character `index` (as in Settings) waving, standing in `rect`, for previews. The notch's
/// own character is left as it was.
pub(crate) fn draw_character_preview(painter: Painter<'_>, rect: Rect, index: u8) {
    let unit = rect.height() / (ROBOT_HEIGHT + 12.0);
    let time = seconds();
    let stage = Stage {
        painter,
        unit,
        alpha: 255,
        x: rect.center_x(),
        floor: rect.bottom - 3.0 * unit,
        mid: rect.center_y(),
        time,
        elapsed: time,
        fade: 1.0,
    };
    characters::with_character(index, || {
        let mut pose = Pose::new(Eyes::Happy);
        pose.right_arm = -0.45 + (time / 0.9 * TAU).sin() * 0.45;
        pose.rise = (time * 2.0).sin().abs() * 0.6;
        stage.robot(pose);
    });
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
        // Another character chosen: it draws the same pose in its own body.
        if let Some(hands) = self.creature(pose) {
            return hands;
        }
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
        // The shell colour chosen in Settings; the face stays cream-white.
        let shell = SHELLS[life::colour().min(SHELLS.len() - 1)];
        self.line(right_shoulder, right, 2.4, self.color(1.0, shell));
        self.line(left_shoulder, left, 2.4, self.color(1.0, shell));
        painter.fill_round_rect(body, Radius::uniform(2.4 * unit), self.color(1.0, shell));

        // Head: a cream shell around a black screen, with a thicker chin like a monitor.
        let head = Rect::from_xywh(
            self.x - 8.0 * unit,
            body.top - 13.0 * unit - lift * 0.5,
            16.0 * unit,
            13.6 * unit,
        );
        self.antenna(head, shell);
        painter.fill_round_rect(head, Radius::uniform(3.8 * unit), self.color(1.0, shell));
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
                            eye_x + (dx - 1.15) * unit,
                            eye_y + (dy - 2.0) * unit,
                            2.3 * unit,
                            3.9 * unit,
                        ),
                        Radius::uniform(1.15 * unit),
                        self.color(1.0, SHELL),
                    );
                }
            }
        }
        if pose.headset {
            // A band over the top of the head and ear cups at its sides, a mic at the mouth.
            let band = self.color(1.0, (60, 60, 70));
            let top = Point::new(head.center_x(), head.top - 1.0 * unit);
            for side in [-1.0, 1.0] {
                let ear = Point::new(head.center_x() + side * 8.6 * unit, head.center_y());
                self.line(top, Point::new(ear.x, ear.y - 2.0 * unit), 0.8, band);
                painter.fill_round_rect(
                    Rect::from_xywh(ear.x - 1.4 * unit, ear.y - 2.2 * unit, 2.8 * unit, 4.4 * unit),
                    Radius::uniform(1.2 * unit),
                    band,
                );
            }
            self.line(
                Point::new(head.center_x() + 8.6 * unit, head.center_y() + 1.6 * unit),
                Point::new(head.center_x() + 4.0 * unit, head.bottom - 1.2 * unit),
                0.5,
                band,
            );
        }
        self.wear(head, screen, body, eye_y);
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
        // The plant grows through the scene, and a little taller for every day the robot has
        // been looked after, flowering once fully grown.
        let stage = crate::core::mascot_memory::plant_stage(life::active_days());
        let growth = 0.5 + 0.5 * ease(self.elapsed / (scene_seconds() * 0.7));
        let height = 6.0 + f32::from(stage) * 0.8;
        let stem_base = self.at(22.0, -4.4);
        let stem_top = self.at(22.0, -4.4 - height * growth);
        if stage >= 5 {
            for petal in 0..5 {
                let angle = petal as f32 * TAU / 5.0 + self.time * 0.3;
                self.painter.fill_circle(
                    Point::new(
                        stem_top.x + angle.cos() * 1.1 * self.unit,
                        stem_top.y + angle.sin() * 1.1 * self.unit,
                    ),
                    0.8 * self.unit,
                    self.prop(growth, (255, 150, 190)),
                );
            }
            self.painter
                .fill_circle(stem_top, 0.7 * self.unit, self.prop(growth, (255, 214, 90)));
        }
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
        let progress = ease(self.elapsed / (scene_seconds() * 0.8));
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

    /// Sips from a glass every couple of seconds.
    fn sip_water(&self) {
        let u = self.unit;
        let t = self.elapsed;
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
    }

    /// Notes rising on either side, in turn.
    fn notes(&self) {
        let colors = [(130, 220, 255), (255, 160, 210), (255, 220, 120)];
        for index in 0..3 {
            let phase = (self.time / 1.6 + index as f32 / 3.0).fract();
            let side = if index % 2 == 0 { 1.0 } else { -1.0 };
            let x = self.x + side * (10.0 + phase * 2.0) * self.unit
                - if side < 0.0 { 5.0 * self.unit } else { 0.0 };
            self.text(
                if index == 1 { "♫" } else { "♪" },
                x,
                self.floor - (6.0 + phase * 12.0) * self.unit,
                7.5,
                self.prop((phase * PI).sin(), colors[index]),
            );
        }
    }

    /// Headphones on, eyes shut, nodding to the beat.
    fn groove(&self) {
        let u = self.unit;
        let nod = (self.time * 2.0 * TAU).sin();
        let mut pose = Pose::new(Eyes::Happy);
        pose.lean = nod * 5.0;
        pose.rise = nod.abs() * 0.5;
        pose.right_arm = 0.9 + nod * 0.2;
        pose.left_arm = 0.9 - nod * 0.2;
        self.robot(pose);
        let save = self.painter.save();
        let pivot = self.at(0.0, 0.0);
        self.painter.translate(Vec2::new(pivot.x, pivot.y));
        self.painter.rotate_degrees(pose.lean);
        self.painter.translate(Vec2::new(-pivot.x, -pivot.y));
        let band = self.at(0.0, -14.5);
        let clip = self.painter.save();
        self.painter.clip_rect(Rect::from_ltrb(
            band.x - 10.0 * u,
            band.y - 10.0 * u,
            band.x + 10.0 * u,
            band.y,
        ));
        self.painter
            .stroke_circle(band, 8.6 * u, 1.2 * u, self.prop(1.0, (60, 60, 70)));
        self.painter.restore_to(clip);
        for side in [-1.0, 1.0] {
            self.painter.fill_round_rect(
                Rect::from_xywh(
                    band.x + side * 8.6 * u - 1.6 * u,
                    band.y - 1.4 * u,
                    3.2 * u,
                    4.6 * u,
                ),
                Radius::uniform(1.2 * u),
                self.prop(1.0, (232, 104, 96)),
            );
        }
        self.painter.restore_to(save);
        self.notes();
    }

    fn dance(&self) {
        // Three moves, six seconds each: arm swaps with a bounce, a side-step with claps,
        // and arms up swaying side to side.
        let beat = (self.time * 2.0 * TAU).sin();
        let step = (self.time * TAU).sin();
        let mut pose = Pose::new(Eyes::Happy);
        let mut dx = 0.0;
        match ((self.time / 6.0) as usize) % 3 {
            0 => {
                pose.right_arm = -0.9 + beat * 0.8;
                pose.left_arm = -0.9 - beat * 0.8;
                pose.rise = beat.abs() * 1.4;
                pose.lean = step * 8.0;
            }
            1 => {
                dx = step * 3.0;
                let clap = beat.abs();
                pose.right_arm = 0.2 + clap * 0.9;
                pose.left_arm = pose.right_arm;
                pose.lean = -step * 6.0;
                pose.rise = clap * 0.8;
            }
            _ => {
                pose.right_arm = -1.6 + beat * 0.25;
                pose.left_arm = -1.6 - beat * 0.25;
                pose.lean = step * 12.0;
                pose.rise = (beat * 0.5 + 0.5) * 1.6;
            }
        }
        self.shifted(dx).robot(pose);
        self.notes();
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
            &caption_text(key),
            self.x + 11.0 * self.unit,
            self.mid + 3.0 * self.unit,
            CAPTION_SIZE,
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
                self.sip_water();
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
            Reaction::Jump => {
                let mut pose = Pose::new(Eyes::Happy);
                pose.rise = (t / 1.0 * PI).sin() * 4.0;
                pose.right_arm = -1.2;
                pose.left_arm = -1.2;
                self.robot(pose);
            }
            Reaction::Offline => {
                // Holds up an unplugged cable, glum.
                let mut pose = Pose::new(Eyes::Closed);
                pose.lean = -4.0;
                pose.right_arm = 0.2;
                let hands = self.robot(pose);
                let grey = self.prop(1.0, (200, 200, 210));
                let plug = Rect::from_xywh(
                    hands.right.x + 1.0 * u,
                    hands.right.y - 1.2 * u,
                    2.6 * u,
                    2.4 * u,
                );
                self.line(
                    Point::new(hands.right.x - 2.0 * u, hands.right.y + 3.0 * u),
                    hands.right,
                    0.6,
                    grey,
                );
                self.painter
                    .fill_round_rect(plug, Radius::uniform(0.5 * u), grey);
                for pin in [-0.5, 0.5] {
                    self.line(
                        Point::new(plug.right, plug.center_y() + pin * u),
                        Point::new(plug.right + 1.0 * u, plug.center_y() + pin * u),
                        0.4,
                        grey,
                    );
                }
                self.caption("reaction_offline", t);
            }
            Reaction::Online => {
                let mut pose = Pose::new(Eyes::Happy);
                pose.right_arm = -1.4;
                pose.rise = (t * 5.0).sin().abs() * 0.8;
                self.robot(pose);
                // Wi-Fi arcs above the raised hand, lighting up one by one.
                let center = self.at(9.0, -15.0);
                let save = self.painter.save();
                self.painter.clip_rect(Rect::from_ltrb(
                    center.x - 6.0 * u,
                    center.y - 6.0 * u,
                    center.x + 6.0 * u,
                    center.y,
                ));
                for ring in 0..3 {
                    let lit = ease((t * 2.0 - ring as f32 * 0.4) / 0.3);
                    self.painter.stroke_circle(
                        center,
                        (1.6 + ring as f32 * 1.6) * u,
                        0.7 * u,
                        self.prop(lit, (120, 210, 140)),
                    );
                }
                self.painter.restore_to(save);
                self.caption("reaction_online", t);
            }
            Reaction::HotCpu => {
                // Fans itself with a hand fan, sweating.
                let mut pose = Pose::new(Eyes::Closed);
                pose.right_arm = -0.9 + (t * 12.0).sin() * 0.35;
                let hands = self.robot(pose);
                self.painter.fill_oval(
                    Rect::from_xywh(
                        hands.right.x - 1.6 * u,
                        hands.right.y - 3.8 * u,
                        3.2 * u,
                        4.0 * u,
                    ),
                    self.prop(1.0, (255, 170, 120)),
                );
                for index in 0..2 {
                    let phase = (t * 1.2 + index as f32 * 0.5).fract();
                    self.painter.fill_circle(
                        self.at(-9.0 + index as f32 * 1.4, -17.0 + phase * 6.0),
                        0.7 * u,
                        self.prop(1.0 - phase, (130, 190, 255)),
                    );
                }
                self.caption("reaction_hot_cpu", t);
            }
            Reaction::Downloaded => {
                let caught = t > 0.9;
                let mut pose = Pose::new(if caught {
                    Eyes::Happy
                } else {
                    Eyes::Open(0.0, -1.2)
                });
                pose.right_arm = 0.1;
                pose.left_arm = 0.1;
                let hands = self.robot(pose);
                let start = rect.top - 8.0 * u;
                let y = start + (hands.right.y - 4.0 * u - start) * ease(t / 0.9);
                let parcel = Rect::from_xywh(self.x - 3.2 * u, y, 6.4 * u, 5.0 * u);
                self.painter.fill_round_rect(
                    parcel,
                    Radius::uniform(0.6 * u),
                    self.prop(1.0, (205, 160, 100)),
                );
                self.line(
                    Point::new(parcel.center_x(), parcel.top),
                    Point::new(parcel.center_x(), parcel.bottom),
                    0.8,
                    self.prop(1.0, (240, 220, 170)),
                );
                self.caption("reaction_downloaded", t);
            }
            Reaction::Trophy => {
                let mut pose = Pose::new(Eyes::Happy);
                pose.right_arm = -1.5;
                pose.rise = (t * 3.0).sin().abs() * 1.2;
                let hands = self.robot(pose);
                let gold = self.prop(1.0, (255, 200, 60));
                let cup = Rect::from_xywh(
                    hands.right.x - 2.2 * u,
                    hands.right.y - 6.0 * u,
                    4.4 * u,
                    3.6 * u,
                );
                self.painter
                    .fill_round_rect(cup, Radius::uniform(1.4 * u), gold);
                for side in [-1.0, 1.0] {
                    self.painter.stroke_circle(
                        Point::new(cup.center_x() + side * 2.6 * u, cup.top + 1.2 * u),
                        0.9 * u,
                        0.5 * u,
                        gold,
                    );
                }
                self.line(
                    Point::new(cup.center_x(), cup.bottom),
                    Point::new(cup.center_x(), cup.bottom + 1.6 * u),
                    0.8,
                    gold,
                );
                self.confetti(rect, t);
                self.caption("reaction_trophy", t);
            }
            Reaction::Diwali => self.diwali(rect),
            Reaction::Cheer
            | Reaction::Bedtime
            | Reaction::Trick
            | Reaction::Notice
            | Reaction::Catch
            | Reaction::Streak
            | Reaction::Recap
            | Reaction::Milestone
            | Reaction::FirstShot => {
                self.react_more(reaction, rect);
            }
            Reaction::Holi | Reaction::Christmas | Reaction::NewYear | Reaction::Birthday => {
                self.festival(reaction, rect);
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
                self.confetti(rect, t);
            }
        }
    }

    /// Falling confetti across `rect`.
    fn confetti(&self, rect: Rect, t: f32) {
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
            let spin = (t * 6.0 + seed).sin() * 0.8 * self.unit;
            self.line(
                Point::new(x - spin, y),
                Point::new(x + spin, y + 0.6 * self.unit),
                0.9,
                self.prop(1.0, colors[index % colors.len()]),
            );
        }
    }

    /// Diwali: a string of festive lights, a glowing diya held in both hands, sparkles and
    /// fireworks bursting on either side.
    fn diwali(&self, rect: Rect) {
        let t = self.elapsed;
        let u = self.unit;
        let warm = [
            (255, 190, 70),
            (255, 120, 80),
            (255, 225, 120),
            (240, 100, 160),
        ];
        // Lights along the top edge, twinkling in turn.
        let count = ((rect.width() / u) / 5.0) as usize;
        for index in 0..count {
            let x = rect.left + (index as f32 + 0.5) * rect.width() / count as f32;
            let sag = ((index as f32 / count as f32) * PI * 3.0).sin().abs() * 1.6 * u;
            let twinkle = 0.45 + 0.55 * ((t * 4.0 + index as f32 * 0.9).sin() * 0.5 + 0.5);
            self.painter.fill_circle(
                Point::new(x, rect.top + 2.6 * u + sag),
                0.7 * u,
                self.prop(twinkle, warm[index % warm.len()]),
            );
        }
        // Fireworks: bursts that bloom and fade, one each side, taking turns.
        for (side, delay) in [(-1.0, 0.0), (1.0, 1.1)] {
            let phase = ((t + delay) % 2.2) / 2.2;
            let center = Point::new(self.x + side * 24.0 * u, self.mid - 3.0 * u);
            let radius = ease(phase / 0.6) * 6.0 * u;
            let fade = 1.0 - ease((phase - 0.4) / 0.6);
            let color = warm[((t + delay) / 2.2) as usize % warm.len()];
            for ray in 0..10 {
                let angle = ray as f32 / 10.0 * TAU;
                let (sin, cos) = angle.sin_cos();
                self.line(
                    Point::new(
                        center.x + cos * radius * 0.45,
                        center.y + sin * radius * 0.45,
                    ),
                    Point::new(center.x + cos * radius, center.y + sin * radius),
                    0.6,
                    self.prop(fade, color),
                );
            }
        }
        // The robot holds a diya up, flame flickering.
        let mut pose = Pose::new(Eyes::Happy);
        pose.right_arm = 0.0;
        pose.left_arm = 0.0;
        pose.rise = (t * 2.0).sin().abs() * 0.6;
        let hands = self.robot(pose);
        let lamp = Point::new(
            (hands.left.x + hands.right.x) / 2.0,
            hands.right.y - 1.0 * u,
        );
        self.painter.fill_oval(
            Rect::from_xywh(lamp.x - 3.6 * u, lamp.y - 1.4 * u, 7.2 * u, 2.8 * u),
            self.prop(1.0, (200, 100, 50)),
        );
        let flicker = 1.0 + (t * 17.0).sin() * 0.12 + (t * 11.0).cos() * 0.08;
        self.painter.fill_circle(
            Point::new(lamp.x, lamp.y - 3.0 * u),
            2.6 * u * flicker,
            self.prop(0.25, (255, 190, 70)),
        );
        self.painter.fill_oval(
            Rect::from_xywh(
                lamp.x - 0.9 * u,
                lamp.y - 4.2 * u * flicker,
                1.8 * u,
                3.0 * u * flicker,
            ),
            self.prop(1.0, (255, 200, 70)),
        );
        self.painter.fill_oval(
            Rect::from_xywh(lamp.x - 0.4 * u, lamp.y - 3.0 * u, 0.8 * u, 1.6 * u),
            self.prop(1.0, (255, 250, 220)),
        );
        self.sparkle(self.at(-10.0, -16.0), 0.5 + 0.5 * (t * 3.0).sin());
        self.sparkle(self.at(11.0, -18.0), 0.5 + 0.5 * (t * 3.7).cos());
        if let Some(key) = Reaction::Diwali.caption_key() {
            self.caption(key, t);
        }
    }

    /// A filled triangle through three points.
    fn triangle(&self, a: Point, b: Point, c: Point, color: Rgba) {
        let svg = format!("M{} {} L{} {} L{} {} Z", a.x, a.y, b.x, b.y, c.x, c.y);
        if let Some(path) = winisland_render::Path::from_svg(&svg) {
            self.painter.fill_path(&path, color);
        }
    }

    /// A burst that blooms and fades over `period` seconds.
    fn firework(&self, center: Point, period: f32, delay: f32, colors: &[(u8, u8, u8)]) {
        let t = self.elapsed + delay;
        let phase = (t % period) / period;
        let radius = ease(phase / 0.6) * 6.0 * self.unit;
        let fade = 1.0 - ease((phase - 0.4) / 0.6);
        let color = colors[(t / period) as usize % colors.len()];
        for ray in 0..10 {
            let (sin, cos) = (ray as f32 / 10.0 * TAU).sin_cos();
            self.line(
                Point::new(
                    center.x + cos * radius * 0.45,
                    center.y + sin * radius * 0.45,
                ),
                Point::new(center.x + cos * radius, center.y + sin * radius),
                0.6,
                self.prop(fade, color),
            );
        }
    }

    /// A pointed hat sitting on the robot's head.
    fn hat(&self, rise: f32, cone: (u8, u8, u8), trim: (u8, u8, u8)) {
        let top = self.floor - (22.0 + rise) * self.unit;
        let left = Point::new(self.x - 5.6 * self.unit, top + 1.2 * self.unit);
        let right = Point::new(self.x + 5.6 * self.unit, top + 1.2 * self.unit);
        let tip = Point::new(self.x + 3.0 * self.unit, top - 6.0 * self.unit);
        self.triangle(left, right, tip, self.prop(1.0, cone));
        self.painter.fill_round_rect(
            Rect::from_xywh(
                left.x - 0.6 * self.unit,
                left.y - 0.4 * self.unit,
                12.4 * self.unit,
                1.6 * self.unit,
            ),
            Radius::uniform(0.8 * self.unit),
            self.prop(1.0, trim),
        );
        self.painter
            .fill_circle(tip, 1.1 * self.unit, self.prop(1.0, trim));
    }

    fn festival(&self, reaction: Reaction, rect: Rect) {
        let t = self.elapsed;
        let u = self.unit;
        match reaction {
            Reaction::Holi => {
                // Puffs of coloured powder bloom around a delighted robot.
                let colors = [
                    (255, 90, 170),
                    (90, 210, 120),
                    (255, 210, 60),
                    (90, 160, 255),
                    (190, 110, 255),
                ];
                for index in 0..6 {
                    let seed = index as f32 * 7.31;
                    let phase = ((t * 0.7 + (seed.sin() * 99.0).fract().abs()) % 1.0).abs();
                    let x = rect.left + ((seed.cos() * 431.7).fract().abs()) * rect.width();
                    let y = rect.top + (0.25 + 0.5 * (seed * 1.7).sin().abs()) * rect.height();
                    self.painter.fill_circle(
                        Point::new(x, y),
                        (1.5 + phase * 5.0) * u,
                        self.prop(0.55 * (1.0 - phase), colors[index % colors.len()]),
                    );
                }
                let mut pose = Pose::new(Eyes::Happy);
                pose.right_arm = -1.2 + (t * 4.0).sin() * 0.6;
                pose.rise = (t * 3.0).sin().abs() * 1.0;
                self.robot(pose);
                // Colour smudges on its shell.
                for (dx, dy, color) in [
                    (-3.0, -6.0, colors[0]),
                    (2.5, -4.6, colors[1]),
                    (-5.0, -17.0, colors[2]),
                ] {
                    self.painter
                        .fill_circle(self.at(dx, dy), 0.9 * u, self.prop(0.9, color));
                }
            }
            Reaction::Christmas => {
                // Snow drifts down past a little tree.
                for index in 0..16 {
                    let seed = index as f32 * 3.77;
                    let column = (seed.sin() * 917.3).fract().abs();
                    let fall = ((t * (0.25 + column * 0.2) + column) % 1.0).abs();
                    let sway = (t * 2.0 + seed).sin() * 1.2 * u;
                    self.painter.fill_circle(
                        Point::new(
                            rect.left + column * rect.width() + sway,
                            rect.top + fall * rect.height(),
                        ),
                        0.55 * u,
                        self.prop(0.9, WHITE),
                    );
                }
                let base = self.at(20.0, 0.0);
                for (level, width) in [(0.0, 7.0), (4.5, 5.6), (8.5, 4.0)] {
                    self.triangle(
                        Point::new(base.x - width * u, base.y - (2.0 + level) * u),
                        Point::new(base.x + width * u, base.y - (2.0 + level) * u),
                        Point::new(base.x, base.y - (8.0 + level) * u),
                        self.prop(1.0, (60, 160, 90)),
                    );
                }
                self.painter.fill_rect(
                    Rect::from_xywh(base.x - 0.8 * u, base.y - 2.0 * u, 1.6 * u, 2.0 * u),
                    self.prop(1.0, (140, 90, 60)),
                );
                self.sparkle(
                    Point::new(base.x, base.y - 16.6 * u),
                    0.7 + 0.3 * (t * 3.0).sin(),
                );
                let mut pose = Pose::new(Eyes::Happy);
                pose.right_arm = -0.45 + (t / 0.9 * TAU).sin() * 0.45;
                self.robot(pose);
                self.hat(0.0, (220, 50, 60), (250, 250, 250));
            }
            Reaction::NewYear => {
                let colors = [
                    (255, 210, 90),
                    (120, 200, 255),
                    (255, 130, 180),
                    (150, 255, 170),
                ];
                self.firework(
                    Point::new(self.x - 22.0 * u, self.mid - 3.0 * u),
                    1.8,
                    0.0,
                    &colors,
                );
                self.firework(
                    Point::new(self.x + 24.0 * u, self.mid - 5.0 * u),
                    1.8,
                    0.9,
                    &colors,
                );
                self.confetti(rect, t);
                let hop = (t * 1.5 * TAU).sin();
                let mut pose = Pose::new(Eyes::Happy);
                pose.rise = hop.abs() * 2.0;
                pose.right_arm = -1.5 + hop * 0.3;
                pose.left_arm = -1.5 - hop * 0.3;
                self.robot(pose);
            }
            Reaction::Birthday => {
                self.confetti(rect, t);
                let mut pose = Pose::new(Eyes::Happy);
                pose.right_arm = 0.2;
                pose.left_arm = 0.2;
                let hands = self.robot(pose);
                self.hat(0.0, (120, 170, 255), (255, 210, 90));
                // A cake with three flickering candles.
                let cake = Rect::from_xywh(
                    (hands.left.x + hands.right.x) / 2.0 - 4.4 * u,
                    hands.right.y - 3.0 * u,
                    8.8 * u,
                    4.2 * u,
                );
                self.painter.fill_round_rect(
                    cake,
                    Radius::uniform(0.8 * u),
                    self.prop(1.0, (250, 225, 235)),
                );
                self.painter.fill_rect(
                    Rect::from_xywh(cake.left, cake.top + 1.6 * u, cake.width(), 0.9 * u),
                    self.prop(1.0, (240, 120, 160)),
                );
                for index in 0..3 {
                    let x = cake.left + (2.0 + index as f32 * 2.4) * u;
                    self.line(
                        Point::new(x, cake.top),
                        Point::new(x, cake.top - 2.0 * u),
                        0.5,
                        self.prop(1.0, (130, 190, 255)),
                    );
                    let flicker = 1.0 + (t * 15.0 + index as f32).sin() * 0.15;
                    self.painter.fill_oval(
                        Rect::from_xywh(
                            x - 0.5 * u,
                            cake.top - (3.4 * flicker) * u,
                            1.0 * u,
                            1.4 * flicker * u,
                        ),
                        self.prop(1.0, (255, 200, 70)),
                    );
                }
            }
            _ => {}
        }
        if let Some(key) = reaction.caption_key() {
            self.caption(key, t);
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

    /// Sits on a cushion asleep while a "z z Z" climbs; in the evening a crescent moon
    /// and a star keep it company.
    fn sleep(&self, night: bool) {
        let u = self.unit;
        let mut pose = Pose::new(Eyes::Closed);
        pose.rise = -1.2;
        pose.right_arm = 1.4;
        pose.left_arm = 1.4;
        self.robot(pose);
        self.painter.fill_round_rect(
            self.rect(-8.5, -3.2, 17.0, 3.4),
            Radius::uniform(1.7 * u),
            self.color(1.0, (206, 202, 214)),
        );
        self.painter.fill_round_rect(
            self.rect(-7.5, -3.2, 15.0, 1.0),
            Radius::uniform(0.5 * u),
            self.color(1.0, (230, 228, 236)),
        );
        // Three letters, growing, each rising and fading in turn.
        for (index, (text, size)) in [("z", 6.0), ("z", 7.5), ("Z", 9.5)].iter().enumerate() {
            let phase = (self.time / 2.7 + index as f32 / 3.0).fract();
            let fade = (phase * PI).sin();
            self.text(
                text,
                self.x + (10.0 + index as f32 * 4.6 + phase * 1.5) * u,
                self.floor - (9.0 + index as f32 * 3.4 + phase * 3.0) * u,
                *size,
                self.color(fade * 0.95, WHITE),
            );
        }
        if night {
            let moon = self.at(30.0, -15.0);
            self.painter
                .fill_circle(moon, 4.2 * u, self.color(1.0, (255, 208, 80)));
            self.painter.fill_circle(
                Point::new(moon.x + 2.0 * u, moon.y - 1.2 * u),
                3.8 * u,
                self.color(1.0, (0, 0, 0)),
            );
            let twinkle = (self.time * 2.1).sin() * 0.5 + 0.5;
            self.sparkle(self.at(35.5, -9.0), 0.5 + 0.5 * twinkle);
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
    fn diwali_window_runs_from_ten_days_before_to_three_after() {
        assert_eq!(diwali_offset(2026, 10, 31), Some(8));
        assert_eq!(diwali_offset(2026, 11, 8), Some(0));
        assert_eq!(diwali_offset(2026, 11, 11), Some(-3));
        assert_eq!(diwali_offset(2026, 11, 12), None);
        assert_eq!(diwali_offset(2026, 10, 28), None);
        assert_eq!(diwali_offset(2027, 10, 29), Some(0));
    }

    #[test]
    fn robot_walks_on_and_off_at_scene_edges() {
        // A robot at x = 100 in a notch from 0 to 300, unit 2, props 10 left and 30 right:
        // it must enter from 50 + 30 + clearance units left and leave 100 + 10 + clearance
        // units right, so it is wholly outside the notch at each scene change.
        set_notch_span(0.0, 300.0);
        let (from, to) = walk_distances(100.0, 2.0, 10.0, 30.0);
        assert_eq!(from, 50.0 + 30.0 + CLEARANCE);
        assert_eq!(to, 100.0 + 10.0 + CLEARANCE);
        let span = (from, to);
        // Starts off to the left, stands in place mid-scene, ends off to the right.
        assert!(scene_shift(Scene::Chai, 0.0, SCENE_SECONDS, span) <= -from + 0.01);
        assert_eq!(scene_shift(Scene::Chai, SCENE_SECONDS / 2.0, SCENE_SECONDS, span), 0.0);
        assert!(scene_shift(Scene::Chai, SCENE_SECONDS, SCENE_SECONDS, span) >= to - 0.01);
        // Moves steadily in, never overshooting.
        let mut last = scene_shift(Scene::Chai, 0.0, SCENE_SECONDS, span);
        for step in 1..=13 {
            let now = scene_shift(Scene::Chai, step as f32 * 0.1, SCENE_SECONDS, span);
            assert!(now >= last && now <= 0.0);
            last = now;
        }
        // Night never walks.
        assert_eq!(scene_shift(Scene::Night, 0.0, SCENE_SECONDS, span), 0.0);
    }
}
