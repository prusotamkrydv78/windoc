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
            Command::WebSearch { url, .. } => {
                if let Err(error) = crate::platform::shell().open_url(&url) {
                    log::warn!("Cannot open search: {error}");
                }
            }
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
            Command::ConvertImages(format) => {
                self.run_image_job(move |path| crate::core::converter::convert(path, format))
            }
            Command::CompressImages(max_bytes) => {
                self.run_image_job(move |path| crate::core::converter::compress(path, max_bytes))
            }
            Command::Convert { .. } | Command::ConvertCurrency { .. } => {}
        }
        if let Some(window_ref) = &self.window {
            window_ref.request_redraw();
        }
    }
}

impl App {
    /// Runs `job` on each file of the active shelf on a background thread, then adds the
    /// outputs to the shelf and reports the result in a notification.
    fn run_image_job(
        &mut self,
        job: impl Fn(&std::path::Path) -> Result<std::path::PathBuf, String> + Send + 'static,
    ) {
        let paths = self.shelf.existing_paths();
        if paths.is_empty() {
            crate::platform::notify().show_toast(
                &winisland_core::i18n::tr("converter_title"),
                &winisland_core::i18n::tr("converter_empty"),
            );
            return;
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let results: Vec<_> = paths.iter().map(|path| job(path)).collect();
            let _ = sender.send(results);
        });
        self.image_job = Some(receiver);
    }

    /// Collects finished conversions. Called from the frame loop.
    pub(super) fn poll_image_job(&mut self) {
        let Some(receiver) = &self.image_job else {
            return;
        };
        let Ok(results) = receiver.try_recv() else {
            return;
        };
        self.image_job = None;
        let mut done = 0;
        let mut failed = 0;
        for result in results {
            match result {
                Ok(path) => {
                    self.shelf.add(path);
                    done += 1;
                }
                Err(error) => {
                    log::warn!("Image conversion failed: {error}");
                    failed += 1;
                }
            }
        }
        crate::platform::notify().show_toast(
            &winisland_core::i18n::tr("converter_title"),
            &winisland_core::i18n::tr_args(
                "converter_result",
                &[&done.to_string(), &failed.to_string()],
            ),
        );
    }
}
