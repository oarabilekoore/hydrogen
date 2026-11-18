use chrono::{Duration, NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarEvent {
    pub title: String,
    pub start: String,
    pub end: String,
    #[serde(rename = "allDay")]
    pub all_day: bool,
}

#[derive(Debug, Clone)]
pub struct RawVEvent {
    pub title: String,
    pub start: NaiveDateTime,
    pub duration: Duration,
    pub all_day: bool,
    pub rrule: Option<String>,
    pub exdates_dt: HashSet<NaiveDateTime>,
    pub exdates_date: HashSet<NaiveDate>,
}
