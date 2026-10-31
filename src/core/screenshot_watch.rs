//! Notices new screenshots and finished downloads. Windows saves Win+PrtScn captures, and
//! Snipping Tool saves its captures by default, to Pictures\Screenshots, so a new image there
//! is a new screenshot; a new finished file in Downloads is a finished download. Each
//! folder is checked about once a second.

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
    static DOWNLOADS: RefCell<Watch> = RefCell::new(Watch::default());
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

fn is_image(path: &Path) -> bool {
    extension(path).is_some_and(|extension| IMAGE_EXTENSIONS.contains(&extension.as_str()))
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
}

fn newest_file(folder: &Path, wanted: fn(&Path) -> bool) -> Option<(SystemTime, PathBuf)> {
    std::fs::read_dir(folder)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()) && wanted(&entry.path()))
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
            .filter_map(|folder| newest_file(folder, is_image))
            .max_by_key(|(modified, _)| *modified)?;
        let is_new = watch.newest.is_none_or(|newest| modified > newest);
        if is_new {
            watch.newest = Some(modified);
        }
        (is_new && !first).then_some(path)
    })
}

/// A finished download: anything new in Downloads that is not a browser's partial file.
fn is_finished_download(path: &Path) -> bool {
    const PARTIAL: [&str; 5] = ["crdownload", "part", "partial", "tmp", "download"];
    !extension(path).is_some_and(|extension| PARTIAL.contains(&extension.as_str()))
        && !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with('.') || name.starts_with("~$"))
}

/// Returns a download finished since the last call, if any; the first call only records
/// what is already there. Call from the frame loop.
pub fn poll_downloads(now: Instant) -> Option<PathBuf> {
    DOWNLOADS.with(|watch| {
        let mut watch = watch.borrow_mut();
        if watch.next_check.is_some_and(|next| now < next) {
            return None;
        }
        let first = watch.next_check.is_none();
        watch.next_check = Some(now + CHECK_INTERVAL);
        let (modified, path) = newest_file(&dirs::download_dir()?, is_finished_download)?;
        let is_new = watch.newest.is_none_or(|newest| modified > newest);
        if is_new {
            watch.newest = Some(modified);
        }
        (is_new && !first).then_some(path)
    })
}
