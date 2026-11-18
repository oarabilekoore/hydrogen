use crate::domain::calendar::model::RawVEvent;
use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use std::collections::HashMap;

const WEEKDAYS: [&str; 7] = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];

pub fn unescape_ics(s: &str) -> String {
    s.replace("\\n", " ")
        .replace("\\N", " ")
        .replace("\\,", ",")
        .replace("\\;", ";")
        .replace("\\\\", "\\")
}

pub fn parse_ics_datetime(params: &HashMap<String, String>, val: &str) -> Option<(NaiveDateTime, bool)> {
    let v = val.trim();
    if params.get("VALUE").map(|s| s.as_str()) == Some("DATE") || v.len() == 8 {
        let d = NaiveDate::parse_from_str(v.get(..8)?, "%Y%m%d").ok()?;
        return Some((d.and_hms_opt(0, 0, 0)?, true));
    }
    if v.ends_with('Z') {
        let naive_utc = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%SZ").ok()?;
        let local_dt = Utc.from_utc_datetime(&naive_utc).with_timezone(&Local);
        return Some((local_dt.naive_local(), false));
    }
    let naive = NaiveDateTime::parse_from_str(v.get(..15)?, "%Y%m%dT%H%M%S").ok()?;
    if let Some(tz_name) = params.get("TZID") {
        if let Ok(tz) = tz_name.parse::<Tz>() {
            if let Some(zoned) = tz.from_local_datetime(&naive).single() {
                return Some((zoned.with_timezone(&Local).naive_local(), false));
            }
        }
    }
    Some((naive, false))
}

fn add_months(dt: NaiveDateTime, months: i32) -> Option<NaiveDateTime> {
    let total_months = (dt.year() * 12) + (dt.month() as i32 - 1) + months;
    let y = total_months.div_euclid(12);
    let m = (total_months.rem_euclid(12) + 1) as u32;
    let new_date = NaiveDate::from_ymd_opt(y, m, dt.day())?;
    Some(new_date.and_time(dt.time()))
}

fn nth_occurrence(start: NaiveDateTime, freq: &str, interval: i64, k: i64) -> Option<NaiveDateTime> {
    match freq {
        "DAILY" => Some(start + Duration::days(interval * k)),
        "WEEKLY" => Some(start + Duration::weeks(interval * k)),
        "MONTHLY" => add_months(start, (interval * k) as i32),
        "YEARLY" => {
            let d = NaiveDate::from_ymd_opt(start.year() + (interval * k) as i32, start.month(), start.day())?;
            Some(d.and_time(start.time()))
        }
        _ => None,
    }
}

pub fn expand_occurrences(
    ev: &RawVEvent,
    win_start: NaiveDateTime,
    win_end: NaiveDateTime,
) -> Vec<NaiveDateTime> {
    let is_visible = |s: NaiveDateTime| -> bool {
        s + ev.duration >= win_start
            && s < win_end
            && !ev.exdates_dt.contains(&s)
            && !ev.exdates_date.contains(&s.date())
    };

    let Some(rrule) = &ev.rrule else {
        return if is_visible(ev.start) {
            vec![ev.start]
        } else {
            Vec::new()
        };
    };

    let mut parts: HashMap<&str, &str> = HashMap::new();
    for piece in rrule.split(';') {
        if let Some((k, v)) = piece.split_once('=') {
            parts.insert(k, v);
        }
    }

    let freq = *parts.get("FREQ").unwrap_or(&"");
    let interval = parts
        .get("INTERVAL")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(1)
        .max(1);
    let count = parts.get("COUNT").and_then(|v| v.parse::<usize>().ok());
    let until = parts.get("UNTIL").and_then(|u| {
        let (mut dt, all_day) = parse_ics_datetime(&HashMap::new(), u)?;
        if all_day {
            dt = dt.date().and_hms_opt(23, 59, 59)?;
        }
        Some(dt)
    });

    let mut byday_indices: Vec<i64> = Vec::new();
    if let Some(bd) = parts.get("BYDAY") {
        for token in bd.split(',') {
            if token.len() >= 2 {
                let code = &token[token.len() - 2..];
                if let Some(idx) = WEEKDAYS.iter().position(|&w| w == code) {
                    byday_indices.push(idx as i64);
                }
            }
        }
    }

    let mut out = Vec::new();
    let mut n = 0usize;
    let mut k = 0i64;
    let mut guard = 0usize;
    let mut cur = Some(ev.start);

    while cur.is_none() || cur.unwrap() < win_end {
        guard += 1;
        if guard > 6000 {
            break;
        }
        let Some(c_dt) = cur else {
            k += 1;
            cur = nth_occurrence(ev.start, freq, interval, k);
            continue;
        };

        let cands: Vec<NaiveDateTime> = if freq == "WEEKLY" && !byday_indices.is_empty() {
            let wd = i64::from(c_dt.weekday().num_days_from_monday());
            let monday = c_dt - Duration::days(wd);
            let mut list: Vec<NaiveDateTime> = byday_indices
                .iter()
                .map(|&d_idx| (monday + Duration::days(d_idx)).date().and_time(ev.start.time()))
                .filter(|&dt| dt >= ev.start)
                .collect();
            list.sort();
            list.dedup();
            list
        } else {
            vec![c_dt]
        };

        let mut stop = false;
        for c in cands {
            if until.map(|u| c > u).unwrap_or(false) || count.map(|max_n| n >= max_n).unwrap_or(false) {
                stop = true;
                break;
            }
            n += 1;
            if is_visible(c) {
                out.push(c);
            }
        }

        if stop || !matches!(freq, "DAILY" | "WEEKLY" | "MONTHLY" | "YEARLY") {
            break;
        }
        k += 1;
        cur = nth_occurrence(ev.start, freq, interval, k);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn expands_weekly_byday_rrule() {
        let start = NaiveDate::from_ymd_opt(2026, 11, 16)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap();
        let ev = RawVEvent {
            title: "Lecture".into(),
            start,
            duration: Duration::hours(1),
            all_day: false,
            rrule: Some("FREQ=WEEKLY;BYDAY=MO,WE;COUNT=4".into()),
            exdates_dt: HashSet::new(),
            exdates_date: HashSet::new(),
        };
        let win_end = start + Duration::days(20);
        let occs = expand_occurrences(&ev, start, win_end);
        assert_eq!(occs.len(), 4);
    }
}
