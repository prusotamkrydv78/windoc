//! Tiny generated sounds for the robot: soft sine notes with a quick fade, played on the
//! default output device from a short-lived thread. No audio files are shipped.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cpal::SampleFormat;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// At most one sound this often, however many reactions fire.
const MIN_GAP_MS: u64 = 30_000;
const VOLUME: f32 = 0.16;

static ENABLED: AtomicBool = AtomicBool::new(false);
static LAST_MS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug)]
pub enum Sound {
    /// Two rising notes: hello, welcome back, charging.
    Hello,
    /// A bright three-note arpeggio: a finished Focus session.
    Done,
    /// Quick high chirps: the little mouse.
    Squeak,
    /// A short soft pop: something caught or saved.
    Pop,
}

impl Sound {
    /// `(frequency Hz, start s, length s)` for each note.
    fn notes(self) -> &'static [(f32, f32, f32)] {
        match self {
            Self::Hello => &[(660.0, 0.0, 0.12), (880.0, 0.11, 0.18)],
            Self::Done => &[(784.0, 0.0, 0.12), (988.0, 0.1, 0.12), (1319.0, 0.2, 0.26)],
            Self::Squeak => &[
                (2400.0, 0.0, 0.05),
                (2900.0, 0.08, 0.05),
                (2600.0, 0.16, 0.06),
            ],
            Self::Pop => &[(520.0, 0.0, 0.08)],
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

/// Plays `sound` if sounds are on and none played in the last half minute.
pub fn play(sound: Sound) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let now = now_ms();
    let last = LAST_MS.load(Ordering::Relaxed);
    if now.saturating_sub(last) < MIN_GAP_MS
        || LAST_MS
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
    {
        return;
    }
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
        for sound in [Sound::Hello, Sound::Done, Sound::Squeak, Sound::Pop] {
            for step in 0..2000 {
                assert!(sample(sound, step as f32 / 4000.0).abs() <= VOLUME * 3.0);
            }
            assert_eq!(sample(sound, 2.0), 0.0);
        }
    }
}
