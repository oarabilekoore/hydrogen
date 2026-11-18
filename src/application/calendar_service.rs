use crate::domain::calendar::ics_builder::build_single_event_ics;
use crate::domain::calendar::model::CalendarEvent;
use crate::domain::calendar::rrule::expand_occurrences;
use crate::error::{HydrogenError, Result};
use crate::ports::calendar_store::CalendarStore;
use crate::ports::desktop::DesktopEnvironment;
use chrono::{Duration, Local, NaiveDate, NaiveDateTime};
use std::collections::HashSet;
use std::env;
use std::fs;

pub struct CalendarService<'a> {
    stores: Vec<&'a dyn CalendarStore>,
    desktop: &'a dyn DesktopEnvironment,
    default_duration_mins: i64,
}

impl<'a> CalendarService<'a> {
    pub fn new(
        stores: Vec<&'a dyn CalendarStore>,
        desktop: &'a dyn DesktopEnvironment,
        default_duration_mins: i64,
    ) -> Self {
        Self {
            stores,
            desktop,
            default_duration_mins,
        }
    }

    pub fn list_upcoming(&self, from_date: Option<&str>, days: i64) -> Result<Vec<CalendarEvent>> {
        let start_day: NaiveDateTime = match from_date {
            Some(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .map_err(|e| HydrogenError::InvalidInput(format!("Invalid --from date '{s}': {e}")))?
                .and_hms_opt(0, 0, 0)
                .unwrap(),
            None => Local::now().date_naive().and_hms_opt(0, 0, 0).unwrap(),
        };
        let win_end = start_day + Duration::days(days.max(1));

        let mut seen: HashSet<(String, NaiveDateTime)> = HashSet::new();
        let mut result: Vec<CalendarEvent> = Vec::new();

        for store in &self.stores {
            for ev in store.load_events() {
                for occ in expand_occurrences(&ev, start_day, win_end) {
                    let key = (ev.title.clone(), occ);
                    if !seen.insert(key) {
                        continue;
                    }
                    result.push(CalendarEvent {
                        title: ev.title.clone(),
                        start: occ.format("%Y-%m-%dT%H:%M:%S").to_string(),
                        end: (occ + ev.duration).format("%Y-%m-%dT%H:%M:%S").to_string(),
                        all_day: ev.all_day,
                    });
                }
            }
        }

        result.sort_by(|a, b| a.start.cmp(&b.start));
        result.truncate(400);
        Ok(result)
    }

    pub fn add_event(&self, date: &str, time: &str, title: &str) -> Result<()> {
        let (short_uid, ics_content) =
            build_single_event_ics(date, time, title, self.default_duration_mins)?;
        let path = env::temp_dir().join(format!("qs-event-{short_uid}.ics"));
        fs::write(&path, ics_content)?;
        self.desktop.open_file_detached(&path)
    }

    pub fn open_calendar(&self, date: Option<&str>) -> Result<()> {
        self.desktop.open_default_calendar(date)
    }
}
