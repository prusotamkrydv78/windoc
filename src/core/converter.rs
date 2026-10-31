//! Local image conversion and compression for files on the shelf.
//!
//! Outputs are written next to each original with a new name; nothing is overwritten and
//! nothing leaves the PC. Decoded images are capped to keep memory bounded.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageFormat, ImageReader};

const MAX_PIXELS: u64 = 80_000_000;
const ICO_MAX_SIDE: u32 = 256;

fn load(path: &Path) -> Result<DynamicImage, String> {
    let reader = ImageReader::open(path)
        .and_then(ImageReader::with_guessed_format)
        .map_err(|error| error.to_string())?;
    let (width, height) = reader
        .into_dimensions()
        .map_err(|error| error.to_string())?;
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err("image is too large".into());
    }
    ImageReader::open(path)
        .and_then(ImageReader::with_guessed_format)
        .map_err(|error| error.to_string())?
        .decode()
        .map_err(|error| error.to_string())
}

/// `photo.png` → `photo-converted.jpg`, adding a number if that name is taken.
fn output_path(original: &Path, extension: &str) -> PathBuf {
    let stem = original
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".to_string());
    let directory = original.parent().unwrap_or_else(|| Path::new("."));
    let mut candidate = directory.join(format!("{stem}-converted.{extension}"));
    let mut counter = 2;
    while candidate.exists() {
        candidate = directory.join(format!("{stem}-converted-{counter}.{extension}"));
        counter += 1;
    }
    candidate
}

/// Converts `original` to `format` (`png`, `jpg`, `ico` or `bmp`). Returns the new file.
pub fn convert(original: &Path, format: &str) -> Result<PathBuf, String> {
    let mut image = load(original)?;
    let target = match format {
        "png" => ImageFormat::Png,
        "jpg" => ImageFormat::Jpeg,
        "bmp" => ImageFormat::Bmp,
        "ico" => {
            if image.width() > ICO_MAX_SIDE || image.height() > ICO_MAX_SIDE {
                image = image.thumbnail(ICO_MAX_SIDE, ICO_MAX_SIDE);
            }
            ImageFormat::Ico
        }
        _ => return Err(format!("unsupported format {format}")),
    };
    if target == ImageFormat::Jpeg {
        // JPEG has no alpha channel.
        image = DynamicImage::ImageRgb8(image.to_rgb8());
    }
    let output = output_path(original, format);
    image
        .save_with_format(&output, target)
        .map_err(|error| error.to_string())?;
    Ok(output)
}

/// Re-encodes `original` as JPEG at the highest quality that fits within `max_bytes`,
/// halving the dimensions if even the lowest quality is too big.
pub fn compress(original: &Path, max_bytes: u64) -> Result<PathBuf, String> {
    let mut image = DynamicImage::ImageRgb8(load(original)?.to_rgb8());
    for _ in 0..4 {
        for quality in [90u8, 80, 70, 60, 50, 40, 30, 20] {
            let mut bytes = Vec::new();
            JpegEncoder::new_with_quality(&mut Cursor::new(&mut bytes), quality)
                .encode_image(&image)
                .map_err(|error| error.to_string())?;
            if bytes.len() as u64 <= max_bytes {
                let output = output_path(original, "jpg");
                std::fs::write(&output, bytes).map_err(|error| error.to_string())?;
                return Ok(output);
            }
        }
        image = image.resize(
            (image.width() / 2).max(1),
            (image.height() / 2).max(1),
            image::imageops::FilterType::Triangle,
        );
    }
    Err("cannot fit the image within that size".into())
}
