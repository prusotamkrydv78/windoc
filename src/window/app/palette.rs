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

impl App {
    /// Runs a Drop session only while the Drop page is open and the island is expanded.
    pub(super) fn poll_drop(&mut self, now: std::time::Instant) {
        use crate::ui::dashboard::{self, Destination};
        let wanted = self.expanded
            && dashboard::showing(Destination::Drop)
            && !crate::core::safe_mode::active();
        let expired = self
            .drop_session
            .as_ref()
            .is_some_and(|session| session.expired(now));
        if !wanted || expired {
            if self.drop_session.take().is_some() {
                log::info!("Drop session closed");
            }
            dashboard::set_drop(None);
            return;
        }
        if self.drop_session.is_none() {
            self.drop_session = crate::core::drop_server::start(self.shelf.existing_paths());
        }
        let Some(session) = &self.drop_session else {
            dashboard::set_drop(None);
            return;
        };
        let received = session.take_received();
        let minutes_left = session.expires.saturating_duration_since(now).as_secs() / 60 + 1;
        dashboard::set_drop(Some((&session.url, minutes_left)));
        for path in received {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            crate::platform::notify()
                .show_toast(&winisland_core::i18n::tr("drop_received_title"), &name);
            self.shelf.add(path);
        }
    }

    /// Notchy Link: mirrors media and Focus to the phone and acts on what the phone sent.
    pub(super) fn poll_phone_link(&mut self, now: std::time::Instant, window: &crate::platform::WindowRef) {
        use crate::core::phone_link::{self, Event};
        use crate::ui::widget::compact::mascot::{self, Reaction};
        if crate::core::safe_mode::active() {
            return;
        }
        for name in crate::core::friends::poll() {
            mascot::react(Reaction::Notice);
            crate::platform::notify().show_toast(&format!("{name} waved at you"), "From a Notchy nearby");
            phone_link::forward_wave(&name);
            window.request_redraw();
        }
        if self.clipboard.revision() != self.phone_history_revision {
            self.phone_history_revision = self.clipboard.revision();
            use winisland_core::clipboard::ClipboardBody;
            phone_link::set_history(
                self.clipboard
                    .items()
                    .iter()
                    .filter_map(|item| match &item.body {
                        ClipboardBody::Text(text) | ClipboardBody::Link(text) => Some(text.clone()),
                        _ => None,
                    })
                    .take(40)
                    .collect(),
            );
        }
        let media = self.current_media_info();
        let media = (!media.title.is_empty())
            .then(|| (media.title.clone(), media.artist.clone(), media.is_playing));
        let events = phone_link::poll(
            now,
            media
                .as_ref()
                .map(|(title, artist, playing)| (title.as_str(), artist.as_str(), *playing)),
            crate::ui::dashboard::focus_remaining(now),
        );
        if events.is_empty() {
            return;
        }
        for event in events {
            match event {
                Event::Clip(text) => {
                    if let Err(error) = winisland_platform_windows::set_clipboard_text(&text) {
                        log::warn!("Cannot copy from phone: {error}");
                    }
                }
                Event::OpenUrl(url) => {
                    if let Err(error) = crate::platform::shell().open_url(&url) {
                        log::warn!("Cannot open link from phone: {error}");
                    }
                }
                Event::Media(command) => match command.as_str() {
                    "prev" => self.smtc.request_prev(),
                    "next" => self.smtc.request_next(),
                    _ => self.smtc.request_toggle_play(),
                },
                Event::Toast(title, text) => crate::platform::notify().show_toast(&title, &text),
                Event::Arrived => mascot::react(Reaction::Notice),
                Event::Thrown => mascot::react(Reaction::Catch),
                Event::File(path) => {
                    let name = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    crate::platform::notify().show_toast("From your phone", &name);
                    mascot::react(Reaction::Catch);
                    self.shelf.add(path);
                }
                Event::PhoneLow(percent) => {
                    mascot::react(Reaction::Surprised);
                    crate::platform::notify().show_toast(
                        &format!("Phone at {percent}%"),
                        "Time to plug it in",
                    );
                }
                Event::Ring => {
                    mascot::react(Reaction::Surprised);
                    for _ in 0..3 {
                        crate::core::chime::play(crate::core::chime::Sound::DeviceIn);
                    }
                    crate::platform::notify()
                        .show_toast("Here I am!", "Your phone is looking for this PC");
                }
            }
        }
        window.request_redraw();
    }
}
