//! Decoded clipboard images for the Clipboard tab.
//!
//! A background thread reads a blob (`core::clipboard_history::read_image_blob`), decodes the
//! PNG once and downscales it to a row thumbnail and a preview; the UI thread turns those into
//! Skia images in [`pump`]. A small least-recently-used cache keeps the last few.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};

use winisland_render::Image;

/// Twice the drawn tile size, for sharp thumbnails on high-DPI screens.
const THUMB_SIDE: u32 = 72;
const PREVIEW_SIDE: u32 = 640;
const CACHE_LIMIT: usize = 24;

/// Pixels decoded on the worker, not yet uploaded.
struct Decoded {
    blob_id: String,
    thumb: Option<(u32, u32, Vec<u8>)>,
    preview: Option<(u32, u32, Vec<u8>)>,
    file_bytes: u64,
}

#[derive(Clone)]
pub struct Images {
    pub thumb: Option<Image>,
    pub preview: Option<Image>,
    /// Size of the stored PNG.
    pub file_bytes: u64,
}

enum Slot {
    Pending,
    Ready(Images),
}

struct Entry {
    slot: Slot,
    used: u64,
}

struct Worker {
    requests: Sender<String>,
    results: Receiver<Decoded>,
}

thread_local! {
    static CACHE: RefCell<HashMap<String, Entry>> = RefCell::new(HashMap::new());
    static WORKER: RefCell<Option<Worker>> = const { RefCell::new(None) };
    static CLOCK: Cell<u64> = const { Cell::new(0) };
}

fn downscale(image: &image::DynamicImage, side: u32) -> (u32, u32, Vec<u8>) {
    let scaled = if image.width() > side || image.height() > side {
        image.thumbnail(side, side).into_rgba8()
    } else {
        image.to_rgba8()
    };
    let (width, height) = scaled.dimensions();
    (width, height, scaled.into_raw())
}

fn decode(blob_id: String) -> Decoded {
    let png = crate::core::clipboard_history::read_image_blob(&blob_id);
    let file_bytes = png.as_ref().map_or(0, |png| png.len() as u64);
    let image = png.and_then(|png| {
        image::load_from_memory_with_format(&png, image::ImageFormat::Png).ok()
    });
    Decoded {
        blob_id,
        thumb: image.as_ref().map(|image| downscale(image, THUMB_SIDE)),
        preview: image.as_ref().map(|image| downscale(image, PREVIEW_SIDE)),
        file_bytes,
    }
}

fn spawn_worker() -> Option<Worker> {
    let (requests, inbox) = channel::<String>();
    let (outbox, results) = channel::<Decoded>();
    std::thread::Builder::new()
        .name("clipboard-thumbnails".into())
        .spawn(move || {
            while let Ok(blob_id) = inbox.recv() {
                if outbox.send(decode(blob_id)).is_err() {
                    break;
                }
            }
        })
        .ok()?;
    Some(Worker { requests, results })
}

fn tick() -> u64 {
    CLOCK.with(|clock| {
        clock.set(clock.get() + 1);
        clock.get()
    })
}

/// The decoded images for `blob_id`, queueing a decode the first time it is asked for.
pub fn get(blob_id: &str) -> Option<Images> {
    let used = tick();
    let found = CACHE.with(|cache| {
        cache.borrow_mut().get_mut(blob_id).map(|entry| {
            entry.used = used;
            match &entry.slot {
                Slot::Ready(images) => Some(images.clone()),
                Slot::Pending => None,
            }
        })
    });
    if let Some(found) = found {
        return found;
    }
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        // Drop the least recently used finished entry once full.
        if cache.len() >= CACHE_LIMIT
            && let Some(oldest) = cache
                .iter()
                .filter(|(_, entry)| matches!(entry.slot, Slot::Ready(_)))
                .min_by_key(|(_, entry)| entry.used)
                .map(|(id, _)| id.clone())
        {
            cache.remove(&oldest);
        }
        cache.insert(
            blob_id.to_string(),
            Entry {
                slot: Slot::Pending,
                used,
            },
        );
    });
    WORKER.with(|worker| {
        let mut worker = worker.borrow_mut();
        if worker.is_none() {
            *worker = spawn_worker();
        }
        if let Some(worker) = worker.as_ref() {
            let _ = worker.requests.send(blob_id.to_string());
        }
    });
    None
}

fn upload(pixels: Option<(u32, u32, Vec<u8>)>) -> Option<Image> {
    let (width, height, rgba) = pixels?;
    Image::from_rgba8(
        i32::try_from(width).ok()?,
        i32::try_from(height).ok()?,
        &rgba,
    )
}

/// Moves finished decodes into the cache. Call every frame.
pub fn pump() {
    let finished: Vec<Decoded> = WORKER.with(|worker| {
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
        for decoded in finished {
            // Evicted while decoding: keep it anyway, it was just asked for.
            let used = cache.get(&decoded.blob_id).map_or(0, |entry| entry.used);
            cache.insert(
                decoded.blob_id,
                Entry {
                    slot: Slot::Ready(Images {
                        thumb: upload(decoded.thumb),
                        preview: upload(decoded.preview),
                        file_bytes: decoded.file_bytes,
                    }),
                    used,
                },
            );
        }
    });
}
