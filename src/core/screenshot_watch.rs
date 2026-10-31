//! Notices new screenshots. Windows saves Win+PrtScn captures, and Snipping Tool saves its
//! captures by default, to the Screenshots folder under Pictures, so a new image there is
//! a new screenshot. The folder is checked about once a second.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

const CHECK_INTERVAL: Duration = Duration::from_secs(1);
const IMAGE_EXTENSIONS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

#[derive(Default)]
struct Watch {
    next_check: Option<Instant>,
    /// The newest capture seen so far; anything newer is reported once.
    newest: Option<SystemTime>,
}

thread_local! {
    static WATCH: RefCell<Watch> = RefCell::new(Watch::default());
}

fn folders() -> Vec<PathBuf> {
    let mut folders = Vec::new();
    if let Some(pictures) = dirs::picture_dir() {
        folders.push(pictures.join("Screenshots"));
    }
    if let Some(home) = dirs::home_dir() {
        let onedrive = home.join("OneDrive").join("Pictures").join("Screenshots");
        if !folders.contains(&onedrive) {
            folders.push(onedrive);
        }
    }
    folders
}

fn newest_image(folder: &Path) -> Option<(SystemTime, PathBuf)> {
    std::fs::read_dir(folder)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    IMAGE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
                })
        })
        .filter_map(|entry| {
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, entry.path()))
        })
        .max_by_key(|(modified, _)| *modified)
}

/// Returns a screenshot saved since the last call, if any. The first call only records
/// what is already there. Call from the frame loop.
pub fn poll(now: Instant) -> Option<PathBuf> {
    WATCH.with(|watch| {
        let mut watch = watch.borrow_mut();
        if watch.next_check.is_some_and(|next| now < next) {
            return None;
        }
        let first = watch.next_check.is_none();
        watch.next_check = Some(now + CHECK_INTERVAL);
        let (modified, path) = folders()
            .iter()
            .filter_map(|folder| newest_image(folder))
            .max_by_key(|(modified, _)| *modified)?;
        let is_new = watch.newest.is_none_or(|newest| modified > newest);
        if is_new {
            watch.newest = Some(modified);
        }
        (is_new && !first).then_some(path)
    })
}
