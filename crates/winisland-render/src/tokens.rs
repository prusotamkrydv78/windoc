//! Design tokens shared by every surface.
//!
//! Values follow `docs/UI_UX_DESIGN_GUIDE.md`. Components read these tokens instead of
//! hardcoding spacing, type sizes, colors or timings, so themes and density changes stay
//! in one place. Everything here is plain data in logical pixels; nothing touches Skia.

use std::time::Duration;

use crate::Rgba;

/// Four-pixel spacing grid.
pub mod space {
    /// Icon gaps and internal micro-spacing.
    pub const S1: f32 = 4.0;
    /// Tight control padding.
    pub const S2: f32 = 8.0;
    /// Standard control padding.
    pub const S3: f32 = 12.0;
    /// Card padding.
    pub const S4: f32 = 16.0;
    /// Page section spacing.
    pub const S5: f32 = 20.0;
    /// Major groups.
    pub const S6: f32 = 24.0;
    /// Large separation.
    pub const S8: f32 = 32.0;
}

/// Corner radii. The resting ribbon is fully rounded and uses half its height instead.
pub mod radius {
    pub const CONTROL: f32 = 7.0;
    pub const ROW: f32 = 10.0;
    pub const CARD: f32 = 14.0;
    pub const WORKSPACE: f32 = 20.0;
}

/// A typographic role. Weight uses the CSS/OpenType 100–900 scale.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TextStyle {
    pub size: f32,
    pub weight: u16,
    /// Use tabular (fixed-width) numerals so changing values do not shift layout.
    pub tabular: bool,
}

impl TextStyle {
    const fn new(size: f32, weight: u16) -> Self {
        Self {
            size,
            weight,
            tabular: false,
        }
    }
}

/// Type scale. Nothing user-facing may be smaller than [`MIN_TEXT_SIZE`].
pub mod typography {
    use super::TextStyle;

    pub const MIN_TEXT_SIZE: f32 = 11.0;

    pub const DISPLAY: TextStyle = TextStyle::new(28.0, 600);
    pub const PAGE_TITLE: TextStyle = TextStyle::new(20.0, 600);
    pub const SECTION_TITLE: TextStyle = TextStyle::new(15.0, 600);
    pub const BODY: TextStyle = TextStyle::new(14.0, 400);
    pub const LABEL: TextStyle = TextStyle::new(12.0, 500);
    pub const CAPTION: TextStyle = TextStyle::new(11.0, 400);
    /// Timers and key measurements. Callers may scale between 20 and 32.
    pub const NUMERIC: TextStyle = TextStyle {
        size: 24.0,
        weight: 550,
        tabular: true,
    };
}

/// Which palette is active.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorScheme {
    Dark,
    Light,
    /// Fallback values only. When Windows high contrast is on, the platform layer should
    /// replace these with the system colors.
    HighContrast,
}

/// Semantic colors. Modules use these roles rather than raw colors.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SemanticColors {
    pub surface_base: Rgba,
    pub surface_raised: Rgba,
    pub surface_overlay: Rgba,
    pub surface_interactive: Rgba,
    pub text_primary: Rgba,
    pub text_secondary: Rgba,
    pub text_disabled: Rgba,
    pub border_subtle: Rgba,
    pub border_strong: Rgba,
    /// Default accent. The active Context Set may override it.
    pub accent_primary: Rgba,
    pub accent_soft: Rgba,
    pub state_success: Rgba,
    pub state_warning: Rgba,
    pub state_error: Rgba,
    pub state_info: Rgba,
    pub privacy_sensitive: Rgba,
}

const DARK: SemanticColors = SemanticColors {
    surface_base: Rgba::from_rgb(22, 24, 29),
    surface_raised: Rgba::from_rgb(30, 33, 40),
    surface_overlay: Rgba::from_rgb(38, 42, 50),
    surface_interactive: Rgba::from_rgb(48, 53, 63),
    text_primary: Rgba::from_rgb(236, 239, 244),
    text_secondary: Rgba::from_rgb(166, 173, 186),
    text_disabled: Rgba::from_rgb(110, 116, 128),
    border_subtle: Rgba::from_argb(31, 255, 255, 255),
    border_strong: Rgba::from_argb(64, 255, 255, 255),
    accent_primary: Rgba::from_rgb(84, 182, 166),
    accent_soft: Rgba::from_argb(46, 84, 182, 166),
    state_success: Rgba::from_rgb(88, 196, 130),
    state_warning: Rgba::from_rgb(236, 178, 78),
    state_error: Rgba::from_rgb(234, 96, 90),
    state_info: Rgba::from_rgb(98, 160, 234),
    privacy_sensitive: Rgba::from_rgb(176, 134, 224),
};

const LIGHT: SemanticColors = SemanticColors {
    surface_base: Rgba::from_rgb(246, 247, 249),
    surface_raised: Rgba::from_rgb(255, 255, 255),
    surface_overlay: Rgba::from_rgb(255, 255, 255),
    surface_interactive: Rgba::from_rgb(232, 235, 240),
    text_primary: Rgba::from_rgb(24, 27, 33),
    text_secondary: Rgba::from_rgb(82, 89, 102),
    text_disabled: Rgba::from_rgb(150, 156, 166),
    border_subtle: Rgba::from_argb(20, 0, 0, 0),
    border_strong: Rgba::from_argb(46, 0, 0, 0),
    accent_primary: Rgba::from_rgb(18, 116, 102),
    accent_soft: Rgba::from_argb(36, 18, 116, 102),
    state_success: Rgba::from_rgb(24, 135, 72),
    state_warning: Rgba::from_rgb(176, 110, 0),
    state_error: Rgba::from_rgb(196, 52, 46),
    state_info: Rgba::from_rgb(32, 102, 196),
    privacy_sensitive: Rgba::from_rgb(124, 76, 180),
};

const HIGH_CONTRAST: SemanticColors = SemanticColors {
    surface_base: Rgba::BLACK,
    surface_raised: Rgba::BLACK,
    surface_overlay: Rgba::BLACK,
    surface_interactive: Rgba::BLACK,
    text_primary: Rgba::WHITE,
    text_secondary: Rgba::WHITE,
    text_disabled: Rgba::from_rgb(63, 242, 63),
    border_subtle: Rgba::WHITE,
    border_strong: Rgba::WHITE,
    accent_primary: Rgba::from_rgb(255, 255, 0),
    accent_soft: Rgba::from_rgb(255, 255, 0),
    state_success: Rgba::from_rgb(63, 242, 63),
    state_warning: Rgba::from_rgb(255, 255, 0),
    state_error: Rgba::from_rgb(255, 128, 128),
    state_info: Rgba::from_rgb(26, 235, 255),
    privacy_sensitive: Rgba::from_rgb(255, 255, 0),
};

impl SemanticColors {
    pub const fn for_scheme(scheme: ColorScheme) -> Self {
        match scheme {
            ColorScheme::Dark => DARK,
            ColorScheme::Light => LIGHT,
            ColorScheme::HighContrast => HIGH_CONTRAST,
        }
    }
}

/// WCAG 2.x relative luminance of an opaque color.
pub fn relative_luminance(color: Rgba) -> f32 {
    fn channel(value: u8) -> f32 {
        let c = f32::from(value) / 255.0;
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
}

/// WCAG contrast ratio between two opaque colors, from 1.0 to 21.0.
///
/// Use it to decide when translucent surfaces must fall back to an opaque one.
pub fn contrast_ratio(a: Rgba, b: Rgba) -> f32 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (light, dark) = if la >= lb { (la, lb) } else { (lb, la) };
    (light + 0.05) / (dark + 0.05)
}

/// Minimum contrast for ordinary text (WCAG AA).
pub const MIN_TEXT_CONTRAST: f32 = 4.5;

/// Motion roles. Each maps to the upper end of the range in the design guide.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Motion {
    HoverPress,
    SmallState,
    RibbonExpansion,
    PageNavigation,
    Dialog,
    SuccessConfirmation,
}

/// Duration used when Windows reduced motion is on: a short crossfade, never zero, so
/// state changes stay visible.
pub const REDUCED_MOTION_DURATION: Duration = Duration::from_millis(80);

impl Motion {
    pub const fn duration(self) -> Duration {
        Duration::from_millis(match self {
            Self::HoverPress => 100,
            Self::SmallState => 160,
            Self::RibbonExpansion => 220,
            Self::PageNavigation => 200,
            Self::Dialog => 220,
            Self::SuccessConfirmation => 400,
        })
    }

    /// Duration honoring the reduced-motion preference.
    pub const fn duration_for(self, reduced_motion: bool) -> Duration {
        if reduced_motion {
            REDUCED_MOTION_DURATION
        } else {
            self.duration()
        }
    }
}

/// Expanded-workspace size class, chosen from the current monitor's work area.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SizeClass {
    Compact,
    Standard,
}

pub const STANDARD_MIN_WORK_WIDTH: f32 = 800.0;
pub const STANDARD_MIN_WORK_HEIGHT: f32 = 480.0;
/// The expanded workspace never exceeds this share of the work-area height.
pub const MAX_EXPANDED_HEIGHT_RATIO: f32 = 0.75;
pub const MIN_EDGE_CLEARANCE: f32 = 12.0;

impl SizeClass {
    /// `work_width` and `work_height` are in logical pixels.
    pub fn for_work_area(work_width: f32, work_height: f32) -> Self {
        if work_width >= STANDARD_MIN_WORK_WIDTH && work_height >= STANDARD_MIN_WORK_HEIGHT {
            Self::Standard
        } else {
            Self::Compact
        }
    }

    /// Inclusive width range of the expanded workspace.
    pub const fn width_range(self) -> (f32, f32) {
        match self {
            Self::Compact => (420.0, 520.0),
            Self::Standard => (600.0, 760.0),
        }
    }

    /// Workspace width for a work area, clamped to the class range and to edge clearance.
    pub fn workspace_width(self, work_width: f32) -> f32 {
        let (min, max) = self.width_range();
        max.min(work_width - MIN_EDGE_CLEARANCE * 2.0)
            .max(min.min(work_width))
    }

    /// Maximum workspace height for a work area.
    pub fn max_workspace_height(work_height: f32) -> f32 {
        work_height * MAX_EXPANDED_HEIGHT_RATIO
    }
}

/// Spacing and target-size density. Density changes size, never information hierarchy.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Density {
    Comfortable,
    Compact,
    Touch,
}

impl Density {
    pub const fn list_row_height(self) -> f32 {
        match self {
            Self::Comfortable => 44.0,
            Self::Compact => 36.0,
            Self::Touch => 52.0,
        }
    }

    pub const fn min_target_size(self) -> f32 {
        match self {
            Self::Touch => 44.0,
            Self::Comfortable | Self::Compact => 32.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMES: [ColorScheme; 3] = [
        ColorScheme::Dark,
        ColorScheme::Light,
        ColorScheme::HighContrast,
    ];

    #[test]
    fn text_meets_aa_on_every_surface() {
        for scheme in SCHEMES {
            let colors = SemanticColors::for_scheme(scheme);
            for surface in [
                colors.surface_base,
                colors.surface_raised,
                colors.surface_overlay,
            ] {
                for text in [colors.text_primary, colors.text_secondary] {
                    let ratio = contrast_ratio(text, surface);
                    assert!(ratio >= MIN_TEXT_CONTRAST, "{scheme:?}: contrast {ratio}");
                }
            }
        }
    }

    #[test]
    fn accent_is_distinguishable_from_base() {
        for scheme in SCHEMES {
            let colors = SemanticColors::for_scheme(scheme);
            assert!(contrast_ratio(colors.accent_primary, colors.surface_base) >= 3.0);
        }
    }

    #[test]
    fn contrast_ratio_bounds() {
        assert!((contrast_ratio(Rgba::BLACK, Rgba::WHITE) - 21.0).abs() < 0.01);
        assert!((contrast_ratio(Rgba::WHITE, Rgba::WHITE) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn size_class_reference_points() {
        // 1280 x 720 work area at 150% and 200% scaling.
        assert_eq!(SizeClass::for_work_area(853.0, 480.0), SizeClass::Standard);
        assert_eq!(SizeClass::for_work_area(640.0, 360.0), SizeClass::Compact);
        // 1920 x 1080 at 200%.
        assert_eq!(SizeClass::for_work_area(960.0, 540.0), SizeClass::Standard);
    }

    #[test]
    fn workspace_width_respects_class_and_edges() {
        assert_eq!(SizeClass::Standard.workspace_width(1920.0), 760.0);
        assert_eq!(SizeClass::Standard.workspace_width(800.0), 760.0);
        assert_eq!(SizeClass::Compact.workspace_width(640.0), 520.0);
        // Tiny work areas never produce a width wider than the screen.
        assert!(SizeClass::Compact.workspace_width(400.0) <= 400.0);
    }

    #[test]
    fn reduced_motion_is_short_but_visible() {
        for motion in [Motion::RibbonExpansion, Motion::SuccessConfirmation] {
            assert!(motion.duration_for(true) < motion.duration());
            assert!(!motion.duration_for(true).is_zero());
        }
    }

    #[test]
    fn type_scale_respects_minimum() {
        use typography::*;
        for style in [
            DISPLAY,
            PAGE_TITLE,
            SECTION_TITLE,
            BODY,
            LABEL,
            CAPTION,
            NUMERIC,
        ] {
            assert!(style.size >= MIN_TEXT_SIZE);
        }
    }
}
