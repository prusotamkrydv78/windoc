//! Calendar tab (the Agenda destination): a seven-day strip across the top, "Up next" with
//! the rest of the week grouped by day on the left, and a Reminders card on the right.
//!
//! Events come from the agenda rows the app publishes (`AGENDA_ROWS`, the next seven days
//! from the subscribed ICS feeds). Reminders belong to this tab and are saved to
//! `reminders.json` in the app data directory.

use std::cell::RefCell;
use std::time::{Instant, SystemTime};

use serde::{Deserialize, Serialize};
use winisland_platform::Key;
use winisland_render::text::FontManager;
use winisland_render::tokens::{SemanticColors, TextStyle, radius, typography};
use winisland_render::{FontStyle, Painter, Point, Radius, Rect, Rgba, StrokeCap};

use super::TabContext;
use crate::ui::components::text::{Align, draw_text};
use crate::ui::dashboard::{DashboardHit, Destination};

const REMINDERS_FILE: &str = "reminders.json";
const MAX_FILE_BYTES: u64 = 512 * 1024;
const MAX_REMINDERS: usize = 200;
const MAX_REMINDER_CHARS: usize = 100;

/// Hit ids (200-299 belong to this tab).
const ID_DAY: u16 = 200;
const ID_SETTINGS: u16 = 210;
const ID_ADD: u16 = 220;
const ID_TOGGLE: u16 = 230;
const ID_DELETE: u16 = 250;
const ID_MORE: u16 = 270;
const ID_CLEAR_DONE: u16 = 271;
const MAX_SLOTS: usize = 20;

const STRIP_HEIGHT: f32 = 52.0;
const CARD_PAD: f32 = 14.0;
const ROW_HEIGHT: f32 = 28.0;
const LINE_HEIGHT: f32 = 22.0;

const DAY_NUMBER: TextStyle = TextStyle {
    size: 14.0,
    weight: 600,
    tabular: true,
};
const EVENT_TITLE: TextStyle = TextStyle {
    size: 20.0,
    weight: 650,
    tabular: false,
};
const TIME: TextStyle = TextStyle {
    size: 12.0,
    weight: 500,
    tabular: true,
};

/// Bar colours for events, picked by title so an event keeps its colour.
const EVENT_COLORS: [Rgba; 6] = [
    Rgba::from_rgb(10, 132, 255),
    Rgba::from_rgb(48, 209, 88),
    Rgba::from_rgb(255, 159, 10),
    Rgba::from_rgb(191, 90, 242),
    Rgba::from_rgb(255, 55, 95),
    Rgba::from_rgb(100, 210, 255),
];

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Reminder {
    id: u64,
    text: String,
    /// Free text such as "23:00" or "Fri", typed after an `@`.
    #[serde(default)]
    due: String,
    #[serde(default)]
    done: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct ReminderFile {
    #[serde(default)]
    reminders: Vec<Reminder>,
    #[serde(default)]
    next_id: u64,
}

#[derive(Default)]
struct State {
    loaded: bool,
    reminders: Vec<Reminder>,
    next_id: u64,
    /// Text of the reminder being typed.
    editing: Option<String>,
    /// Day number picked in the strip; `None` is today.
    selected: Option<i64>,
    /// Page of the reminder list when it does not fit.
    page: usize,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.loaded {
            state.loaded = true;
            match crate::core::json_store::read::<ReminderFile>(REMINDERS_FILE, MAX_FILE_BYTES) {
                Ok(Some(file)) => {
                    let next = file.reminders.iter().map(|r| r.id + 1).max().unwrap_or(0);
                    state.next_id = file.next_id.max(next);
                    state.reminders = file.reminders;
                }
                Ok(None) => {}
                Err(error) => log::warn!("Ignoring saved reminders: {error}"),
            }
        }
        f(&mut state)
    })
}

fn save(state: &State) {
    let file = ReminderFile {
        reminders: state.reminders.clone(),
        next_id: state.next_id,
    };
    if let Err(error) = crate::core::json_store::write(REMINDERS_FILE, &file) {
        log::warn!("Cannot save reminders: {error}");
    }
}

// ----------------------------------------------------------------------------------------
// Dates and events
// ----------------------------------------------------------------------------------------

/// Local day number (days since 1970-01-01) and minute of the day.
fn local_now() -> (i64, i64) {
    let now = crate::platform::shell().local_datetime();
    let day = crate::ui::dashboard::days_from_civil(
        i64::from(now.year),
        u32::from(now.month),
        u32::from(now.day),
    );
    (day, i64::from(now.hour) * 60 + i64::from(now.minute))
}

/// Minutes to add to UTC for local time.
fn local_offset_minutes(today: i64, minute: i64) -> i64 {
    let utc = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|elapsed| (elapsed.as_secs() / 60) as i64)
        .unwrap_or_default();
    ((today * 1440 + minute - utc) as f64 / 15.0).round() as i64 * 15
}

fn weekday(day: i64) -> &'static str {
    WEEKDAYS[(day + 4).rem_euclid(7) as usize]
}

/// "Thu 29 Oct".
fn short_date(day: i64) -> String {
    let (_, month, date) = crate::ui::dashboard::civil_from_days(day);
    let month = MONTHS
        .get(month.saturating_sub(1) as usize)
        .copied()
        .unwrap_or("");
    format!("{} {date} {month}", weekday(day))
}

struct Event {
    day: i64,
    /// Minute of the local day; `None` for all-day events.
    minute: Option<i64>,
    title: String,
    location: String,
    color: Rgba,
}

fn color_for(title: &str) -> Rgba {
    let hash = title.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    EVENT_COLORS[(hash % EVENT_COLORS.len() as u64) as usize]
}

/// Parses an agenda row detail: `YYYY-MM-DD[ HH:MM[ UTC]][ · location]`.
fn parse_event(title: &str, detail: &str, offset: i64) -> Option<Event> {
    let year: i64 = detail.get(0..4)?.parse().ok()?;
    let month: u32 = detail.get(5..7)?.parse().ok()?;
    let date: u32 = detail.get(8..10)?.parse().ok()?;
    let mut day = crate::ui::dashboard::days_from_civil(year, month, date);
    let mut rest = detail.get(10..).unwrap_or("");
    let mut minute = None;
    if let Some(time) = rest.strip_prefix(' ').and_then(|text| text.get(0..5))
        && time.as_bytes().get(2) == Some(&b':')
        && let (Ok(hours), Ok(minutes)) = (time[0..2].parse::<i64>(), time[3..5].parse::<i64>())
    {
        let mut total = hours * 60 + minutes;
        rest = &rest[6..];
        if let Some(after) = rest.strip_prefix(" UTC") {
            rest = after;
            total += offset;
            day += total.div_euclid(1440);
            total = total.rem_euclid(1440);
        }
        minute = Some(total);
    }
    let location = rest.strip_prefix(" · ").unwrap_or("").trim().to_string();
    Some(Event {
        day,
        minute,
        title: title.to_string(),
        location,
        color: color_for(title),
    })
}

/// All published events, sorted, plus whether any rows were published at all.
fn events(today: i64, minute: i64) -> Vec<Event> {
    let offset = local_offset_minutes(today, minute);
    let mut events: Vec<Event> = crate::ui::dashboard::AGENDA_ROWS.with(|cell| {
        cell.borrow()
            .iter()
            .filter_map(|(title, detail)| parse_event(title, detail, offset))
            .collect()
    });
    events.sort_by_key(|event| (event.day, event.minute.unwrap_or(-1)));
    events
}

/// Events still to come: future days, all-day events today, and timed events today that
/// started less than a quarter of an hour ago.
fn upcoming(events: &[Event], today: i64, minute: i64) -> Vec<&Event> {
    events
        .iter()
        .filter(|event| {
            event.day > today
                || (event.day == today && event.minute.is_none_or(|start| start >= minute - 15))
        })
        .collect()
}

fn clock(minute: i64) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

fn until_text(minutes: i64) -> String {
    if minutes <= 0 {
        "now".to_string()
    } else if minutes < 60 {
        format!("in {minutes}m")
    } else if minutes < 24 * 60 {
        let (hours, rest) = (minutes / 60, minutes % 60);
        if rest == 0 {
            format!("in {hours}h")
        } else {
            format!("in {hours}h {rest}m")
        }
    } else {
        let days = (minutes + 12 * 60) / (24 * 60);
        if days == 1 {
            "in 1 day".to_string()
        } else {
            format!("in {days} days")
        }
    }
}

fn day_word(day: i64, today: i64) -> String {
    match day - today {
        0 => "Today".to_string(),
        1 => "Tomorrow".to_string(),
        _ => weekday(day).to_string(),
    }
}

/// "23:30 · in 1h 16m", "Tomorrow 10:00 · in 14h", "All day · Today".
fn when_text(event: &Event, today: i64, minute: i64) -> String {
    let mut text = match event.minute {
        None => format!("All day · {}", day_word(event.day, today)),
        Some(start) => {
            let until = (event.day - today) * 1440 + start - minute;
            if event.day == today {
                format!("{} · {}", clock(start), until_text(until))
            } else {
                format!(
                    "{} {} · {}",
                    day_word(event.day, today),
                    clock(start),
                    until_text(until)
                )
            }
        }
    };
    if !event.location.is_empty() {
        text.push_str(" · ");
        text.push_str(&event.location);
    }
    text
}

// ----------------------------------------------------------------------------------------
// Layout
// ----------------------------------------------------------------------------------------

struct Layout {
    days: [Rect; 7],
    left: Rect,
    reminders: Rect,
}

fn layout(grid: Rect) -> Layout {
    let cell = grid.width() / 7.0;
    let days = std::array::from_fn(|index| {
        Rect::from_xywh(
            grid.left + cell * index as f32 + 3.0,
            grid.top,
            cell - 6.0,
            STRIP_HEIGHT,
        )
    });
    let body_top = grid.top + STRIP_HEIGHT + 8.0;
    let width = (grid.width() * 0.4).clamp(220.0, 320.0).min(grid.width());
    let reminders = Rect::from_ltrb(grid.right - width, body_top, grid.right, grid.bottom);
    let left = Rect::from_ltrb(
        grid.left + 8.0,
        body_top,
        reminders.left - 16.0,
        grid.bottom,
    );
    Layout {
        days,
        left,
        reminders,
    }
}

fn settings_button(left: Rect) -> Rect {
    Rect::from_xywh(left.left, left.top + 82.0, 132.0, 30.0)
}

/// Geometry of the Reminders card.
struct ReminderLayout {
    header_action: Rect,
    rows: Vec<Rect>,
    add: Rect,
}

fn reminder_layout(card: Rect) -> ReminderLayout {
    let add = Rect::from_xywh(
        card.left + 6.0,
        card.bottom - 8.0 - 30.0,
        card.width() - 12.0,
        30.0,
    );
    let top = card.top + 36.0;
    let bottom = add.top - 2.0;
    let capacity = (((bottom - top) / ROW_HEIGHT).floor().max(0.0) as usize).min(MAX_SLOTS);
    let rows = (0..capacity)
        .map(|slot| {
            Rect::from_xywh(
                card.left + 6.0,
                top + ROW_HEIGHT * slot as f32,
                card.width() - 12.0,
                ROW_HEIGHT,
            )
        })
        .collect();
    ReminderLayout {
        header_action: Rect::from_xywh(
            card.right - CARD_PAD - 80.0,
            card.top + 8.0,
            80.0 + 6.0,
            24.0,
        ),
        rows,
        add,
    }
}

/// What occupies each visible reminder slot.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Reminder(usize),
    More { hidden: usize, last_page: bool },
}

/// Open reminders first (oldest first), then done ones, paged into `capacity` slots.
fn slots(state: &State, capacity: usize) -> Vec<Slot> {
    let mut order: Vec<usize> = (0..state.reminders.len())
        .filter(|index| !state.reminders[*index].done)
        .collect();
    order.extend((0..state.reminders.len()).filter(|index| state.reminders[*index].done));
    if order.len() <= capacity || capacity < 2 {
        return order
            .into_iter()
            .take(capacity)
            .map(Slot::Reminder)
            .collect();
    }
    let per_page = capacity - 1;
    let pages = order.len().div_ceil(per_page);
    let page = state.page % pages;
    let start = page * per_page;
    let mut slots: Vec<Slot> = order[start..(start + per_page).min(order.len())]
        .iter()
        .map(|index| Slot::Reminder(*index))
        .collect();
    let last_page = page + 1 == pages;
    slots.push(Slot::More {
        hidden: order.len() - slots.len(),
        last_page,
    });
    slots
}

/// The delete button at the right of a reminder row.
fn delete_button(row: Rect) -> Rect {
    Rect::from_xywh(row.right - 28.0, row.top, 28.0, row.height())
}

// ----------------------------------------------------------------------------------------
// Tab entry points
// ----------------------------------------------------------------------------------------

pub fn custom_layout() -> bool {
    true
}

pub fn poll(_now: Instant) {
    let visible = crate::ui::dashboard::showing(Destination::Agenda);
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !visible && state.editing.is_some() {
            state.editing = None;
        }
    });
}

pub fn hit(grid: Rect, point: Point) -> Option<DashboardHit> {
    let layout = layout(grid);
    if let Some(index) = layout.days.iter().position(|day| day.contains(point)) {
        return Some(DashboardHit::Tab(ID_DAY + index as u16));
    }
    if layout.reminders.contains(point) {
        let card = reminder_layout(layout.reminders);
        if card.add.contains(point) {
            return Some(DashboardHit::Tab(ID_ADD));
        }
        return with_state(|state| {
            if state.reminders.iter().any(|reminder| reminder.done)
                && card.header_action.contains(point)
            {
                return Some(DashboardHit::Tab(ID_CLEAR_DONE));
            }
            let slots = slots(state, card.rows.len());
            let (slot, row) = slots
                .iter()
                .zip(&card.rows)
                .enumerate()
                .find(|(_, (_, row))| row.contains(point))
                .map(|(slot, (kind, row))| ((slot, *kind), *row))?;
            Some(match slot.1 {
                Slot::More { .. } => DashboardHit::Tab(ID_MORE),
                Slot::Reminder(_) if delete_button(row).contains(point) => {
                    DashboardHit::Tab(ID_DELETE + slot.0 as u16)
                }
                Slot::Reminder(_) => DashboardHit::Tab(ID_TOGGLE + slot.0 as u16),
            })
        });
    }
    let empty = crate::ui::dashboard::AGENDA_ROWS.with(|cell| cell.borrow().is_empty());
    (empty && settings_button(layout.left).contains(point))
        .then_some(DashboardHit::Tab(ID_SETTINGS))
}

pub fn apply(hit: DashboardHit) -> bool {
    let DashboardHit::Tab(id) = hit else {
        return false;
    };
    match id {
        _ if (ID_DAY..ID_DAY + 7).contains(&id) => {
            let (today, _) = local_now();
            let day = today + i64::from(id - ID_DAY);
            with_state(|state| {
                state.selected = (day != today && state.selected != Some(day)).then_some(day);
            });
        }
        ID_SETTINGS => {
            crate::ui::dashboard::HEADER_REQUEST
                .with(|cell| cell.set(Some(DashboardHit::OpenSettings)));
        }
        ID_ADD => with_state(|state| {
            if state.editing.is_none() {
                state.editing = Some(String::new());
            }
        }),
        ID_MORE => with_state(|state| state.page = state.page.wrapping_add(1)),
        ID_CLEAR_DONE => with_state(|state| {
            state.reminders.retain(|reminder| !reminder.done);
            state.page = 0;
            save(state);
        }),
        _ if (ID_TOGGLE..ID_TOGGLE + MAX_SLOTS as u16).contains(&id)
            || (ID_DELETE..ID_DELETE + MAX_SLOTS as u16).contains(&id) =>
        {
            let delete = id >= ID_DELETE;
            let slot = usize::from(id - if delete { ID_DELETE } else { ID_TOGGLE });
            with_state(|state| {
                let capacity = current_capacity();
                let Some(Slot::Reminder(index)) = slots(state, capacity).get(slot).copied() else {
                    return;
                };
                if delete {
                    state.reminders.remove(index);
                } else if let Some(reminder) = state.reminders.get_mut(index) {
                    reminder.done = !reminder.done;
                }
                save(state);
            });
        }
        _ => return false,
    }
    true
}

thread_local! {
    /// Reminder rows that fit at the last draw, so a click maps to the slot it hit.
    static CAPACITY: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn current_capacity() -> usize {
    CAPACITY.with(std::cell::Cell::get)
}

pub fn wants_keyboard() -> bool {
    STATE.with(|cell| cell.borrow().editing.is_some())
}

pub fn handle_key(key: &Key) -> bool {
    with_state(|state| {
        let Some(text) = state.editing.as_mut() else {
            return false;
        };
        match key {
            Key::Escape => state.editing = None,
            Key::Backspace => {
                text.pop();
            }
            Key::Enter => {
                let typed = state.editing.take().unwrap_or_default();
                if let Some((text, due)) = parse_reminder(&typed)
                    && state.reminders.len() < MAX_REMINDERS
                {
                    let id = state.next_id;
                    state.next_id += 1;
                    state.reminders.push(Reminder {
                        id,
                        text,
                        due,
                        done: false,
                    });
                    save(state);
                }
            }
            Key::Character(characters) => {
                for character in characters.chars().filter(|c| !c.is_control()) {
                    if text.chars().count() >= MAX_REMINDER_CHARS {
                        break;
                    }
                    text.push(character);
                }
            }
            Key::Tab | Key::Other => return false,
            // Arrow, Home, End and Delete keys have nothing to move in a one-line field.
            _ => {}
        }
        true
    })
}

/// Splits "Pay the bill @ 23:00" into the text and its due note.
fn parse_reminder(typed: &str) -> Option<(String, String)> {
    let typed = typed.trim();
    if typed.is_empty() {
        return None;
    }
    if let Some((text, due)) = typed.rsplit_once('@') {
        let (text, due) = (text.trim(), due.trim());
        if !text.is_empty() && !due.is_empty() && due.chars().count() <= 16 {
            return Some((text.to_string(), due.to_string()));
        }
    }
    Some((typed.to_string(), String::new()))
}

// ----------------------------------------------------------------------------------------
// Drawing
// ----------------------------------------------------------------------------------------

fn text_width(text: &str, style: TextStyle) -> f32 {
    let font = if style.weight >= 600 {
        FontStyle::bold()
    } else {
        FontStyle::normal()
    };
    FontManager::global().measure_text_cached(text, style.size, font)
}

pub fn draw(ctx: &TabContext<'_>) -> bool {
    let painter = ctx.painter;
    let colors = ctx.colors;
    let layout = layout(ctx.grid);
    let (today, minute) = local_now();
    let events = events(today, minute);
    let selected = with_state(|state| {
        if state
            .selected
            .is_some_and(|day| day <= today || day > today + 6)
        {
            state.selected = None;
        }
        state.selected
    });
    draw_strip(
        painter, &layout, &events, today, selected, ctx.hover, colors,
    );
    draw_agenda(
        painter,
        layout.left,
        &events,
        today,
        minute,
        selected,
        ctx.hover,
        colors,
    );
    draw_reminders(painter, layout.reminders, ctx.hover, colors);
    true
}

fn draw_strip(
    painter: Painter<'_>,
    layout: &Layout,
    events: &[Event],
    today: i64,
    selected: Option<i64>,
    hover: Option<DashboardHit>,
    colors: &SemanticColors,
) {
    let active = selected.unwrap_or(today);
    for (index, cell) in layout.days.iter().enumerate() {
        let day = today + index as i64;
        let is_active = day == active;
        let hovered = hover == Some(DashboardHit::Tab(ID_DAY + index as u16));
        if is_active {
            painter.fill_round_rect(*cell, Radius::uniform(14.0), colors.surface_raised);
        } else if hovered {
            painter.fill_round_rect(
                *cell,
                Radius::uniform(14.0),
                colors.surface_raised.with_alpha(140),
            );
        }
        draw_text(
            painter,
            weekday(day),
            typography::LABEL,
            Rect::from_xywh(cell.left, cell.top + 4.0, cell.width(), 16.0),
            Align::Center,
            if is_active {
                colors.text_primary
            } else {
                colors.text_secondary
            },
        );
        let (_, _, date) = crate::ui::dashboard::civil_from_days(day);
        let center = Point::new(cell.center_x(), cell.top + 31.0);
        let number_color = if is_active {
            painter.fill_circle(center, 12.0, colors.text_primary);
            colors.surface_base
        } else if day == today {
            colors.accent_primary
        } else {
            colors.text_primary
        };
        draw_text(
            painter,
            &date.to_string(),
            DAY_NUMBER,
            Rect::from_xywh(cell.left, center.y - 9.0, cell.width(), 18.0),
            Align::Center,
            number_color,
        );
        let mut dots: Vec<Rgba> = events
            .iter()
            .filter(|event| event.day == day)
            .map(|event| event.color)
            .collect();
        dots.truncate(3);
        let spacing = 6.0;
        let start = cell.center_x() - spacing * (dots.len() as f32 - 1.0) / 2.0;
        for (dot, color) in dots.iter().enumerate() {
            painter.fill_circle(
                Point::new(start + spacing * dot as f32, cell.top + 47.0),
                2.0,
                *color,
            );
        }
    }
}

/// One line of the grouped agenda.
enum Line<'a> {
    Header(String),
    Note(String),
    Event(&'a Event),
}

#[allow(clippy::too_many_arguments)]
fn draw_agenda(
    painter: Painter<'_>,
    area: Rect,
    events: &[Event],
    today: i64,
    minute: i64,
    selected: Option<i64>,
    hover: Option<DashboardHit>,
    colors: &SemanticColors,
) {
    if area.width() < 60.0 {
        return;
    }
    let published = crate::ui::dashboard::AGENDA_ROWS.with(|cell| !cell.borrow().is_empty());
    if !published {
        draw_empty(painter, area, hover, colors);
        return;
    }
    let upcoming = upcoming(events, today, minute);
    let mut y = area.top;
    let mut list: Vec<&Event> = Vec::new();
    let mut lines: Vec<Line<'_>> = Vec::new();
    match selected {
        None => {
            let Some((next, rest)) = upcoming.split_first() else {
                draw_message(
                    painter,
                    area,
                    "Nothing else this week",
                    "Your next seven days are free. Enjoy the space.",
                    colors,
                );
                return;
            };
            y = draw_up_next(painter, area, next, today, minute, colors);
            list.extend(rest.iter().copied());
        }
        Some(day) => {
            list.extend(upcoming.iter().copied().filter(|event| event.day >= day));
            if list.first().is_none_or(|event| event.day != day) {
                lines.push(Line::Header(heading(day, today, false)));
                lines.push(Line::Note("No events".to_string()));
            }
        }
    }
    let mut current = None;
    for event in &list {
        if current != Some(event.day) {
            current = Some(event.day);
            lines.push(Line::Header(heading(
                event.day,
                today,
                selected.is_none() && event.day == today,
            )));
        }
        lines.push(Line::Event(event));
    }
    if lines.is_empty() {
        return;
    }
    // Fit lines, keeping room for a "+N more" note when something is cut off.
    let fits = ((area.bottom - y) / LINE_HEIGHT).floor().max(0.0) as usize;
    let mut shown = lines.len();
    if shown > fits {
        shown = fits.saturating_sub(1);
        // Never end on a day heading with nothing under it.
        if shown > 0 && matches!(lines[shown - 1], Line::Header(_)) {
            shown -= 1;
        }
    }
    for line in &lines[..shown] {
        let row = Rect::from_xywh(area.left, y, area.width(), LINE_HEIGHT);
        match line {
            Line::Header(text) => draw_text(
                painter,
                text,
                typography::LABEL,
                Rect::from_xywh(row.left, row.top + 4.0, row.width(), 18.0),
                Align::Start,
                colors.text_secondary,
            ),
            Line::Note(text) => draw_text(
                painter,
                text,
                typography::BODY,
                row,
                Align::Start,
                colors.text_disabled,
            ),
            Line::Event(event) => draw_event_line(painter, row, event, colors),
        }
        y += LINE_HEIGHT;
    }
    let hidden = lines[shown..]
        .iter()
        .filter(|line| matches!(line, Line::Event(_)))
        .count();
    if hidden > 0 {
        draw_text(
            painter,
            &format!("+{hidden} more this week"),
            typography::CAPTION,
            Rect::from_xywh(area.left, y + 2.0, area.width(), LINE_HEIGHT - 4.0),
            Align::Start,
            colors.text_secondary,
        );
    }
}

/// "Later today", "Today · Sat 31 Oct", "Tomorrow · Sun 1 Nov" or "Mon 2 Nov".
fn heading(day: i64, today: i64, later: bool) -> String {
    match day - today {
        0 if later => "Later today".to_string(),
        0 => format!("Today · {}", short_date(day)),
        1 => format!("Tomorrow · {}", short_date(day)),
        _ => short_date(day),
    }
}

/// Draws the "Up next" block and returns the y where the list continues.
fn draw_up_next(
    painter: Painter<'_>,
    area: Rect,
    event: &Event,
    today: i64,
    minute: i64,
    colors: &SemanticColors,
) -> f32 {
    draw_text(
        painter,
        "Up next",
        typography::LABEL,
        Rect::from_xywh(area.left, area.top, area.width(), 16.0),
        Align::Start,
        colors.text_secondary,
    );
    let block = Rect::from_xywh(area.left, area.top + 22.0, area.width(), 44.0);
    painter.fill_round_rect(
        Rect::from_xywh(block.left, block.top + 1.0, 4.0, block.height() - 2.0),
        Radius::uniform(2.0),
        event.color,
    );
    draw_text(
        painter,
        &event.title,
        EVENT_TITLE,
        Rect::from_xywh(block.left + 14.0, block.top, block.width() - 14.0, 24.0),
        Align::Start,
        colors.text_primary,
    );
    draw_text(
        painter,
        &when_text(event, today, minute),
        typography::BODY,
        Rect::from_xywh(
            block.left + 14.0,
            block.top + 26.0,
            block.width() - 14.0,
            18.0,
        ),
        Align::Start,
        colors.text_secondary,
    );
    block.bottom + 4.0
}

fn draw_event_line(painter: Painter<'_>, row: Rect, event: &Event, colors: &SemanticColors) {
    let time = event.minute.map_or_else(|| "All day".to_string(), clock);
    draw_text(
        painter,
        &time,
        TIME,
        Rect::from_xywh(row.left, row.top, 52.0, row.height()),
        Align::Start,
        colors.text_secondary,
    );
    painter.fill_round_rect(
        Rect::from_xywh(row.left + 58.0, row.center_y() - 7.0, 3.0, 14.0),
        Radius::uniform(1.5),
        event.color,
    );
    let title_left = row.left + 70.0;
    let title_width = (row.right - title_left).max(0.0);
    let title_style = typography::BODY;
    draw_text(
        painter,
        &event.title,
        title_style,
        Rect::from_xywh(title_left, row.top, title_width, row.height()),
        Align::Start,
        colors.text_primary,
    );
    if !event.location.is_empty() {
        let used = text_width(&event.title, title_style) + 10.0;
        if used + 40.0 < title_width {
            draw_text(
                painter,
                &event.location,
                typography::CAPTION,
                Rect::from_xywh(title_left + used, row.top, title_width - used, row.height()),
                Align::Start,
                colors.text_secondary,
            );
        }
    }
}

fn draw_message(
    painter: Painter<'_>,
    area: Rect,
    title: &str,
    detail: &str,
    colors: &SemanticColors,
) {
    draw_text(
        painter,
        title,
        typography::SECTION_TITLE,
        Rect::from_xywh(area.left, area.top + 20.0, area.width(), 22.0),
        Align::Start,
        colors.text_primary,
    );
    draw_text(
        painter,
        detail,
        typography::LABEL,
        Rect::from_xywh(area.left, area.top + 44.0, area.width(), 16.0),
        Align::Start,
        colors.text_secondary,
    );
}

/// No agenda rows: explain how to subscribe a calendar feed.
fn draw_empty(
    painter: Painter<'_>,
    area: Rect,
    hover: Option<DashboardHit>,
    colors: &SemanticColors,
) {
    draw_text(
        painter,
        "Nothing in the next 7 days",
        typography::SECTION_TITLE,
        Rect::from_xywh(area.left, area.top + 6.0, area.width(), 22.0),
        Align::Start,
        colors.text_primary,
    );
    for (index, line) in [
        "To see your events here, subscribe to a calendar feed:",
        "copy the ICS link from Google, Outlook or iCloud and paste",
        "it under Settings → General → Calendar feeds.",
    ]
    .iter()
    .enumerate()
    {
        draw_text(
            painter,
            line,
            typography::LABEL,
            Rect::from_xywh(
                area.left,
                area.top + 32.0 + 15.0 * index as f32,
                area.width(),
                15.0,
            ),
            Align::Start,
            colors.text_secondary,
        );
    }
    let button = settings_button(area);
    let hovered = hover == Some(DashboardHit::Tab(ID_SETTINGS));
    painter.fill_round_rect(
        button,
        Radius::uniform(button.height() / 2.0),
        if hovered {
            colors.text_primary.with_alpha(220)
        } else {
            colors.text_primary
        },
    );
    draw_text(
        painter,
        "Open Settings",
        typography::LABEL,
        button,
        Align::Center,
        colors.surface_base,
    );
}

fn draw_reminders(
    painter: Painter<'_>,
    card: Rect,
    hover: Option<DashboardHit>,
    colors: &SemanticColors,
) {
    painter.fill_round_rect(
        card,
        Radius::uniform(radius::CARD + 6.0),
        colors.surface_raised,
    );
    let layout = reminder_layout(card);
    CAPACITY.with(|cell| cell.set(layout.rows.len()));
    with_state(|state| {
        let header = Rect::from_xywh(
            card.left + CARD_PAD,
            card.top + 12.0,
            card.width() - CARD_PAD * 2.0,
            16.0,
        );
        draw_text(
            painter,
            "Reminders",
            typography::LABEL,
            header,
            Align::Start,
            colors.text_secondary,
        );
        let open = state.reminders.iter().filter(|r| !r.done).count();
        let any_done = state.reminders.len() > open;
        let (note, note_color) = if any_done {
            let hovered = hover == Some(DashboardHit::Tab(ID_CLEAR_DONE));
            (
                "Clear done".to_string(),
                if hovered {
                    colors.text_primary
                } else {
                    colors.accent_primary
                },
            )
        } else if open > 0 {
            (open.to_string(), colors.text_secondary)
        } else {
            (String::new(), colors.text_secondary)
        };
        if !note.is_empty() {
            let width = text_width(&note, typography::LABEL) + 1.0;
            draw_text(
                painter,
                &note,
                typography::LABEL,
                Rect::from_xywh(header.right - width, header.top, width, header.height()),
                Align::Start,
                note_color,
            );
        }
        let slots = slots(state, layout.rows.len());
        if slots.is_empty() && state.editing.is_none() {
            if let Some(row) = layout.rows.first() {
                draw_text(
                    painter,
                    "Nothing to remember yet",
                    typography::BODY,
                    Rect::from_xywh(row.left + 8.0, row.top, row.width() - 16.0, row.height()),
                    Align::Start,
                    colors.text_disabled,
                );
            }
        }
        for (slot, (kind, row)) in slots.iter().zip(&layout.rows).enumerate() {
            match *kind {
                Slot::More { hidden, last_page } => {
                    let hovered = hover == Some(DashboardHit::Tab(ID_MORE));
                    if hovered {
                        painter.fill_round_rect(*row, Radius::uniform(8.0), colors.surface_overlay);
                    }
                    let text = if last_page {
                        "Back to the top".to_string()
                    } else {
                        format!("Show more ({hidden})")
                    };
                    draw_text(
                        painter,
                        &text,
                        typography::LABEL,
                        Rect::from_xywh(row.left + 8.0, row.top, row.width() - 16.0, row.height()),
                        Align::Start,
                        colors.accent_primary,
                    );
                }
                Slot::Reminder(index) => {
                    let Some(reminder) = state.reminders.get(index) else {
                        continue;
                    };
                    let toggle_hovered = hover == Some(DashboardHit::Tab(ID_TOGGLE + slot as u16));
                    let delete_hovered = hover == Some(DashboardHit::Tab(ID_DELETE + slot as u16));
                    draw_reminder_row(
                        painter,
                        *row,
                        reminder,
                        toggle_hovered,
                        delete_hovered,
                        colors,
                    );
                }
            }
        }
        draw_add_row(
            painter,
            layout.add,
            state.editing.as_deref(),
            hover == Some(DashboardHit::Tab(ID_ADD)),
            colors,
        );
    });
}

fn draw_reminder_row(
    painter: Painter<'_>,
    row: Rect,
    reminder: &Reminder,
    toggle_hovered: bool,
    delete_hovered: bool,
    colors: &SemanticColors,
) {
    let hovered = toggle_hovered || delete_hovered;
    if hovered {
        painter.fill_round_rect(row, Radius::uniform(8.0), colors.surface_overlay);
    }
    let circle = Point::new(row.left + 16.0, row.center_y());
    if reminder.done {
        painter.fill_circle(circle, 8.0, colors.accent_primary);
        let tick = colors.surface_base;
        painter.stroke_line(
            Point::new(circle.x - 3.5, circle.y + 0.2),
            Point::new(circle.x - 1.0, circle.y + 2.8),
            1.8,
            tick,
            StrokeCap::Round,
        );
        painter.stroke_line(
            Point::new(circle.x - 1.0, circle.y + 2.8),
            Point::new(circle.x + 3.8, circle.y - 2.6),
            1.8,
            tick,
            StrokeCap::Round,
        );
    } else {
        painter.stroke_circle(
            circle,
            7.5,
            1.5,
            if toggle_hovered {
                colors.text_primary
            } else {
                colors.text_secondary
            },
        );
    }
    // Right side: the due note, or a delete cross while hovered.
    let right_width = if hovered {
        let delete = delete_button(row);
        let center = Point::new(delete.center_x(), delete.center_y());
        let color = if delete_hovered {
            colors.state_error
        } else {
            colors.text_secondary
        };
        for (dx, dy) in [(1.0, 1.0), (1.0, -1.0)] {
            painter.stroke_line(
                Point::new(center.x - 4.0 * dx, center.y - 4.0 * dy),
                Point::new(center.x + 4.0 * dx, center.y + 4.0 * dy),
                1.6,
                color,
                StrokeCap::Round,
            );
        }
        delete.width()
    } else if !reminder.due.is_empty() {
        let width = text_width(&reminder.due, TIME).min(row.width() * 0.35) + 1.0;
        draw_text(
            painter,
            &reminder.due,
            TIME,
            Rect::from_xywh(row.right - 8.0 - width, row.top, width, row.height()),
            Align::Start,
            colors.text_secondary,
        );
        width + 8.0
    } else {
        8.0
    };
    let text_left = row.left + 32.0;
    let text_right = row.right - right_width - 8.0;
    let text_rect = Rect::from_ltrb(text_left, row.top, text_right.max(text_left), row.bottom);
    let color = if reminder.done {
        colors.text_disabled
    } else {
        colors.text_primary
    };
    draw_text(
        painter,
        &reminder.text,
        typography::BODY,
        text_rect,
        Align::Start,
        color,
    );
    if reminder.done {
        let width = text_width(&reminder.text, typography::BODY).min(text_rect.width());
        painter.fill_rect(
            Rect::from_xywh(text_left, row.center_y(), width, 1.0),
            colors.text_disabled,
        );
    }
}

fn draw_add_row(
    painter: Painter<'_>,
    row: Rect,
    editing: Option<&str>,
    hovered: bool,
    colors: &SemanticColors,
) {
    let plus = Point::new(row.left + 16.0, row.center_y());
    match editing {
        None => {
            if hovered {
                painter.fill_round_rect(row, Radius::uniform(8.0), colors.surface_overlay);
            }
            let color = if hovered {
                colors.text_primary
            } else {
                colors.text_secondary
            };
            painter.stroke_line(
                Point::new(plus.x - 6.0, plus.y),
                Point::new(plus.x + 6.0, plus.y),
                1.6,
                color,
                StrokeCap::Round,
            );
            painter.stroke_line(
                Point::new(plus.x, plus.y - 6.0),
                Point::new(plus.x, plus.y + 6.0),
                1.6,
                color,
                StrokeCap::Round,
            );
            draw_text(
                painter,
                "Add a reminder",
                typography::BODY,
                Rect::from_xywh(row.left + 32.0, row.top, row.width() - 40.0, row.height()),
                Align::Start,
                color,
            );
        }
        Some(text) => {
            painter.fill_round_rect(row, Radius::uniform(8.0), colors.surface_overlay);
            painter.stroke_round_rect(
                row,
                Radius::uniform(8.0),
                1.0,
                colors.accent_primary.with_alpha(160),
            );
            painter.stroke_circle(plus, 7.5, 1.5, colors.text_disabled);
            let hint = "Enter ↵";
            let hint_width = text_width(hint, typography::CAPTION) + 1.0;
            draw_text(
                painter,
                hint,
                typography::CAPTION,
                Rect::from_xywh(
                    row.right - 10.0 - hint_width,
                    row.top,
                    hint_width,
                    row.height(),
                ),
                Align::Start,
                colors.text_secondary,
            );
            let field = Rect::from_ltrb(
                row.left + 32.0,
                row.top,
                row.right - hint_width - 18.0,
                row.bottom,
            );
            if text.is_empty() {
                painter.fill_rect(
                    Rect::from_xywh(field.left, row.center_y() - 8.0, 1.5, 16.0),
                    colors.text_primary,
                );
                draw_text(
                    painter,
                    "Type, add @ for when",
                    typography::BODY,
                    Rect::from_ltrb(field.left + 5.0, field.top, field.right, field.bottom),
                    Align::Start,
                    colors.text_disabled,
                );
            } else {
                // Show the end of long text so the caret stays visible.
                let style = typography::BODY;
                let room = (field.width() - 4.0).max(0.0);
                let mut visible = text;
                while !visible.is_empty() && text_width(visible, style) > room {
                    let mut chars = visible.chars();
                    chars.next();
                    visible = chars.as_str();
                }
                draw_text(
                    painter,
                    visible,
                    style,
                    field,
                    Align::Start,
                    colors.text_primary,
                );
                let caret = field.left + text_width(visible, style) + 1.0;
                painter.fill_rect(
                    Rect::from_xywh(caret, row.center_y() - 8.0, 1.5, 16.0),
                    colors.text_primary,
                );
            }
        }
    }
}
