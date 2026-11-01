//! Links tab: a web search box with engine chips over the saved quick links.
//!
//! The top card takes a query or a pasted address: Enter (or the "Search" pill) opens a
//! typed address directly, or searches it with the selected engine. The bottom card pages
//! through the saved links from the configuration (published with `dashboard::set_links`).
//! Links are added and edited in Settings; "+ Add Link" and the tile menu open it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use winisland_core::text_edit::{EditKey, TextBuffer};
use winisland_platform::Key;
use winisland_render::text::FontManager;
use winisland_render::tokens::{TextStyle, radius};
use winisland_render::{
    FontStyle, Painter, Path, Point, Radius, Rect, Rgba, StrokeCap, StrokeJoin, Vec2,
};

use super::TabContext;
use crate::ui::components::text::{Align, draw_text};
use crate::ui::dashboard::DashboardHit;

// Hit ids (900-999).
const ID_FIELD: u16 = 900;
const ID_GO: u16 = 901;
const ID_PASTE: u16 = 902;
const ID_ENGINE: u16 = 910; // ..=915
const ID_PREV: u16 = 920;
const ID_NEXT: u16 = 921;
const ID_ADD: u16 = 922;
const ID_TILE: u16 = 930; // ..=949
const ID_MORE: u16 = 950; // ..=969
const ID_MENU: u16 = 970; // ..=972
const ID_BACKGROUND: u16 = 999;

const MAX_LINKS: usize = 20;
const MAX_QUERY_CHARS: usize = 2000;
const PAD: f32 = 14.0;
const CARD_GAP: f32 = 8.0;
const SEARCH_CARD_HEIGHT: f32 = 100.0;
const TILE_MIN_WIDTH: f32 = 150.0;
const TILE_HEIGHT: f32 = 46.0;
const TILE_GAP: f32 = 8.0;
const COPIED_FOR: Duration = Duration::from_millis(1500);

const LABEL: TextStyle = TextStyle {
    size: 12.0,
    weight: 500,
    tabular: false,
};
const INPUT: TextStyle = TextStyle {
    size: 15.0,
    weight: 400,
    tabular: false,
};
const CHIP: TextStyle = TextStyle {
    size: 12.5,
    weight: 500,
    tabular: false,
};
const PILL: TextStyle = TextStyle {
    size: 13.0,
    weight: 600,
    tabular: false,
};
const TITLE: TextStyle = TextStyle {
    size: 13.0,
    weight: 600,
    tabular: false,
};
const DOMAIN: TextStyle = TextStyle {
    size: 11.5,
    weight: 400,
    tabular: false,
};

struct Engine {
    name: &'static str,
    monogram: &'static str,
    color: Rgba,
    search: &'static str,
    home: &'static str,
}

const ENGINES: [Engine; 6] = [
    Engine {
        name: "Google",
        monogram: "G",
        color: Rgba::from_rgb(66, 133, 244),
        search: "https://www.google.com/search?q=",
        home: "https://www.google.com",
    },
    Engine {
        name: "YouTube",
        monogram: "",
        color: Rgba::from_rgb(255, 0, 51),
        search: "https://www.youtube.com/results?search_query=",
        home: "https://www.youtube.com",
    },
    Engine {
        name: "GitHub",
        monogram: "GH",
        color: Rgba::from_rgb(110, 118, 129),
        search: "https://github.com/search?q=",
        home: "https://github.com",
    },
    Engine {
        name: "Stack Overflow",
        monogram: "SO",
        color: Rgba::from_rgb(244, 128, 36),
        search: "https://stackoverflow.com/search?q=",
        home: "https://stackoverflow.com",
    },
    Engine {
        name: "ChatGPT",
        monogram: "C",
        color: Rgba::from_rgb(16, 163, 127),
        search: "https://chatgpt.com/?q=",
        home: "https://chatgpt.com",
    },
    Engine {
        name: "Perplexity",
        monogram: "P",
        color: Rgba::from_rgb(32, 128, 141),
        search: "https://www.perplexity.ai/search?q=",
        home: "https://www.perplexity.ai",
    },
];

/// Tints for saved link monograms, picked by domain.
const TINTS: [Rgba; 6] = [
    Rgba::from_rgb(162, 89, 255),
    Rgba::from_rgb(52, 199, 89),
    Rgba::from_rgb(10, 132, 255),
    Rgba::from_rgb(255, 159, 10),
    Rgba::from_rgb(255, 69, 58),
    Rgba::from_rgb(100, 210, 255),
];

const MENU_ITEMS: [&str; 3] = ["Open", "Copy link", "Edit in Settings"];

const SEARCH_ICON: &str = "M11 4 A7 7 0 1 1 10.99 4 M16 16 L20 20";
const ARROW_ICON: &str = "M7 17 L17 7 M9 7 H17 V15";
const BOOKMARK_ICON: &str = "M6 3 H18 V21 L12 16.5 L6 21 Z";
const PREV_ICON: &str = "M15 6 L9 12 L15 18";
const NEXT_ICON: &str = "M9 6 L15 12 L9 18";
const PLUS_ICON: &str = "M12 5 V19 M5 12 H19";

struct State {
    query: TextBuffer,
    focused: bool,
    focus_since: Instant,
    last_drawn: Option<Instant>,
    engine: usize,
    page: usize,
    /// Saved link whose ⋮ menu is open.
    menu: Option<usize>,
    copied_at: Option<Instant>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State {
        query: TextBuffer::new(MAX_QUERY_CHARS),
        focused: false,
        focus_since: Instant::now(),
        last_drawn: None,
        engine: 0,
        page: 0,
        menu: None,
        copied_at: None,
    });
    static ICONS: RefCell<HashMap<&'static str, Option<Path>>> = RefCell::new(HashMap::new());
}

fn links() -> Vec<(String, String)> {
    crate::ui::dashboard::LINKS.with(|cell| cell.borrow().iter().take(MAX_LINKS).cloned().collect())
}

/// The newest clipboard text, single line, when clipboard history has one.
fn latest_clip() -> Option<String> {
    crate::ui::dashboard::CLIPBOARD_VIEW.with(|cell| {
        let view = cell.borrow();
        if !view.enabled {
            return None;
        }
        let text = view.previews.first()?;
        let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
        Some(line.chars().take(MAX_QUERY_CHARS).collect())
    })
}

/// The host of `url` without `www.`, for display.
fn domain(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(rest)
        .rsplit('@')
        .next()
        .unwrap_or(rest);
    host.strip_prefix("www.").unwrap_or(host).to_string()
}

/// `text` as an address to open directly, or `None` for a search.
fn as_url(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || text.chars().any(char::is_whitespace) {
        return None;
    }
    let lower = text.to_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some(text.to_string());
    }
    let host = lower.split(['/', '?', '#']).next().unwrap_or(&lower);
    let host = host.split(':').next().unwrap_or(host);
    if host == "localhost" {
        return Some(format!("http://{text}"));
    }
    let is_ip = host.split('.').count() == 4
        && host
            .split('.')
            .all(|part| !part.is_empty() && part.parse::<u8>().is_ok());
    if is_ip {
        return Some(format!("http://{text}"));
    }
    let (name, tld) = host.rsplit_once('.')?;
    let valid_tld = (2..=24).contains(&tld.len()) && tld.chars().all(|c| c.is_ascii_alphabetic());
    let valid_name = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '.'));
    (valid_tld && valid_name).then(|| format!("https://{text}"))
}

fn encode_query(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len() * 3);
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push('+'),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn open(url: &str) {
    if let Err(error) = crate::platform::shell().open_url(url) {
        log::warn!("Cannot open link: {error}");
    }
}

fn open_settings() {
    crate::ui::dashboard::HEADER_REQUEST.with(|cell| cell.set(Some(DashboardHit::OpenSettings)));
}

fn wake_after(delay: Duration) {
    let _ = std::thread::Builder::new()
        .name("links-redraw".into())
        .spawn(move || {
            std::thread::sleep(delay);
            crate::platform::wake();
        });
}

impl State {
    /// Opens the typed address, or searches the query with the selected engine.
    fn submit(&mut self) {
        let query = self.query.text().trim().to_string();
        let engine = &ENGINES[self.engine.min(ENGINES.len() - 1)];
        let url = if query.is_empty() {
            engine.home.to_string()
        } else if let Some(url) = as_url(&query) {
            url
        } else {
            format!("{}{}", engine.search, encode_query(&query))
        };
        open(&url);
        self.query.clear();
        self.focused = false;
    }

    fn focus(&mut self) {
        if !self.focused {
            self.focused = true;
            self.focus_since = Instant::now();
            let end = self.query.char_count();
            self.query.set_caret(end);
        }
    }

    fn paste(&mut self) {
        if let Some(text) = latest_clip() {
            self.query.insert(&text);
        }
    }
}

fn font_style(style: TextStyle) -> FontStyle {
    if style.weight >= 600 {
        FontStyle::bold()
    } else {
        FontStyle::normal()
    }
}

fn text_width(text: &str, style: TextStyle) -> f32 {
    FontManager::global().measure_text_cached(text, style.size, font_style(style))
}

/// Strokes a 24×24 SVG path icon centered in `rect`.
fn draw_svg_icon(painter: Painter<'_>, svg: &'static str, rect: Rect, color: Rgba, stroke: f32) {
    let size = rect.width().min(rect.height());
    if size <= 0.0 {
        return;
    }
    ICONS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let Some(path) = cache
            .entry(svg)
            .or_insert_with(|| Path::from_svg(svg))
            .as_ref()
        else {
            return;
        };
        let scale = size / 24.0;
        let save = painter.save();
        painter.translate(Vec2::new(
            rect.center_x() - 12.0 * scale,
            rect.center_y() - 12.0 * scale,
        ));
        painter.scale(Vec2::new(scale, scale));
        painter.stroke_path(path, stroke, color, StrokeCap::Round, StrokeJoin::Round);
        painter.restore_to(save);
    });
}

struct Layout {
    search_card: Rect,
    links_card: Rect,
    label: Rect,
    field: Rect,
    go: Rect,
    paste: Option<Rect>,
    engines: Vec<Rect>,
    header: Rect,
    prev: Rect,
    next: Rect,
    add: Rect,
    /// Visible tiles with their link index.
    tiles: Vec<(Rect, usize)>,
    tile_area: Rect,
    page: usize,
    pages: usize,
    menu: Option<(Rect, usize)>,
}

fn go_label(state: &State) -> &'static str {
    if as_url(state.query.text()).is_some() {
        "Open"
    } else {
        "Search"
    }
}

fn more_button(tile: Rect) -> Rect {
    Rect::from_xywh(tile.right - 24.0, tile.center_y() - 14.0, 20.0, 28.0)
}

fn menu_item(menu: Rect, index: usize) -> Rect {
    Rect::from_xywh(
        menu.left + 4.0,
        menu.top + 4.0 + index as f32 * 26.0,
        menu.width() - 8.0,
        26.0,
    )
}

fn compute_layout(grid: Rect, state: &State, link_count: usize) -> Layout {
    let search_card = Rect::from_xywh(
        grid.left,
        grid.top,
        grid.width(),
        SEARCH_CARD_HEIGHT.min(grid.height()),
    );
    let links_card = Rect::from_ltrb(
        grid.left,
        (search_card.bottom + CARD_GAP).min(grid.bottom),
        grid.right,
        grid.bottom,
    );
    let x0 = search_card.left + PAD;
    let x1 = search_card.right - PAD;
    let top = search_card.top;

    let label = Rect::from_ltrb(x0, top + 9.0, x1, top + 25.0);
    let go_width = text_width(go_label(state), PILL) + 14.0 + 6.0 + 28.0;
    let go = Rect::from_xywh(x1 - go_width, top + 29.0, go_width, 32.0);
    let field = Rect::from_ltrb(x0, go.top, go.left - 10.0, go.bottom);
    let paste = (state.query.is_empty() && latest_clip().is_some()).then(|| {
        let width = text_width("Paste", LABEL) + 20.0;
        Rect::from_xywh(
            field.right - 6.0 - width,
            field.center_y() - 11.0,
            width,
            22.0,
        )
    });

    let mut engines = Vec::with_capacity(ENGINES.len());
    let mut x = x0 - 6.0;
    for engine in &ENGINES {
        let width = 8.0 + 16.0 + 7.0 + text_width(engine.name, CHIP) + 10.0;
        engines.push(Rect::from_xywh(x, top + 67.0, width, 26.0));
        x += width + 4.0;
    }

    let lx0 = links_card.left + PAD;
    let lx1 = links_card.right - PAD;
    let header = Rect::from_ltrb(lx0, links_card.top + 8.0, lx1, links_card.top + 34.0);
    let add_width = text_width("Add Link", PILL) + 14.0 + 4.0 + 26.0;
    let add = Rect::from_xywh(lx1 - add_width + 4.0, header.top, add_width, 26.0);
    let next = Rect::from_xywh(add.left - 8.0 - 26.0, header.top, 26.0, 26.0);
    let prev = Rect::from_xywh(next.left - 2.0 - 26.0, header.top, 26.0, 26.0);

    let tile_area = Rect::from_ltrb(
        lx0 - 6.0,
        header.bottom + 4.0,
        lx1 + 6.0,
        links_card.bottom - 6.0,
    );
    let columns = (((tile_area.width() + TILE_GAP) / (TILE_MIN_WIDTH + TILE_GAP)).floor() as usize)
        .clamp(1, 6);
    let rows =
        (((tile_area.height() + TILE_GAP) / (TILE_HEIGHT + TILE_GAP)).floor() as usize).max(1);
    let per_page = columns * rows;
    let pages = link_count.div_ceil(per_page).max(1);
    let page = state.page.min(pages - 1);
    let tile_width = (tile_area.width() - TILE_GAP * (columns - 1) as f32) / columns as f32;
    let used_height = rows as f32 * TILE_HEIGHT + (rows - 1) as f32 * TILE_GAP;
    let tiles_top = tile_area.top + ((tile_area.height() - used_height) / 2.0).max(0.0);
    let tiles = (page * per_page..((page + 1) * per_page).min(link_count))
        .enumerate()
        .map(|(slot, index)| {
            let (row, column) = (slot / columns, slot % columns);
            (
                Rect::from_xywh(
                    tile_area.left + column as f32 * (tile_width + TILE_GAP),
                    tiles_top + row as f32 * (TILE_HEIGHT + TILE_GAP),
                    tile_width,
                    TILE_HEIGHT,
                ),
                index,
            )
        })
        .collect::<Vec<_>>();

    let menu = state.menu.and_then(|index| {
        let (tile, _) = tiles.iter().find(|(_, i)| *i == index)?;
        let anchor = more_button(*tile);
        let (width, height) = (150.0, 8.0 + 26.0 * MENU_ITEMS.len() as f32);
        let left = (anchor.right - width).clamp(grid.left, grid.right - width);
        let above = anchor.top - 4.0 - height;
        let top = if above >= grid.top {
            above
        } else {
            (anchor.bottom + 4.0).min(grid.bottom - height)
        };
        Some((Rect::from_xywh(left, top, width, height), index))
    });

    Layout {
        search_card,
        links_card,
        label,
        field,
        go,
        paste,
        engines,
        header,
        prev,
        next,
        add,
        tiles,
        tile_area,
        page,
        pages,
        menu,
    }
}

/// Draws the tab. Return `false` to fall back to the legacy page.
pub fn draw(ctx: &TabContext<'_>) -> bool {
    let links = links();
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        state.last_drawn = Some(ctx.now);
        if state
            .copied_at
            .is_some_and(|at| ctx.now.duration_since(at) > COPIED_FOR)
        {
            state.copied_at = None;
        }
        if state.menu.is_some_and(|index| index >= links.len()) {
            state.menu = None;
        }
        let layout = compute_layout(ctx.grid, &state, links.len());
        state.page = layout.page;
        let painter = ctx.painter;
        let save = painter.save();
        painter.clip_rect(ctx.grid);
        draw_search(ctx, &state, &layout);
        draw_saved(ctx, &state, &layout, &links);
        if let Some((menu, _)) = layout.menu {
            draw_menu(ctx, menu);
        }
        painter.restore_to(save);
    });
    true
}

fn hovered(ctx: &TabContext<'_>, id: u16) -> bool {
    ctx.hover == Some(DashboardHit::Tab(id))
}

fn draw_search(ctx: &TabContext<'_>, state: &State, layout: &Layout) {
    let painter = ctx.painter;
    let colors = ctx.colors;
    painter.fill_round_rect(
        layout.search_card,
        Radius::uniform(radius::CARD + 6.0),
        colors.surface_raised,
    );

    // "Search" label.
    let icon = Rect::from_xywh(layout.label.left, layout.label.center_y() - 6.5, 13.0, 13.0);
    draw_svg_icon(painter, SEARCH_ICON, icon, colors.text_secondary, 2.0);
    draw_text(
        painter,
        "Search",
        LABEL,
        Rect::from_ltrb(
            icon.right + 6.0,
            layout.label.top,
            layout.label.right,
            layout.label.bottom,
        ),
        Align::Start,
        colors.text_secondary,
    );

    // Field.
    let field = layout.field;
    let field_hover = hovered(ctx, ID_FIELD);
    painter.fill_round_rect(
        field,
        Radius::uniform(radius::ROW),
        if state.focused || field_hover {
            colors.surface_overlay
        } else {
            colors.surface_overlay.with_alpha_f(0.55)
        },
    );
    if state.focused {
        painter.stroke_round_rect(
            field,
            Radius::uniform(radius::ROW),
            1.0,
            colors.accent_primary,
        );
    }
    let text_left = field.left + 12.0;
    let text_right = layout
        .paste
        .map_or(field.right - 12.0, |paste| paste.left - 8.0);
    if state.query.is_empty() {
        draw_text(
            painter,
            "Search or paste a link",
            INPUT,
            Rect::from_ltrb(text_left, field.top, text_right, field.bottom),
            Align::Start,
            colors.text_secondary,
        );
    } else {
        // Scroll long text so the caret stays visible.
        let available = (text_right - text_left).max(1.0);
        let before = text_width(state.query.before_caret(), INPUT);
        let offset = (before - available + 2.0).max(0.0);
        let full = text_width(state.query.text(), INPUT);
        let field_save = painter.save();
        painter.clip_rect(Rect::from_ltrb(
            text_left - 1.0,
            field.top,
            text_right,
            field.bottom,
        ));
        draw_text(
            painter,
            state.query.text(),
            INPUT,
            Rect::from_xywh(text_left - offset, field.top, full + 4.0, field.height()),
            Align::Start,
            colors.text_primary,
        );
        painter.restore_to(field_save);
    }
    if state.focused && ctx.now.duration_since(state.focus_since).as_millis() / 530 % 2 == 0 {
        let available = (text_right - text_left).max(1.0);
        let before = text_width(state.query.before_caret(), INPUT);
        let x = text_left + before.min(available - 1.0) + 0.5;
        painter.stroke_line(
            Point::new(x, field.center_y() - 9.0),
            Point::new(x, field.center_y() + 9.0),
            1.5,
            colors.accent_primary,
            StrokeCap::Round,
        );
    }
    if let Some(paste) = layout.paste {
        let hover = hovered(ctx, ID_PASTE);
        painter.fill_round_rect(
            paste,
            Radius::uniform(paste.height() / 2.0),
            if hover {
                colors.surface_interactive
            } else {
                colors.surface_raised
            },
        );
        draw_text(
            painter,
            "Paste",
            LABEL,
            paste,
            Align::Center,
            if hover {
                colors.text_primary
            } else {
                colors.text_secondary
            },
        );
    }

    // Go pill (inverted colors so it reads as the primary action in both themes).
    let go = layout.go;
    let go_hover = hovered(ctx, ID_GO);
    let fill = if go_hover {
        colors.text_primary.with_alpha_f(0.85)
    } else {
        colors.text_primary
    };
    painter.fill_round_rect(go, Radius::uniform(go.height() / 2.0), fill);
    let label = go_label(state);
    let label_width = text_width(label, PILL);
    let content = label_width + 6.0 + 14.0;
    let start = go.center_x() - content / 2.0;
    draw_text(
        painter,
        label,
        PILL,
        Rect::from_xywh(start, go.top, label_width + 2.0, go.height()),
        Align::Start,
        colors.surface_base,
    );
    draw_svg_icon(
        painter,
        ARROW_ICON,
        Rect::from_xywh(start + label_width + 6.0, go.center_y() - 7.0, 14.0, 14.0),
        colors.surface_base,
        2.4,
    );

    // Engine chips.
    for (index, (chip, engine)) in layout.engines.iter().zip(ENGINES.iter()).enumerate() {
        if chip.right > layout.search_card.right - 4.0 {
            break;
        }
        let selected = index == state.engine;
        let hover = hovered(ctx, ID_ENGINE + index as u16);
        if selected {
            painter.fill_round_rect(
                *chip,
                Radius::uniform(chip.height() / 2.0),
                colors.surface_interactive,
            );
        } else if hover {
            painter.fill_round_rect(
                *chip,
                Radius::uniform(chip.height() / 2.0),
                colors.surface_overlay,
            );
        }
        let badge = Rect::from_xywh(chip.left + 8.0, chip.center_y() - 8.0, 16.0, 16.0);
        draw_engine_badge(painter, engine, badge, selected || hover);
        draw_text(
            painter,
            engine.name,
            CHIP,
            Rect::from_ltrb(badge.right + 7.0, chip.top, chip.right - 4.0, chip.bottom),
            Align::Start,
            if selected || hover {
                colors.text_primary
            } else {
                colors.text_secondary
            },
        );
    }
}

fn draw_engine_badge(painter: Painter<'_>, engine: &Engine, rect: Rect, active: bool) {
    let color = if active {
        engine.color
    } else {
        engine.color.with_alpha_f(0.8)
    };
    painter.fill_round_rect(rect, Radius::uniform(4.5), color);
    let white = Rgba::from_rgb(255, 255, 255);
    if engine.monogram.is_empty() {
        // A play triangle.
        let mut builder = winisland_render::PathBuilder::new();
        let (cx, cy) = (rect.center_x() + 0.5, rect.center_y());
        builder
            .move_to(Point::new(cx - 3.0, cy - 4.0))
            .line_to(Point::new(cx + 4.0, cy))
            .line_to(Point::new(cx - 3.0, cy + 4.0))
            .close();
        painter.fill_path(&builder.detach(), white);
        return;
    }
    let size = if engine.monogram.chars().count() > 1 {
        7.5
    } else {
        10.0
    };
    draw_text(
        painter,
        engine.monogram,
        TextStyle {
            size,
            weight: 700,
            tabular: false,
        },
        Rect::from_ltrb(rect.left - 2.0, rect.top, rect.right + 2.0, rect.bottom),
        Align::Center,
        white,
    );
}

fn draw_saved(ctx: &TabContext<'_>, state: &State, layout: &Layout, links: &[(String, String)]) {
    let painter = ctx.painter;
    let colors = ctx.colors;
    let card = layout.links_card;
    if card.height() < 40.0 {
        return;
    }
    painter.fill_round_rect(
        card,
        Radius::uniform(radius::CARD + 6.0),
        colors.surface_raised,
    );

    // Header.
    let header = layout.header;
    let icon = Rect::from_xywh(header.left, header.center_y() - 6.5, 13.0, 13.0);
    draw_svg_icon(painter, BOOKMARK_ICON, icon, colors.text_secondary, 2.0);
    let title = "Saved links";
    let title_width = text_width(title, LABEL);
    let title_rect = Rect::from_xywh(
        icon.right + 6.0,
        header.top,
        title_width + 2.0,
        header.height(),
    );
    draw_text(
        painter,
        title,
        LABEL,
        title_rect,
        Align::Start,
        colors.text_secondary,
    );
    let (note, note_color) = if state.copied_at.is_some() {
        ("Link copied".to_string(), colors.state_success)
    } else {
        let count = links.len();
        (
            format!("· {count} link{}", if count == 1 { "" } else { "s" }),
            colors.text_disabled,
        )
    };
    draw_text(
        painter,
        &note,
        LABEL,
        Rect::from_ltrb(
            title_rect.right + 6.0,
            header.top,
            layout.prev.left - 8.0,
            header.bottom,
        ),
        Align::Start,
        note_color,
    );

    // Paging.
    for (rect, svg, id, enabled) in [
        (layout.prev, PREV_ICON, ID_PREV, layout.page > 0),
        (
            layout.next,
            NEXT_ICON,
            ID_NEXT,
            layout.page + 1 < layout.pages,
        ),
    ] {
        let hover = enabled && hovered(ctx, id);
        if hover {
            painter.fill_circle(
                Point::new(rect.center_x(), rect.center_y()),
                rect.width() / 2.0,
                colors.surface_overlay,
            );
        }
        draw_svg_icon(
            painter,
            svg,
            rect.inset(6.0),
            if enabled {
                colors.text_primary
            } else {
                colors.text_disabled
            },
            2.0,
        );
    }

    // Add pill.
    let add = layout.add;
    let fill = if hovered(ctx, ID_ADD) {
        colors.text_primary.with_alpha_f(0.85)
    } else {
        colors.text_primary
    };
    painter.fill_round_rect(add, Radius::uniform(add.height() / 2.0), fill);
    let label_width = text_width("Add Link", PILL);
    let content = 12.0 + 4.0 + label_width;
    let start = add.center_x() - content / 2.0;
    draw_svg_icon(
        painter,
        PLUS_ICON,
        Rect::from_xywh(start, add.center_y() - 6.0, 12.0, 12.0),
        colors.surface_base,
        2.6,
    );
    draw_text(
        painter,
        "Add Link",
        PILL,
        Rect::from_xywh(start + 16.0, add.top, label_width + 2.0, add.height()),
        Align::Start,
        colors.surface_base,
    );

    if links.is_empty() {
        let area = layout.tile_area;
        draw_text(
            painter,
            "No saved links yet",
            TITLE,
            Rect::from_xywh(area.left, area.center_y() - 18.0, area.width(), 18.0),
            Align::Center,
            colors.text_primary,
        );
        draw_text(
            painter,
            "Use + Add Link to keep the sites you open most one click away",
            DOMAIN,
            Rect::from_xywh(area.left, area.center_y() + 2.0, area.width(), 16.0),
            Align::Center,
            colors.text_secondary,
        );
        return;
    }

    // Tiles.
    for (tile, index) in &layout.tiles {
        let Some((name, url)) = links.get(*index) else {
            continue;
        };
        let id = *index as u16;
        let more = more_button(*tile);
        let menu_open = state.menu == Some(*index);
        let hover = hovered(ctx, ID_TILE + id) || hovered(ctx, ID_MORE + id) || menu_open;
        if hover {
            painter.fill_round_rect(*tile, Radius::uniform(radius::ROW), colors.surface_overlay);
        }
        let host = domain(url);
        let tint = TINTS[host.bytes().fold(0usize, |hash, byte| {
            hash.wrapping_mul(31).wrapping_add(usize::from(byte))
        }) % TINTS.len()];
        let square = Rect::from_xywh(tile.left + 8.0, tile.center_y() - 15.0, 30.0, 30.0);
        painter.fill_round_rect(square, Radius::uniform(8.0), tint.with_alpha_f(0.22));
        let letter: String = name
            .chars()
            .chain(host.chars())
            .find(|c| c.is_alphanumeric())
            .map(|c| c.to_uppercase().collect())
            .unwrap_or_else(|| "•".to_string());
        draw_text(
            painter,
            &letter,
            TextStyle {
                size: 13.0,
                weight: 650,
                tabular: false,
            },
            square,
            Align::Center,
            colors.text_primary,
        );
        let text_left = square.right + 10.0;
        let text_right = more.left - 4.0;
        let title = if name.trim().is_empty() { &host } else { name };
        draw_text(
            painter,
            title,
            TITLE,
            Rect::from_ltrb(text_left, tile.top + 6.0, text_right, tile.top + 24.0),
            Align::Start,
            colors.text_primary,
        );
        draw_text(
            painter,
            &host,
            DOMAIN,
            Rect::from_ltrb(text_left, tile.top + 24.0, text_right, tile.bottom - 6.0),
            Align::Start,
            colors.text_secondary,
        );
        // ⋮ button.
        let more_hover = hovered(ctx, ID_MORE + id) || menu_open;
        if more_hover {
            painter.fill_round_rect(
                more,
                Radius::uniform(radius::CONTROL),
                colors.surface_interactive,
            );
        }
        let dot_color = if more_hover {
            colors.text_primary
        } else {
            colors.text_secondary
        };
        for step in [-5.0, 0.0, 5.0] {
            painter.fill_circle(
                Point::new(more.center_x(), more.center_y() + step),
                1.6,
                dot_color,
            );
        }
    }
}

fn draw_menu(ctx: &TabContext<'_>, menu: Rect) {
    let painter = ctx.painter;
    let colors = ctx.colors;
    painter.fill_round_rect(menu, Radius::uniform(radius::ROW), colors.surface_overlay);
    painter.stroke_round_rect(
        menu,
        Radius::uniform(radius::ROW),
        1.0,
        colors.border_strong,
    );
    for (index, label) in MENU_ITEMS.iter().enumerate() {
        let item = menu_item(menu, index);
        let hover = hovered(ctx, ID_MENU + index as u16);
        if hover {
            painter.fill_round_rect(
                item,
                Radius::uniform(radius::CONTROL),
                colors.surface_interactive,
            );
        }
        draw_text(
            painter,
            label,
            CHIP,
            Rect::from_ltrb(item.left + 10.0, item.top, item.right - 8.0, item.bottom),
            Align::Start,
            colors.text_primary,
        );
    }
}

/// `true` when this tab draws its own cards instead of sitting on the shared page card.
pub fn custom_layout() -> bool {
    true
}

/// Hit test in page coordinates. Use `DashboardHit::Tab(id)` with ids private to this tab.
pub fn hit(grid: Rect, point: Point) -> Option<DashboardHit> {
    if !grid.contains(point) {
        return None;
    }
    let count = crate::ui::dashboard::LINKS.with(|cell| cell.borrow().len().min(MAX_LINKS));
    let id =
        STATE.with(|cell| {
            let state = cell.borrow();
            let layout = compute_layout(grid, &state, count);
            if let Some((menu, _)) = layout.menu {
                if let Some(index) =
                    (0..MENU_ITEMS.len()).find(|index| menu_item(menu, *index).contains(point))
                {
                    return ID_MENU + index as u16;
                }
                if menu.contains(point) {
                    return ID_BACKGROUND;
                }
            }
            if let Some(paste) = layout.paste
                && paste.contains(point)
            {
                return ID_PASTE;
            }
            if layout.field.contains(point) {
                return ID_FIELD;
            }
            if layout.go.contains(point) {
                return ID_GO;
            }
            if let Some(index) = layout.engines.iter().position(|chip| {
                chip.contains(point) && chip.right <= layout.search_card.right - 4.0
            }) {
                return ID_ENGINE + index as u16;
            }
            if layout.prev.contains(point) {
                return ID_PREV;
            }
            if layout.next.contains(point) {
                return ID_NEXT;
            }
            if layout.add.contains(point) {
                return ID_ADD;
            }
            for (tile, index) in &layout.tiles {
                if more_button(*tile).contains(point) {
                    return ID_MORE + *index as u16;
                }
                if tile.contains(point) {
                    return ID_TILE + *index as u16;
                }
            }
            ID_BACKGROUND
        });
    Some(DashboardHit::Tab(id))
}

/// Applies a click from [`hit`]. `true` when handled.
pub fn apply(hit: DashboardHit) -> bool {
    let DashboardHit::Tab(id @ 900..=999) = hit else {
        STATE.with(|cell| {
            let mut state = cell.borrow_mut();
            state.focused = false;
            state.menu = None;
        });
        return false;
    };
    let count = crate::ui::dashboard::LINKS.with(|cell| cell.borrow().len().min(MAX_LINKS));
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let menu = state.menu.take();
        match id {
            ID_FIELD => state.focus(),
            ID_PASTE => {
                state.paste();
                state.focus();
            }
            ID_GO => state.submit(),
            ID_PREV => state.page = state.page.saturating_sub(1),
            ID_NEXT => state.page += 1,
            ID_ADD => {
                state.focused = false;
                open_settings();
            }
            ID_BACKGROUND => state.focused = false,
            _ if (ID_ENGINE..ID_ENGINE + ENGINES.len() as u16).contains(&id) => {
                state.engine = usize::from(id - ID_ENGINE);
            }
            _ if (ID_TILE..ID_TILE + MAX_LINKS as u16).contains(&id) => {
                let index = usize::from(id - ID_TILE);
                if index < count {
                    crate::ui::dashboard::open_link(index);
                }
            }
            _ if (ID_MORE..ID_MORE + MAX_LINKS as u16).contains(&id) => {
                let index = usize::from(id - ID_MORE);
                // A second click on the same ⋮ closes its menu.
                if menu != Some(index) && index < count {
                    state.menu = Some(index);
                }
            }
            _ if (ID_MENU..ID_MENU + MENU_ITEMS.len() as u16).contains(&id) => {
                if let Some(index) = menu {
                    match id - ID_MENU {
                        0 => crate::ui::dashboard::open_link(index),
                        1 => {
                            let url = crate::ui::dashboard::LINKS
                                .with(|cell| cell.borrow().get(index).map(|link| link.1.clone()));
                            if let Some(url) = url {
                                match winisland_platform_windows::set_clipboard_text(&url) {
                                    Ok(()) => {
                                        state.copied_at = Some(Instant::now());
                                        wake_after(COPIED_FOR + Duration::from_millis(50));
                                    }
                                    Err(error) => log::warn!("Cannot copy link: {error}"),
                                }
                            }
                        }
                        _ => {
                            state.focused = false;
                            open_settings();
                        }
                    }
                }
            }
            _ => {}
        }
    });
    true
}

pub fn wants_keyboard() -> bool {
    STATE.with(|cell| cell.borrow().focused)
}

pub fn handle_key(key: &Key) -> bool {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.focused {
            return false;
        }
        state.focus_since = Instant::now();
        match key {
            Key::Escape => {
                if state.query.is_empty() {
                    state.focused = false;
                } else {
                    state.query.clear();
                }
            }
            Key::Enter => state.submit(),
            Key::Backspace => {
                state.query.apply(EditKey::Backspace);
            }
            Key::Delete => {
                state.query.apply(EditKey::Delete);
            }
            Key::ArrowLeft => {
                state.query.apply(EditKey::Left);
            }
            Key::ArrowRight => {
                state.query.apply(EditKey::Right);
            }
            Key::Home => {
                state.query.apply(EditKey::Home);
            }
            Key::End => {
                state.query.apply(EditKey::End);
            }
            Key::Tab | Key::ArrowDown => state.engine = (state.engine + 1) % ENGINES.len(),
            Key::ArrowUp => state.engine = (state.engine + ENGINES.len() - 1) % ENGINES.len(),
            Key::Character(text) => {
                // Ctrl+V arrives as the SYN control character on some layouts.
                if text == "\u{16}" {
                    state.paste();
                } else {
                    let printable: String = text.chars().filter(|c| !c.is_control()).collect();
                    if !printable.is_empty() {
                        state.query.insert(&printable);
                    }
                }
            }
            Key::Other => {}
        }
        true
    })
}

/// Every frame, visible or not.
pub fn poll(now: Instant) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        // Stop editing once the tab is no longer on screen.
        if state.focused
            && state.last_drawn.is_none_or(|drawn| {
                now.saturating_duration_since(drawn) > Duration::from_millis(500)
            })
        {
            state.focused = false;
            state.menu = None;
        }
    });
}
