//! Tiny generated sounds for things the user should notice (charger, devices, downloads,
//! network, breaks, finished sessions): soft sine notes with a quick fade, played on the
//! default output device from a short-lived thread. Each event has its own sound so it can
//! be told apart without looking. No audio files are shipped.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cpal::SampleFormat;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// The same sound at most this often, so a flapping device or network does not chatter.
const MIN_GAP_MS: u64 = 4_000;
/// Any sound at most this often, so two events at once do not overlap.
const ANY_GAP_MS: u64 = 700;
const VOLUME: f32 = 0.16;

static ENABLED: AtomicBool = AtomicBool::new(false);
static LAST_ANY_MS: AtomicU64 = AtomicU64::new(0);
static LAST_MS: [AtomicU64; Sound::COUNT] = [const { AtomicU64::new(0) }; Sound::COUNT];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    /// A bright three-note arpeggio: a finished Focus session.
    Done,
    /// A short fanfare: a trophy for the day's third session.
    Fanfare,
    /// A rising power-up sweep: the charger went in.
    PowerOn,
    /// Two falling notes: the charger came out.
    PowerOff,
    /// Three slow falling notes: the battery is low.
    LowBattery,
    /// A low-high pair: a device (mouse, drive) was plugged in.
    DeviceIn,
    /// Two quick bright dings: a download finished.
    Download,
    /// A quick double click: a screenshot was saved.
    Shutter,
    /// A tiny tick: something was copied.
    Copy,
    /// A soft falling minor third: the internet dropped.
    Offline,
    /// A rising major third: the internet is back.
    Online,
    /// Two low pulses: the PC is working hard.
    Warning,
    /// A gentle bell: a water, stretch, lunch or wrap-up break.
    Reminder,
}

impl Sound {
    const COUNT: usize = 13;

    fn index(self) -> usize {
        self as usize
    }

    /// `(frequency Hz, start s, length s)` for each note.
    fn notes(self) -> &'static [(f32, f32, f32)] {
        match self {
            Self::Done => &[(784.0, 0.0, 0.12), (988.0, 0.1, 0.12), (1319.0, 0.2, 0.26)],
            Self::Fanfare => &[
                (523.0, 0.0, 0.1),
                (659.0, 0.09, 0.1),
                (784.0, 0.18, 0.1),
                (1047.0, 0.27, 0.34),
            ],
            Self::PowerOn => &[
                (440.0, 0.0, 0.07),
                (554.0, 0.05, 0.07),
                (659.0, 0.1, 0.07),
                (880.0, 0.15, 0.2),
            ],
            Self::PowerOff => &[(784.0, 0.0, 0.1), (523.0, 0.1, 0.2)],
            Self::LowBattery => &[(659.0, 0.0, 0.14), (587.0, 0.16, 0.14), (440.0, 0.32, 0.3)],
            Self::DeviceIn => &[(392.0, 0.0, 0.09), (784.0, 0.09, 0.16)],
            Self::Download => &[(1175.0, 0.0, 0.12), (1568.0, 0.12, 0.22)],
            Self::Shutter => &[(1900.0, 0.0, 0.025), (1400.0, 0.06, 0.035)],
            Self::Copy => &[(1320.0, 0.0, 0.04)],
            Self::Offline => &[(587.0, 0.0, 0.14), (494.0, 0.14, 0.26)],
            Self::Online => &[(523.0, 0.0, 0.12), (659.0, 0.12, 0.22)],
            Self::Warning => &[(330.0, 0.0, 0.12), (330.0, 0.2, 0.12)],
            Self::Reminder => &[(880.0, 0.0, 0.5), (1320.0, 0.0, 0.35)],
        }
    }
}

/// Turns sounds on or off; the caller folds in Do Not Disturb and quiet modes.
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |time| time.as_millis() as u64)
}

/// Plays `sound` if sounds are on and it has not just played.
pub fn play(sound: Sound) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let now = now_ms();
    let slot = &LAST_MS[sound.index()];
    let last = slot.load(Ordering::Relaxed);
    if now.saturating_sub(last) < MIN_GAP_MS
        || now.saturating_sub(LAST_ANY_MS.load(Ordering::Relaxed)) < ANY_GAP_MS
        || slot
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
    {
        return;
    }
    LAST_ANY_MS.store(now, Ordering::Relaxed);
    let _ = std::thread::Builder::new()
        .name("winisland-chime".into())
        .spawn(move || {
            if let Err(error) = render(sound) {
                log::warn!("Robot sound failed: {error}");
            }
        });
}

/// One sample of `sound` at `time` seconds: summed notes, each with a fast attack and an
/// exponential fade so nothing clicks.
fn sample(sound: Sound, time: f32) -> f32 {
    sound
        .notes()
        .iter()
        .filter(|(_, start, length)| time >= *start && time < start + length)
        .map(|(frequency, start, length)| {
            let local = time - start;
            let attack = (local / 0.008).min(1.0);
            let fade = (-4.0 * local / length).exp() * (1.0 - local / length);
            (local * frequency * std::f32::consts::TAU).sin() * attack * fade
        })
        .sum::<f32>()
        * VOLUME
}

fn render(sound: Sound) -> Result<(), String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("no output device")?;
    let config = device
        .default_output_config()
        .map_err(|error| error.to_string())?;
    if config.sample_format() != SampleFormat::F32 {
        return Err(format!("unsupported format {:?}", config.sample_format()));
    }
    let stream_config = config.config();
    let rate = stream_config.sample_rate as f32;
    let channels = usize::from(stream_config.channels.max(1));
    let total = sound
        .notes()
        .iter()
        .map(|(_, start, length)| start + length)
        .fold(0.0, f32::max);
    let mut frame = 0u64;
    let stream = device
        .build_output_stream(
            stream_config,
            move |data: &mut [f32], _| {
                for out in data.chunks_mut(channels) {
                    let value = sample(sound, frame as f32 / rate);
                    out.fill(value);
                    frame += 1;
                }
            },
            |error| log::warn!("Robot sound stream error: {error}"),
            None,
        )
        .map_err(|error| error.to_string())?;
    stream.play().map_err(|error| error.to_string())?;
    std::thread::sleep(Duration::from_secs_f32(total + 0.15));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sounds_stay_quiet_and_end_silent() {
        for sound in [
            Sound::Done,
            Sound::Fanfare,
            Sound::PowerOn,
            Sound::PowerOff,
            Sound::LowBattery,
            Sound::DeviceIn,
            Sound::Download,
            Sound::Shutter,
            Sound::Copy,
            Sound::Offline,
            Sound::Online,
            Sound::Warning,
            Sound::Reminder,
        ] {
            assert!(sound.index() < Sound::COUNT);
            for step in 0..2000 {
                assert!(sample(sound, step as f32 / 4000.0).abs() <= VOLUME * 3.0);
            }
            assert_eq!(sample(sound, 2.0), 0.0);
        }
    }
}
