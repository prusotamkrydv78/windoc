//! Image thumbnails and pixel sizes for the Shelf and File converter tabs.
//!
//! A single background thread decodes requested images to small RGBA thumbnails; the UI
//! thread turns them into textures in [`pump`], called from the tabs' `poll`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use image::ImageReader;
use winisland_render::Image;

const THUMB_SIDE: u32 = 160;
const MAX_FILE_BYTES: u64 = 96 * 1024 * 1024;
const MAX_PIXELS: u64 = 100_000_000;
const CACHE_LIMIT: usize = 300;

struct Probe {
    path: PathBuf,
    dims: Option<(u32, u32)>,
    pixels: Option<(u32, u32, Vec<u8>)>,
}

enum Slot {
    Pending,
    Ready {
        image: Option<Image>,
        dims: Option<(u32, u32)>,
    },
}

struct Worker {
    requests: Sender<PathBuf>,
    results: Receiver<Probe>,
}

thread_local! {
    static CACHE: RefCell<HashMap<PathBuf, Slot>> = RefCell::new(HashMap::new());
    static WORKER: RefCell<Option<Worker>> = const { RefCell::new(None) };
}

fn probe(path: &Path) -> Probe {
    let reader = || ImageReader::open(path).ok()?.with_guessed_format().ok();
    let small_enough = std::fs::metadata(path).is_ok_and(|meta| meta.len() <= MAX_FILE_BYTES);
    let dims = reader().and_then(|reader| reader.into_dimensions().ok());
    let pixels = dims
        .filter(|(width, height)| {
            small_enough && u64::from(*width) * u64::from(*height) <= MAX_PIXELS
        })
        .and_then(|_| reader()?.decode().ok())
        .map(|decoded| {
            let thumb = decoded.thumbnail(THUMB_SIDE, THUMB_SIDE).into_rgba8();
            let (width, height) = thumb.dimensions();
            (width, height, thumb.into_raw())
        });
    Probe {
        path: path.to_path_buf(),
        dims,
        pixels,
    }
}

fn spawn_worker() -> Option<Worker> {
    let (requests, inbox) = channel::<PathBuf>();
    let (outbox, results) = channel::<Probe>();
    std::thread::Builder::new()
        .name("shelf-thumbnails".into())
        .spawn(move || {
            while let Ok(path) = inbox.recv() {
                if outbox.send(probe(&path)).is_err() {
                    break;
                }
            }
        })
        .ok()?;
    Some(Worker { requests, results })
}

/// Queues `path` for decoding unless it is known already.
fn request(path: &Path) {
    if !super::kit::is_image(path) {
        return;
    }
    let known = CACHE.with(|cache| cache.borrow().contains_key(path));
    if known {
        return;
    }
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= CACHE_LIMIT {
            cache.retain(|_, slot| matches!(slot, Slot::Pending));
        }
        cache.insert(path.to_path_buf(), Slot::Pending);
    });
    WORKER.with(|worker| {
        let mut worker = worker.borrow_mut();
        if worker.is_none() {
            *worker = spawn_worker();
        }
        if let Some(worker) = worker.as_ref() {
            let _ = worker.requests.send(path.to_path_buf());
        }
    });
}

/// The thumbnail of an image file, once decoded.
pub fn thumbnail(path: &Path) -> Option<Image> {
    request(path);
    CACHE.with(|cache| match cache.borrow().get(path) {
        Some(Slot::Ready { image, .. }) => image.clone(),
        _ => None,
    })
}

/// Pixel width and height of an image file, once probed.
pub fn dimensions(path: &Path) -> Option<(u32, u32)> {
    request(path);
    CACHE.with(|cache| match cache.borrow().get(path) {
        Some(Slot::Ready { dims, .. }) => *dims,
        _ => None,
    })
}

/// Forgets `path` so it is decoded again (after the file changed).
pub fn forget(path: &Path) {
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if matches!(cache.get(path), Some(Slot::Ready { .. })) {
            cache.remove(path);
        }
    });
}

/// Moves finished thumbnails into the cache. Call every frame.
pub fn pump() {
    let finished: Vec<Probe> = WORKER.with(|worker| {
        worker
            .borrow()
            .as_ref()
            .map(|worker| worker.results.try_iter().collect())
            .unwrap_or_default()
    });
    if finished.is_empty() {
        return;
    }
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        for probe in finished {
            let image = probe.pixels.and_then(|(width, height, rgba)| {
                Image::from_rgba8(
                    i32::try_from(width).ok()?,
                    i32::try_from(height).ok()?,
                    &rgba,
                )
            });
            cache.insert(
                probe.path,
                Slot::Ready {
                    image,
                    dims: probe.dims,
                },
            );
        }
    });
}
