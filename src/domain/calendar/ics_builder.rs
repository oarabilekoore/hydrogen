use crate::error::{GoodSearchError, Result};
use chrono::{Duration, NaiveDate, NaiveDateTime, Utc};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn escape_ics(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
}

pub fn build_single_event_ics(
    date_str: &str,
    time_str: &str,
    title: &str,
    duration_mins: i64,
) -> Result<(String, String)> {
    let summary = if title.trim().is_empty() {
        "New event"
    } else {
        title.trim()
    };

    let (dtstart, dtend) = if time_str.is_empty() || time_str.eq_ignore_ascii_case("allday") {
        let d = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
            .map_err(|e| GoodSearchError::InvalidInput(format!("Invalid date '{date_str}': {e}")))?;
        let next_d = d + Duration::days(1);
        (
            format!("DTSTART;VALUE=DATE:{}", d.format("%Y%m%d")),
            format!("DTEND;VALUE=DATE:{}", next_d.format("%Y%m%d")),
        )
    } else {
        let combined = format!("{date_str} {time_str}");
        let s = NaiveDateTime::parse_from_str(&combined, "%Y-%m-%d %H:%M").map_err(|e| {
            GoodSearchError::InvalidInput(format!("Invalid datetime '{combined}': {e}"))
        })?;
        let e = s + Duration::minutes(duration_mins);
        (
            format!("DTSTART:{}", s.format("%Y%m%dT%H%M%S")),
            format!("DTEND:{}", e.format("%Y%m%dT%H%M%S")),
        )
    };

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let uid = format!("{nanos:x}");
    let short_uid = &uid[..uid.len().min(8)];
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");

    let ics = [
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//quickshell//gsearch//EN".to_string(),
        "BEGIN:VEVENT".to_string(),
        format!("UID:{uid}@quickshell"),
        format!("DTSTAMP:{stamp}"),
        dtstart,
        dtend,
        format!("SUMMARY:{}", escape_ics(summary)),
        "END:VEVENT".to_string(),
        "END:VCALENDAR".to_string(),
        String::new(),
    ]
    .join("\r\n");

    Ok((short_uid.to_string(), ics))
}
