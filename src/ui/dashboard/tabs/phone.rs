//! Phone tab: Notchy Link, the lasting connection to the Notchy phone app.
//!
//! Not paired: a pairing QR on the left and what linking gives you on the right.
//! Paired: the phone card (name, battery, Find phone, Send clipboard, the phone's music,
//! three switches, "Pair a new phone") on the left; the phone's notifications on the right,
//! with one-tap copy for codes. State lives in `core::phone_link`.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use winisland_platform::Key;
use winisland_render::tokens::{TextStyle, radius, typography};
use winisland_render::{Point, Radius, Rect, Rgba};

use super::TabContext;
use super::shelf::kit::{self, style, text, text_center, text_right, text_width};
use crate::core::friends::{self, Friend};
use crate::core::phone_link::{self, PhoneNotification};
use crate::ui::dashboard::DashboardHit;

const ID_FIND: u16 = 1600;
const ID_SEND_CLIP: u16 = 1601;
const ID_PAIR_AGAIN: u16 = 1602;
const ID_CLEAR: u16 = 1603;
const ID_SEND_FILE: u16 = 1604;
const ID_TOGGLE: u16 = 1610;
const ID_MEDIA: u16 = 1620;
const ID_COPY_CODE: u16 = 1640;
const ID_WAVE: u16 = 1660;
const MAX_FRIENDS: usize = 6;
const MAX_ROWS: u16 = 12;

const GAP: f32 = 10.0;
const PAD: f32 = 14.0;
const ROW_H: f32 = 44.0;
const SWITCH_H: f32 = 26.0;
const FEEDBACK: Duration = Duration::from_millis(1600);

const TITLE: TextStyle = style(16.0, 650);
const NAME: TextStyle = style(13.0, 600);
const BUTTON: TextStyle = style(12.0, 600);

const RING: &str = "M12 3 A6 6 0 0 1 18 9 V14 L20 17 H4 L6 14 V9 A6 6 0 0 1 12 3 Z M10 20 H14";
const COPY: &str = "M9 9 H19 V19 H9 Z M5 15 V5 H15";
const PREV: &str = "M17 6 L9 12 L17 18 Z M7 6 V18";
const NEXT: &str = "M7 6 L15 12 L7 18 Z M17 6 V18";
const PLAY: &str = "M8 5 L19 12 L8 19 Z";
const PAUSE: &str = "M8 5 V19 M16 5 V19";
const PHONE: &str = "M8 2.5 H16 V21.5 H8 Z M11 18.5 H13";
const BELL: &str = "M12 4 A5 5 0 0 1 17 9 V13 L19 16 H5 L7 13 V9 A5 5 0 0 1 12 4 Z M10 19 H14";

const SWITCHES: [(&str, &str); 3] = [
    ("Clipboard sync", "Copy on one, paste on the other"),
    ("Phone alerts on PC", "Messages pop up here; codes copy themselves"),
    ("Lock PC when phone leaves", "Walk away with your phone and the PC locks"),
];

struct Qr {
    code: String,
    modules: Vec<bool>,
    width: usize,
}

#[derive(Default)]
struct State {
    feedback: Option<(u16, Instant)>,
    code: Option<String>,
    code_at: Option<Instant>,
    notifications: Vec<PhoneNotification>,
    friends: Vec<Friend>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    static QR: RefCell<Option<Qr>> = const { RefCell::new(None) };
    static LAST_GRID: Cell<Rect> = const { Cell::new(Rect::from_xywh(0.0, 0.0, 720.0, 230.0)) };
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|cell| f(&mut cell.borrow_mut()))
}

struct Layout {
    left: Rect,
    right: Rect,
}

fn layout(grid: Rect) -> Layout {
    let left_w = (grid.width() * 0.52).clamp(300.0, 420.0).min(grid.width());
    let left = Rect::from_xywh(grid.left, grid.top, left_w, grid.height());
    let right = Rect::from_ltrb(left.right + GAP, grid.top, grid.right, grid.bottom);
    Layout { left, right }
}

/// The paired card's pieces.
struct PhoneCard {
    find: Rect,
    send_clip: Rect,
    send_file: Rect,
    media: Option<[Rect; 3]>,
    switches: [Rect; 3],
    pair_again: Rect,
}

fn phone_card(left: Rect, has_media: bool) -> PhoneCard {
    let inner = left.inset(PAD);
    let buttons_top = inner.top + 50.0;
    let third = (inner.width() - 12.0) / 3.0;
    let find = Rect::from_xywh(inner.left, buttons_top, third, 30.0);
    let send_clip = Rect::from_xywh(inner.left + third + 6.0, buttons_top, third, 30.0);
    let send_file = Rect::from_xywh(inner.left + (third + 6.0) * 2.0, buttons_top, third, 30.0);
    let mut top = find.bottom + 10.0;
    let media = has_media.then(|| {
        let row = top;
        top += 34.0;
        let right = inner.right;
        [
            Rect::from_xywh(right - 90.0, row, 28.0, 28.0),
            Rect::from_xywh(right - 60.0, row, 28.0, 28.0),
            Rect::from_xywh(right - 30.0, row, 28.0, 28.0),
        ]
    });
    let switches = std::array::from_fn(|index| {
        Rect::from_xywh(inner.left, top + index as f32 * (SWITCH_H + 6.0), inner.width(), SWITCH_H)
    });
    let pair_w = text_width("Pair a new phone", typography::CAPTION) + 8.0;
    let pair_again = Rect::from_xywh(inner.left, inner.bottom - 16.0, pair_w, 16.0);
    PhoneCard {
        find,
        send_clip,
        send_file,
        media,
        switches,
        pair_again,
    }
}

fn rows_rect(right: Rect, friends: bool) -> Rect {
    let inner = right.inset(PAD);
    let bottom = if friends { right.bottom - 46.0 } else { right.bottom - 8.0 };
    Rect::from_ltrb(inner.left - 6.0, inner.top + 34.0, inner.right + 6.0, bottom)
}

/// "Nearby" chips along the bottom of the right card, one per friend's PC.
fn friend_chips(right: Rect, friends: &[Friend]) -> Vec<Rect> {
    let inner = right.inset(PAD);
    let top = right.bottom - PAD - 28.0;
    let mut left = inner.left + text_width("Nearby", typography::CAPTION) + 10.0;
    let mut chips = Vec::new();
    for friend in friends.iter().take(MAX_FRIENDS) {
        let width = text_width(&friend.name, BUTTON) + 40.0;
        if left + width > inner.right {
            break;
        }
        chips.push(Rect::from_xywh(left, top, width, 28.0));
        left += width + 6.0;
    }
    chips
}

const WAVE: &str = "M7 12 V6.5 A1.5 1.5 0 0 1 10 6.5 V11 M10 10 V4.5 A1.5 1.5 0 0 1 13 4.5 V11 M13 10.5 V5.5 A1.5 1.5 0 0 1 16 5.5 V12 M16 11 V8.5 A1.5 1.5 0 0 1 19 8.5 V14 A7 7 0 0 1 12 21 A6 6 0 0 1 7 17 L4.5 13 A1.5 1.5 0 0 1 7 12";

fn draw_friends(ctx: &TabContext<'_>, layout: &Layout, state: &State) {
    if state.friends.is_empty() {
        return;
    }
    let painter = ctx.painter;
    let colors = ctx.colors;
    let inner = layout.right.inset(PAD);
    let chips = friend_chips(layout.right, &state.friends);
    let top = layout.right.bottom - PAD - 28.0;
    text(
        painter,
        "Nearby",
        typography::CAPTION,
        Rect::from_xywh(inner.left, top, 60.0, 28.0),
        colors.text_secondary,
    );
    for (index, chip) in chips.iter().enumerate() {
        let id = ID_WAVE + index as u16;
        let waved = flashed(state, id, ctx.now);
        let hover = hovered(ctx, id);
        painter.fill_round_rect(
            *chip,
            Radius::uniform(14.0),
            if waved {
                colors.accent_soft
            } else if hover {
                colors.surface_interactive
            } else {
                colors.surface_overlay
            },
        );
        kit::glyph(
            painter,
            WAVE,
            kit::square(Point::new(chip.left + 15.0, chip.center_y()), 14.0),
            if waved { colors.accent_primary } else { colors.text_primary },
        );
        text(
            painter,
            &state.friends[index].name,
            BUTTON,
            Rect::from_ltrb(chip.left + 26.0, chip.top, chip.right - 6.0, chip.bottom),
            colors.text_primary,
        );
    }
}

fn row_rect(rows: Rect, slot: usize) -> Rect {
    Rect::from_xywh(rows.left, rows.top + slot as f32 * ROW_H, rows.width(), ROW_H - 2.0)
}

fn row_count(rows: Rect, items: usize) -> usize {
    ((rows.height() / ROW_H).floor().max(0.0) as usize)
        .min(MAX_ROWS as usize)
        .min(items)
}

fn copy_chip(row: Rect, code: &str) -> Rect {
    let width = text_width(code, BUTTON) + 34.0;
    Rect::from_xywh(row.right - width - 6.0, row.center_y() - 13.0, width, 26.0)
}

fn clear_rect(right: Rect) -> Rect {
    let inner = right.inset(PAD);
    let width = text_width("Clear", BUTTON) + 20.0;
    Rect::from_xywh(inner.right - width, inner.top, width, 26.0)
}

fn grow(rect: Rect) -> Rect {
    Rect::from_ltrb(rect.left - 4.0, rect.top - 2.0, rect.right + 4.0, rect.bottom + 2.0)
}

fn hovered(ctx: &TabContext<'_>, id: u16) -> bool {
    ctx.hover == Some(DashboardHit::Tab(id))
}

fn flashed(state: &State, id: u16, now: Instant) -> bool {
    state
        .feedback
        .is_some_and(|(which, at)| which == id && now.saturating_duration_since(at) < FEEDBACK)
}

fn card(ctx: &TabContext<'_>, rect: Rect) {
    ctx.painter
        .fill_round_rect(rect, Radius::uniform(radius::CARD), ctx.colors.surface_raised);
    ctx.painter.stroke_round_rect(
        rect.inset(0.5),
        Radius::uniform(radius::CARD),
        1.0,
        ctx.colors.border_subtle,
    );
}

fn button(ctx: &TabContext<'_>, rect: Rect, svg: &'static str, label: &str, hover: bool, primary: bool) {
    let colors = ctx.colors;
    let fill = match (primary, hover) {
        (true, false) => colors.accent_primary,
        (true, true) => colors.accent_primary.with_alpha(220),
        (false, false) => colors.surface_overlay,
        (false, true) => colors.surface_interactive,
    };
    ctx.painter
        .fill_round_rect(rect, Radius::uniform(rect.height() / 2.0), fill);
    let color = if primary {
        Rgba::WHITE
    } else if hover {
        colors.text_primary
    } else {
        colors.text_secondary
    };
    let label_w = text_width(label, BUTTON);
    let start = rect.center_x() - (label_w + 18.0) / 2.0;
    kit::glyph(
        ctx.painter,
        svg,
        kit::square(Point::new(start + 7.0, rect.center_y()), 14.0),
        color,
    );
    text(
        ctx.painter,
        label,
        BUTTON,
        Rect::from_xywh(start + 18.0, rect.top, label_w + 2.0, rect.height()),
        color,
    );
}

/// Draws the tab.
pub fn draw(ctx: &TabContext<'_>) -> bool {
    LAST_GRID.with(|cell| cell.set(ctx.grid));
    let layout = layout(ctx.grid);
    card(ctx, layout.left);
    card(ctx, layout.right);
    with_state(|state| {
        state.notifications = phone_link::notifications();
        state.friends = friends::nearby();
        if phone_link::paired() {
            draw_phone(ctx, &layout, state);
            draw_notifications(ctx, &layout, state);
        } else {
            draw_pairing(ctx, &layout, state);
            draw_features(ctx, &layout);
        }
        draw_friends(ctx, &layout, state);
    });
    true
}

fn draw_pairing(ctx: &TabContext<'_>, layout: &Layout, state: &mut State) {
    let painter = ctx.painter;
    let colors = ctx.colors;
    // The address can change; refresh the code now and then.
    if state
        .code_at
        .is_none_or(|at| ctx.now.saturating_duration_since(at) > Duration::from_secs(20))
    {
        state.code = phone_link::pairing_code();
        state.code_at = Some(ctx.now);
    }
    let inner = layout.left.inset(PAD);
    let Some(code) = state.code.clone() else {
        text_center(
            painter,
            "Linking needs Wi-Fi",
            typography::SECTION_TITLE,
            Rect::from_xywh(inner.left, inner.center_y() - 20.0, inner.width(), 20.0),
            colors.text_primary,
        );
        text_center(
            painter,
            "Connect this PC and your phone to the same network",
            typography::CAPTION,
            Rect::from_xywh(inner.left, inner.center_y() + 4.0, inner.width(), 16.0),
            colors.text_secondary,
        );
        return;
    };
    let side = inner.height().min(inner.width() * 0.45).max(0.0);
    let qr = Rect::from_xywh(inner.left, inner.center_y() - side / 2.0, side, side);
    painter.fill_round_rect(qr, Radius::uniform(radius::ROW + 2.0), Rgba::WHITE);
    draw_qr(ctx, qr.inset(6.0), &code);
    let info = Rect::from_ltrb(qr.right + PAD, inner.top, inner.right, inner.bottom);
    text(
        painter,
        "Link your phone",
        TITLE,
        Rect::from_xywh(info.left, info.top, info.width(), 22.0),
        colors.text_primary,
    );
    let steps = ["Open Notchy on the phone", "Tap Link this PC", "Scan this code"];
    let mut top = info.top + 34.0;
    for (index, step) in steps.iter().enumerate() {
        let center = Point::new(info.left + 10.0, top + 10.0);
        painter.fill_circle(center, 10.0, colors.accent_soft);
        text_center(
            painter,
            &(index + 1).to_string(),
            style(11.0, 650),
            kit::square(center, 20.0),
            colors.accent_primary,
        );
        text(
            painter,
            step,
            typography::LABEL,
            Rect::from_ltrb(info.left + 28.0, top, info.right, top + 20.0),
            colors.text_primary,
        );
        top += 28.0;
    }
    text(
        painter,
        "Encrypted · same Wi-Fi · stays linked",
        typography::CAPTION,
        Rect::from_ltrb(info.left, info.bottom - 16.0, info.right, info.bottom),
        colors.text_secondary,
    );
}

fn draw_features(ctx: &TabContext<'_>, layout: &Layout) {
    let painter = ctx.painter;
    let colors = ctx.colors;
    let inner = layout.right.inset(PAD);
    text(
        painter,
        "One Notchy, two screens",
        TITLE,
        Rect::from_xywh(inner.left, inner.top, inner.width(), 22.0),
        colors.text_primary,
    );
    let features = [
        "Clipboard flows between phone and PC",
        "Phone messages and codes show up here",
        "Music on either one, controlled from both",
        "Phone becomes a touchpad and remote",
        "Find my phone, and lock when you walk away",
        "The robot follows you from PC to phone",
        "Unlock this PC with your phone's fingerprint",
    ];
    let mut top = inner.top + 32.0;
    for feature in features {
        if top + 20.0 > inner.bottom - 34.0 {
            break;
        }
        painter.fill_circle(Point::new(inner.left + 4.0, top + 9.0), 3.0, colors.accent_primary);
        text(
            painter,
            feature,
            typography::LABEL,
            Rect::from_ltrb(inner.left + 16.0, top, inner.right, top + 18.0),
            colors.text_secondary,
        );
        top += 24.0;
    }
}

fn draw_phone(ctx: &TabContext<'_>, layout: &Layout, state: &State) {
    let painter = ctx.painter;
    let colors = ctx.colors;
    let connection = phone_link::connection();
    let pairing = phone_link::pairing().unwrap_or_default();
    let media = phone_link::phone_media();
    let parts = phone_card(layout.left, media.is_some());
    let inner = layout.left.inset(PAD);

    // Header: phone glyph, name, status line.
    let badge = Rect::from_xywh(inner.left, inner.top, 36.0, 36.0);
    painter.fill_round_rect(badge, Radius::uniform(10.0), colors.surface_overlay);
    kit::glyph(
        painter,
        PHONE,
        kit::square(Point::new(badge.center_x(), badge.center_y()), 20.0),
        colors.text_primary,
    );
    let name = connection
        .as_ref()
        .map(|connection| connection.name.clone())
        .or(pairing.device.clone())
        .unwrap_or_else(|| "Phone".to_string());
    text(
        painter,
        &name,
        TITLE,
        Rect::from_ltrb(badge.right + 10.0, inner.top - 1.0, inner.right, inner.top + 19.0),
        colors.text_primary,
    );
    let status_rect = Rect::from_ltrb(badge.right + 10.0, inner.top + 19.0, inner.right, inner.top + 36.0);
    let dot = Point::new(status_rect.left + 4.0, status_rect.center_y());
    match &connection {
        Some(connection) => {
            painter.fill_circle(dot, 3.5, colors.state_success);
            let status = match connection.battery {
                Some((percent, true)) => format!("Connected · {percent}% charging"),
                Some((percent, false)) => format!("Connected · {percent}%"),
                None => "Connected".to_string(),
            };
            text(
                painter,
                &status,
                typography::CAPTION,
                Rect::from_ltrb(status_rect.left + 12.0, status_rect.top, status_rect.right, status_rect.bottom),
                colors.text_secondary,
            );
        }
        None => {
            painter.fill_circle(dot, 3.5, colors.text_disabled);
            text(
                painter,
                "Not nearby · open Notchy on the phone",
                typography::CAPTION,
                Rect::from_ltrb(status_rect.left + 12.0, status_rect.top, status_rect.right, status_rect.bottom),
                colors.text_secondary,
            );
        }
    }

    let online = connection.is_some();
    let ringing = flashed(state, ID_FIND, ctx.now);
    button(
        ctx,
        parts.find,
        RING,
        if ringing { "Ringing…" } else { "Find" },
        online && hovered(ctx, ID_FIND),
        online,
    );
    let sent = flashed(state, ID_SEND_CLIP, ctx.now);
    button(
        ctx,
        parts.send_clip,
        if sent { kit::CHECK } else { COPY },
        if sent { "Sent" } else { "Clipboard" },
        online && hovered(ctx, ID_SEND_CLIP),
        false,
    );
    let sending = phone_link::file_progress();
    let file_label = match sending {
        Some(percent) => format!("{percent}%"),
        None if flashed(state, ID_SEND_FILE, ctx.now) => "Sent".to_string(),
        None => "File".to_string(),
    };
    button(
        ctx,
        parts.send_file,
        kit::FOLDER,
        &file_label,
        online && hovered(ctx, ID_SEND_FILE),
        false,
    );

    if let (Some(media), Some(controls)) = (&media, parts.media) {
        let row = Rect::from_ltrb(inner.left, controls[0].top, controls[0].left - 8.0, controls[0].bottom);
        text(
            painter,
            &media.title,
            NAME,
            Rect::from_ltrb(row.left, row.top - 1.0, row.right, row.top + 15.0),
            colors.text_primary,
        );
        text(
            painter,
            &format!("On your phone · {}", media.artist),
            typography::CAPTION,
            Rect::from_ltrb(row.left, row.top + 15.0, row.right, row.bottom),
            colors.text_secondary,
        );
        let glyphs = [PREV, if media.playing { PAUSE } else { PLAY }, NEXT];
        for (index, rect) in controls.iter().enumerate() {
            kit::icon_button(
                painter,
                *rect,
                glyphs[index],
                hovered(ctx, ID_MEDIA + index as u16),
                true,
                colors,
            );
        }
    }

    let values = [pairing.clipboard_sync, pairing.alerts, pairing.lock_on_leave];
    for (index, rect) in parts.switches.iter().enumerate() {
        let (label, detail) = SWITCHES[index];
        let hover = hovered(ctx, ID_TOGGLE + index as u16);
        if hover {
            painter.fill_round_rect(grow(*rect), Radius::uniform(radius::ROW), colors.surface_overlay);
        }
        let track = Rect::from_xywh(rect.right - 34.0, rect.center_y() - 9.0, 34.0, 18.0);
        let on = values[index];
        painter.fill_round_rect(
            track,
            Radius::uniform(9.0),
            if on { colors.accent_primary } else { colors.surface_interactive },
        );
        let knob = if on { track.right - 9.0 } else { track.left + 9.0 };
        painter.fill_circle(Point::new(knob, track.center_y()), 7.0, Rgba::WHITE);
        let label_w = text_width(label, NAME) + 2.0;
        text(
            painter,
            label,
            NAME,
            Rect::from_xywh(rect.left, rect.top, label_w, rect.height()),
            colors.text_primary,
        );
        text(
            painter,
            detail,
            typography::CAPTION,
            Rect::from_ltrb(rect.left + label_w + 8.0, rect.top + 1.0, track.left - 8.0, rect.bottom),
            colors.text_secondary,
        );
    }

    text(
        painter,
        "Pair a new phone",
        typography::CAPTION,
        parts.pair_again,
        if hovered(ctx, ID_PAIR_AGAIN) {
            colors.text_primary
        } else {
            colors.text_disabled
        },
    );
}

fn ago(at: Instant, now: Instant) -> String {
    let seconds = now.saturating_duration_since(at).as_secs();
    match seconds {
        0..60 => "now".to_string(),
        60..3600 => format!("{}m", seconds / 60),
        _ => format!("{}h", seconds / 3600),
    }
}

fn draw_notifications(ctx: &TabContext<'_>, layout: &Layout, state: &State) {
    let painter = ctx.painter;
    let colors = ctx.colors;
    let inner = layout.right.inset(PAD);
    text(
        painter,
        "From your phone",
        TITLE,
        Rect::from_xywh(inner.left, inner.top, inner.width() - 80.0, 26.0),
        colors.text_primary,
    );
    let rows = rows_rect(layout.right, !state.friends.is_empty());
    if state.notifications.is_empty() {
        let center = rows.center_y() - 10.0;
        kit::glyph(
            painter,
            BELL,
            kit::square(Point::new(rows.center_x(), center - 14.0), 26.0),
            colors.text_disabled,
        );
        text_center(
            painter,
            "Phone notifications land here",
            typography::LABEL,
            Rect::from_xywh(rows.left, center + 6.0, rows.width(), 18.0),
            colors.text_secondary,
        );
        text_center(
            painter,
            "Allow notification access in the phone app",
            typography::CAPTION,
            Rect::from_xywh(rows.left, center + 26.0, rows.width(), 16.0),
            colors.text_disabled,
        );
        return;
    }
    let clear = clear_rect(layout.right);
    let hover = hovered(ctx, ID_CLEAR);
    if hover {
        painter.fill_round_rect(clear, Radius::uniform(13.0), colors.surface_overlay);
    }
    text_center(
        painter,
        "Clear",
        BUTTON,
        clear,
        if hover { colors.text_primary } else { colors.text_secondary },
    );
    let shown = row_count(rows, state.notifications.len());
    for (slot, notification) in state.notifications.iter().take(shown).enumerate() {
        let row = row_rect(rows, slot);
        let initial = notification
            .app
            .chars()
            .next()
            .map_or("·".to_string(), |first| first.to_uppercase().to_string());
        let icon = Point::new(row.left + 22.0, row.center_y());
        painter.fill_circle(icon, 14.0, colors.surface_overlay);
        text_center(painter, &initial, style(13.0, 650), kit::square(icon, 28.0), colors.text_primary);
        let left = row.left + 44.0;
        let chip = notification.otp.as_ref().map(|code| copy_chip(row, code));
        let right = chip.map_or(row.right - 8.0, |chip| chip.left - 8.0);
        let when_w = text_right(
            painter,
            &ago(notification.at, ctx.now),
            typography::CAPTION,
            Rect::from_ltrb(left, row.top + 4.0, right, row.top + 20.0),
            colors.text_disabled,
        );
        let title = if notification.title.is_empty() {
            notification.app.clone()
        } else {
            format!("{} · {}", notification.app, notification.title)
        };
        text(
            painter,
            &title,
            NAME,
            Rect::from_ltrb(left, row.top + 3.0, right - when_w - 6.0, row.top + 21.0),
            colors.text_primary,
        );
        text(
            painter,
            &notification.text.replace('\n', " "),
            typography::CAPTION,
            Rect::from_ltrb(left, row.top + 21.0, right, row.bottom - 2.0),
            colors.text_secondary,
        );
        if let (Some(chip), Some(code)) = (chip, &notification.otp) {
            let id = ID_COPY_CODE + slot as u16;
            let copied = flashed(state, id, ctx.now);
            let fill = if hovered(ctx, id) {
                colors.accent_primary.with_alpha(220)
            } else {
                colors.accent_primary
            };
            painter.fill_round_rect(chip, Radius::uniform(13.0), fill);
            kit::glyph(
                painter,
                if copied { kit::CHECK } else { COPY },
                kit::square(Point::new(chip.left + 14.0, chip.center_y()), 12.0),
                Rgba::WHITE,
            );
            text(
                painter,
                code,
                BUTTON,
                Rect::from_ltrb(chip.left + 24.0, chip.top, chip.right - 4.0, chip.bottom),
                Rgba::WHITE,
            );
        }
    }
}

/// Draws the QR modules centred in `area`, snapped to whole pixels.
fn draw_qr(ctx: &TabContext<'_>, area: Rect, code: &str) {
    QR.with(|cell| {
        let mut cached = cell.borrow_mut();
        if cached.as_ref().is_none_or(|qr| qr.code != code) {
            *cached = qrcode::QrCode::new(code.as_bytes()).ok().map(|qr| Qr {
                code: code.to_string(),
                width: qr.width(),
                modules: qr
                    .to_colors()
                    .into_iter()
                    .map(|color| color == qrcode::Color::Dark)
                    .collect(),
            });
        }
        let Some(qr) = cached.as_ref() else {
            return;
        };
        let span = qr.width as f32 + 4.0;
        let fine = area.width().min(area.height()) / span;
        let module = if fine >= 2.0 { fine.floor() } else { fine };
        let size = module * qr.width as f32;
        let left = (area.center_x() - size / 2.0).round();
        let top = (area.center_y() - size / 2.0).round();
        for row in 0..qr.width {
            let mut column = 0;
            while column < qr.width {
                if !qr.modules[row * qr.width + column] {
                    column += 1;
                    continue;
                }
                let start = column;
                while column < qr.width && qr.modules[row * qr.width + column] {
                    column += 1;
                }
                ctx.painter.fill_rect(
                    Rect::from_xywh(
                        left + start as f32 * module,
                        top + row as f32 * module,
                        (column - start) as f32 * module,
                        module,
                    ),
                    Rgba::BLACK,
                );
            }
        }
    });
}

pub fn custom_layout() -> bool {
    true
}

/// Hit test in page coordinates.
pub fn hit(grid: Rect, point: Point) -> Option<DashboardHit> {
    if !grid.contains(point) {
        return None;
    }
    LAST_GRID.with(|cell| cell.set(grid));
    let layout = layout(grid);
    let friends = with_state(|state| state.friends.clone());
    if let Some(index) = friend_chips(layout.right, &friends)
        .iter()
        .position(|chip| chip.contains(point))
    {
        return Some(DashboardHit::Tab(ID_WAVE + index as u16));
    }
    if !phone_link::paired() {
        return None;
    }
    let parts = phone_card(layout.left, phone_link::phone_media().is_some());
    let online = phone_link::connection().is_some();
    let id = if online && parts.find.contains(point) {
        Some(ID_FIND)
    } else if online && parts.send_clip.contains(point) {
        Some(ID_SEND_CLIP)
    } else if online && parts.send_file.contains(point) {
        Some(ID_SEND_FILE)
    } else if parts.pair_again.contains(point) {
        Some(ID_PAIR_AGAIN)
    } else if let Some(index) = parts.switches.iter().position(|rect| grow(*rect).contains(point)) {
        Some(ID_TOGGLE + index as u16)
    } else if let Some(index) = parts
        .media
        .and_then(|controls| controls.iter().position(|rect| rect.contains(point)))
    {
        Some(ID_MEDIA + index as u16)
    } else if clear_rect(layout.right).contains(point) {
        Some(ID_CLEAR)
    } else {
        let rows = rows_rect(layout.right, !friends.is_empty());
        with_state(|state| {
            let shown = row_count(rows, state.notifications.len());
            (0..shown).find_map(|slot| {
                let code = state.notifications[slot].otp.as_ref()?;
                copy_chip(row_rect(rows, slot), code)
                    .contains(point)
                    .then_some(ID_COPY_CODE + slot as u16)
            })
        })
    };
    id.map(DashboardHit::Tab)
}

/// Applies a click from [`hit`]. `true` when handled.
pub fn apply(hit: DashboardHit) -> bool {
    let DashboardHit::Tab(id) = hit else {
        return false;
    };
    if !(1600..=1699).contains(&id) {
        return false;
    }
    let now = Instant::now();
    with_state(|state| match id {
        ID_FIND => {
            if phone_link::ring_phone() {
                state.feedback = Some((id, now));
            }
        }
        ID_SEND_CLIP => {
            if let Some(text) = winisland_platform_windows::get_clipboard_text()
                && phone_link::send_clip(&text)
            {
                state.feedback = Some((id, now));
            }
        }
        ID_SEND_FILE => {
            if phone_link::file_progress().is_none() {
                std::thread::spawn(|| {
                    if let Some(paths) = rfd::FileDialog::new()
                        .set_title("Send to phone")
                        .pick_files()
                    {
                        phone_link::send_files(paths);
                    }
                });
            }
        }
        ID_PAIR_AGAIN => {
            phone_link::pair_again();
            state.code = None;
            state.code_at = None;
        }
        ID_CLEAR => phone_link::clear_notifications(),
        _ if (ID_TOGGLE..ID_TOGGLE + 3).contains(&id) => {
            phone_link::update_settings(|pairing| match id - ID_TOGGLE {
                0 => pairing.clipboard_sync = !pairing.clipboard_sync,
                1 => pairing.alerts = !pairing.alerts,
                _ => pairing.lock_on_leave = !pairing.lock_on_leave,
            });
        }
        _ if (ID_MEDIA..ID_MEDIA + 3).contains(&id) => {
            phone_link::phone_media_command(["prev", "toggle", "next"][(id - ID_MEDIA) as usize]);
        }
        _ if (ID_WAVE..ID_WAVE + MAX_FRIENDS as u16).contains(&id) => {
            if let Some(friend) = state.friends.get((id - ID_WAVE) as usize)
                && friends::wave(friend.address)
            {
                state.feedback = Some((id, now));
            }
        }
        _ if (ID_COPY_CODE..ID_COPY_CODE + MAX_ROWS).contains(&id) => {
            let slot = (id - ID_COPY_CODE) as usize;
            if let Some(code) = state.notifications.get(slot).and_then(|item| item.otp.clone())
                && winisland_platform_windows::set_clipboard_text(&code).is_ok()
            {
                state.feedback = Some((id, now));
            }
        }
        _ => {}
    });
    true
}

pub fn wants_keyboard() -> bool {
    false
}

pub fn handle_key(_key: &Key) -> bool {
    false
}

pub fn poll(_now: Instant) {}

