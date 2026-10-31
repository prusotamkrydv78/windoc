//! Windows time zones for world clocks, with daylight saving handled by Windows.

use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::System::SystemInformation::GetSystemTime;
use windows::Win32::System::Time::{
    DYNAMIC_TIME_ZONE_INFORMATION, EnumDynamicTimeZoneInformation,
    SystemTimeToTzSpecificLocalTimeEx,
};
use winisland_platform::LocalDateTime;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeZoneEntry {
    /// Stable Windows key such as `W. Europe Standard Time`.
    pub key: String,
    /// Localized display name such as `(UTC+01:00) Amsterdam, Berlin, ...`.
    pub display: String,
}

fn wide(text: &[u16]) -> String {
    let end = text
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(text.len());
    String::from_utf16_lossy(&text[..end])
}

fn zones() -> Vec<DYNAMIC_TIME_ZONE_INFORMATION> {
    let mut zones = Vec::new();
    for index in 0.. {
        let mut info = DYNAMIC_TIME_ZONE_INFORMATION::default();
        // SAFETY: `info` is a valid out-parameter for the zone at `index`.
        if unsafe { EnumDynamicTimeZoneInformation(index, &mut info) } != 0 {
            break;
        }
        zones.push(info);
    }
    zones
}

/// All time zones known to Windows.
pub fn time_zones() -> Vec<TimeZoneEntry> {
    zones()
        .iter()
        .map(|zone| TimeZoneEntry {
            key: wide(&zone.TimeZoneKeyName),
            display: wide(&zone.StandardName),
        })
        .collect()
}

/// Current local time in the zone with Windows key `key`.
pub fn now_in_zone(key: &str) -> Option<LocalDateTime> {
    let zone = zones()
        .into_iter()
        .find(|zone| wide(&zone.TimeZoneKeyName) == key)?;
    // SAFETY: Reading the current UTC time has no preconditions.
    let utc = unsafe { GetSystemTime() };
    let mut local = SYSTEMTIME::default();
    // SAFETY: All pointers refer to live, correctly typed structures.
    unsafe { SystemTimeToTzSpecificLocalTimeEx(Some(&zone), &utc, &mut local) }.ok()?;
    Some(LocalDateTime {
        year: local.wYear,
        month: local.wMonth,
        day: local.wDay,
        day_of_week: local.wDayOfWeek,
        hour: local.wHour,
        minute: local.wMinute,
        second: local.wSecond,
        millisecond: local.wMilliseconds,
    })
}
