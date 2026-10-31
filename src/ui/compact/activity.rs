//! Short live activities shown in the collapsed notch with one shared layout:
//! `[icon] Primary · secondary [progress]`.

use std::time::{Duration, Instant};

use winisland_render::text::{DrawTextCachedParams, FontManager};
use winisland_render::{FontStyle, Painter, Radius, Rect, Rgba, Vec2};

use crate::ui::compact::CompactSize;
use crate::ui::components::icon::{Icon, draw_icon};

const DISPLAY_DURATION: Duration = Duration::from_millis(2600);
const ENTER_DURATION: Duration = Duration::from_millis(220);
const FADE_DURATION: Duration = Duration::from_millis(260);
const TEXT_SIZE: f32 = 13.0;
const ICON_SIZE: f32 = 18.0;
const SIDE_PADDING: f32 = 18.0;
const ICON_GAP: f32 = 10.0;
const PROGRESS_WIDTH: f32 = 54.0;
const PROGRESS_GAP: f32 = 12.0;
const MIN_WIDTH: f32 = 150.0;
pub(super) const MAX_WIDTH: f32 = 340.0;

#[derive(Clone, PartialEq)]
pub struct Activity {
    pub icon: Icon,
    pub accent: Rgba,
    pub primary: String,
    pub secondary: String,
    pub progress: Option<f32>,
}

#[derive(Default)]
pub(super) struct ActivityIndicator {
    current: Option<Activity>,
    shown_at: Option<Instant>,
    updated_at: Option<Instant>,
}

impl ActivityIndicator {
    /// Shows `activity`. An activity with the same icon updates in place instead of
    /// replaying its entrance, so repeated events do not flicker.
    pub(super) fn push(&mut self, activity: Activity) {
        let now = Instant::now();
        let same_kind = self.is_visible()
            && self
                .current
                .as_ref()
                .is_some_and(|current| current.icon == activity.icon);
        if !same_kind {
            self.shown_at = Some(now);
        }
        self.updated_at = Some(now);
        self.current = Some(activity);
    }

    pub(super) fn is_visible(&self) -> bool {
        self.updated_at
            .is_some_and(|at| at.elapsed() < DISPLAY_DURATION + FADE_DURATION)
    }

    pub(super) fn target_size(&self, base_width: f32, base_height: f32, scale: f32) -> CompactSize {
        let Some(activity) = &self.current else {
            return CompactSize {
                width: base_width * scale,
                height: base_height * scale,
            };
        };
        let fonts = FontManager::global();
        let mut width = 2.0 * SIDE_PADDING + ICON_SIZE + ICON_GAP;
        width += fonts.measure_text_cached(&activity.primary, TEXT_SIZE, FontStyle::bold());
        if !activity.secondary.is_empty() {
            width += fonts.measure_text_cached(
                &format!("  ·  {}", activity.secondary),
                TEXT_SIZE,
                FontStyle::normal(),
            );
        }
        if activity.progress.is_some() {
            width += PROGRESS_GAP + PROGRESS_WIDTH;
        }
        CompactSize {
            width: width.clamp(MIN_WIDTH.max(base_width), MAX_WIDTH) * scale,
            height: base_height * scale,
        }
    }

    /// Opacity and vertical offset for the entrance and exit.
    fn presentation(&self) -> (f32, f32) {
        let (Some(shown), Some(updated)) = (self.shown_at, self.updated_at) else {
            return (0.0, 0.0);
        };
        let enter = (shown.elapsed().as_secs_f32() / ENTER_DURATION.as_secs_f32()).min(1.0);
        let since_update = updated.elapsed();
        let exit = if since_update > DISPLAY_DURATION {
            ((since_update - DISPLAY_DURATION).as_secs_f32() / FADE_DURATION.as_secs_f32()).min(1.0)
        } else {
            0.0
        };
        let eased = 1.0 - (1.0 - enter).powi(3);
        (eased * (1.0 - exit), (1.0 - eased) * 4.0 - exit * 4.0)
    }

    pub(super) fn draw(&self, painter: Painter<'_>, rect: Rect, scale: f32, alpha: f32) {
        let Some(activity) = &self.current else {
            return;
        };
        let (opacity, offset_y) = self.presentation();
        let alpha = (alpha * opacity).clamp(0.0, 1.0);
        if alpha <= 0.01 {
            return;
        }
        let to_alpha = |value: f32| (value * alpha * 255.0) as u8;
        painter.save();
        painter.translate(Vec2::new(0.0, offset_y * scale));

        let center_y = rect.center_y();
        let left = rect.left + SIDE_PADDING * scale;
        let mut right = rect.right - SIDE_PADDING * scale;
        let icon_size = ICON_SIZE * scale;
        draw_icon(
            painter,
            activity.icon,
            Rect::from_xywh(left, center_y - icon_size / 2.0, icon_size, icon_size),
            activity.accent.with_alpha(to_alpha(1.0)),
        );

        if let Some(progress) = activity.progress {
            let width = PROGRESS_WIDTH * scale;
            let height = 4.0 * scale;
            let bar = Rect::from_xywh(right - width, center_y - height / 2.0, width, height);
            let radius = Radius::uniform(height / 2.0);
            painter.fill_round_rect(bar, radius, Rgba::WHITE.with_alpha(to_alpha(0.18)));
            let filled = Rect::from_xywh(
                bar.left,
                bar.top,
                (width * progress.clamp(0.0, 1.0)).max(height),
                height,
            );
            painter.fill_round_rect(filled, radius, activity.accent.with_alpha(to_alpha(1.0)));
            right -= width + PROGRESS_GAP * scale;
        }

        let fonts = FontManager::global();
        let size = TEXT_SIZE * scale;
        let baseline = center_y + size * 0.36;
        let mut x = left + icon_size + ICON_GAP * scale;
        let primary = crate::utils::settings_ui::ellipsize_text(
            fonts,
            &activity.primary,
            size,
            FontStyle::bold(),
            (right - x).max(0.0),
        );
        fonts.draw_text_cached(DrawTextCachedParams {
            painter,
            text: &primary,
            x,
            y: baseline,
            size,
            bold: true,
            color: Rgba::WHITE.with_alpha(to_alpha(1.0)),
            blur: None,
        });
        x += fonts.measure_text_cached(&primary, size, FontStyle::bold());
        if !activity.secondary.is_empty() && right - x > 24.0 * scale {
            let secondary = crate::utils::settings_ui::ellipsize_text(
                fonts,
                &format!("  ·  {}", activity.secondary),
                size,
                FontStyle::normal(),
                right - x,
            );
            fonts.draw_text_cached(DrawTextCachedParams {
                painter,
                text: &secondary,
                x,
                y: baseline,
                size,
                bold: false,
                color: Rgba::WHITE.with_alpha(to_alpha(0.55)),
                blur: None,
            });
        }
        painter.restore();
    }
}
