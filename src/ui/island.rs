mod background;
mod expanded;
mod mini;

use std::cell::RefCell;
use std::collections::HashMap;

pub(crate) use mini::{
    lyric_font_size as mini_lyric_font_size, lyric_insets as mini_lyric_insets,
    lyric_pair_height as mini_lyric_pair_height,
};

use self::background::{BackgroundParams, draw_background};
use self::expanded::{ExpandedContentParams, draw_expanded_content};
use self::mini::{MiniContentParams, draw_mini_content};

use crate::core::smtc::MediaInfo;
use crate::ui::compact::CompactOverlay;
use crate::ui::expanded::music_view::{default_media_palette, get_media_palette};
use crate::ui::expanded::pager::{self, ExpandedPage, PagerParams};
use winisland_core::config::{
    CompactWidgetSlot, LyricTransitionAnimation, PluginWidgetSlot, WidgetSlot,
};
use winisland_core::lyrics::LyricHighlight;
use winisland_plugin_host::draw::replay::PreparedFrame;
use winisland_plugin_host::host::PluginHost;
use winisland_render::DrawingContext;
use winisland_render::{BlurSpec, Image, Painter, Path, Point, RasterSurface, Rect, Rgba, Vec2};

pub struct LayoutParams {
    pub pager_above: bool,
    pub pager_bar_hover: f32,
    pub pager_close_hover: f32,
    pub current_w: f32,
    pub current_h: f32,
    pub current_r: f32,
    pub sigmas: (f32, f32),
    pub shadow_static: bool,
    pub expansion_progress: f32,
    pub view_offset: f32,
    pub compact_scale: f32,
    pub expanded_scale: f32,
    pub hide_progress: f32,
    pub compact_widget_opacity: f32,
    pub island_x: f32,
    pub island_y: f32,
    pub stable_island_y: f32,
    pub base_h: f32,
}

pub struct MediaParams<'a> {
    pub media: &'a MediaInfo,
    pub music_active: bool,
    pub available_controls: u32,
}

pub struct LyricsParams<'a> {
    pub current_lyric: &'a str,
    pub current_secondary_lyric: &'a str,
    pub old_lyric: &'a str,
    pub old_secondary_lyric: &'a str,
    pub lyric_highlight: Option<LyricHighlight>,
    pub lyric_transition: f32,
    pub lyric_transition_animation: LyricTransitionAnimation,
    pub lyric_scroll_offset: f32,
    pub lyric_side_gap: f32,
}

pub struct StyleParams<'a> {
    pub island_style: &'a str,
    pub host_backdrop: bool,
    pub use_blur: bool,
    pub font_size: f32,
    pub dt: f32,
    pub expanded_width: f32,
    pub expanded_height: f32,
    pub widget_layout: &'a [WidgetSlot],
    pub plugin_widget_layout: &'a [PluginWidgetSlot],
    pub plugin_widgets: &'a winisland_core::widgets::WidgetManager,
    pub plugin_frames: &'a HashMap<u64, PreparedFrame>,
    pub plugin_host: Option<&'a PluginHost>,
    pub compact_widget_layout: &'a [CompactWidgetSlot],
    pub pages: &'a [ExpandedPage],
    /// Draw the island attached to the top screen edge with concave shoulders.
    pub attached: bool,
    /// Height of the top bar in window pixels, or 0. When set, the notch hangs from the bar's
    /// lower edge instead of the top of the screen.
    pub bar_height: f32,
}

use winisland_core::context::MiniContent;

const MIN_VISIBLE_OPACITY: f32 = 0.01;
const MIN_BLUR_SIGMA: f32 = 0.1;
const MINI_FADE_RATE: f32 = 1.5;
const COLLAPSED_VISUALIZER_HEIGHT_SCALE: f32 = 0.45;
const BORDER_WIDTH: f32 = 1.0;
const BORDER_INSET: f32 = BORDER_WIDTH / 2.0;
const SOLID_STYLE: &str = "default";
const SOLID_BORDER_ALPHA: u8 = 30;
const EFFECT_BORDER_ALPHA: u8 = 40;

struct CachedShadow {
    key: (i32, i32, i32, i32, u8),
    image: Image,
}

thread_local! {
    static SHADOW_CACHE: RefCell<Option<CachedShadow>> = const { RefCell::new(None) };
}

pub struct DrawIslandParams<'a> {
    pub layout: LayoutParams,
    pub media: MediaParams<'a>,
    pub lyrics: LyricsParams<'a>,
    pub mini_content: Option<MiniContent<'a>>,
    pub compact_overlay: &'a CompactOverlay,
    pub style: StyleParams<'a>,
    pub attention_alpha: f32,
}

pub fn expanded_content_alpha(
    expansion_progress: f32,
    hide_progress: f32,
    compact_overlay_visible: bool,
) -> f32 {
    if compact_overlay_visible {
        0.0
    } else {
        // Content waits until the shape is about a third open, then eases in, so the panel
        // grows first and its contents arrive after (as in the design prototype).
        ((expansion_progress - 0.35) / 0.65)
            .clamp(0.0, 1.0)
            .powf(1.5)
            * (1.0 - hide_progress)
    }
}

pub fn draw_island(
    drawing_context: &mut DrawingContext<'_>,
    painter: Painter<'_>,
    params: DrawIslandParams<'_>,
) -> bool {
    let layout = &params.layout;
    let rect = Rect::from_xywh(
        layout.island_x,
        layout.island_y,
        layout.current_w,
        layout.current_h,
    );
    let dim = idle_dim(params.style.dt);
    if dim < 0.995 {
        painter.save_alpha((dim * 255.0) as u8);
    }
    let island_path = island_shape(&params, rect);
    let blur_filter = if layout.sigmas.0 > MIN_BLUR_SIGMA || layout.sigmas.1 > MIN_BLUR_SIGMA {
        Some(BlurSpec {
            sigma: layout.sigmas,
            tile: None,
        })
    } else {
        None
    };
    draw_expanded_shadow(painter, &params, &island_path);
    draw_album_glow(painter, &params, rect);
    let edge_color = edge_light_color(&params);
    if params.attention_alpha > 0.0 {
        // Attention: a soft amber glow around the real notch shape, behind it, so only the
        // light spilling past the edge shows.
        let scale = params.layout.compact_scale.max(0.5);
        painter.fill_path_blurred(
            &island_path,
            Rgba::from_argb(
                (150.0 * params.attention_alpha.clamp(0.0, 1.0)) as u8,
                255,
                196,
                64,
            ),
            BlurSpec {
                sigma: (9.0 * scale, 9.0 * scale),
                tile: None,
            },
        );
    }
    draw_background_layer(painter, drawing_context, &params, rect, &island_path);
    if let Some(color) = edge_color {
        draw_edge_line(painter, &params, rect, &island_path, color);
    }
    painter.save();
    painter.clip_path(&island_path);

    let compact_overlay_visible = params.compact_overlay.is_visible();
    let expanded_alpha = expanded_content_alpha(
        layout.expansion_progress,
        layout.hide_progress,
        compact_overlay_visible,
    );
    let mini_alpha = if compact_overlay_visible {
        0.0
    } else {
        (1.0 - layout.expansion_progress * MINI_FADE_RATE).clamp(0.0, 1.0)
            * (1.0 - layout.hide_progress)
    };
    let palette = if expanded_alpha > MIN_VISIBLE_OPACITY || mini_alpha > MIN_VISIBLE_OPACITY {
        get_media_palette(params.media.media)
    } else {
        default_media_palette()
    };
    let visualizer_height_scale = COLLAPSED_VISUALIZER_HEIGHT_SCALE
        + (1.0 - COLLAPSED_VISUALIZER_HEIGHT_SCALE) * layout.expansion_progress;
    let widget_animating = draw_expanded_layer(
        painter,
        blur_filter,
        &params,
        &palette,
        expanded_alpha,
        visualizer_height_scale,
    );
    if compact_overlay_visible {
        params.compact_overlay.draw(
            painter,
            rect,
            layout.compact_scale,
            1.0 - layout.hide_progress,
        );
    } else {
        draw_compact_layer(
            painter,
            &params,
            &palette,
            mini_alpha,
            visualizer_height_scale,
        );
    }
    painter.restore();
    draw_island_border(painter, &params);
    draw_privacy_dots(painter, &params, rect);
    // The rim belongs to the small notch; the open panel is pure black with no outline.
    let rim = (1.0 - params.layout.expansion_progress).clamp(0.0, 1.0);
    if let Some(color) = edge_color.filter(|_| rim > 0.01) {
        painter.stroke_path(
            &island_path,
            1.2,
            color.with_alpha((210.0 * rim) as u8),
            winisland_render::StrokeCap::Round,
            winisland_render::StrokeJoin::Round,
        );
    }
    draw_pager(painter, drawing_context, &params, rect, expanded_alpha);
    if dim < 0.995 {
        painter.restore();
    }
    widget_animating
}

fn draw_expanded_shadow(painter: Painter<'_>, params: &DrawIslandParams<'_>, island_path: &Path) {
    let layout = &params.layout;
    let opacity = layout.expansion_progress.clamp(0.0, 1.0).powi(2)
        * (1.0 - layout.hide_progress).clamp(0.0, 1.0);
    if opacity <= MIN_VISIBLE_OPACITY {
        return;
    }
    let scale = layout.expanded_scale;
    let offset_y = 2.0 * scale;
    let bounds = island_path.bounds();
    let (surface_width, surface_height) = painter.surface_size();
    let margin = bounds
        .left
        .min(surface_width as f32 - bounds.right)
        .min(bounds.top + offset_y)
        .min(surface_height as f32 - bounds.bottom - offset_y)
        .max(0.0);
    let sigma = (3.0 * scale).min(margin / 3.0);
    let alpha = (28.0 * opacity) as u8;
    if layout.shadow_static && sigma > 0.0 {
        let key = (
            (bounds.width() * 16.0).round() as i32,
            (bounds.height() * 16.0).round() as i32,
            (layout.current_r * 16.0).round() as i32,
            (sigma * 16.0).round() as i32,
            alpha,
        );
        let cached = SHADOW_CACHE.with(|cell| {
            let mut cache = cell.borrow_mut();
            if cache.as_ref().is_none_or(|entry| entry.key != key) {
                let padding = (sigma * 3.0 + offset_y).ceil() as i32 + 2;
                let width = bounds.width().ceil() as i32 + padding * 2;
                let height = bounds.height().ceil() as i32 + padding * 2;
                let mut surface = RasterSurface::new(width, height)?;
                surface.clear(Rgba::TRANSPARENT);
                let raster = surface.painter();
                raster.translate(Vec2::new(
                    padding as f32 - bounds.left,
                    padding as f32 - bounds.top,
                ));
                raster.translate(Vec2::new(0.0, offset_y));
                raster.fill_path_blurred(
                    island_path,
                    Rgba::BLACK.with_alpha(alpha),
                    BlurSpec::uniform(sigma),
                );
                *cache = Some(CachedShadow {
                    key,
                    image: surface.snapshot(),
                });
            }
            cache.as_ref().map(|entry| entry.image.clone())
        });
        if let Some(image) = cached {
            let padding = (sigma * 3.0 + offset_y).ceil() + 2.0;
            painter.save();
            painter.clip_path_difference(island_path);
            painter.draw_image_at(
                &image,
                Point::new(bounds.left - padding, bounds.top - padding),
            );
            painter.restore();
            return;
        }
    }
    painter.save();
    painter.clip_path_difference(island_path);
    painter.translate(Vec2::new(0.0, offset_y));
    painter.fill_path_blurred(
        island_path,
        Rgba::BLACK.with_alpha(alpha),
        BlurSpec::uniform(sigma),
    );
    painter.restore();
}

fn draw_background_layer(
    painter: Painter<'_>,
    drawing_context: &mut DrawingContext<'_>,
    params: &DrawIslandParams<'_>,
    rect: Rect,
    island_path: &Path,
) {
    draw_background(BackgroundParams {
        painter,
        drawing_context,
        rect,
        island_path,
        island_style: params.style.island_style,
        host_backdrop: params.style.host_backdrop,
        media: params.media.media,
    });
}

fn draw_expanded_layer(
    painter: Painter<'_>,
    blur_filter: Option<BlurSpec>,
    params: &DrawIslandParams<'_>,
    palette: &[Rgba],
    alpha: f32,
    visualizer_height_scale: f32,
) -> bool {
    let layout = &params.layout;
    let media = &params.media;
    let style = &params.style;
    let original_h = (style.expanded_height * layout.expanded_scale).min(layout.current_h);
    let widget_animating = draw_expanded_content(ExpandedContentParams {
        painter,
        blur_filter,
        expanded_alpha: alpha,
        pages: style.pages,
        view_offset: layout.view_offset,
        current_w: layout.current_w,
        offset_x: layout.island_x,
        offset_y: layout.island_y,
        current_h: original_h,
        media: media.media,
        music_active: media.music_active,
        available_controls: media.available_controls,
        global_scale: layout.expanded_scale,
        expansion_progress: layout.expansion_progress,
        viz_h_scale: visualizer_height_scale,
        use_blur: style.use_blur,
        font_size: style.font_size,
        dt: style.dt,
        expanded_width: style.expanded_width,
        expanded_height: style.expanded_height,
        text_color: Rgba::WHITE,
        text_color_sec: Rgba::WHITE,
        palette,
        widget_layout: style.widget_layout,
        plugin_widget_layout: style.plugin_widget_layout,
        plugin_widgets: style.plugin_widgets,
        plugin_frames: style.plugin_frames,
        plugin_host: style.plugin_host,
    });
    widget_animating
}

fn draw_compact_layer(
    painter: Painter<'_>,
    params: &DrawIslandParams<'_>,
    palette: &[Rgba],
    alpha: f32,
    visualizer_height_scale: f32,
) {
    let layout = &params.layout;
    let lyrics = &params.lyrics;
    let style = &params.style;
    let has_mini_content = params.mini_content.is_some();
    let center_occupied = crate::ui::widget::compact::has_center_widget(
        style.compact_widget_layout,
        has_mini_content,
    );
    let visible_mini_content = if center_occupied {
        None
    } else {
        params.mini_content
    };
    let (left_extension, right_extension) = crate::ui::widget::compact::side_extensions(
        style.compact_widget_layout,
        center_occupied || has_mini_content,
        has_mini_content,
    );
    let left_extension = left_extension * layout.compact_scale;
    let right_extension = right_extension * layout.compact_scale;
    draw_mini_content(MiniContentParams {
        painter,
        content: visible_mini_content,
        mini_alpha: alpha,
        current_w: (layout.current_w - left_extension - right_extension).max(0.0),
        global_scale: layout.compact_scale,
        media: params.media.media,
        offset_x: layout.island_x + left_extension,
        stable_offset_y: layout.stable_island_y,
        base_h: layout.base_h,
        palette,
        viz_h_scale: visualizer_height_scale,
        current_lyric: lyrics.current_lyric,
        current_secondary_lyric: lyrics.current_secondary_lyric,
        old_lyric: lyrics.old_lyric,
        old_secondary_lyric: lyrics.old_secondary_lyric,
        lyric_highlight: lyrics.lyric_highlight,
        expansion_progress: layout.expansion_progress,
        font_size: style.font_size,
        lyric_scroll_offset: lyrics.lyric_scroll_offset,
        lyric_side_gap: lyrics.lyric_side_gap,
        lyric_transition: lyrics.lyric_transition,
        lyric_transition_animation: lyrics.lyric_transition_animation,
        text_color: Rgba::WHITE,
        robot_beside: style
            .compact_widget_layout
            .iter()
            .any(|slot| slot.widget == Some(winisland_core::config::CompactWidgetKind::Mascot)),
    });
    crate::ui::widget::compact::draw(
        painter,
        style.compact_widget_layout,
        Rect::from_xywh(
            layout.island_x,
            layout.stable_island_y,
            layout.current_w,
            layout.base_h,
        ),
        layout.compact_scale,
        (alpha * layout.compact_widget_opacity * f32::from(u8::MAX)) as u8,
        has_mini_content,
    );
}

fn draw_pager(
    painter: Painter<'_>,
    drawing_context: &mut DrawingContext<'_>,
    params: &DrawIslandParams<'_>,
    rect: Rect,
    alpha: f32,
) {
    let pages = params.style.pages;
    // As a notch the panel navigates with its own dock, so the old pager (a bar and a close
    // button under the panel) is never shown.
    if alpha <= MIN_VISIBLE_OPACITY || params.style.attached {
        return;
    }
    // The dashboard has its own dock; a second pager under it is just noise.
    let current = params.layout.view_offset.round().max(0.0) as usize;
    let alpha = if pages.get(current) == Some(&ExpandedPage::Home) {
        let distance = (params.layout.view_offset - current as f32).abs();
        alpha * (distance * 2.0).min(1.0)
    } else {
        alpha
    };
    if alpha <= MIN_VISIBLE_OPACITY {
        return;
    }
    let backdrop = (params.style.island_style == "dynamic")
        .then(|| {
            crate::utils::backdrop::get_blurred_cover_background(
                drawing_context,
                params.media.media,
            )
        })
        .flatten();
    pager::draw(PagerParams {
        painter,
        island: rect,
        count: pages.len(),
        pages,
        position: params.layout.view_offset,
        scale: params.layout.expanded_scale,
        above: params.layout.pager_above,
        alpha,
        bar_hover: params.layout.pager_bar_hover,
        close_hover: params.layout.pager_close_hover,
        host_blur: params.style.host_backdrop,
        backdrop: backdrop.as_ref(),
    });
}

/// A short light that runs along the collapsed notch's edge from the left shoulder, down,
/// across the bottom and up to the right shoulder, then starts again: a bright core with a
/// soft glow, brightest at its head and fading along its tail.
fn draw_edge_line(
    painter: Painter<'_>,
    params: &DrawIslandParams<'_>,
    rect: Rect,
    island_path: &Path,
    color: Rgba,
) {
    const PASS_SECONDS: f32 = 4.5;
    // Many short flat-ended pieces join into one smooth line that tapers to its tail;
    // round caps on a few long pieces showed as a row of blobs.
    const PIECES: usize = 20;
    let opacity = 1.0 - params.layout.expansion_progress.clamp(0.0, 1.0);
    if opacity <= 0.01 {
        return;
    }
    let layout = &params.layout;
    let scale = layout.compact_scale.max(0.5);
    let outline = if params.style.attached {
        let shoulder = notch_shoulder(layout.current_r, scale);
        let bar = params.style.bar_height;
        if bar > 0.0 {
            let rect = Rect::from_ltrb(
                rect.left,
                rect.top,
                rect.right,
                rect.bottom.max(bar + 12.0 * scale),
            );
            Path::notch_outline(rect, bar, layout.current_r, shoulder)
        } else {
            Path::notch_outline(rect, rect.top.min(0.0), layout.current_r, shoulder)
        }
    } else {
        island_path.clone()
    };
    let total = outline.length();
    if total <= 1.0 {
        return;
    }
    // The light enters on the left and fully leaves on the right before the next pass.
    let comet = 120.0 * scale;
    let travel = total + comet;
    let head = (crate::ui::top_bar::animation_seconds() / PASS_SECONDS * travel).rem_euclid(travel);
    for piece in 0..PIECES {
        let from = head - comet * (piece + 1) as f32 / PIECES as f32;
        let to = head - comet * piece as f32 / PIECES as f32;
        let segment = outline.segment(from.max(0.0), to.min(total));
        if to <= 0.0 || from >= total || segment.is_empty() {
            continue;
        }
        // Brightest at the head, easing out to nothing at the tail.
        let t = 1.0 - piece as f32 / PIECES as f32;
        let strength = opacity * t * t;
        // Only the leading tip is rounded; everywhere else the pieces butt together.
        let cap = if piece == 0 {
            winisland_render::StrokeCap::Round
        } else {
            winisland_render::StrokeCap::Butt
        };
        // A narrow soft halo hugging the edge, then the bright core line.
        for (width, alpha) in [(6.0, 45.0), (3.0, 110.0), (1.4, 240.0)] {
            painter.stroke_path(
                &segment,
                width * scale,
                color.with_alpha((alpha * strength) as u8),
                cap,
                winisland_render::StrokeJoin::Round,
            );
        }
    }
}

fn draw_island_border(painter: Painter<'_>, params: &DrawIslandParams<'_>) {
    if params.style.attached {
        // A notch reads as hardware: pure black, no outline.
        return;
    }
    let layout = &params.layout;
    let solid_style = params.style.island_style == SOLID_STYLE;
    let alpha = if solid_style {
        SOLID_BORDER_ALPHA
    } else {
        EFFECT_BORDER_ALPHA
    };
    // No outline on the open panel, whatever the style.
    let opacity = 1.0 - layout.expansion_progress.clamp(0.0, 1.0);
    let border_path = Path::continuous_rounded_rect(
        Rect::from_xywh(
            layout.island_x + BORDER_INSET,
            layout.island_y + BORDER_INSET,
            (layout.current_w - BORDER_WIDTH).max(0.0),
            (layout.current_h - BORDER_WIDTH).max(0.0),
        ),
        (layout.current_r - BORDER_INSET).max(0.0),
    );
    painter.stroke_path(
        &border_path,
        BORDER_WIDTH,
        Rgba::WHITE.with_alpha((alpha as f32 * opacity) as u8),
        winisland_render::StrokeCap::Butt,
        winisland_render::StrokeJoin::Miter,
    );
}

thread_local! {
    /// Edge light mode: 0 off, 1 theme accent, 2 album art colour, 3 a fixed colour.
    static EDGE_LIGHT: std::cell::Cell<(u8, Rgba)> = const { std::cell::Cell::new((0, Rgba::WHITE)) };
}

/// Applies the configured edge light: `off`, `theme`, `music` or a fixed colour.
pub fn set_edge_light(mode: &str) {
    let setting = match mode {
        "theme" => (1, Rgba::WHITE),
        "music" => (2, Rgba::WHITE),
        "white" => (3, Rgba::WHITE),
        "amber" => (3, Rgba::from_rgb(255, 176, 32)),
        "blue" => (3, Rgba::from_rgb(90, 170, 255)),
        "green" => (3, Rgba::from_rgb(80, 210, 130)),
        "pink" => (3, Rgba::from_rgb(255, 120, 180)),
        _ => (0, Rgba::WHITE),
    };
    EDGE_LIGHT.with(|cell| cell.set(setting));
}

/// Applies the Notch colour setting.
pub fn set_notch_color(name: &str) {
    background::set_notch_color(name);
}

fn edge_light_color(params: &DrawIslandParams<'_>) -> Option<Rgba> {
    let (mode, fixed) = EDGE_LIGHT.with(std::cell::Cell::get);
    if mode == 0 || params.layout.hide_progress > 0.5 {
        return None;
    }
    if mode == 3 {
        return Some(fixed);
    }
    if mode == 2 && params.media.music_active {
        if let Some(color) = get_media_palette(params.media.media).first() {
            return Some(*color);
        }
    }
    Some(crate::ui::theme::colors().accent_primary)
}

/// The island outline: a notch hanging from the top edge, or a floating pill.
fn island_shape(params: &DrawIslandParams<'_>, rect: Rect) -> Path {
    let layout = &params.layout;
    if !params.style.attached {
        return Path::continuous_rounded_rect(rect, layout.current_r);
    }
    let scale = layout.compact_scale.max(0.5);
    // Small, tight shoulders where the notch meets the screen edge.
    let shoulder = notch_shoulder(layout.current_r, scale);
    let bar = params.style.bar_height;
    if bar > 0.0 {
        // Under the top bar the notch always dips visibly below it, so it never collapses
        // into a thin strip when the compact content is short.
        let rect = Rect::from_ltrb(
            rect.left,
            rect.top,
            rect.right,
            rect.bottom.max(bar + 12.0 * scale),
        );
        return Path::notch(rect, bar, layout.current_r, shoulder);
    }
    Path::notch(rect, rect.top.min(0.0), layout.current_r, shoulder)
}

thread_local! {
    /// Microphone and camera in-use flags for the privacy dots.
    static CAPTURE_IN_USE: std::cell::Cell<(bool, bool)> = const { std::cell::Cell::new((false, false)) };
}

/// Shows an orange dot while the microphone is in use and a green one for the camera.
pub fn set_capture_in_use(microphone: bool, camera: bool) {
    CAPTURE_IN_USE.with(|cell| cell.set((microphone, camera)));
}

/// Privacy dots at the right end of the notch, like the iPhone recording indicator.
fn draw_privacy_dots(painter: Painter<'_>, params: &DrawIslandParams<'_>, rect: Rect) {
    let (microphone, camera) = CAPTURE_IN_USE.with(std::cell::Cell::get);
    if !microphone && !camera {
        return;
    }
    let layout = &params.layout;
    let scale = layout.compact_scale;
    let alpha = (1.0 - layout.hide_progress).clamp(0.0, 1.0);
    let radius = 3.0 * scale;
    let center_y = layout.stable_island_y + layout.base_h.min(rect.height()) / 2.0;
    let mut x = rect.right - 7.0 * scale;
    for (on, color) in [
        (camera, Rgba::from_argb(255, 52, 199, 89)),
        (microphone, Rgba::from_argb(255, 255, 159, 10)),
    ] {
        if on {
            painter.fill_circle(
                Point::new(x, center_y),
                radius,
                color.with_alpha((255.0 * alpha) as u8),
            );
            x -= radius * 2.0 + 3.0 * scale;
        }
    }
}

thread_local! {
    /// Target and current opacity for idle dimming.
    static IDLE_DIM: std::cell::Cell<(f32, f32)> = const { std::cell::Cell::new((1.0, 1.0)) };
}

/// Dims the notch to 40% after a long time without input so it does not burn into OLED
/// panels; it brightens again on the next input.
pub fn set_idle_dimmed(dimmed: bool) {
    IDLE_DIM.with(|cell| {
        let (_, current) = cell.get();
        cell.set((if dimmed { 0.4 } else { 1.0 }, current));
    });
}

/// Eases the idle opacity towards its target; `dt` is in 60 Hz frames.
fn idle_dim(dt: f32) -> f32 {
    IDLE_DIM.with(|cell| {
        let (target, current) = cell.get();
        let step = 0.02 * dt.max(0.0);
        let next = if target < current {
            (current - step).max(target)
        } else {
            (current + step * 4.0).min(target)
        };
        cell.set((target, next));
        next
    })
}

/// Microphone and camera in-use flags, as last published.
pub fn capture_in_use() -> (bool, bool) {
    CAPTURE_IN_USE.with(std::cell::Cell::get)
}

/// While music plays, a soft light in the album's accent colour spills out from under the
/// notch, as on the macOS notch players. Drawn behind the island so only the spill shows.
fn draw_album_glow(painter: Painter<'_>, params: &DrawIslandParams<'_>, rect: Rect) {
    let media = params.media.media;
    if !params.media.music_active || !media.is_playing || params.layout.hide_progress > 0.5 {
        return;
    }
    let palette = get_media_palette(media);
    let Some(accent) = crate::ui::expanded::music_view::readable_accent(&palette) else {
        return;
    };
    let scale = params.layout.compact_scale.max(0.5);
    // Fades out as the panel opens; the open panel has its own shadow.
    let alpha = (70.0 * (1.0 - params.layout.expansion_progress).clamp(0.0, 1.0)) as u8;
    if alpha == 0 {
        return;
    }
    let glow = Rect::from_xywh(
        rect.left + rect.width() * 0.12,
        rect.bottom - 12.0 * scale,
        rect.width() * 0.76,
        22.0 * scale,
    );
    painter.fill_path_blurred(
        &Path::continuous_rounded_rect(glow, 11.0 * scale),
        accent.with_alpha(alpha),
        BlurSpec {
            sigma: (14.0 * scale, 10.0 * scale),
            tile: None,
        },
    );
}

/// Radius of the concave fillet where the notch meets the top screen edge.
pub fn notch_shoulder(radius: f32, scale: f32) -> f32 {
    (radius * 0.4).clamp(5.0 * scale, 9.0 * scale)
}
