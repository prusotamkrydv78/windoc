//! Drop tab: send files between phone and PC over the local network.
//!
//! Left card: the QR code of the live session on a white tile, a countdown pill, "New link",
//! the URL with a copy button and a Wi-Fi hint. Right card: this run's transfers (newest
//! first, with progress while a file is moving), or a three-step explainer when empty.
//! The session itself is owned by the app (`App::poll_drop`); this tab reads
//! `core::drop_server`'s shared status and asks it for a new link.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::time::{Duration, Instant};

use winisland_platform::Key;
use winisland_render::tokens::{TextStyle, radius, typography};
use winisland_render::{Point, Radius, Rect, Rgba};

use super::TabContext;
use super::shelf::kit::{self, format_bytes, style, text, text_center, text_right, text_width};
use crate::core::drop_server::{self, Direction, Transfer, TransferState};
use crate::ui::dashboard::DashboardHit;

const ID_NEW_LINK: u16 = 1300;
const ID_COPY: u16 = 1301;
const ID_OPEN_FOLDER: u16 = 1302;
const ID_ROW: u16 = 1310;
const ID_REVEAL: u16 = 1340;
const MAX_ROWS: u16 = 20;

const GAP: f32 = 10.0;
const PAD: f32 = 14.0;
const ROW_H: f32 = 38.0;
const FEEDBACK: Duration = Duration::from_millis(1600);

const TITLE: TextStyle = style(16.0, 650);
const NAME: TextStyle = style(13.0, 500);
const URL: TextStyle = TextStyle {
    size: 11.0,
    weight: 400,
    tabular: true,
};
const PILL: TextStyle = TextStyle {
    size: 12.0,
    weight: 600,
    tabular: true,
};

const QR_MARK: &str = "M4 4 H10 V10 H4 Z M14 4 H20 V10 H14 Z M4 14 H10 V20 H4 Z M14 14 H16 M18 14 H20 V16 M14 18 V20 H16 M18 18 H20 V20";
const REFRESH: &str = "M19 12 A7 7 0 1 1 16.5 6.6 M17 3 V7 H13";
const COPY: &str = "M9 9 H19 V19 H9 Z M5 15 V5 H15";
const WIFI: &str = "M3.5 9.5 A12 12 0 0 1 20.5 9.5 M6.5 12.8 A7.5 7.5 0 0 1 17.5 12.8 M9.5 16 A3 3 0 0 1 14.5 16 M12 19.2 V19.3";
const ARROW_DOWN: &str = "M12 5 V18 M7 13 L12 18 L17 13";
const ARROW_UP: &str = "M12 19 V6 M7 11 L12 6 L17 11";

struct Qr {
    url: String,
    modules: Vec<bool>,
    width: usize,
}

#[derive(Default)]
struct State {
    copied: Option<Instant>,
    renewing: Option<Instant>,
    /// Snapshot of the transfers drawn last, so clicks map onto the same rows.
    transfers: Vec<Transfer>,
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
    qr: Rect,
    /// The text column right of the QR tile.
    info: Rect,
    new_link: Rect,
    url: Rect,
    copy: Rect,
    wifi: Rect,
    header: Rect,
    open_folder: Rect,
    rows: Rect,
}

fn layout(grid: Rect) -> Layout {
    let left_w = (grid.width() * 0.56).clamp(300.0, 440.0).min(grid.width());
    let left = Rect::from_xywh(grid.left, grid.top, left_w, grid.height());
    let right = Rect::from_ltrb(left.right + GAP, grid.top, grid.right, grid.bottom);
    let inner = left.inset(PAD);
    let side = inner.height().min(inner.width() * 0.5).max(0.0);
    let qr = Rect::from_xywh(inner.left, inner.center_y() - side / 2.0, side, side);
    let info = Rect::from_ltrb(qr.right + PAD, inner.top, inner.right, inner.bottom);
    let new_link_w = text_width("New link", typography::LABEL) + 40.0;
    let new_link = Rect::from_xywh(info.left, info.top + 76.0, new_link_w.min(info.width()), 28.0);
    let wifi = Rect::from_ltrb(info.left, info.bottom - 16.0, info.right, info.bottom);
    let url = Rect::from_ltrb(info.left, wifi.top - 34.0, info.right, wifi.top - 6.0);
    let copy = Rect::from_ltrb(url.right - 28.0, url.top, url.right, url.bottom);
    let right_inner = right.inset(PAD);
    let header = Rect::from_xywh(right_inner.left, right_inner.top, right_inner.width(), 26.0);
    let folder_w = text_width("Open folder", typography::LABEL) + 38.0;
    let open_folder = Rect::from_xywh(header.right - folder_w, header.top, folder_w, 26.0);
    let rows = Rect::from_ltrb(
        right_inner.left - 6.0,
        header.bottom + 8.0,
        right_inner.right + 6.0,
        right.bottom - 8.0,
    );
    Layout {
        left,
        right,
        qr,
        info,
        new_link,
        url,
        copy,
        wifi,
        header,
        open_folder,
        rows,
    }
}

fn row_count(layout: &Layout, transfers: usize) -> usize {
    let fit = ((layout.rows.height() / ROW_H).floor().max(0.0) as usize).min(MAX_ROWS as usize);
    fit.min(transfers)
}

fn row_rect(layout: &Layout, slot: usize) -> Rect {
    Rect::from_xywh(
        layout.rows.left,
        layout.rows.top + slot as f32 * ROW_H,
        layout.rows.width(),
        ROW_H - 2.0,
    )
}

fn reveal_rect(row: Rect) -> Rect {
    Rect::from_xywh(row.right - 34.0, row.center_y() - 13.0, 26.0, 26.0)
}

fn hovered(ctx: &TabContext<'_>, id: u16) -> bool {
    ctx.hover == Some(DashboardHit::Tab(id))
}

/// Draws the tab. Return `false` to fall back to the legacy page.
pub fn draw(ctx: &TabContext<'_>) -> bool {
    LAST_GRID.with(|cell| cell.set(ctx.grid));
    let layout = layout(ctx.grid);
    let link = drop_server::current_link();
    with_state(|state| {
        state.transfers = drop_server::transfers();
        if link.is_some()
            && state
                .renewing
                .is_some_and(|at| ctx.now.saturating_duration_since(at) > Duration::from_millis(400))
        {
            state.renewing = None;
        }
        draw_link_card(ctx, &layout, state, link.as_ref());
        draw_transfers_card(ctx, &layout, state);
    });
    true
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

/// A pill button with an optional glyph; `primary` fills it with the accent.
fn button(
    ctx: &TabContext<'_>,
    rect: Rect,
    svg: Option<&'static str>,
    label: &str,
    hover: bool,
    primary: bool,
) {
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
    let label_w = text_width(label, typography::LABEL);
    let glyph_w = if svg.is_some() { 18.0 } else { 0.0 };
    let start = rect.center_x() - (label_w + glyph_w) / 2.0;
    if let Some(svg) = svg {
        kit::glyph(
            ctx.painter,
            svg,
            kit::square(Point::new(start + 7.0, rect.center_y()), 14.0),
            color,
        );
    }
    text(
        ctx.painter,
        label,
        style(12.0, 600),
        Rect::from_xywh(start + glyph_w, rect.top, label_w + 2.0, rect.height()),
        color,
    );
}

fn draw_link_card(
    ctx: &TabContext<'_>,
    layout: &Layout,
    state: &State,
    link: Option<&(String, Instant)>,
) {
    let painter = ctx.painter;
    let colors = ctx.colors;
    card(ctx, layout.left);
    let Some((url, expires)) = link else {
        let center = layout.left.center_y();
        let starting = state.renewing.is_some();
        kit::glyph(
            painter,
            WIFI,
            kit::square(Point::new(layout.left.center_x(), center - 30.0), 30.0),
            colors.text_secondary,
        );
        text_center(
            painter,
            if starting {
                "Making a new link…"
            } else {
                "Drop needs a network"
            },
            typography::SECTION_TITLE,
            Rect::from_xywh(layout.left.left, center - 6.0, layout.left.width(), 20.0),
            colors.text_primary,
        );
        if !starting {
            text_center(
                painter,
                "Connect this PC to Wi-Fi or Ethernet to share files with your phone",
                typography::CAPTION,
                Rect::from_xywh(
                    layout.left.left + PAD,
                    center + 18.0,
                    layout.left.width() - PAD * 2.0,
                    16.0,
                ),
                colors.text_secondary,
            );
        }
        return;
    };

    // The QR code on a white tile; the tile's padding plus empty modules form the quiet zone.
    painter.fill_round_rect(layout.qr, Radius::uniform(radius::ROW + 2.0), Rgba::WHITE);
    draw_qr(ctx, layout.qr.inset(6.0), url);

    // Text column.
    let info = layout.info;
    text(
        painter,
        "Scan with your phone",
        TITLE,
        Rect::from_xywh(info.left, info.top, info.width(), 22.0),
        colors.text_primary,
    );
    text(
        painter,
        "Point the camera at the code",
        typography::LABEL,
        Rect::from_xywh(info.left, info.top + 22.0, info.width(), 17.0),
        colors.text_secondary,
    );
    let left = expires.saturating_duration_since(ctx.now).as_secs();
    let countdown = format!("Link expires in {}:{:02}", left / 60, left % 60);
    let soon = left < 60;
    let pill_w = (text_width(&countdown, PILL) + 24.0).min(info.width());
    let pill = Rect::from_xywh(info.left, info.top + 46.0, pill_w, 22.0);
    painter.fill_round_rect(
        pill,
        Radius::uniform(11.0),
        if soon {
            colors.state_warning.with_alpha(46)
        } else {
            colors.accent_soft
        },
    );
    let dot = Point::new(pill.left + 10.0, pill.center_y());
    painter.fill_circle(
        dot,
        3.0,
        if soon {
            colors.state_warning
        } else {
            colors.accent_primary
        },
    );
    text(
        painter,
        &countdown,
        PILL,
        Rect::from_ltrb(pill.left + 18.0, pill.top, pill.right - 6.0, pill.bottom),
        colors.text_primary,
    );
    button(
        ctx,
        layout.new_link,
        Some(REFRESH),
        "New link",
        hovered(ctx, ID_NEW_LINK),
        false,
    );

    // URL field with a copy button.
    painter.fill_round_rect(layout.url, Radius::uniform(radius::CONTROL), colors.surface_base);
    let short = url.trim_start_matches("http://").trim_end_matches('/');
    text(
        painter,
        short,
        URL,
        Rect::from_ltrb(
            layout.url.left + 8.0,
            layout.url.top,
            layout.copy.left - 2.0,
            layout.url.bottom,
        ),
        colors.text_secondary,
    );
    let copied = state
        .copied
        .is_some_and(|at| ctx.now.saturating_duration_since(at) < FEEDBACK);
    let copy_hover = hovered(ctx, ID_COPY);
    if copy_hover {
        painter.fill_round_rect(
            layout.copy.inset(2.0),
            Radius::uniform(radius::CONTROL),
            colors.surface_overlay,
        );
    }
    kit::glyph(
        painter,
        if copied { kit::CHECK } else { COPY },
        kit::square(
            Point::new(layout.copy.center_x(), layout.copy.center_y()),
            14.0,
        ),
        if copied {
            colors.state_success
        } else if copy_hover {
            colors.text_primary
        } else {
            colors.text_secondary
        },
    );

    // Wi-Fi hint.
    kit::glyph(
        painter,
        WIFI,
        kit::square(
            Point::new(layout.wifi.left + 7.0, layout.wifi.center_y()),
            14.0,
        ),
        colors.text_secondary,
    );
    text(
        painter,
        "Phone and PC on the same Wi-Fi",
        typography::CAPTION,
        Rect::from_ltrb(
            layout.wifi.left + 20.0,
            layout.wifi.top,
            layout.wifi.right,
            layout.wifi.bottom,
        ),
        colors.text_secondary,
    );
}

/// Draws the QR modules centred in `area`, snapped to whole pixels, with a quiet zone.
fn draw_qr(ctx: &TabContext<'_>, area: Rect, url: &str) {
    QR.with(|cell| {
        let mut cached = cell.borrow_mut();
        if cached.as_ref().is_none_or(|qr| qr.url != url) {
            *cached = qrcode::QrCode::new(url.as_bytes()).ok().map(|code| Qr {
                url: url.to_string(),
                width: code.width(),
                modules: code
                    .to_colors()
                    .into_iter()
                    .map(|color| color == qrcode::Color::Dark)
                    .collect(),
            });
        }
        let Some(qr) = cached.as_ref() else {
            kit::glyph(
                ctx.painter,
                QR_MARK,
                kit::square(Point::new(area.center_x(), area.center_y()), 32.0),
                Rgba::BLACK,
            );
            return;
        };
        // Two modules of quiet zone inside the tile's own padding.
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

fn draw_transfers_card(ctx: &TabContext<'_>, layout: &Layout, state: &State) {
    let painter = ctx.painter;
    let colors = ctx.colors;
    card(ctx, layout.right);
    let header = layout.header;
    let title_w = text_width("Transfers", TITLE) + 2.0;
    text(
        painter,
        "Transfers",
        TITLE,
        Rect::from_xywh(header.left, header.top, title_w, header.height()),
        colors.text_primary,
    );
    if !state.transfers.is_empty() {
        text(
            painter,
            &state.transfers.len().to_string(),
            typography::LABEL,
            Rect::from_ltrb(
                header.left + title_w + 6.0,
                header.top + 1.0,
                layout.open_folder.left - 6.0,
                header.bottom,
            ),
            colors.text_secondary,
        );
    }
    button(
        ctx,
        layout.open_folder,
        Some(kit::FOLDER),
        "Open folder",
        hovered(ctx, ID_OPEN_FOLDER),
        false,
    );

    if state.transfers.is_empty() {
        draw_steps(ctx, layout);
        return;
    }
    let shown = row_count(layout, state.transfers.len());
    for (slot, transfer) in state.transfers.iter().take(shown).enumerate() {
        draw_row(ctx, row_rect(layout, slot), slot, transfer);
    }
    let hidden = state.transfers.len() - shown;
    if hidden > 0 {
        text_right(
            painter,
            &format!("+{hidden} older"),
            typography::CAPTION,
            Rect::from_ltrb(
                layout.rows.left,
                layout.right.bottom - 16.0,
                layout.rows.right - 8.0,
                layout.right.bottom - 2.0,
            ),
            colors.text_disabled,
        );
    }
}

/// The empty state: how Drop works in three steps.
fn draw_steps(ctx: &TabContext<'_>, layout: &Layout) {
    let painter = ctx.painter;
    let colors = ctx.colors;
    let steps = [
        "Scan the code with your phone camera",
        "Pick files on the page that opens",
        "They land in Downloads › WinIsland Drop",
    ];
    let left = layout.rows.left + 8.0;
    let right = layout.rows.right - 8.0;
    let mut top = layout.rows.top + 4.0;
    for (index, step) in steps.iter().enumerate() {
        let center = Point::new(left + 11.0, top + 11.0);
        painter.fill_circle(center, 11.0, colors.accent_soft);
        text_center(
            painter,
            &(index + 1).to_string(),
            style(12.0, 650),
            kit::square(center, 22.0),
            colors.accent_primary,
        );
        text(
            painter,
            step,
            typography::BODY,
            Rect::from_ltrb(left + 32.0, top, right, top + 22.0),
            colors.text_primary,
        );
        top += 32.0;
    }
    text(
        painter,
        "Files on your Shelf can be downloaded from the same page",
        typography::CAPTION,
        Rect::from_ltrb(left, top + 2.0, right, top + 18.0),
        colors.text_secondary,
    );
}

fn ago(started: Instant, now: Instant) -> String {
    let seconds = now.saturating_duration_since(started).as_secs();
    match seconds {
        0..60 => "just now".to_string(),
        60..3600 => format!("{}m ago", seconds / 60),
        _ => format!("{}h ago", seconds / 3600),
    }
}

fn draw_row(ctx: &TabContext<'_>, row: Rect, slot: usize, transfer: &Transfer) {
    let painter = ctx.painter;
    let colors = ctx.colors;
    let row_id = ID_ROW + slot as u16;
    let reveal_id = ID_REVEAL + slot as u16;
    let openable = transfer.state == TransferState::Done && transfer.path.is_some();
    let hover = openable && (hovered(ctx, row_id) || hovered(ctx, reveal_id));
    if hover {
        painter.fill_round_rect(row, Radius::uniform(radius::ROW), colors.surface_overlay);
    }
    // File icon with a small direction badge.
    let icon = Rect::from_xywh(row.left + 8.0, row.center_y() - 14.0, 28.0, 28.0);
    let path = transfer
        .path
        .as_deref()
        .unwrap_or_else(|| Path::new(&transfer.name));
    kit::file_icon(painter, icon, path, false, false, colors);
    let badge = Point::new(icon.right - 1.0, icon.bottom - 3.0);
    let badge_color = match transfer.state {
        TransferState::Failed => colors.state_error,
        _ if transfer.direction == Direction::Received => colors.accent_primary,
        _ => colors.state_info,
    };
    painter.fill_circle(badge, 7.0, colors.surface_raised);
    painter.fill_circle(badge, 6.0, badge_color);
    kit::glyph(
        painter,
        if transfer.direction == Direction::Received {
            ARROW_DOWN
        } else {
            ARROW_UP
        },
        kit::square(badge, 9.0),
        Rgba::WHITE,
    );

    let text_left = icon.right + 10.0;
    let text_right_edge = if hover {
        reveal_rect(row).left - 4.0
    } else {
        row.right - 8.0
    };
    let status = match transfer.state {
        TransferState::Active => {
            let percent = (transfer.done_bytes * 100)
                .checked_div(transfer.total_bytes)
                .unwrap_or(0)
                .min(100);
            format!("{percent}%")
        }
        TransferState::Done => ago(transfer.started, ctx.now),
        TransferState::Failed => "Failed".to_string(),
    };
    let status_color = match transfer.state {
        TransferState::Active => colors.accent_primary,
        TransferState::Done => colors.text_secondary,
        TransferState::Failed => colors.state_error,
    };
    let status_w = if hover {
        0.0
    } else {
        text_right(
            painter,
            &status,
            typography::CAPTION,
            Rect::from_ltrb(text_left, row.top + 3.0, text_right_edge, row.top + 19.0),
            status_color,
        )
    };
    text(
        painter,
        &transfer.name,
        NAME,
        Rect::from_ltrb(
            text_left,
            row.top + 2.0,
            text_right_edge - status_w - 8.0,
            row.top + 20.0,
        ),
        if transfer.state == TransferState::Failed {
            colors.text_secondary
        } else {
            colors.text_primary
        },
    );
    let detail_rect = Rect::from_ltrb(text_left, row.top + 20.0, text_right_edge, row.bottom - 1.0);
    if transfer.state == TransferState::Active {
        let track = Rect::from_xywh(
            detail_rect.left,
            detail_rect.center_y() - 2.0,
            detail_rect.width(),
            4.0,
        );
        painter.fill_round_rect(track, Radius::uniform(2.0), colors.surface_interactive);
        let fraction = if transfer.total_bytes == 0 {
            0.0
        } else {
            (transfer.done_bytes as f32 / transfer.total_bytes as f32).clamp(0.0, 1.0)
        };
        if fraction > 0.0 {
            painter.fill_round_rect(
                Rect::from_xywh(track.left, track.top, (track.width() * fraction).max(4.0), 4.0),
                Radius::uniform(2.0),
                colors.accent_primary,
            );
        }
    } else {
        let verb = match (transfer.direction, transfer.state) {
            (Direction::Received, TransferState::Failed) => "Receive failed",
            (Direction::Sent, TransferState::Failed) => "Send failed",
            (Direction::Received, _) => "From phone",
            (Direction::Sent, _) => "To phone",
        };
        let detail = if transfer.total_bytes > 0 {
            format!("{} · {verb}", format_bytes(transfer.total_bytes))
        } else {
            verb.to_string()
        };
        text(
            painter,
            &detail,
            typography::CAPTION,
            detail_rect,
            colors.text_secondary,
        );
    }
    if hover {
        let reveal = reveal_rect(row);
        let over = hovered(ctx, reveal_id);
        if over {
            painter.fill_round_rect(
                reveal,
                Radius::uniform(reveal.height() / 2.0),
                colors.surface_interactive,
            );
        }
        kit::glyph(
            painter,
            kit::FOLDER,
            kit::square(Point::new(reveal.center_x(), reveal.center_y()), 15.0),
            if over {
                colors.text_primary
            } else {
                colors.text_secondary
            },
        );
    }
}

/// `true` when this tab draws its own cards instead of sitting on the shared page card.
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
    let link = drop_server::current_link().is_some();
    if link {
        if layout.new_link.contains(point) {
            return Some(DashboardHit::Tab(ID_NEW_LINK));
        }
        if layout.copy.contains(point) || layout.url.contains(point) {
            return Some(DashboardHit::Tab(ID_COPY));
        }
    }
    if layout.open_folder.contains(point) {
        return Some(DashboardHit::Tab(ID_OPEN_FOLDER));
    }
    if !layout.rows.contains(point) {
        return None;
    }
    with_state(|state| {
        let shown = row_count(&layout, state.transfers.len());
        (0..shown).find_map(|slot| {
            let row = row_rect(&layout, slot);
            let transfer = &state.transfers[slot];
            if !row.contains(point)
                || transfer.state != TransferState::Done
                || transfer.path.is_none()
            {
                return None;
            }
            Some(if reveal_rect(row).contains(point) {
                ID_REVEAL + slot as u16
            } else {
                ID_ROW + slot as u16
            })
        })
    })
    .map(DashboardHit::Tab)
}

/// Applies a click from [`hit`]. `true` when handled.
pub fn apply(hit: DashboardHit) -> bool {
    let DashboardHit::Tab(id) = hit else {
        return false;
    };
    if !(1300..=1399).contains(&id) {
        return false;
    }
    let now = Instant::now();
    with_state(|state| match id {
        ID_NEW_LINK => {
            drop_server::request_new_link();
            state.renewing = Some(now);
        }
        ID_COPY => {
            if let Some((url, _)) = drop_server::current_link() {
                match winisland_platform_windows::set_clipboard_text(&url) {
                    Ok(()) => state.copied = Some(now),
                    Err(error) => log::warn!("Cannot copy Drop link: {error}"),
                }
            }
        }
        ID_OPEN_FOLDER => {
            let folder = drop_server::received_folder();
            if let Err(error) = std::fs::create_dir_all(&folder) {
                log::warn!("Cannot create Drop folder: {error}");
            }
            if let Err(error) = crate::platform::shell().open_url(&folder.to_string_lossy()) {
                log::warn!("Cannot open Drop folder: {error}");
            }
        }
        _ if (ID_ROW..ID_ROW + MAX_ROWS).contains(&id) => {
            if let Some(path) = row_path(state, (id - ID_ROW) as usize) {
                if let Err(error) = crate::platform::shell().open_url(&path.to_string_lossy()) {
                    log::warn!("Cannot open Drop file: {error}");
                }
            }
        }
        _ if (ID_REVEAL..ID_REVEAL + MAX_ROWS).contains(&id) => {
            if let Some(path) = row_path(state, (id - ID_REVEAL) as usize) {
                if let Err(error) = crate::platform::shell().reveal_path(&path) {
                    log::warn!("Cannot show Drop file: {error}");
                }
            }
        }
        _ => {}
    });
    true
}

fn row_path(state: &State, slot: usize) -> Option<std::path::PathBuf> {
    state
        .transfers
        .get(slot)
        .filter(|transfer| transfer.state == TransferState::Done)
        .and_then(|transfer| transfer.path.clone())
}

pub fn wants_keyboard() -> bool {
    false
}

pub fn handle_key(_key: &Key) -> bool {
    false
}

/// Every frame, visible or not.
pub fn poll(_now: Instant) {}
