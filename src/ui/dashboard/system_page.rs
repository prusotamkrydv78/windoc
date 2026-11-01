//! System tab: power, network, performance and audio at a glance, in four cards.
//!
//! Data comes from the background `system_watch` snapshot the app publishes with
//! [`set_snapshot`]; the volume slider and mute button hand requests back through
//! [`take_request`].

use std::cell::{Cell, RefCell};

use winisland_render::tokens::{SemanticColors, TextStyle, radius, space, typography};
use winisland_render::{Painter, PathBuilder, Point, Radius, Rect, Rgba, StrokeCap, StrokeJoin};

use crate::core::system_watch::SystemSnapshot;
use crate::ui::components::text::{Align, draw_text};

use super::DashboardHit;

/// Large figures: tabular so changing numbers do not shift.
const HERO: TextStyle = TextStyle {
    size: 28.0,
    weight: 650,
    tabular: true,
};
const FIGURE: TextStyle = TextStyle {
    size: 22.0,
    weight: 650,
    tabular: true,
};
const CARD_PAD: f32 = 16.0;
const GREEN: Rgba = Rgba::from_rgb(48, 209, 88);
const RED: Rgba = Rgba::from_rgb(255, 69, 58);

/// A change the user asked for from the System tab.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SystemRequest {
    /// Set the default output volume (0..=1).
    Volume(f32),
    ToggleMute,
}

thread_local! {
    static SNAPSHOT: RefCell<SystemSnapshot> = RefCell::new(SystemSnapshot::default());
    static REQUEST: Cell<Option<SystemRequest>> = const { Cell::new(None) };
}

/// Publishes the latest system snapshot for the tab.
pub fn set_snapshot(snapshot: SystemSnapshot) {
    SNAPSHOT.with(|cell| *cell.borrow_mut() = snapshot);
}

/// The volume or mute change requested since the last call.
pub fn take_request() -> Option<SystemRequest> {
    REQUEST.with(Cell::take)
}

/// The four cards: battery (top left), network (top right), performance (bottom left) and
/// volume (bottom right).
struct Cards {
    battery: Rect,
    network: Rect,
    performance: Rect,
    volume: Rect,
}

fn cards(grid: Rect) -> Cards {
    let gap = space::S3;
    let half_w = (grid.width() - gap) / 2.0;
    let half_h = (grid.height() - gap) / 2.0;
    let at = |col: f32, row: f32| {
        Rect::from_xywh(
            grid.left + col * (half_w + gap),
            grid.top + row * (half_h + gap),
            half_w,
            half_h,
        )
    };
    Cards {
        battery: at(0.0, 0.0),
        network: at(1.0, 0.0),
        performance: at(0.0, 1.0),
        volume: at(1.0, 1.0),
    }
}

/// The volume slider track and the mute button inside the volume card.
fn volume_controls(card: Rect) -> (Rect, Rect) {
    let mute = Rect::from_xywh(
        card.right - CARD_PAD - 36.0,
        card.top + CARD_PAD + 22.0,
        36.0,
        36.0,
    );
    let track = Rect::from_xywh(
        card.left + CARD_PAD,
        card.bottom - CARD_PAD - 18.0,
        card.width() - CARD_PAD * 2.0,
        18.0,
    );
    (track, mute)
}

/// Hit test for the tab's controls, in page coordinates.
pub fn hit(grid: Rect, point: Point) -> Option<DashboardHit> {
    let (track, mute) = volume_controls(cards(grid).volume);
    if mute.contains(point) {
        return Some(DashboardHit::SystemMute);
    }
    if track.inset(-6.0).contains(point) {
        let fraction = ((point.x - track.left) / track.width()).clamp(0.0, 1.0);
        return Some(DashboardHit::SystemVolume((fraction * 100.0).round() as u8));
    }
    None
}

/// Applies a click from [`hit`].
pub fn apply(hit: DashboardHit) -> bool {
    let request = match hit {
        DashboardHit::SystemVolume(percent) => SystemRequest::Volume(f32::from(percent) / 100.0),
        DashboardHit::SystemMute => SystemRequest::ToggleMute,
        _ => return false,
    };
    // Show the change at once; the next snapshot confirms it.
    SNAPSHOT.with(|cell| {
        let mut snapshot = cell.borrow_mut();
        if let Some((level, muted)) = snapshot.volume.as_mut() {
            match request {
                SystemRequest::Volume(value) => *level = value,
                SystemRequest::ToggleMute => *muted = !*muted,
            }
        }
    });
    REQUEST.with(|cell| cell.set(Some(request)));
    true
}

pub fn draw(
    painter: Painter<'_>,
    grid: Rect,
    hover: Option<DashboardHit>,
    colors: &SemanticColors,
) {
    let snapshot = SNAPSHOT.with(|cell| cell.borrow().clone());
    let cards = cards(grid);
    for card in [
        cards.battery,
        cards.network,
        cards.performance,
        cards.volume,
    ] {
        painter.fill_round_rect(
            card,
            Radius::uniform(radius::CARD + 6.0),
            colors.surface_raised,
        );
    }
    draw_battery(painter, cards.battery, &snapshot, colors);
    draw_network(painter, cards.network, &snapshot, colors);
    draw_performance(painter, cards.performance, &snapshot, colors);
    draw_volume(painter, cards.volume, &snapshot, hover, colors);
}

/// Card title at the top left, an optional note at the top right.
fn card_header(painter: Painter<'_>, card: Rect, title: &str, note: &str, colors: &SemanticColors) {
    let row = Rect::from_xywh(
        card.left + CARD_PAD,
        card.top + CARD_PAD - 2.0,
        card.width() - CARD_PAD * 2.0,
        18.0,
    );
    draw_text(
        painter,
        title,
        typography::LABEL,
        row,
        Align::Start,
        colors.text_secondary,
    );
    if !note.is_empty() {
        let width = winisland_render::text::FontManager::global().measure_text_cached(
            note,
            typography::LABEL.size,
            winisland_render::FontStyle::normal(),
        );
        draw_text(
            painter,
            note,
            typography::LABEL,
            Rect::from_xywh(row.right - width - 2.0, row.top, width + 2.0, row.height()),
            Align::Start,
            colors.text_secondary,
        );
    }
}

fn hero(painter: Painter<'_>, card: Rect, text: &str, colors: &SemanticColors) -> Rect {
    let rect = Rect::from_xywh(
        card.left + CARD_PAD,
        card.top + CARD_PAD + 16.0,
        card.width() - CARD_PAD * 2.0,
        34.0,
    );
    draw_text(painter, text, HERO, rect, Align::Start, colors.text_primary);
    rect
}

/// One line of secondary text along the bottom of a card.
fn footer(painter: Painter<'_>, card: Rect, text: &str, colors: &SemanticColors) {
    draw_text(
        painter,
        text,
        typography::LABEL,
        Rect::from_xywh(
            card.left + CARD_PAD,
            card.bottom - CARD_PAD - 16.0,
            card.width() - CARD_PAD * 2.0,
            16.0,
        ),
        Align::Start,
        colors.text_secondary,
    );
}

fn draw_battery(
    painter: Painter<'_>,
    card: Rect,
    snapshot: &SystemSnapshot,
    colors: &SemanticColors,
) {
    let Some((level, charging)) = snapshot.battery else {
        card_header(painter, card, "Power", "AC", colors);
        hero(painter, card, "Plugged in", colors);
        footer(painter, card, "No battery in this PC", colors);
        return;
    };
    card_header(
        painter,
        card,
        "Battery",
        if charging { "Charging" } else { "On battery" },
        colors,
    );
    hero(painter, card, &format!("{level}%"), colors);
    // Level bar: green while charging, red when low.
    let bar = Rect::from_xywh(
        card.left + CARD_PAD,
        card.bottom - CARD_PAD - 10.0,
        card.width() - CARD_PAD * 2.0,
        10.0,
    );
    painter.fill_round_rect(bar, Radius::uniform(5.0), colors.surface_overlay);
    let fill = if charging {
        GREEN
    } else if level <= 15 {
        RED
    } else {
        colors.text_primary
    };
    painter.fill_round_rect(
        Rect::from_xywh(
            bar.left,
            bar.top,
            (bar.width() * f32::from(level.min(100)) / 100.0).max(bar.height()),
            bar.height(),
        ),
        Radius::uniform(5.0),
        fill,
    );
}

fn draw_network(
    painter: Painter<'_>,
    card: Rect,
    snapshot: &SystemSnapshot,
    colors: &SemanticColors,
) {
    let (name, detail) = match (
        &snapshot.wifi,
        snapshot.online || snapshot.download.is_some(),
    ) {
        (Some(wifi), _) => (
            wifi.name.clone(),
            match wifi.receive_mbps {
                Some(rate) => format!("{} · {rate} Mbps", wifi.standard),
                None => wifi.standard.clone(),
            },
        ),
        (None, true) => (
            "Ethernet".to_string(),
            snapshot
                .link_bits_per_second
                .filter(|bits| *bits > 0)
                .map(|bits| format!("{} Mbps link", bits / 1_000_000))
                .unwrap_or_default(),
        ),
        (None, false) => ("Offline".to_string(), "No network connection".to_string()),
    };
    // The link detail sits beside the title so the rates have the bottom row to themselves.
    card_header(painter, card, "Network", &detail, colors);
    hero(painter, card, &name, colors);
    // Download and upload side by side, each with its live sparkline.
    let row_top = card.bottom - CARD_PAD - 20.0;
    let half = (card.width() - CARD_PAD * 2.0 - space::S3) / 2.0;
    for (index, (arrow, rate, history)) in [
        ("↓", snapshot.download, &snapshot.download_history),
        ("↑", snapshot.upload, &snapshot.upload_history),
    ]
    .into_iter()
    .enumerate()
    {
        let left = card.left + CARD_PAD + index as f32 * (half + space::S3);
        let text = format!(
            "{arrow} {}",
            rate.map_or_else(|| "—".to_string(), format_rate)
        );
        draw_text(
            painter,
            &text,
            typography::BODY,
            Rect::from_xywh(left, row_top, half * 0.55, 20.0),
            Align::Start,
            colors.text_primary,
        );
        let peak = history.iter().copied().fold(1024.0_f32, f32::max);
        sparkline(
            painter,
            Rect::from_xywh(left + half * 0.58, row_top + 1.0, half * 0.42, 18.0),
            history,
            peak,
            colors.text_primary,
        );
    }
}

fn draw_performance(
    painter: Painter<'_>,
    card: Rect,
    snapshot: &SystemSnapshot,
    colors: &SemanticColors,
) {
    card_header(painter, card, "Performance", "", colors);
    let columns = [
        ("CPU", snapshot.cpu, &snapshot.cpu_history, None),
        (
            "RAM",
            snapshot.memory,
            &snapshot.memory_history,
            snapshot.memory_bytes,
        ),
        (
            "Disk",
            snapshot.disk,
            &snapshot.disk_history,
            snapshot.disk_bytes,
        ),
    ];
    let inner = card.width() - CARD_PAD * 2.0;
    let width = inner / 3.0;
    for (index, (label, value, history, bytes)) in columns.into_iter().enumerate() {
        let left = card.left + CARD_PAD + index as f32 * width;
        if index > 0 {
            painter.fill_rect(
                Rect::from_xywh(
                    left - space::S2,
                    card.top + CARD_PAD + 26.0,
                    1.0,
                    card.height() - CARD_PAD * 2.0 - 26.0,
                ),
                colors.border_subtle,
            );
        }
        let figure =
            value.map_or_else(|| "—".to_string(), |value| format!("{:.0}%", value * 100.0));
        let figure_row = Rect::from_xywh(left, card.top + CARD_PAD + 18.0, width - space::S4, 30.0);
        draw_text(
            painter,
            &figure,
            FIGURE,
            figure_row,
            Align::Start,
            colors.text_primary,
        );
        // The trend sits beside the figure; "100%" is the widest it gets.
        let figure_w = 52.0;
        if figure_row.width() - figure_w > 20.0 {
            sparkline(
                painter,
                Rect::from_xywh(
                    figure_row.left + figure_w,
                    figure_row.center_y() - 9.0,
                    figure_row.width() - figure_w,
                    18.0,
                ),
                history,
                1.0,
                colors.text_primary,
            );
        }
        // The amounts only when they fit beside the label.
        let caption = bytes
            .map(|(used, total)| format!("{label} {}", format_pair(used, total)))
            .filter(|caption| {
                winisland_render::text::FontManager::global().measure_text_cached(
                    caption,
                    typography::CAPTION.size,
                    winisland_render::FontStyle::normal(),
                ) <= width - space::S4
            })
            .unwrap_or_else(|| label.to_string());
        draw_text(
            painter,
            &caption,
            typography::CAPTION,
            Rect::from_xywh(left, card.bottom - CARD_PAD - 16.0, width - space::S4, 16.0),
            Align::Start,
            colors.text_secondary,
        );
    }
}

fn draw_volume(
    painter: Painter<'_>,
    card: Rect,
    snapshot: &SystemSnapshot,
    hover: Option<DashboardHit>,
    colors: &SemanticColors,
) {
    let Some((level, muted)) = snapshot.volume else {
        card_header(painter, card, "Volume", "", colors);
        hero(painter, card, "—", colors);
        return;
    };
    card_header(painter, card, "Volume", "Default output", colors);
    hero(
        painter,
        card,
        &if muted {
            "Muted".to_string()
        } else {
            format!("{:.0}%", level * 100.0)
        },
        colors,
    );
    let (track, mute) = volume_controls(card);
    // Mute button: a circle with a speaker glyph, red while muted.
    let hovered = hover == Some(DashboardHit::SystemMute);
    painter.fill_circle(
        Point::new(mute.center_x(), mute.center_y()),
        mute.width() / 2.0,
        if muted {
            RED.with_alpha(200)
        } else if hovered {
            colors.surface_interactive
        } else {
            colors.surface_overlay
        },
    );
    crate::ui::top_bar::draw_glyph(painter, crate::ui::top_bar::SPEAKER, mute, 1.0);
    // Slider: a rounded track, a white fill and a round knob.
    let bar = Rect::from_xywh(track.left, track.center_y() - 3.0, track.width(), 6.0);
    painter.fill_round_rect(bar, Radius::uniform(3.0), colors.surface_overlay);
    let fill_w = bar.width() * if muted { 0.0 } else { level.clamp(0.0, 1.0) };
    painter.fill_round_rect(
        Rect::from_xywh(bar.left, bar.top, fill_w.max(bar.height()), bar.height()),
        Radius::uniform(3.0),
        colors.text_primary,
    );
    painter.fill_circle(
        Point::new(bar.left + fill_w, bar.center_y()),
        9.0,
        colors.text_primary,
    );
}

/// A thin line chart of `values` scaled to `peak`, newest on the right.
fn sparkline(painter: Painter<'_>, rect: Rect, values: &[f32], peak: f32, color: Rgba) {
    if values.len() < 2 || peak <= 0.0 {
        painter.fill_rect(
            Rect::from_xywh(rect.left, rect.bottom - 1.0, rect.width(), 1.0),
            color.with_alpha(40),
        );
        return;
    }
    let step = rect.width() / (values.len() - 1) as f32;
    let mut builder = PathBuilder::new();
    for (index, value) in values.iter().enumerate() {
        let point = Point::new(
            rect.left + index as f32 * step,
            rect.bottom - (value / peak).clamp(0.0, 1.0) * rect.height(),
        );
        if index == 0 {
            builder.move_to(point);
        } else {
            builder.line_to(point);
        }
    }
    painter.stroke_path(
        &builder.detach(),
        1.5,
        color.with_alpha(200),
        StrokeCap::Round,
        StrokeJoin::Round,
    );
}

/// "2.4 MB/s", "312 KB/s", "0 B/s".
fn format_rate(bytes_per_second: f64) -> String {
    if bytes_per_second >= 1_048_576.0 {
        format!("{:.1} MB/s", bytes_per_second / 1_048_576.0)
    } else if bytes_per_second >= 1024.0 {
        format!("{:.0} KB/s", bytes_per_second / 1024.0)
    } else {
        format!("{bytes_per_second:.0} B/s")
    }
}

/// "21/32 GB", "0.4/1.8 TB": used and total sharing one unit so it stays short.
fn format_pair(used: u64, total: u64) -> String {
    const GB: f64 = 1_073_741_824.0;
    let (used, total) = (used as f64 / GB, total as f64 / GB);
    if total >= 1024.0 {
        format!("{:.1}/{:.1} TB", used / 1024.0, total / 1024.0)
    } else {
        format!("{used:.0}/{total:.0} GB")
    }
}
