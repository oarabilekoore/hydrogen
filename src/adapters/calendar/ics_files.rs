use crate::domain::calendar::model::RawVEvent;
use crate::domain::calendar::rrule::{parse_ics_datetime, unescape_ics};
use crate::ports::calendar_store::CalendarStore;
use chrono::Duration;
use glob::glob;
use std::collections::{HashMap, HashSet};
use std::fs;

pub struct IcsFilesStore {
    globs: Vec<String>,
}

impl IcsFilesStore {
    pub fn new(globs: Vec<String>) -> Self {
        Self { globs }
    }
}

impl CalendarStore for IcsFilesStore {
    fn load_events(&self) -> Vec<RawVEvent> {
        let mut out = Vec::new();
        for pattern in &self.globs {
            let Ok(paths) = glob(pattern) else {
                continue;
            };
            for path in paths.flatten() {
                if let Ok(bytes) = fs::read(&path) {
                    let text = String::from_utf8_lossy(&bytes);
                    out.extend(parse_vcalendar_text(&text));
                }
            }
        }
        out
    }
}

pub fn parse_vcalendar_text(text: &str) -> Vec<RawVEvent> {
    // RFC 5545 line unfolding (CRLF/LF followed by single space or tab)
    let normalized = text.replace("\r\n", "\n");
    let mut unfolded_lines: Vec<String> = Vec::new();
    for raw_line in normalized.lines() {
        if let Some(rest) = raw_line
            .strip_prefix(' ')
            .or_else(|| raw_line.strip_prefix('\t'))
        {
            if let Some(last) = unfolded_lines.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        unfolded_lines.push(raw_line.to_string());
    }

    let mut events = Vec::new();
    let mut current: Option<Vec<String>> = None;

    for line in unfolded_lines {
        let up = line.to_ascii_uppercase();
        if up.starts_with("BEGIN:VEVENT") {
            current = Some(Vec::new());
        } else if up.starts_with("END:VEVENT") {
            if let Some(lines) = current.take() {
                if let Some(ev) = parse_vevent_lines(&lines) {
                    events.push(ev);
                }
            }
        } else if let Some(ref mut lines) = current {
            lines.push(line);
        }
    }

    events
}

fn extract_props<'a>(
    lines: &'a [String],
    prop_name: &str,
) -> Vec<(HashMap<String, String>, &'a str)> {
    let mut found = Vec::new();
    for line in lines {
        let Some((key, val)) = line.split_once(':') else {
            continue;
        };
        let mut parts = key.split(';');
        let base = parts.next().unwrap_or("");
        if !base.eq_ignore_ascii_case(prop_name) {
            continue;
        }
        let mut params = HashMap::new();
        for p in parts {
            if let Some((k, v)) = p.split_once('=') {
                params.insert(k.to_ascii_uppercase(), v.trim_matches('"').to_string());
            }
        }
        found.push((params, val));
    }
    found
}

fn parse_vevent_lines(lines: &[String]) -> Option<RawVEvent> {
    let ds = extract_props(lines, "DTSTART");
    let (ds_params, ds_val) = ds.first()?;
    let (start, all_day) = parse_ics_datetime(ds_params, ds_val)?;

    let de = extract_props(lines, "DTEND");
    let duration = if let Some((de_params, de_val)) = de.first() {
        if let Some((end, _)) = parse_ics_datetime(de_params, de_val) {
            (end - start).max(Duration::zero())
        } else if all_day {
            Duration::days(1)
        } else {
            Duration::zero()
        }
    } else if all_day {
        Duration::days(1)
    } else {
        Duration::zero()
    };

    let summaries = extract_props(lines, "SUMMARY");
    let title = summaries
        .first()
        .map(|(_, v)| unescape_ics(v))
        .unwrap_or_else(|| "(no title)".to_string());

    let rrule = extract_props(lines, "RRULE")
        .first()
        .map(|(_, v)| v.trim().to_string());

    let mut exdates_dt = HashSet::new();
    let mut exdates_date = HashSet::new();
    for (ex_params, ex_val) in extract_props(lines, "EXDATE") {
        for v in ex_val.split(',') {
            if let Some((dt, ad)) = parse_ics_datetime(&ex_params, v) {
                if ad {
                    exdates_date.insert(dt.date());
                } else {
                    exdates_dt.insert(dt);
                }
            }
        }
    }

    Some(RawVEvent {
        title,
        start,
        duration,
        all_day,
        rrule,
        exdates_dt,
        exdates_date,
    })
}
