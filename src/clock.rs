//! Границы локального дня в UTC.
//!
//! API `/v1/usage` понимает даты только как UTC, если передавать `YYYY-MM-DD`
//! (даже с `timezone`), поэтому границы считаем сами и отправляем полные
//! ISO-datetime с `Z`.

use jiff::tz::TimeZone;
use jiff::{Span, Timestamp, Zoned};

/// Границы дня для запроса к API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayBounds {
    /// Начало дня, `2026-10-30T21:00:00Z`.
    pub start: String,
    /// Последняя секунда дня, `2026-10-31T20:59:59Z`.
    pub end: String,
    /// Локальная дата, `2026-10-31`.
    pub day: String,
    /// Локальное время, `16:12`.
    pub time: String,
}

/// Границы дня `now` в таймзоне `tz_name`.
pub fn day_bounds(tz_name: &str, now: Timestamp) -> Result<DayBounds, String> {
    let tz = TimeZone::get(tz_name).map_err(|e| format!("таймзона {tz_name}: {e}"))?;
    let zoned: Zoned = now.to_zoned(tz);
    bounds_of(&zoned)
}

fn bounds_of(zoned: &Zoned) -> Result<DayBounds, String> {
    let start = zoned.start_of_day().map_err(|e| e.to_string())?;
    let next = start
        .checked_add(Span::new().days(1))
        .map_err(|e| e.to_string())?;
    let end = next
        .checked_sub(Span::new().seconds(1))
        .map_err(|e| e.to_string())?;
    Ok(DayBounds {
        start: rfc3339_utc(start.timestamp()),
        end: rfc3339_utc(end.timestamp()),
        day: start.strftime("%Y-%m-%d").to_string(),
        time: zoned.strftime("%H:%M").to_string(),
    })
}

/// `2026-10-30T21:00:00Z` — без долей секунды.
fn rfc3339_utc(ts: Timestamp) -> String {
    format!("{:.0}", ts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn moscow_day_starts_at_21_utc() {
        let b = day_bounds("Europe/Moscow", ts("2026-10-31T13:12:00Z")).unwrap();
        assert_eq!(b.start, "2026-10-30T21:00:00Z");
        assert_eq!(b.end, "2026-10-31T20:59:59Z");
        assert_eq!(b.day, "2026-10-31");
        assert_eq!(b.time, "16:12");
    }

    #[test]
    fn utc_day_matches_calendar() {
        let b = day_bounds("UTC", ts("2026-10-31T13:12:00Z")).unwrap();
        assert_eq!(b.start, "2026-10-31T00:00:00Z");
        assert_eq!(b.end, "2026-10-31T23:59:59Z");
        assert_eq!(b.day, "2026-10-31");
    }

    #[test]
    fn late_evening_keeps_local_day() {
        // 23:30 по Москве — всё ещё 31 октября, хотя в UTC уже 20:30.
        let b = day_bounds("Europe/Moscow", ts("2026-10-31T20:30:00Z")).unwrap();
        assert_eq!(b.day, "2026-10-31");
        assert_eq!(b.time, "23:30");
        assert_eq!(b.start, "2026-10-30T21:00:00Z");
    }

    #[test]
    fn day_length_is_one_day_in_seconds() {
        let b = day_bounds("Europe/Moscow", ts("2026-10-31T13:12:00Z")).unwrap();
        let start: Timestamp = b.start.parse().unwrap();
        let end: Timestamp = b.end.parse().unwrap();
        assert_eq!(end.as_second() - start.as_second(), 86_399);
    }

    #[test]
    fn dst_day_is_long() {
        // Ночь на 25.10.2026 в Берлине: переход с летнего времени на зимнее.
        let b = day_bounds("Europe/Berlin", ts("2026-10-24T22:30:00Z")).unwrap();
        let start: Timestamp = b.start.parse().unwrap();
        let end: Timestamp = b.end.parse().unwrap();
        assert_eq!(b.day, "2026-10-25");
        assert_eq!(end.as_second() - start.as_second(), 89_999);
    }

    #[test]
    fn unknown_timezone_is_an_error() {
        assert!(day_bounds("Mars/Olympus", ts("2026-10-31T13:12:00Z")).is_err());
    }
}
