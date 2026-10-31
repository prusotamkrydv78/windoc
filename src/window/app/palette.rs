//! Runs command palette actions and keeps the island's keyboard focus in step with it.

use winisland_core::palette::{Command, KnownFolder};

use super::App;
use crate::platform::window;

impl App {
    /// Gives the island keyboard focus while the palette is open and gives it back after.
    pub(super) fn sync_palette_focus(&mut self) {
        let open = crate::ui::dashboard::wants_keyboard();
        if open == self.palette_focused {
            return;
        }
        self.palette_focused = open;
        if let Some(window_ref) = self.window {
            window().set_keyboard_focus(window_ref.id(), open);
            if !open {
                Self::enforce_overlay_window(&window_ref);
            }
        }
    }

    pub(super) fn run_palette_command(&mut self, command: Command) {
        use crate::ui::dashboard::{self, Destination};
        match command {
            Command::StartTimer(duration) => dashboard::start_countdown(duration),
            Command::StartPomodoro => dashboard::start_pomodoro(),
            Command::StartStopwatch => dashboard::start_stopwatch(),
            Command::QuickNote(text) => dashboard::add_quick_note(&text),
            Command::OpenFolder(folder) => {
                let path = match folder {
                    KnownFolder::Downloads => dirs::download_dir(),
                    KnownFolder::Documents => dirs::document_dir(),
                    KnownFolder::Desktop => dirs::desktop_dir(),
                    KnownFolder::Pictures => dirs::picture_dir(),
                    KnownFolder::Music => dirs::audio_dir(),
                    KnownFolder::Videos => dirs::video_dir(),
                };
                if let Some(path) = path
                    && let Err(error) = crate::platform::shell().open_url(&path.to_string_lossy())
                {
                    log::warn!("Cannot open folder: {error}");
                }
            }
            Command::SearchClipboard(_) => dashboard::show(Destination::Clipboard),
            Command::OpenModule(module) => match module {
                "settings" => self.open_settings(),
                "home" => dashboard::show(Destination::Home),
                "media" => dashboard::show(Destination::Media),
                "clipboard" => dashboard::show(Destination::Clipboard),
                "shelf" => dashboard::show(Destination::Shelf),
                "focus" => dashboard::show(Destination::Focus),
                "notes" => dashboard::show(Destination::Notes),
                "clock" => dashboard::show(Destination::Clock),
                _ => dashboard::show(Destination::Home),
            },
            Command::Convert { .. } => {}
        }
        if let Some(window_ref) = &self.window {
            window_ref.request_redraw();
        }
    }
}
