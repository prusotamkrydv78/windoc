//! Image conversion for the File converter tab: decode, resize, encode (JPG, PNG, PDF,
//! ICO) and save next to the original without ever overwriting a file.

use std::fs::OpenOptions;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use image::codecs::ico::IcoEncoder;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::imageops::FilterType;
use image::{DynamicImage, ImageReader, RgbImage};

const MAX_PIXELS: u64 = 100_000_000;
const ICO_MAX_SIDE: u32 = 256;
/// PDF pages are sized as if the image were 96 dpi.
const POINTS_PER_PIXEL: f32 = 0.75;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Jpg,
    Png,
    Pdf,
    Ico,
}

impl Format {
    pub const ALL: [Self; 4] = [Self::Jpg, Self::Png, Self::Pdf, Self::Ico];

    pub fn label(self) -> &'static str {
        match self {
            Self::Jpg => "JPG",
            Self::Png => "PNG",
            Self::Pdf => "PDF",
            Self::Ico => "ICO",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Jpg => "jpg",
            Self::Png => "png",
            Self::Pdf => "pdf",
            Self::Ico => "ico",
        }
    }

    /// Whether quality and target size apply (JPEG-based outputs).
    pub fn lossy(self) -> bool {
        matches!(self, Self::Jpg | Self::Pdf)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Resize {
    Original,
    Half,
    Max1920,
    Max1280,
}

impl Resize {
    pub fn label(self) -> &'static str {
        match self {
            Self::Original => "Original",
            Self::Half => "50%",
            Self::Max1920 => "Max 1920 px",
            Self::Max1280 => "Max 1280 px",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Original => Self::Half,
            Self::Half => Self::Max1920,
            Self::Max1920 => Self::Max1280,
            Self::Max1280 => Self::Original,
        }
    }

    /// The output size for an image of `width` × `height` (never upscales).
    pub fn apply(self, width: u32, height: u32) -> (u32, u32) {
        let longest = width.max(height).max(1);
        let limit = match self {
            Self::Original => return (width, height),
            Self::Half => return ((width / 2).max(1), (height / 2).max(1)),
            Self::Max1920 => 1920,
            Self::Max1280 => 1280,
        };
        if longest <= limit {
            return (width, height);
        }
        let scale = limit as f64 / longest as f64;
        (
            ((width as f64 * scale).round() as u32).max(1),
            ((height as f64 * scale).round() as u32).max(1),
        )
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub format: Format,
    /// JPEG quality, 1..=100, when `target_percent` is `None`.
    pub quality: u8,
    /// Aim for this share of each original's size instead of a fixed quality.
    pub target_percent: Option<u8>,
    pub resize: Resize,
    /// `None` saves next to each original.
    pub folder: Option<PathBuf>,
}

fn load(path: &Path) -> Result<DynamicImage, String> {
    let open = || {
        ImageReader::open(path)
            .and_then(ImageReader::with_guessed_format)
            .map_err(|error| error.to_string())
    };
    let (width, height) = open()?
        .into_dimensions()
        .map_err(|_| "not a supported image".to_string())?;
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err("image is too large".into());
    }
    open()?.decode().map_err(|error| error.to_string())
}

/// Flattens transparency onto white (JPEG and the PDF image have no alpha).
fn flatten(image: &DynamicImage) -> RgbImage {
    if !image.color().has_alpha() {
        return image.to_rgb8();
    }
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    RgbImage::from_fn(width, height, |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let blend = |channel: u8| {
            ((u16::from(channel) * u16::from(a) + 255 * (255 - u16::from(a))) / 255) as u8
        };
        image::Rgb([blend(r), blend(g), blend(b)])
    })
}

fn jpeg(image: &RgbImage, quality: u8) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    JpegEncoder::new_with_quality(&mut Cursor::new(&mut bytes), quality.clamp(1, 100))
        .encode_image(image)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

/// The best JPEG at most `target` bytes: highest quality that fits, shrinking the image
/// when even the lowest quality is too big. Falls back to the smallest attempt.
fn jpeg_for_target(image: RgbImage, target: u64) -> Result<(Vec<u8>, u32, u32), String> {
    let mut image = image;
    let mut smallest: Option<(Vec<u8>, u32, u32)> = None;
    for _ in 0..6 {
        let (mut low, mut high) = (5u8, 95u8);
        let mut best = None;
        while low <= high {
            let quality = low + (high - low) / 2;
            let bytes = jpeg(&image, quality)?;
            if bytes.len() as u64 <= target {
                best = Some(bytes);
                low = quality + 1;
            } else {
                if smallest
                    .as_ref()
                    .is_none_or(|(small, _, _)| bytes.len() < small.len())
                {
                    smallest = Some((bytes, image.width(), image.height()));
                }
                high = quality - 1;
            }
        }
        if let Some(bytes) = best {
            return Ok((bytes, image.width(), image.height()));
        }
        if image.width() <= 16 || image.height() <= 16 {
            break;
        }
        image = image::imageops::resize(
            &image,
            (image.width() * 3 / 4).max(1),
            (image.height() * 3 / 4).max(1),
            FilterType::Triangle,
        );
    }
    smallest.ok_or_else(|| "cannot encode the image".to_string())
}

/// A one-page PDF showing a JPEG image of `width` × `height` pixels.
fn pdf(jpeg: &[u8], width: u32, height: u32) -> Vec<u8> {
    let page_w = width as f32 * POINTS_PER_PIXEL;
    let page_h = height as f32 * POINTS_PER_PIXEL;
    let content = format!("q {page_w:.2} 0 0 {page_h:.2} 0 0 cm /Im0 Do Q\n");
    let mut out: Vec<u8> = Vec::with_capacity(jpeg.len() + 1024);
    let mut offsets = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    let mut object = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    object(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
    object(&mut out, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    object(
        &mut out,
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {page_w:.2} {page_h:.2}] \
             /Resources << /XObject << /Im0 4 0 R >> /ProcSet [/PDF /ImageC] >> \
             /Contents 5 0 R >>"
        )
        .as_bytes(),
    );
    let mut image = format!(
        "<< /Type /XObject /Subtype /Image /Width {width} /Height {height} \
         /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
        jpeg.len()
    )
    .into_bytes();
    image.extend_from_slice(jpeg);
    image.extend_from_slice(b"\nendstream");
    object(&mut out, &image);
    let mut contents = format!("<< /Length {} >>\nstream\n", content.trim_end().len()).into_bytes();
    contents.extend_from_slice(content.as_bytes());
    contents.extend_from_slice(b"endstream");
    object(&mut out, &contents);
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes(),
    );
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// Writes `bytes` as `stem.ext` in `folder`, or `stem (1).ext`, `stem (2).ext`… when the
/// name is taken. Never replaces an existing file.
fn save_unique(
    folder: &Path,
    stem: &str,
    extension: &str,
    bytes: &[u8],
) -> Result<PathBuf, String> {
    for counter in 0..1000 {
        let name = if counter == 0 {
            format!("{stem}.{extension}")
        } else {
            format!("{stem} ({counter}).{extension}")
        };
        let candidate = folder.join(name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(mut file) => {
                if let Err(error) = file.write_all(bytes) {
                    drop(file);
                    let _ = std::fs::remove_file(&candidate);
                    return Err(error.to_string());
                }
                return Ok(candidate);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("no free file name".into())
}

/// Converts `original` with `settings`. Returns the new file and its size.
pub fn convert(original: &Path, settings: &Settings) -> Result<(PathBuf, u64), String> {
    let original_bytes = std::fs::metadata(original)
        .map_err(|error| error.to_string())?
        .len();
    let mut image = load(original)?;
    let (width, height) = settings.resize.apply(image.width(), image.height());
    if (width, height) != (image.width(), image.height()) {
        image = image.resize_exact(width, height, FilterType::Lanczos3);
    }
    let target = settings
        .target_percent
        .map(|percent| (original_bytes * u64::from(percent) / 100).max(4096));
    let bytes = match settings.format {
        Format::Jpg | Format::Pdf => {
            let rgb = flatten(&image);
            let (encoded, width, height) = match target {
                Some(target) => jpeg_for_target(rgb, target)?,
                None => {
                    let (width, height) = rgb.dimensions();
                    (jpeg(&rgb, settings.quality)?, width, height)
                }
            };
            if settings.format == Format::Pdf {
                pdf(&encoded, width, height)
            } else {
                encoded
            }
        }
        Format::Png => {
            let mut bytes = Vec::new();
            image
                .write_with_encoder(PngEncoder::new_with_quality(
                    &mut bytes,
                    CompressionType::Best,
                    PngFilter::Adaptive,
                ))
                .map_err(|error| error.to_string())?;
            bytes
        }
        Format::Ico => {
            if image.width() > ICO_MAX_SIDE || image.height() > ICO_MAX_SIDE {
                image = image.thumbnail(ICO_MAX_SIDE, ICO_MAX_SIDE);
            }
            let mut bytes = Vec::new();
            DynamicImage::ImageRgba8(image.to_rgba8())
                .write_with_encoder(IcoEncoder::new(&mut bytes))
                .map_err(|error| error.to_string())?;
            bytes
        }
    };
    let folder = match &settings.folder {
        Some(folder) => folder.clone(),
        None => original
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf),
    };
    let stem = original.file_stem().map_or_else(
        || "image".to_string(),
        |stem| stem.to_string_lossy().into_owned(),
    );
    let output = save_unique(&folder, &stem, settings.format.extension(), &bytes)?;
    Ok((output, bytes.len() as u64))
}

/// A rough output size for an image of `width` × `height` pixels whose file is
/// `original_bytes`, for the footer estimate.
pub fn estimate(
    settings: &Settings,
    original_bytes: u64,
    dims: Option<(u32, u32)>,
    same_format: bool,
) -> u64 {
    let Some((width, height)) = dims else {
        return original_bytes;
    };
    let (width, height) = settings.resize.apply(width, height);
    let pixels = f64::from(width) * f64::from(height);
    match settings.format {
        Format::Jpg | Format::Pdf => {
            let overhead = if settings.format == Format::Pdf {
                1200.0
            } else {
                600.0
            };
            let estimate = match settings.target_percent {
                Some(percent) => (original_bytes * u64::from(percent) / 100).max(4096) as f64,
                None => {
                    let q = f64::from(settings.quality.clamp(1, 100)) / 100.0;
                    pixels * (0.05 + 0.95 * q.powi(6))
                }
            };
            (estimate + overhead) as u64
        }
        Format::Png => {
            if same_format && settings.resize == Resize::Original {
                original_bytes
            } else {
                (pixels * 1.9) as u64
            }
        }
        Format::Ico => {
            let side = f64::from(width.max(height).min(ICO_MAX_SIDE)).max(1.0);
            let scale = side / f64::from(width.max(height).max(1));
            (pixels * scale * scale * 1.6) as u64 + 64
        }
    }
}
