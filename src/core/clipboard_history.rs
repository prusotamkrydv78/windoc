//! Connects clipboard history to the Windows clipboard monitor and stores it encrypted.
//!
//! History lives in `clipboard.bin`: the JSON snapshot encrypted with DPAPI for the
//! current Windows user, so another account or a copied disk cannot read it. Images are kept
//! beside it as PNG blobs in `clipboard_blobs/<sha256>.bin`, encrypted the same way, named by
//! a hash of their pixels so identical copies share one file. Hashing, encoding and writing
//! run on a worker thread; the item is added once its blob is on disk.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant, SystemTime};

use sha2::{Digest, Sha256};

use winisland_core::clipboard::{
    ClipboardBody, ClipboardHistory, ClipboardItem, ClipboardSettings, ClipboardSnapshot,
    SourceFormat,
};
use winisland_platform_windows::{
    ClipboardChange, ClipboardContent, ClipboardFormat, ClipboardMonitor, dpapi,
};

use super::json_store;

pub(crate) const FILE_NAME: &str = "clipboard.bin";
const MAX_FILE_BYTES: u64 = 96 * 1024 * 1024;
/// Binds the encrypted file to this app; it is not a secret on its own.
const ENTROPY: &[u8] = b"winisland-next/clipboard/v1";
const SAVE_DELAY: Duration = Duration::from_secs(10);
const BLOB_DIR: &str = "clipboard_blobs";
/// An encrypted PNG of the largest image captured (40 MB of pixels) stays well below this.
const MAX_BLOB_BYTES: u64 = 64 * 1024 * 1024;

fn blob_file(blob_id: &str) -> String {
    format!("{BLOB_DIR}/{blob_id}.bin")
}

/// Ids are lowercase hex from [`store_image`]; anything else never reaches the file system.
fn valid_blob_id(blob_id: &str) -> bool {
    !blob_id.is_empty() && blob_id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The PNG bytes of a stored image. Blocking file and DPAPI work: call off the UI thread.
pub fn read_image_blob(blob_id: &str) -> Option<Vec<u8>> {
    if !valid_blob_id(blob_id) {
        return None;
    }
    let bytes = json_store::read_bytes(&blob_file(blob_id), MAX_BLOB_BYTES).ok()??;
    dpapi::unprotect(&bytes, Some(ENTROPY)).ok()
}

/// A captured image waiting for its blob to be written.
struct ImageJob {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    formats: Vec<ClipboardFormat>,
    source_app: Option<String>,
}

/// An image whose blob is on disk, ready to enter history.
struct StoredImage {
    width: u32,
    height: u32,
    blob_id: String,
    formats: Vec<ClipboardFormat>,
    source_app: Option<String>,
}

struct ImageWorker {
    jobs: Sender<ImageJob>,
    done: Receiver<StoredImage>,
}

/// Hashes, encodes and writes one image. Runs on the image worker thread.
fn store_image(job: ImageJob) -> Option<StoredImage> {
    let mut hasher = Sha256::new();
    hasher.update(job.width.to_le_bytes());
    hasher.update(job.height.to_le_bytes());
    hasher.update(&job.rgba);
    let blob_id: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let file = blob_file(&blob_id);
    if json_store::file_size(&file).is_none() {
        let image = image::RgbaImage::from_raw(job.width, job.height, job.rgba)?;
        let mut png = Vec::new();
        if let Err(error) =
            image.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        {
            log::warn!("Cannot encode clipboard image: {error}");
            return None;
        }
        let written = dpapi::protect(&png, Some(ENTROPY))
            .map_err(|error| error.to_string())
            .and_then(|encrypted| json_store::write_bytes(&file, &encrypted));
        if let Err(error) = written {
            log::warn!("Cannot save clipboard image: {error}");
            return None;
        }
    }
    Some(StoredImage {
        width: job.width,
        height: job.height,
        blob_id,
        formats: job.formats,
        source_app: job.source_app,
    })
}

fn spawn_image_worker() -> Option<ImageWorker> {
    let (jobs, inbox) = channel::<ImageJob>();
    let (outbox, done) = channel::<StoredImage>();
    std::thread::Builder::new()
        .name("clipboard-images".into())
        .spawn(move || {
            while let Ok(job) = inbox.recv() {
                if let Some(stored) = store_image(job)
                    && outbox.send(stored).is_err()
                {
                    break;
                }
            }
        })
        .ok()?;
    Some(ImageWorker { jobs, done })
}

fn blob_id_of(item: &ClipboardItem) -> Option<&str> {
    match &item.body {
        ClipboardBody::Image { blob_id, .. } => Some(blob_id),
        _ => None,
    }
}

/// Deletes blob files that no item in `history` refers to (left by evictions while the
/// app was closed, or by a crash before a save).
fn sweep_blobs(history: &ClipboardHistory) {
    let directory = crate::platform::shell().data_dir().join(BLOB_DIR);
    let Ok(entries) = std::fs::read_dir(&directory) else {
        return;
    };
    let used: std::collections::HashSet<&str> =
        history.items().iter().filter_map(blob_id_of).collect();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let id = name.strip_suffix(".bin").unwrap_or(&name);
        if !used.contains(id) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Something the dashboard asks of history, by item id. Queued on the UI thread and
/// applied by the next [`ClipboardService::poll`], which also saves the change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardRequest {
    Pin(u64, bool),
    Delete(u64),
    /// Puts the item back on the system clipboard.
    Copy(u64),
}

/// History as last published by the service, for views that need ids, times and pins.
#[derive(Clone, Default)]
pub struct Published {
    pub revision: u64,
    pub enabled: bool,
    /// Newest first.
    pub items: Rc<Vec<ClipboardItem>>,
}

thread_local! {
    static REQUESTS: RefCell<Vec<ClipboardRequest>> = const { RefCell::new(Vec::new()) };
    /// `None` until the service first publishes.
    static PUBLISHED: RefCell<Option<Published>> = const { RefCell::new(None) };
}

/// Queues a request for the service.
pub fn request(request: ClipboardRequest) {
    REQUESTS.with(|queue| queue.borrow_mut().push(request));
}

/// The latest published history, or `None` before the service has run.
pub fn published() -> Option<Published> {
    PUBLISHED.with(|cell| cell.borrow().clone())
}

pub struct ClipboardService {
    history: ClipboardHistory,
    monitor: Option<ClipboardMonitor>,
    save_due: Option<Instant>,
    revision: u64,
    /// Set when an item was added since the last [`Self::take_captured`].
    just_captured: bool,
    /// Started on the first captured image.
    images: Option<ImageWorker>,
}

pub fn settings_for(config: &winisland_core::config::AppConfig) -> ClipboardSettings {
    ClipboardSettings {
        enabled: config.clipboard_history_enabled && !super::safe_mode::active(),
        ..ClipboardSettings::default()
    }
}

fn load_history(settings: ClipboardSettings) -> ClipboardHistory {
    let bytes = match json_store::read_bytes(FILE_NAME, MAX_FILE_BYTES) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return ClipboardHistory::new(settings),
        Err(error) => {
            log::warn!("Ignoring saved clipboard history: {error}");
            return ClipboardHistory::new(settings);
        }
    };
    let snapshot = dpapi::unprotect(&bytes, Some(ENTROPY))
        .map_err(|error| error.to_string())
        .and_then(|plain| {
            serde_json::from_slice::<ClipboardSnapshot>(&plain).map_err(|error| error.to_string())
        });
    match snapshot.and_then(|snapshot| {
        ClipboardHistory::restore(snapshot, settings.clone(), SystemTime::now())
            .map_err(|error| error.to_string())
    }) {
        Ok((history, _evicted)) => history,
        Err(error) => {
            log::warn!("Ignoring saved clipboard history: {error}");
            ClipboardHistory::new(settings)
        }
    }
}

impl ClipboardService {
    pub fn load(settings: ClipboardSettings) -> Self {
        let mut service = Self {
            history: load_history(settings),
            monitor: None,
            save_due: None,
            revision: 0,
            just_captured: false,
            images: None,
        };
        sweep_blobs(&service.history);
        service.sync_monitor();
        service
    }

    fn sync_monitor(&mut self) {
        if !self.history.settings().enabled {
            self.monitor = None;
            return;
        }
        if self.monitor.is_none() {
            match ClipboardMonitor::start() {
                Ok(monitor) => self.monitor = Some(monitor),
                Err(error) => log::warn!("Clipboard history unavailable: {error}"),
            }
        }
    }

    /// Applies changed settings. Turning history off stops monitoring immediately and
    /// keeps existing items until the user clears them.
    pub fn apply_settings(&mut self, settings: ClipboardSettings, now: Instant) {
        if *self.history.settings() == settings {
            return;
        }
        let evicted = self.history.set_settings(settings, SystemTime::now());
        if !evicted.is_empty() {
            self.drop_blobs(&evicted);
            self.changed(now);
        }
        self.sync_monitor();
    }

    /// Call from the frame loop.
    pub fn poll(&mut self, now: Instant) {
        let changes = self
            .monitor
            .as_ref()
            .map(ClipboardMonitor::drain)
            .unwrap_or_default();
        for change in changes {
            self.ingest(change, now);
        }
        let stored: Vec<StoredImage> = self
            .images
            .as_ref()
            .map(|worker| worker.done.try_iter().collect())
            .unwrap_or_default();
        for image in stored {
            let body = ClipboardBody::Image {
                width: image.width,
                height: image.height,
                blob_id: image.blob_id,
            };
            self.add(body, &image.formats, image.source_app, now);
        }
        for request in REQUESTS.with(|queue| std::mem::take(&mut *queue.borrow_mut())) {
            self.apply_request(request, now);
        }
        self.publish();
        if self.save_due.is_some_and(|due| now >= due) {
            self.save();
        }
    }

    fn ingest(&mut self, change: ClipboardChange, now: Instant) {
        let body = match change.content {
            ClipboardContent::Text(text) => ClipboardBody::from_text(&text),
            ClipboardContent::Files(files) => ClipboardBody::Files(files),
            ClipboardContent::Image {
                width,
                height,
                rgba,
            } => {
                // Private or disabled captures never reach the disk.
                let formats = source_formats(&change.formats);
                if !self.history.settings().enabled
                    || winisland_core::clipboard::is_excluded(&formats)
                {
                    return;
                }
                if self.images.is_none() {
                    self.images = spawn_image_worker();
                }
                if let Some(worker) = &self.images {
                    let _ = worker.jobs.send(ImageJob {
                        width,
                        height,
                        rgba,
                        formats: change.formats,
                        source_app: change.source_app,
                    });
                }
                return;
            }
            ClipboardContent::Unsupported | ClipboardContent::Withheld => return,
        };
        self.add(body, &change.formats, change.source_app, now);
    }

    fn add(
        &mut self,
        body: ClipboardBody,
        formats: &[ClipboardFormat],
        source_app: Option<String>,
        now: Instant,
    ) {
        let capture = self.history.capture(
            body,
            &source_formats(formats),
            source_app,
            SystemTime::now(),
        );
        if matches!(
            capture.outcome,
            winisland_core::clipboard::CaptureOutcome::Added(_)
        ) {
            self.just_captured = true;
        }
        self.drop_blobs(&capture.evicted);
        self.changed(now);
    }

    /// Deletes the blobs of `removed` items that no remaining item shares.
    fn drop_blobs(&self, removed: &[ClipboardItem]) {
        for blob_id in removed.iter().filter_map(blob_id_of) {
            let shared = self
                .history
                .items()
                .iter()
                .any(|item| blob_id_of(item) == Some(blob_id));
            if !shared
                && valid_blob_id(blob_id)
                && let Err(error) = json_store::remove(&blob_file(blob_id))
            {
                log::warn!("Cannot delete clipboard image: {error}");
            }
        }
    }

    fn apply_request(&mut self, request: ClipboardRequest, now: Instant) {
        match request {
            ClipboardRequest::Pin(id, pinned) => {
                if self.history.set_pinned(id, pinned) {
                    self.changed(now);
                }
            }
            ClipboardRequest::Delete(id) => {
                if let Some(item) = self.history.delete(id) {
                    self.drop_blobs(std::slice::from_ref(&item));
                    self.changed(now);
                }
            }
            ClipboardRequest::Copy(id) => {
                if let Some(index) = self.history.items().iter().position(|item| item.id == id) {
                    self.copy_again(index);
                }
            }
        }
    }

    /// Republishes history for views when it changed since the last publish.
    fn publish(&self) {
        let enabled = self.enabled();
        PUBLISHED.with(|cell| {
            let mut cell = cell.borrow_mut();
            if cell
                .as_ref()
                .is_some_and(|last| last.revision == self.revision && last.enabled == enabled)
            {
                return;
            }
            *cell = Some(Published {
                revision: self.revision,
                enabled,
                items: Rc::new(self.history.items().to_vec()),
            });
        });
    }

    fn changed(&mut self, now: Instant) {
        self.revision = self.revision.wrapping_add(1);
        self.save_due.get_or_insert(now + SAVE_DELAY);
    }

    fn save(&mut self) {
        self.save_due = None;
        let result = serde_json::to_vec(&self.history.snapshot())
            .map_err(|error| error.to_string())
            .and_then(|plain| {
                dpapi::protect(&plain, Some(ENTROPY)).map_err(|error| error.to_string())
            })
            .and_then(|encrypted| json_store::write_bytes(FILE_NAME, &encrypted));
        if let Err(error) = result {
            log::warn!("Cannot save clipboard history: {error}");
        }
    }

    /// Saves pending changes. Call before the app exits.
    pub fn shutdown(&mut self) {
        self.monitor = None;
        if self.save_due.is_some() {
            self.save();
        }
    }

    /// Whether an item was added since the last call, for the copy confirmation.
    pub fn take_captured(&mut self) -> bool {
        std::mem::take(&mut self.just_captured)
    }

    pub fn enabled(&self) -> bool {
        self.history.settings().enabled
    }

    /// Newest first.
    pub fn items(&self) -> &[ClipboardItem] {
        self.history.items()
    }

    /// Deletes every item, pinned ones included, in memory and on disk.
    pub fn clear(&mut self) {
        let ids: Vec<u64> = self.history.items().iter().map(|item| item.id).collect();
        for id in ids {
            self.history.delete(id);
        }
        self.save_due = None;
        self.revision = self.revision.wrapping_add(1);
        if let Err(error) = json_store::remove(FILE_NAME) {
            log::warn!("Cannot delete clipboard history: {error}");
        }
        sweep_blobs(&self.history);
    }

    /// Increases whenever history changes, so views can refresh cheaply.
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

impl ClipboardService {
    /// Puts item `index` (newest first) back on the clipboard as text.
    pub fn copy_again(&self, index: usize) -> bool {
        use winisland_core::clipboard::ClipboardBody;
        let Some(item) = self.history.items().get(index) else {
            return false;
        };
        let text = match &item.body {
            ClipboardBody::Text(text) | ClipboardBody::Link(text) => text.clone(),
            ClipboardBody::Files(files) => files
                .iter()
                .map(|file| file.display().to_string())
                .collect::<Vec<_>>()
                .join("\r\n"),
            ClipboardBody::Image { blob_id, .. } => {
                // Reading, decrypting and decoding the blob can take a moment.
                let blob_id = blob_id.clone();
                return std::thread::Builder::new()
                    .name("clipboard-copy-image".into())
                    .spawn(move || copy_image(&blob_id))
                    .is_ok();
            }
        };
        match winisland_platform_windows::set_clipboard_text(&text) {
            Ok(()) => true,
            Err(error) => {
                log::warn!("Cannot copy clipboard item: {error}");
                false
            }
        }
    }
}

fn source_formats(formats: &[ClipboardFormat]) -> Vec<SourceFormat<'_>> {
    formats
        .iter()
        .map(|format| SourceFormat {
            name: format.name,
            value: format.value,
        })
        .collect()
}

/// Puts a stored image back on the clipboard. Blocking: runs on its own thread.
fn copy_image(blob_id: &str) {
    let Some(png) = read_image_blob(blob_id) else {
        log::warn!("Cannot copy clipboard image: its file is missing");
        return;
    };
    let decoded = match image::load_from_memory_with_format(&png, image::ImageFormat::Png) {
        Ok(decoded) => decoded.into_rgba8(),
        Err(error) => {
            log::warn!("Cannot copy clipboard image: {error}");
            return;
        }
    };
    let (width, height) = decoded.dimensions();
    if let Err(error) =
        winisland_platform_windows::set_clipboard_image(width, height, decoded.as_raw())
    {
        log::warn!("Cannot copy clipboard image: {error}");
    }
}
