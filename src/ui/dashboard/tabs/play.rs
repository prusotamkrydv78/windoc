//! Play tab: two quick games to share scores with friends.
//!
//! Reflex: wait for the robot's eye to turn green, then click (or press Space) as fast as
//! possible. Catch: sparks pop up across the arena for 20 seconds; click as many as you can
//! before each one fades. Best scores are kept in `play.json` in the data folder.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use winisland_platform::Key;
use winisland_render::tokens::{TextStyle, radius};
use winisland_render::{Point, Radius, Rect, Rgba};

use super::TabContext;
use super::shelf::kit::{style, text, text_center, text_right, text_width};
use crate::ui::dashboard::DashboardHit;

const ID_MODE_REFLEX: u16 = 1500;
const ID_MODE_CATCH: u16 = 1501;
const ID_ARENA: u16 = 1502;
const ID_RESET: u16 = 1503;
const ID_SPARK: u16 = 1510;
const MAX_SPARKS: usize = 6;

const GAP: f32 = 10.0;
const PAD: f32 = 14.0;
const HISTORY: usize = 8;
const CATCH_ROUND: Duration = Duration::from_secs(20);
const SPARK_LIFE: Duration = Duration::from_millis(1400);
const FILE_NAME: &str = "play.json";

const TITLE: TextStyle = style(16.0, 650);
const LABEL: TextStyle = style(12.0, 500);
const BUTTON: TextStyle = style(12.0, 600);
const BIG: TextStyle = TextStyle {
    size: 30.0,
    weight: 700,
    tabular: true,
};
const NUMBER: TextStyle = TextStyle {
    size: 13.0,
    weight: 600,
    tabular: true,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Reflex,
    Catch,
}

#[derive(Clone, Copy, PartialEq)]
enum Reflex {
    Idle,
    /// Waiting for green, which comes at the given moment.
    Waiting(Instant),
    Go(Instant),
    TooSoon,
    Result(u32),
}

struct Spark {
    center: Point,
    born: Instant,
    hue: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum Catch {
    Idle,
    Playing(Instant),
    Over,
}

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    best_reflex_ms: Option<u32>,
    best_catch: u32,
}

struct State {
    mode: Mode,
    reflex: Reflex,
    /// Recent reaction times, newest last.
    history: Vec<u32>,
    catch: Catch,
    sparks: Vec<Spark>,
    caught: u32,
    missed: u32,
    next_spark: Option<Instant>,
    new_best: bool,
    saved: Saved,
    rng: u64,
}

impl State {
    fn new() -> Self {
        let saved = match crate::core::json_store::read::<Saved>(FILE_NAME, 4096) {
            Ok(saved) => saved.unwrap_or_default(),
            Err(error) => {
                log::warn!("Cannot read Play scores: {error}");
                Saved::default()
            }
        };
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9E37_79B9, |time| time.as_nanos() as u64);
        Self {
            mode: Mode::Reflex,
            reflex: Reflex::Idle,
            history: Vec::new(),
            catch: Catch::Idle,
            sparks: Vec::new(),
            caught: 0,
            missed: 0,
            next_spark: None,
            new_best: false,
            saved,
            rng: seed | 1,
        }
    }

    /// A number in `0.0..1.0` (xorshift; good enough for a game).
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    fn save(&self) {
        if let Err(error) = crate::core::json_store::write(FILE_NAME, &self.saved) {
            log::warn!("Cannot save Play scores: {error}");
        }
    }

    fn running(&self) -> bool {
        match self.mode {
            Mode::Reflex => matches!(self.reflex, Reflex::Waiting(_) | Reflex::Go(_)),
            Mode::Catch => matches!(self.catch, Catch::Playing(_)),
        }
    }
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static LAST_GRID: Cell<Rect> = const { Cell::new(Rect::from_xywh(0.0, 0.0, 720.0, 230.0)) };
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|cell| f(cell.borrow_mut().get_or_insert_with(State::new)))
}

struct Layout {
    arena: Rect,
    side: Rect,
    reflex_tab: Rect,
    catch_tab: Rect,
    reset: Rect,
}

fn layout(grid: Rect) -> Layout {
    let side_w = (grid.width() * 0.32).clamp(190.0, 260.0).min(grid.width());
    let side = Rect::from_ltrb(grid.right - side_w, grid.top, grid.right, grid.bottom);
    let arena = Rect::from_ltrb(grid.left, grid.top, side.left - GAP, grid.bottom);
    let inner = side.inset(PAD);
    let tab_w = (inner.width() - 4.0) / 2.0;
    let reflex_tab = Rect::from_xywh(inner.left, inner.top, tab_w, 28.0);
    let catch_tab = Rect::from_xywh(reflex_tab.right + 4.0, inner.top, tab_w, 28.0);
    let reset_w = text_width("Reset best", BUTTON) + 24.0;
    let reset = Rect::from_xywh(inner.right - reset_w, inner.bottom - 24.0, reset_w, 24.0);
    Layout {
        arena,
        side,
        reflex_tab,
        catch_tab,
        reset,
    }
}

/// Sparks spawn inside this part of the arena, clear of the status text.
fn spark_field(arena: Rect) -> Rect {
    Rect::from_ltrb(
        arena.left + 34.0,
        arena.top + 56.0,
        arena.right - 34.0,
        arena.bottom - 26.0,
    )
}

const SPARK_RADIUS: f32 = 17.0;

const HUES: [Rgba; 5] = [
    Rgba::from_rgba(255, 122, 89, 255),
    Rgba::from_rgba(255, 196, 61, 255),
    Rgba::from_rgba(94, 214, 140, 255),
    Rgba::from_rgba(92, 170, 255, 255),
    Rgba::from_rgba(196, 120, 255, 255),
];

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

fn hovered(ctx: &TabContext<'_>, id: u16) -> bool {
    ctx.hover == Some(DashboardHit::Tab(id))
}

fn segment(ctx: &TabContext<'_>, rect: Rect, label: &str, selected: bool, hover: bool) {
    let colors = ctx.colors;
    let fill = if selected {
        colors.accent_primary
    } else if hover {
        colors.surface_interactive
    } else {
        colors.surface_overlay
    };
    ctx.painter
        .fill_round_rect(rect, Radius::uniform(rect.height() / 2.0), fill);
    let color = if selected {
        Rgba::WHITE
    } else {
        colors.text_secondary
    };
    text_center(ctx.painter, label, BUTTON, rect, color);
}

/// Draws the tab. Return `false` to fall back to the legacy page.
pub fn draw(ctx: &TabContext<'_>) -> bool {
    LAST_GRID.with(|cell| cell.set(ctx.grid));
    let layout = layout(ctx.grid);
    with_state(|state| {
        tick(state, ctx.now, layout.arena);
        card(ctx, layout.arena);
        card(ctx, layout.side);
        let saved = ctx.painter.save();
        ctx.painter
            .clip_round_rect(layout.arena, Radius::uniform(radius::CARD));
        match state.mode {
            Mode::Reflex => draw_reflex(ctx, &layout, state),
            Mode::Catch => draw_catch(ctx, &layout, state),
        }
        ctx.painter.restore_to(saved);
        draw_side(ctx, &layout, state);
    });
    true
}

fn draw_reflex(ctx: &TabContext<'_>, layout: &Layout, state: &State) {
    let colors = ctx.colors;
    let arena = layout.arena;
    let (eye, title, hint) = match state.reflex {
        Reflex::Idle => (
            colors.accent_primary,
            "Reflex".to_string(),
            "Click when the eye turns green. Click (or Space) to start.".to_string(),
        ),
        Reflex::Waiting(_) => (
            colors.state_error,
            "Wait for it…".to_string(),
            "Don't click yet".to_string(),
        ),
        Reflex::Go(_) => (colors.state_success, "NOW!".to_string(), String::new()),
        Reflex::TooSoon => (
            colors.state_warning,
            "Too soon!".to_string(),
            "Click to try again".to_string(),
        ),
        Reflex::Result(ms) => (
            colors.accent_primary,
            format!("{ms} ms"),
            format!(
                "{}{}  ·  click to go again",
                rating(ms),
                if state.new_best { "  ·  new best!" } else { "" }
            ),
        ),
    };
    let hover = hovered(ctx, ID_ARENA);
    if matches!(state.reflex, Reflex::Go(_)) {
        ctx.painter.fill_round_rect(
            arena,
            Radius::uniform(radius::CARD),
            colors.state_success.with_alpha(40),
        );
    }
    // The robot's eye: a glowing pill that breathes while waiting.
    let pulse = match state.reflex {
        Reflex::Waiting(at) => {
            let t = at.saturating_duration_since(ctx.now).as_secs_f32();
            0.5 + 0.5 * (t * 4.0).sin()
        }
        _ => 1.0,
    };
    let center = Point::new(arena.center_x(), arena.top + arena.height() * 0.42);
    let eye_w = (arena.width() * 0.28).clamp(80.0, 160.0);
    let eye_h = eye_w * 0.42;
    let eye_rect = Rect::from_xywh(center.x - eye_w / 2.0, center.y - eye_h / 2.0, eye_w, eye_h);
    for ring in 1..=3 {
        let grow = ring as f32 * 7.0;
        ctx.painter.fill_round_rect(
            eye_rect.inset(-grow),
            Radius::uniform(eye_h / 2.0 + grow),
            eye.with_alpha((26.0 * pulse) as u8 / ring as u8),
        );
    }
    ctx.painter
        .fill_round_rect(eye_rect, Radius::uniform(eye_h / 2.0), eye);
    if hover && !state.running() {
        ctx.painter.stroke_round_rect(
            eye_rect.inset(-3.0),
            Radius::uniform(eye_h / 2.0 + 3.0),
            1.5,
            colors.text_secondary,
        );
    }
    let title_rect = Rect::from_xywh(arena.left, eye_rect.bottom + 14.0, arena.width(), 38.0);
    text_center(ctx.painter, &title, BIG, title_rect, colors.text_primary);
    let hint_rect = Rect::from_xywh(arena.left + PAD, title_rect.bottom, arena.width() - PAD * 2.0, 20.0);
    text_center(ctx.painter, &hint, LABEL, hint_rect, colors.text_secondary);
}

fn rating(ms: u32) -> &'static str {
    match ms {
        0..=180 => "Lightning",
        181..=230 => "Sharp",
        231..=290 => "Quick",
        291..=380 => "Okay",
        _ => "Sleepy",
    }
}

fn draw_catch(ctx: &TabContext<'_>, layout: &Layout, state: &State) {
    let colors = ctx.colors;
    let arena = layout.arena;
    let header = Rect::from_xywh(arena.left + PAD, arena.top + PAD, arena.width() - PAD * 2.0, 22.0);
    match state.catch {
        Catch::Idle | Catch::Over => {
            let center = arena.center_y();
            let (title, hint) = if state.catch == Catch::Idle {
                (
                    "Catch".to_string(),
                    "Sparks pop up for 20 seconds. Click them before they fade. Click to start."
                        .to_string(),
                )
            } else {
                (
                    format!("{} caught", state.caught),
                    format!(
                        "{} missed{}  ·  click to play again",
                        state.missed,
                        if state.new_best { "  ·  new best!" } else { "" }
                    ),
                )
            };
            for (index, hue) in HUES.iter().enumerate() {
                let x = arena.center_x() + (index as f32 - 2.0) * 30.0;
                ctx.painter
                    .fill_circle(Point::new(x, center - 40.0), 9.0, *hue);
            }
            text_center(
                ctx.painter,
                &title,
                BIG,
                Rect::from_xywh(arena.left, center - 18.0, arena.width(), 38.0),
                colors.text_primary,
            );
            text_center(
                ctx.painter,
                &hint,
                LABEL,
                Rect::from_xywh(arena.left + PAD, center + 22.0, arena.width() - PAD * 2.0, 20.0),
                colors.text_secondary,
            );
        }
        Catch::Playing(started) => {
            let left = CATCH_ROUND.saturating_sub(ctx.now.saturating_duration_since(started));
            text(
                ctx.painter,
                &format!("{} caught", state.caught),
                TITLE,
                header,
                colors.text_primary,
            );
            text_right(
                ctx.painter,
                &format!("{:.1}s", left.as_secs_f32()),
                NUMBER,
                header,
                colors.text_secondary,
            );
            // Time bar under the header.
            let bar = Rect::from_xywh(header.left, header.bottom + 6.0, header.width(), 3.0);
            ctx.painter
                .fill_round_rect(bar, Radius::uniform(1.5), colors.surface_overlay);
            let share = left.as_secs_f32() / CATCH_ROUND.as_secs_f32();
            ctx.painter.fill_round_rect(
                Rect::from_xywh(bar.left, bar.top, bar.width() * share, bar.height()),
                Radius::uniform(1.5),
                colors.accent_primary,
            );
            for (index, spark) in state.sparks.iter().enumerate() {
                let age = ctx.now.saturating_duration_since(spark.born).as_secs_f32()
                    / SPARK_LIFE.as_secs_f32();
                // Pops in, then shrinks and fades.
                let grow = (age * 8.0).min(1.0);
                let fade = (1.0 - age).clamp(0.0, 1.0);
                let radius = SPARK_RADIUS * grow * (0.55 + 0.45 * fade);
                let hue = HUES[spark.hue % HUES.len()];
                ctx.painter.fill_circle(
                    spark.center,
                    radius + 6.0,
                    hue.with_alpha((60.0 * fade) as u8),
                );
                ctx.painter
                    .fill_circle(spark.center, radius, hue.with_alpha((255.0 * fade) as u8));
                if hovered(ctx, ID_SPARK + index as u16) {
                    ctx.painter.stroke_circle(
                        spark.center,
                        radius + 3.0,
                        1.5,
                        Rgba::WHITE.with_alpha(200),
                    );
                }
            }
        }
    }
}

fn draw_side(ctx: &TabContext<'_>, layout: &Layout, state: &State) {
    let colors = ctx.colors;
    segment(
        ctx,
        layout.reflex_tab,
        "Reflex",
        state.mode == Mode::Reflex,
        hovered(ctx, ID_MODE_REFLEX),
    );
    segment(
        ctx,
        layout.catch_tab,
        "Catch",
        state.mode == Mode::Catch,
        hovered(ctx, ID_MODE_CATCH),
    );
    let inner = layout.side.inset(PAD);
    let mut y = layout.reflex_tab.bottom + 16.0;
    let row = |label: &str, value: &str, y: f32| {
        let rect = Rect::from_xywh(inner.left, y, inner.width(), 20.0);
        text(ctx.painter, label, LABEL, rect, colors.text_secondary);
        text_right(ctx.painter, value, NUMBER, rect, colors.text_primary);
    };
    match state.mode {
        Mode::Reflex => {
            let best = state
                .saved
                .best_reflex_ms
                .map_or_else(|| "—".to_string(), |ms| format!("{ms} ms"));
            row("Best ever", &best, y);
            y += 24.0;
            if !state.history.is_empty() {
                let average =
                    state.history.iter().sum::<u32>() / state.history.len() as u32;
                row("Average now", &format!("{average} ms"), y);
                y += 30.0;
                draw_history(ctx, Rect::from_ltrb(inner.left, y, inner.right, layout.reset.top - 10.0), &state.history);
            }
        }
        Mode::Catch => {
            row("Best ever", &state.saved.best_catch.to_string(), y);
            y += 24.0;
            if state.catch != Catch::Idle {
                row("This round", &state.caught.to_string(), y);
                y += 24.0;
                let shots = state.caught + state.missed;
                if shots > 0 {
                    row("Accuracy", &format!("{}%", state.caught * 100 / shots), y);
                }
            }
        }
    }
    let has_best = match state.mode {
        Mode::Reflex => state.saved.best_reflex_ms.is_some(),
        Mode::Catch => state.saved.best_catch > 0,
    };
    if has_best && !state.running() {
        let hover = hovered(ctx, ID_RESET);
        if hover {
            ctx.painter.fill_round_rect(
                layout.reset,
                Radius::uniform(12.0),
                colors.surface_overlay,
            );
        }
        text_center(
            ctx.painter,
            "Reset best",
            BUTTON,
            layout.reset,
            if hover {
                colors.text_primary
            } else {
                colors.text_disabled
            },
        );
    }
}

/// Bars for the recent reaction times: shorter is faster, so faster bars are taller.
fn draw_history(ctx: &TabContext<'_>, area: Rect, history: &[u32]) {
    if area.height() < 20.0 {
        return;
    }
    let slot = area.width() / HISTORY as f32;
    let start = HISTORY - history.len();
    for (index, &ms) in history.iter().enumerate() {
        let speed = (1.0 - (ms as f32 - 150.0) / 450.0).clamp(0.12, 1.0);
        let h = area.height() * speed;
        let x = area.left + (start + index) as f32 * slot + slot * 0.2;
        let bar = Rect::from_xywh(x, area.bottom - h, slot * 0.6, h);
        let color = if index + 1 == history.len() {
            ctx.colors.accent_primary
        } else {
            ctx.colors.accent_primary.with_alpha(110)
        };
        ctx.painter
            .fill_round_rect(bar, Radius::uniform((slot * 0.3).min(4.0)), color);
    }
}

/// Moves the games along: green light, spark spawns and expiry, end of round.
fn tick(state: &mut State, now: Instant, arena: Rect) {
    if let Reflex::Waiting(at) = state.reflex {
        if now >= at {
            state.reflex = Reflex::Go(now);
        }
    }
    let Catch::Playing(started) = state.catch else {
        return;
    };
    if now.saturating_duration_since(started) >= CATCH_ROUND {
        state.missed += state.sparks.len() as u32;
        state.sparks.clear();
        state.catch = Catch::Over;
        state.new_best = state.caught > state.saved.best_catch;
        if state.new_best {
            state.saved.best_catch = state.caught;
            state.save();
        }
        return;
    }
    let before = state.sparks.len();
    state
        .sparks
        .retain(|spark| now.saturating_duration_since(spark.born) < SPARK_LIFE);
    state.missed += (before - state.sparks.len()) as u32;
    if state.next_spark.is_none_or(|at| now >= at) && state.sparks.len() < MAX_SPARKS {
        let field = spark_field(arena);
        // Sparks come faster as the round goes on.
        let progress = now.saturating_duration_since(started).as_secs_f32()
            / CATCH_ROUND.as_secs_f32();
        for _ in 0..8 {
            let center = Point::new(
                field.left + state.random() * field.width().max(1.0),
                field.top + state.random() * field.height().max(1.0),
            );
            let clear = state.sparks.iter().all(|spark| {
                let (dx, dy) = (spark.center.x - center.x, spark.center.y - center.y);
                dx * dx + dy * dy > (SPARK_RADIUS * 2.6).powi(2)
            });
            if clear {
                let hue = (state.random() * HUES.len() as f32) as usize;
                state.sparks.push(Spark {
                    center,
                    born: now,
                    hue,
                });
                break;
            }
        }
        let gap = 0.75 - 0.4 * progress + state.random() * 0.25;
        state.next_spark = Some(now + Duration::from_secs_f32(gap));
    }
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
    with_state(|state| {
        if layout.reflex_tab.contains(point) {
            return Some(ID_MODE_REFLEX);
        }
        if layout.catch_tab.contains(point) {
            return Some(ID_MODE_CATCH);
        }
        if layout.reset.contains(point) && !state.running() {
            return Some(ID_RESET);
        }
        if !layout.arena.contains(point) {
            return None;
        }
        if state.mode == Mode::Catch && matches!(state.catch, Catch::Playing(_)) {
            let found = state.sparks.iter().position(|spark| {
                let (dx, dy) = (spark.center.x - point.x, spark.center.y - point.y);
                dx * dx + dy * dy <= (SPARK_RADIUS + 6.0).powi(2)
            });
            return Some(found.map_or(ID_ARENA, |index| ID_SPARK + index as u16));
        }
        Some(ID_ARENA)
    })
    .map(DashboardHit::Tab)
}

/// Applies a click from [`hit`]. `true` when handled.
pub fn apply(hit: DashboardHit) -> bool {
    let DashboardHit::Tab(id) = hit else {
        return false;
    };
    if !(1500..=1599).contains(&id) {
        return false;
    }
    let now = Instant::now();
    with_state(|state| match id {
        ID_MODE_REFLEX | ID_MODE_CATCH => {
            state.mode = if id == ID_MODE_REFLEX {
                Mode::Reflex
            } else {
                Mode::Catch
            };
            state.reflex = Reflex::Idle;
            state.catch = Catch::Idle;
            state.sparks.clear();
            state.new_best = false;
        }
        ID_RESET => {
            match state.mode {
                Mode::Reflex => {
                    state.saved.best_reflex_ms = None;
                    state.history.clear();
                }
                Mode::Catch => state.saved.best_catch = 0,
            }
            state.new_best = false;
            state.save();
        }
        ID_ARENA => press(state, now),
        _ if (ID_SPARK..ID_SPARK + MAX_SPARKS as u16).contains(&id) => {
            let index = (id - ID_SPARK) as usize;
            if matches!(state.catch, Catch::Playing(_)) && index < state.sparks.len() {
                state.sparks.remove(index);
                state.caught += 1;
            }
        }
        _ => {}
    });
    true
}

/// A click on the arena, or Space/Enter.
fn press(state: &mut State, now: Instant) {
    match state.mode {
        Mode::Reflex => match state.reflex {
            Reflex::Idle | Reflex::TooSoon | Reflex::Result(_) => {
                let wait = 1.2 + state.random() * 2.8;
                state.reflex = Reflex::Waiting(now + Duration::from_secs_f32(wait));
                state.new_best = false;
            }
            Reflex::Waiting(at) if now < at => state.reflex = Reflex::TooSoon,
            Reflex::Waiting(at) | Reflex::Go(at) => {
                let ms = now.saturating_duration_since(at).as_millis().min(9_999) as u32;
                state.reflex = Reflex::Result(ms);
                state.history.push(ms);
                if state.history.len() > HISTORY {
                    state.history.remove(0);
                }
                state.new_best = state.saved.best_reflex_ms.is_none_or(|best| ms < best);
                if state.new_best {
                    state.saved.best_reflex_ms = Some(ms);
                    state.save();
                }
            }
        },
        Mode::Catch => match state.catch {
            Catch::Idle | Catch::Over => {
                state.catch = Catch::Playing(now);
                state.sparks.clear();
                state.caught = 0;
                state.missed = 0;
                state.next_spark = Some(now + Duration::from_millis(500));
                state.new_best = false;
            }
            // A click on empty arena during a round counts as a miss.
            Catch::Playing(_) => state.missed += 1,
        },
    }
}

/// While a game runs the tab takes Space and Enter, and the panel stays open.
pub fn wants_keyboard() -> bool {
    animating()
}

pub fn handle_key(key: &Key) -> bool {
    let start = matches!(key, Key::Enter) || matches!(key, Key::Character(text) if text == " ");
    if !start {
        return false;
    }
    with_state(|state| {
        if state.mode == Mode::Reflex {
            press(state, Instant::now());
            true
        } else {
            false
        }
    })
}

/// Whether the visible Play tab needs smooth frames.
pub fn animating() -> bool {
    STATE.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_some_and(|state| state.running())
    })
}

/// Every frame, visible or not.
pub fn poll(_now: Instant) {}
