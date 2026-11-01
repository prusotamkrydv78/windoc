//! What makes the robot feel alive with the user: what they are doing (an editor, a meeting,
//! a fullscreen video), a mood that builds over the day, what it remembers between runs
//! (`core::mascot_memory`), the accessories it unlocks, the weather rubbing off on it, and
//! the ways it can be played with (a double-click trick, noticing a hovering cursor).

use std::cell::RefCell;

use crate::core::mascot_memory::{self, Memory};

use super::{Reaction, Scene, days_from_civil, demo, react, seconds};

/// What the user is doing, from the foreground app, the microphone and camera.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) enum Activity {
    #[default]
    Idle,
    /// A code editor or IDE is in front: the robot types along.
    Editor,
    /// The microphone or camera is in use: a headset on, quiet.
    Meeting,
    /// A fullscreen window (a video or a game) is in front: popcorn.
    Fullscreen,
}

/// Executables that count as code editors, lower case.
const EDITORS: [&str; 16] = [
    "code.exe",
    "code - insiders.exe",
    "cursor.exe",
    "windsurf.exe",
    "zed.exe",
    "devenv.exe",
    "idea64.exe",
    "pycharm64.exe",
    "webstorm64.exe",
    "rider64.exe",
    "clion64.exe",
    "goland64.exe",
    "rustrover64.exe",
    "studio64.exe",
    "sublime_text.exe",
    "notepad++.exe",
];

/// Sorts what is in front into an activity. A meeting wins over everything else.
pub(super) fn classify(app: &str, fullscreen: bool, meeting: bool) -> Activity {
    if meeting {
        return Activity::Meeting;
    }
    let app = app.to_ascii_lowercase();
    if EDITORS.contains(&app.as_str()) {
        Activity::Editor
    } else if fullscreen && !app.is_empty() && app != "explorer.exe" {
        Activity::Fullscreen
    } else {
        Activity::Idle
    }
}

/// How the robot feels, built up over the day.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) enum Mood {
    #[default]
    Calm,
    /// Breaks taken, a good morning: bouncier walks and more waving.
    Happy,
    /// Late, or a long stretch without a break: slow walks, heavy blinks, more naps.
    Sleepy,
    /// Focus sessions done today: a spring in its step.
    Proud,
    /// Ignored for hours: looks around more.
    Bored,
}

/// The mood for `hour`, today's `breaks` and Focus `sessions`, minutes since the user last
/// played with the robot and minutes of unbroken activity.
pub(super) fn mood_for(
    hour: u16,
    breaks: u32,
    sessions: u32,
    minutes_since_play: f32,
    active_minutes: f32,
) -> Mood {
    if sessions >= 2 {
        Mood::Proud
    } else if !(6..22).contains(&hour) || (active_minutes > 150.0 && breaks == 0) {
        Mood::Sleepy
    } else if minutes_since_play > 180.0 {
        Mood::Bored
    } else if breaks >= 2 || (9..12).contains(&hour) {
        Mood::Happy
    } else {
        Mood::Calm
    }
}

/// How much longer (or shorter) walks take in this mood.
pub(super) fn walk_pace(mood: Mood) -> f32 {
    match mood {
        Mood::Sleepy => 1.4,
        Mood::Bored => 1.15,
        Mood::Calm => 1.0,
        Mood::Proud => 0.9,
        Mood::Happy => 0.85,
    }
}

/// Blink period and length in seconds: sleepy eyes close slowly and often.
pub(super) fn blink_timing(mood: Mood) -> (f32, f32) {
    match mood {
        Mood::Sleepy => (2.6, 0.38),
        Mood::Bored => (3.0, 0.16),
        _ => (3.7, 0.13),
    }
}

/// The daytime scene to play instead of `scene`, given the mood.
pub(super) fn mood_scene(scene: Scene, mood: Mood) -> Scene {
    match (mood, scene) {
        (Mood::Sleepy, Scene::Stretch | Scene::Paint) => Scene::Nap,
        (Mood::Bored, Scene::Read) => Scene::LookAround,
        (Mood::Happy | Mood::Proud, Scene::Nap) => Scene::Stretch,
        _ => scene,
    }
}

/// Something the robot wears.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Accessory {
    Scarf,
    Glasses,
    Cap,
}

const ACCESSORIES: [Accessory; 3] = [Accessory::Scarf, Accessory::Glasses, Accessory::Cap];

/// What to wear: `choice` 0 is nothing, 1 the newest unlocked, then each by name once
/// unlocked by `active_days` of use.
pub(super) fn accessory_for(choice: u8, active_days: u32) -> Option<Accessory> {
    let unlocked = mascot_memory::unlocked_count(active_days);
    match choice {
        0 => None,
        1 => unlocked.checked_sub(1).map(|index| ACCESSORIES[index]),
        n => {
            let index = usize::from(n - 2);
            (index < unlocked).then(|| ACCESSORIES[index])
        }
    }
}

/// Weather that rubs off on the robot.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) enum Weather {
    #[default]
    None,
    /// An umbrella.
    Rain,
    /// Sunglasses.
    Sunny,
}

/// Unbroken activity this long earns a cheer.
const CHEER_AFTER: f32 = 45.0 * 60.0;
/// Hold the cursor over the robot this long and it notices.
const NOTICE_AFTER: f32 = 1.5;
/// Two clicks this close together are a double-click.
const DOUBLE_CLICK: f32 = 0.4;
/// Save the memory at most this often while only focus time changes.
const SAVE_EVERY: f32 = 60.0;

#[derive(Default)]
struct Life {
    memory: Option<Memory>,
    activity: Activity,
    mood: Mood,
    weather: Weather,
    colour: u8,
    accessory_choice: u8,
    /// When the user last played with the robot, in robot seconds.
    last_play: f32,
    last_click: Option<f32>,
    hover_since: Option<f32>,
    /// The last tick, to add Focus time.
    last_tick: Option<f32>,
    last_save: f32,
    dirty: bool,
    /// The streak to show after the current celebration, and the caption number.
    caption_arg: String,
}

thread_local! {
    static LIFE: RefCell<Life> = RefCell::new(Life::default());
}

fn today() -> i32 {
    let clock = crate::platform::shell().local_datetime();
    days_from_civil(
        i32::from(clock.year),
        u32::from(clock.month),
        u32::from(clock.day),
    )
}

fn with_memory<R>(f: impl FnOnce(&mut Memory) -> R) -> R {
    LIFE.with(|life| {
        let mut life = life.borrow_mut();
        let memory = life.memory.get_or_insert_with(Memory::load);
        let result = f(memory);
        life.dirty = true;
        result
    })
}

fn save_now() {
    LIFE.with(|life| {
        let mut life = life.borrow_mut();
        if let Some(memory) = life.memory.as_ref() {
            memory.save();
        }
        life.dirty = false;
        life.last_save = seconds();
    });
}

/// The number shown in the current reaction's caption, such as a streak length.
pub(super) fn caption_arg() -> String {
    LIFE.with(|life| life.borrow().caption_arg.clone())
}

fn set_caption_arg(text: String) {
    LIFE.with(|life| life.borrow_mut().caption_arg = text);
}

pub(super) fn activity() -> Activity {
    // Test hook: WINISLAND_MASCOT_DEMO_ACTIVITY=meeting|popcorn|editor in demo mode.
    if demo()
        && let Some(forced) = std::env::var("WINISLAND_MASCOT_DEMO_ACTIVITY")
            .ok()
            .and_then(|name| match name.as_str() {
                "meeting" => Some(Activity::Meeting),
                "popcorn" => Some(Activity::Fullscreen),
                "editor" => Some(Activity::Editor),
                "idle" => Some(Activity::Idle),
                _ => None,
            })
    {
        return forced;
    }
    LIFE.with(|life| life.borrow().activity)
}

/// Test hook: with WINISLAND_MASCOT_DEMO_REACTIONS=1 in demo mode, the newer reactions play
/// one after another so each can be watched.
pub(super) fn demo_reactions() {
    const NEW: [Reaction; 9] = [
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
    thread_local! {
        static NEXT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }
    if !demo() || std::env::var_os("WINISLAND_MASCOT_DEMO_REACTIONS").is_none() {
        return;
    }
    if super::REACTION.with(std::cell::Cell::get).is_some() {
        return;
    }
    let index = NEXT.with(|next| {
        let index = next.get();
        next.set(index + 1);
        index
    });
    if index < NEW.len() {
        set_caption_arg(["3", "", "", "", "", "3", "6", "500", ""][index].to_string());
        super::REACTION.with(|cell| cell.set(Some((NEW[index], None))));
    }
}

pub(super) fn mood() -> Mood {
    if demo()
        && let Some(forced) = std::env::var("WINISLAND_MASCOT_DEMO_MOOD")
            .ok()
            .and_then(|name| match name.as_str() {
                "happy" => Some(Mood::Happy),
                "sleepy" => Some(Mood::Sleepy),
                "proud" => Some(Mood::Proud),
                "bored" => Some(Mood::Bored),
                "calm" => Some(Mood::Calm),
                _ => None,
            })
    {
        return forced;
    }
    LIFE.with(|life| life.borrow().mood)
}

pub(super) fn weather() -> Weather {
    LIFE.with(|life| life.borrow().weather)
}

/// The shell colour index chosen in Settings.
pub(super) fn colour() -> usize {
    LIFE.with(|life| usize::from(life.borrow().colour))
}

pub(super) fn accessory() -> Option<Accessory> {
    let (choice, days) = LIFE.with(|life| {
        let mut life = life.borrow_mut();
        let choice = life.accessory_choice;
        let days = life.memory.get_or_insert_with(Memory::load).active_days;
        (choice, days)
    });
    // The demo shows every accessory in turn.
    if demo() && choice == 1 {
        return Some(ACCESSORIES[(seconds() / 15.0) as usize % ACCESSORIES.len()]);
    }
    accessory_for(choice, days)
}

/// How many days the robot has been used, for the plant's growth.
pub(super) fn active_days() -> u32 {
    with_memory(|memory| memory.active_days)
}

/// Applies the colour and accessory chosen in Settings. Call each tick.
pub(crate) fn set_look(colour: u8, accessory: u8) {
    LIFE.with(|life| {
        let mut life = life.borrow_mut();
        life.colour = colour.min(4);
        life.accessory_choice = accessory.min(4);
    });
}

/// What is in front, and whether the microphone or camera is in use. Call each tick.
pub(crate) fn set_activity(app: &str, fullscreen: bool, meeting: bool) {
    let activity = classify(app, fullscreen, meeting);
    LIFE.with(|life| life.borrow_mut().activity = activity);
}

/// The weather's condition key (such as `weather_rain`), or `None` when weather is off.
pub(crate) fn set_weather(condition: Option<&str>) {
    let weather = match condition {
        Some(
            "weather_rain" | "weather_drizzle" | "weather_showers" | "weather_thunderstorm",
        ) => Weather::Rain,
        Some("weather_clear") => Weather::Sunny,
        _ => Weather::None,
    };
    LIFE.with(|life| life.borrow_mut().weather = weather);
}

/// Called every tick from `set_context`: rolls the day over, counts Focus time, settles
/// the mood and offers a cheer or a bedtime nudge.
pub(super) fn tick(focus_running: bool, away: bool, active_seconds: Option<f32>) {
    let now = seconds();
    let today = today();
    let clock = crate::platform::shell().local_datetime();
    let (new_day, recap) = with_memory(|memory| {
        memory.roll_day(today);
        let new_day = !away && memory.mark_active(today);
        let recap = if new_day { memory.recap_due(today) } else { None };
        (new_day, recap)
    });
    if let Some(seconds) = recap {
        set_caption_arg(format!("{:.0}", seconds / 3600.0));
        react(Reaction::Recap);
    }
    let dt = LIFE.with(|life| {
        let mut life = life.borrow_mut();
        let dt = life.last_tick.map_or(0.0, |last| (now - last).clamp(0.0, 5.0));
        life.last_tick = Some(now);
        dt
    });
    if focus_running && dt > 0.0 {
        with_memory(|memory| memory.add_focus(today, dt));
    }
    let (breaks, sessions) = with_memory(|memory| (memory.breaks, memory.focus_sessions));
    let since_play = LIFE.with(|life| (now - life.borrow().last_play) / 60.0);
    let active_minutes = active_seconds.unwrap_or(0.0) / 60.0;
    let mood = mood_for(clock.hour, breaks, sessions, since_play, active_minutes);
    LIFE.with(|life| life.borrow_mut().mood = mood);

    if !away && activity() != Activity::Meeting {
        if active_seconds.is_some_and(|active| active >= CHEER_AFTER) {
            react(Reaction::Cheer);
        }
        if (1..5).contains(&clock.hour) {
            react(Reaction::Bedtime);
        }
    }
    let due = LIFE.with(|life| {
        let life = life.borrow();
        life.dirty && (new_day || now - life.last_save >= SAVE_EVERY)
    });
    if due {
        save_now();
    }
}

/// Hooks run when a reaction is accepted: remembering breaks, Focus sessions and copies,
/// and swapping in a streak or milestone when one is reached.
pub(super) fn on_reaction(reaction: Reaction) -> Reaction {
    let today = today();
    let swapped = match reaction {
        Reaction::WelcomeBack => {
            with_memory(|memory| memory.add_break(today));
            None
        }
        Reaction::Celebrate | Reaction::Trophy => {
            let (_, streak) = with_memory(|memory| memory.focus_finished(today));
            // A streak of two days or more shows next, after the celebration.
            if let Some(days) = streak.filter(|days| *days >= 2) {
                set_caption_arg(days.to_string());
                super::queue_after(Reaction::Streak);
            }
            None
        }
        Reaction::Copied => with_memory(Memory::copied).map(|count| {
            set_caption_arg(count.to_string());
            Reaction::Milestone
        }),
        Reaction::Jump | Reaction::Trick | Reaction::Notice => {
            LIFE.with(|life| life.borrow_mut().last_play = seconds());
            None
        }
        _ => None,
    };
    if matches!(
        reaction,
        Reaction::WelcomeBack | Reaction::Celebrate | Reaction::Trophy | Reaction::Copied
    ) {
        save_now();
    }
    swapped.unwrap_or(reaction)
}

/// A screenshot was saved: the first of the day earns a little moment.
pub(crate) fn screenshot_taken() {
    if with_memory(|memory| memory.screenshot(today())) {
        save_now();
        super::queue(Reaction::FirstShot);
    }
}

/// The resting robot was clicked: a hop, or a trick for a double-click.
pub(crate) fn clicked() {
    let now = seconds();
    let double = LIFE.with(|life| {
        let mut life = life.borrow_mut();
        let double = life
            .last_click
            .is_some_and(|last| now - last <= DOUBLE_CLICK);
        life.last_click = if double { None } else { Some(now) };
        double
    });
    react(if double {
        Reaction::Trick
    } else {
        Reaction::Jump
    });
}

/// Whether the cursor rests over the collapsed notch. Held long enough, the robot notices.
pub(crate) fn hovering(over: bool) {
    let now = seconds();
    let notice = LIFE.with(|life| {
        let mut life = life.borrow_mut();
        if !over {
            life.hover_since = None;
            return false;
        }
        let since = *life.hover_since.get_or_insert(now);
        if now - since >= NOTICE_AFTER {
            // Once per hover.
            life.hover_since = Some(f32::INFINITY);
            return true;
        }
        false
    });
    if notice {
        react(Reaction::Notice);
    }
}

/// A file was dropped on the notch and parked on the Shelf: the robot catches it.
pub(crate) fn file_dropped() {
    super::queue(Reaction::Catch);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editors_meetings_and_fullscreen_are_told_apart() {
        assert_eq!(classify("Code.exe", false, false), Activity::Editor);
        assert_eq!(classify("devenv.exe", true, false), Activity::Editor);
        assert_eq!(classify("Code.exe", false, true), Activity::Meeting);
        assert_eq!(classify("vlc.exe", true, false), Activity::Fullscreen);
        assert_eq!(classify("explorer.exe", true, false), Activity::Idle);
        assert_eq!(classify("chrome.exe", false, false), Activity::Idle);
        assert_eq!(classify("", true, false), Activity::Idle);
    }

    #[test]
    fn mood_builds_from_the_day() {
        assert_eq!(mood_for(14, 0, 2, 0.0, 0.0), Mood::Proud);
        assert_eq!(mood_for(23, 3, 0, 0.0, 0.0), Mood::Sleepy);
        assert_eq!(mood_for(15, 0, 0, 0.0, 200.0), Mood::Sleepy);
        assert_eq!(mood_for(15, 1, 0, 240.0, 30.0), Mood::Bored);
        assert_eq!(mood_for(10, 0, 0, 0.0, 30.0), Mood::Happy);
        assert_eq!(mood_for(15, 2, 0, 0.0, 30.0), Mood::Happy);
        assert_eq!(mood_for(15, 0, 0, 10.0, 30.0), Mood::Calm);
    }

    #[test]
    fn mood_changes_pace_blinks_and_scenes() {
        assert!(walk_pace(Mood::Sleepy) > walk_pace(Mood::Calm));
        assert!(walk_pace(Mood::Happy) < walk_pace(Mood::Calm));
        assert!(blink_timing(Mood::Sleepy).1 > blink_timing(Mood::Calm).1);
        assert_eq!(mood_scene(Scene::Paint, Mood::Sleepy), Scene::Nap);
        assert_eq!(mood_scene(Scene::Nap, Mood::Happy), Scene::Stretch);
        assert_eq!(mood_scene(Scene::Chai, Mood::Bored), Scene::Chai);
    }

    #[test]
    fn accessories_follow_unlocks_and_choice() {
        assert_eq!(accessory_for(1, 0), None);
        assert_eq!(accessory_for(1, 3), Some(Accessory::Scarf));
        assert_eq!(accessory_for(1, 14), Some(Accessory::Cap));
        assert_eq!(accessory_for(0, 14), None);
        assert_eq!(accessory_for(3, 7), Some(Accessory::Glasses));
        // Not unlocked yet.
        assert_eq!(accessory_for(4, 7), None);
    }
}
