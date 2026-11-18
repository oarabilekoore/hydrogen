use crate::domain::calendar::model::RawVEvent;

pub trait CalendarStore {
    fn load_events(&self) -> Vec<RawVEvent>;
}
