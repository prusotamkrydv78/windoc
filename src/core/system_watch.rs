//! Background sampling of system state the notch, the taskbar and the System tab show.
//!
//! These queries block for a few milliseconds each (a registry scan, window enumeration with
//! a process lookup per window, shell, power and audio calls), so a worker thread refreshes
//! them and the UI thread only reads the latest snapshot.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use winisland_platform::MetricSelection;
use winisland_platform_windows::shell_bar::{self, AppWindow};

const INTERVAL: Duration = Duration::from_secs(2);
/// Samples kept for the System tab's sparklines (about 80 s).
const HISTORY: usize = 40;
/// Wi-Fi details come from `netsh`, which is slower; refresh them every 5th sample.
const WIFI_EVERY: u32 = 5;

#[derive(Clone, Debug, Default)]
pub struct WifiInfo {
    pub name: String,
    /// Friendly standard, such as "Wi-Fi 6".
    pub standard: String,
    pub receive_mbps: Option<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct SystemSnapshot {
    pub microphone: bool,
    pub camera: bool,
    pub do_not_disturb: bool,
    pub battery: Option<(u8, bool)>,
    /// Taskbar-style app windows.
    pub apps: Vec<AppWindow>,
    /// Whole-system CPU load over the last interval, 0 to 100.
    pub cpu_percent: f32,
    /// Whether Windows reports an internet connection.
    pub online: bool,
    /// False for the empty snapshot before the first sample.
    pub sampled: bool,
    /// CPU, memory and system-drive use as 0..=1.
    pub cpu: Option<f32>,
    pub memory: Option<f32>,
    pub memory_bytes: Option<(u64, u64)>,
    pub disk: Option<f32>,
    pub disk_bytes: Option<(u64, u64)>,
    /// Current download and upload rates in bytes per second.
    pub download: Option<f64>,
    pub upload: Option<f64>,
    pub link_bits_per_second: Option<u64>,
    pub wifi: Option<WifiInfo>,
    /// Default output volume 0..=1 and mute.
    pub volume: Option<(f32, bool)>,
    /// Oldest first, one value per sample.
    pub cpu_history: Vec<f32>,
    pub memory_history: Vec<f32>,
    pub disk_history: Vec<f32>,
    pub download_history: Vec<f32>,
    pub upload_history: Vec<f32>,
}

static SNAPSHOT: Mutex<Option<SystemSnapshot>> = Mutex::new(None);
static STARTED: OnceLock<()> = OnceLock::new();

/// Starts the worker once; later calls do nothing.
pub fn start() {
    STARTED.get_or_init(|| {
        let spawned = std::thread::Builder::new()
            .name("system-watch".into())
            .spawn(|| {
                let mut sampler = Sampler::default();
                loop {
                    let snapshot = sampler.sample();
                    if let Ok(mut slot) = SNAPSHOT.lock() {
                        *slot = Some(snapshot);
                    }
                    crate::platform::wake();
                    std::thread::sleep(INTERVAL);
                }
            });
        if let Err(error) = spawned {
            log::warn!("System watcher could not start: {error}");
        }
    });
}

/// The latest snapshot, or an empty one before the first sample.
pub fn snapshot() -> SystemSnapshot {
    SNAPSHOT
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
        .unwrap_or_default()
}

#[derive(Default)]
struct Sampler {
    count: u32,
    cpu_ticks: Option<(u64, u64)>,
    network: Option<(winisland_platform_windows::NetworkTotals, Instant)>,
    wifi: Option<WifiInfo>,
    cpu_history: VecDeque<f32>,
    memory_history: VecDeque<f32>,
    disk_history: VecDeque<f32>,
    download_history: VecDeque<f32>,
    upload_history: VecDeque<f32>,
}

fn push(history: &mut VecDeque<f32>, value: f32) {
    if history.len() == HISTORY {
        history.pop_front();
    }
    history.push_back(value);
}

/// "802.11ax" → "Wi-Fi 6", the names people know.
fn wifi_standard(radio: &str) -> String {
    let radio = radio.to_ascii_lowercase();
    let name = if radio.contains("be") {
        "Wi-Fi 7"
    } else if radio.contains("ax") {
        "Wi-Fi 6"
    } else if radio.contains("ac") {
        "Wi-Fi 5"
    } else if radio.ends_with('n') {
        "Wi-Fi 4"
    } else {
        return radio;
    };
    name.to_string()
}

impl Sampler {
    fn sample(&mut self) -> SystemSnapshot {
        let (microphone, camera) = crate::platform::capture_in_use();
        let metrics = crate::platform::metrics()
            .sample(MetricSelection {
                cpu: true,
                memory: true,
                network: false,
                disk: true,
                gpu: false,
            })
            .unwrap_or_default();

        let cpu = match (
            metrics.cpu_idle_ticks,
            metrics.cpu_total_ticks,
            self.cpu_ticks,
        ) {
            (Some(idle), Some(total), Some((last_idle, last_total))) if total > last_total => {
                let busy = (total - last_total).saturating_sub(idle.saturating_sub(last_idle));
                Some(busy as f32 / (total - last_total) as f32)
            }
            _ => None,
        };
        if let (Some(idle), Some(total)) = (metrics.cpu_idle_ticks, metrics.cpu_total_ticks) {
            self.cpu_ticks = Some((idle, total));
        }
        let memory_bytes = metrics.memory_used_bytes.zip(metrics.memory_total_bytes);
        let memory = memory_bytes.map(|(used, total)| used as f32 / total.max(1) as f32);
        let disk_bytes = metrics
            .disk_total_bytes
            .zip(metrics.disk_free_bytes)
            .map(|(total, free)| (total.saturating_sub(free), total));
        let disk = disk_bytes.map(|(used, total)| used as f32 / total.max(1) as f32);

        let now = Instant::now();
        let totals = winisland_platform_windows::network_totals();
        let (download, upload) = match (totals, self.network) {
            (Some(current), Some((previous, at))) => {
                let seconds = now.duration_since(at).as_secs_f64().max(0.001);
                (
                    Some(current.received.saturating_sub(previous.received) as f64 / seconds),
                    Some(current.sent.saturating_sub(previous.sent) as f64 / seconds),
                )
            }
            _ => (None, None),
        };
        if let Some(current) = totals {
            self.network = Some((current, now));
        }
        if self.count % WIFI_EVERY == 0 {
            self.wifi =
                winisland_platform_windows::wifi_connection().map(|(name, radio, rate)| WifiInfo {
                    name,
                    standard: wifi_standard(&radio),
                    receive_mbps: rate,
                });
        }
        self.count = self.count.wrapping_add(1);

        if let Some(cpu) = cpu {
            push(&mut self.cpu_history, cpu);
        }
        if let Some(memory) = memory {
            push(&mut self.memory_history, memory);
        }
        if let Some(disk) = disk {
            push(&mut self.disk_history, disk);
        }
        if let (Some(down), Some(up)) = (download, upload) {
            push(&mut self.download_history, down as f32);
            push(&mut self.upload_history, up as f32);
        }

        SystemSnapshot {
            microphone,
            camera,
            do_not_disturb: crate::platform::do_not_disturb(),
            battery: crate::platform::battery(),
            apps: shell_bar::app_windows(),
            cpu_percent: cpu.map_or(0.0, |cpu| cpu * 100.0),
            online: online(),
            sampled: true,
            cpu,
            memory,
            memory_bytes,
            disk,
            disk_bytes,
            download,
            upload,
            link_bits_per_second: totals.map(|totals| totals.link_bits_per_second),
            wifi: self.wifi.clone(),
            volume: crate::platform::audio()
                .volume()
                .ok()
                .map(|state| (state.level, state.muted)),
            cpu_history: self.cpu_history.iter().copied().collect(),
            memory_history: self.memory_history.iter().copied().collect(),
            disk_history: self.disk_history.iter().copied().collect(),
            download_history: self.download_history.iter().copied().collect(),
            upload_history: self.upload_history.iter().copied().collect(),
        }
    }
}

fn online() -> bool {
    use windows::Win32::Networking::WinInet::{INTERNET_CONNECTION, InternetGetConnectedState};
    let mut flags = INTERNET_CONNECTION(0);
    // SAFETY: `flags` is a valid out-pointer; the reserved argument must be 0.
    unsafe { InternetGetConnectedState(&mut flags, Some(0)) }.is_ok()
}
