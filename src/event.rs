use crate::schedule::{EventDraft, Reminder};
use chrono::{Days, Local, NaiveDate};
use serde::Deserialize;

#[derive(Clone, Deserialize)]
pub struct Event {
    pub id: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    #[serde(rename = "colorId")]
    pub color_id: Option<String>,
    pub visibility: Option<String>,
    pub transparency: Option<String>,
    pub reminders: Option<EventReminders>,
    pub start: EventTime,
    pub end: EventTime,
    #[serde(rename = "recurringEventId")]
    pub recurring_event_id: Option<String>,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventReminders {
    pub use_default: bool,
    #[serde(default)]
    pub overrides: Vec<Reminder>,
}

#[derive(Clone, Deserialize)]
pub struct EventTime {
    pub date: Option<NaiveDate>,
    #[serde(rename = "dateTime")]
    pub date_time: Option<chrono::DateTime<chrono::FixedOffset>>,
}

impl Event {
    pub fn draft(&self, date: NaiveDate) -> EventDraft {
        let mut draft = EventDraft::new(date);
        draft.title = self.summary.clone().unwrap_or_default();
        draft.description = self.description.clone().unwrap_or_default();
        draft.location = self.location.clone().unwrap_or_default();
        draft.color_id = self.color_id.clone().unwrap_or_default();
        draft.visibility = self.visibility.clone().unwrap_or_else(|| "default".into());
        draft.busy = self.transparency.as_deref() != Some("transparent");
        if let Some(reminders) = &self.reminders {
            draft.default_reminders = reminders.use_default;
            draft.reminders = reminders.overrides.clone();
        }
        if let (Some(start), Some(end)) = (self.start.date, self.end.date) {
            draft.all_day = true;
            draft.start_date = start.to_string();
            draft.end_date = end
                .checked_sub_days(Days::new(1))
                .unwrap_or(start)
                .to_string();
        } else if let (Some(start), Some(end)) = (self.start.date_time, self.end.date_time) {
            let start = start.with_timezone(&Local);
            let end = end.with_timezone(&Local);
            draft.start_date = start.date_naive().to_string();
            draft.end_date = end.date_naive().to_string();
            draft.start_time = start.time().format("%H:%M:%S%.f").to_string();
            draft.end_time = end.time().format("%H:%M:%S%.f").to_string();
        }
        draft
    }

    pub fn title(&self) -> &str {
        self.summary.as_deref().unwrap_or("(제목 없음)")
    }
    pub fn occurs_on(&self, day: NaiveDate) -> bool {
        if let (Some(start), Some(end)) = (self.start.date, self.end.date) {
            return start <= day && day < end;
        }
        if let (Some(start), Some(end)) = (self.start.date_time, self.end.date_time) {
            let start = start.with_timezone(&chrono::Local);
            let end = end.with_timezone(&chrono::Local);
            return start.date_naive() <= day
                && (day < end.date_naive()
                    || (day == end.date_naive()
                        && (end.time() != chrono::NaiveTime::MIN || start == end)));
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn existing_details_and_multi_day_dates_round_trip() {
        let event: Event = serde_json::from_value(json!({
            "id": "test", "summary": "출장", "description": "준비물\n노트북",
            "location": "서울역", "colorId": "7", "visibility": "private", "transparency": "transparent",
            "start": { "date": "2026-11-11" }, "end": { "date": "2026-11-14" },
            "reminders": { "useDefault": false, "overrides": [
                { "method": "popup", "minutes": 30 }, { "method": "email", "minutes": 1440 }
            ] }
        })).unwrap();
        let draft = event.draft(NaiveDate::from_ymd_opt(2026, 11, 12).unwrap());
        assert!(draft.all_day);
        assert_eq!(draft.end_date, "2026-11-13");
        let payload = draft.payload(true).unwrap();
        assert_eq!(payload["end"]["date"], "2026-11-14");
        assert_eq!(payload["description"], "준비물\n노트북");
        assert_eq!(payload["location"], "서울역");
        assert_eq!(payload["colorId"], "7");
        assert_eq!(payload["visibility"], "private");
        assert_eq!(payload["transparency"], "transparent");
        assert_eq!(
            payload["reminders"]["overrides"].as_array().unwrap().len(),
            2
        );
    }
    #[test]
    fn title_edit_preserves_precise_times_across_local_timezones() {
        let event: Event = serde_json::from_value(json!({
            "id": "test", "summary": "회의",
            "start": { "dateTime": "2026-11-11T00:15:34.123+09:00" },
            "end": { "dateTime": "2026-11-12T02:45:12+09:00" }
        }))
        .unwrap();
        let mut draft = event.draft(NaiveDate::from_ymd_opt(2026, 11, 11).unwrap());
        draft.title = "새 제목".into();
        let payload = draft.payload(true).unwrap();
        for (field, expected) in [
            ("start", event.start.date_time.unwrap()),
            ("end", event.end.date_time.unwrap()),
        ] {
            let actual =
                chrono::DateTime::parse_from_rfc3339(payload[field]["dateTime"].as_str().unwrap())
                    .unwrap();
            assert_eq!(actual, expected);
        }
        assert_eq!(payload["reminders"]["useDefault"], true);
    }
}
