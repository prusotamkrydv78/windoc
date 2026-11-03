//! Line icons for modules and navigation.
//!
//! Each icon is an original path on a 24×24 grid, stroked with round caps and joins so it
//! stays crisp at any size and takes any theme colour.

use std::cell::RefCell;
use std::collections::HashMap;

use winisland_render::{Painter, Path, Rect, Rgba, StrokeCap, StrokeJoin, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Home,
    Music,
    Grid,
    Calendar,
    Clipboard,
    Shelf,
    Focus,
    Notes,
    Clock,
    Weather,
    More,
    Link,
    Chart,
    Phone,
    /// A phone beside a laptop, for Notchy Link.
    Devices,
    Agenda,
    Settings,
    Search,
    Check,
    Plus,
    Battery,
    Bolt,
    Gauge,
    Convert,
    Ruler,
    Message,
    Sparkle,
    /// The island's robot, drawn by the mascot widget rather than from a path.
    Pet,
    /// The robot holding up a picture, for saved screenshots.
    PetPhoto,
    /// The robot worn out, for low battery.
    PetTired,
}

impl Icon {
    const fn path(self) -> &'static str {
        match self {
            Self::Pet | Self::PetPhoto | Self::PetTired => "",
            Self::Home => "M4 11 L12 4 L20 11 M6 9.5 V20 H18 V9.5 M10 20 V14 H14 V20",
            Self::Music => {
                "M9 17 V6 L19 4 V15 M9 17 A2.5 2.5 0 1 1 4 17 A2.5 2.5 0 1 1 9 17 \
                 M19 15 A2.5 2.5 0 1 1 14 15 A2.5 2.5 0 1 1 19 15"
            }
            Self::Grid => {
                "M4 4 H10 V10 H4 Z M14 4 H20 V10 H14 Z M4 14 H10 V20 H4 Z M14 14 H20 V20 H14 Z"
            }
            Self::Calendar => "M4 6 H20 V20 H4 Z M4 10 H20 M8 3 V7 M16 3 V7 M8 14 H10 M14 14 H16",
            Self::Clipboard => "M8 5 H6 V21 H18 V5 H16 M9 3 H15 V7 H9 Z M9 12 H15 M9 16 H13",
            Self::Shelf => "M3 13 L6 5 H18 L21 13 V19 H3 Z M3 13 H8 L9.5 15.5 H14.5 L16 13 H21",
            Self::Focus => "M12 5 A8 8 0 1 1 11.99 5 M12 9 V13 L14.5 14.5 M10 2 H14",
            Self::Notes => "M6 3 H15 L19 7 V21 H6 Z M15 3 V7 H19 M9 12 H16 M9 16 H14",
            Self::Clock => "M12 3 A9 9 0 1 1 11.99 3 M12 7 V12 L15.5 14",
            Self::Weather => "M7 18 H17 A4 4 0 0 0 17 10 A6 6 0 0 0 6 11.5 A3.3 3.3 0 0 0 7 18 Z",
            Self::More => "M5 12 H5.01 M12 12 H12.01 M19 12 H19.01",
            Self::Link => {
                "M10 14 L14 10 M9 7 L11 5 A4 4 0 0 1 17 11 L15 13 \
                 M15 17 L13 19 A4 4 0 0 1 7 13 L9 11"
            }
            Self::Chart => "M4 4 V20 H20 M8 16 V11 M12 16 V7 M16 16 V13",
            Self::Phone => "M8 2.5 H16 V21.5 H8 Z M11 18.5 H13",
            Self::Devices => "M3 6 H15 V15 H3 Z M1 18 H13 M17 9 H22 V20 H17 Z M19 17.5 H20",
            Self::Agenda => "M9 6 H20 M9 12 H20 M9 18 H20 M4.5 6 H4.51 M4.5 12 H4.51 M4.5 18 H4.51",
            Self::Settings => {
                "M12 9 A3 3 0 1 1 11.99 9 M12 2.5 V5 M12 19 V21.5 M2.5 12 H5 M19 12 H21.5 \
                 M5.3 5.3 L7 7 M17 17 L18.7 18.7 M5.3 18.7 L7 17 M17 7 L18.7 5.3"
            }
            Self::Search => "M11 4 A7 7 0 1 1 10.99 4 M16 16 L20 20",
            Self::Check => "M5 12.5 L10 17 L19 7",
            Self::Plus => "M12 5 V19 M5 12 H19",
            Self::Battery => "M3 8 H18 V16 H3 Z M18 10.5 H20.5 V13.5 H18",
            Self::Bolt => "M13 3 L6 13 H11.5 L10.5 21 L18 10.5 H12.5 Z",
            Self::Gauge => "M4 17 A8 8 0 1 1 20 17 M12 16 L15.5 10.5 M12 16 H12.01",
            Self::Convert => "M4 8 H18 M14 4 L18 8 L14 12 M20 16 H6 M10 12 L6 16 L10 20",
            Self::Ruler => "M3 15 L15 3 L21 9 L9 21 Z M7 11 L9 13 M10 8 L12 10 M13 5 L15 7",
            Self::Message => "M4 5 H20 V16 H10 L6 20 V16 H4 Z M8 9 H16 M8 12.5 H13",
            Self::Sparkle => {
                "M12 3 L13.8 10.2 L21 12 L13.8 13.8 L12 21 L10.2 13.8 L3 12 L10.2 10.2 Z \
                 M19 3 V6 M17.5 4.5 H20.5"
            }
        }
    }

    /// Whether the icon is made of dots that need a heavier stroke to read.
    const fn dotted(self) -> bool {
        matches!(self, Self::More | Self::Agenda)
    }
}

thread_local! {
    static PATHS: RefCell<HashMap<Icon, Option<Path>>> = RefCell::new(HashMap::new());
}

/// Draws `icon` centred in `rect`, scaled to fit, with a stroke of about 1.8 px at 24 px.
pub fn draw_icon(painter: Painter<'_>, icon: Icon, rect: Rect, color: Rgba) {
    let size = rect.width().min(rect.height());
    if size <= 0.0 {
        return;
    }
    PATHS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let Some(path) = cache
            .entry(icon)
            .or_insert_with(|| Path::from_svg(icon.path()))
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
        let width = if icon.dotted() { 3.2 } else { 1.8 };
        painter.stroke_path(path, width, color, StrokeCap::Round, StrokeJoin::Round);
        painter.restore_to(save);
    });
}
