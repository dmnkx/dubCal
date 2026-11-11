use anyhow::{bail, Context, Result};
use chrono::{Days, Local, NaiveDate, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reminder {
    pub method: String,
    pub minutes: u32,
}

pub const EVENT_COLORS: [(&str, &str, u32); 12] = [
    ("", "캘린더 기본색", 0x888888),
    ("1", "라벤더", 0x7986cb),
    ("2", "세이지", 0x33b679),
    ("3", "포도", 0x8e24aa),
    ("4", "플라밍고", 0xe67c73),
    ("5", "바나나", 0xf6c026),
    ("6", "귤", 0xf5511d),
    ("7", "피콕", 0x039be5),
    ("8", "그래파이트", 0x616161),
    ("9", "블루베리", 0x3f51b5),
    ("10", "바질", 0x0b8043),
    ("11", "토마토", 0xd50000),
];

pub struct EventDraft {
    pub title: String,
    pub all_day: bool,
    pub start_date: String,
    pub end_date: String,
    pub start_time: String,
    pub end_time: String,
    pub description: String,
    pub location: String,
    pub color_id: String,
    pub visibility: String,
    pub busy: bool,
    pub default_reminders: bool,
    pub reminders: Vec<Reminder>,
}

impl EventDraft {
    pub fn new(date: NaiveDate) -> Self {
        Self {
            title: String::new(),
            all_day: false,
            start_date: date.to_string(),
            end_date: date.to_string(),
            start_time: "09:00".into(),
            end_time: "10:00".into(),
            description: String::new(),
            location: String::new(),
            color_id: String::new(),
            visibility: "default".into(),
            busy: true,
            default_reminders: true,
            reminders: Vec::new(),
        }
    }

    pub fn payload(&self, editing: bool) -> Result<Value> {
        self.payload_in(editing, &Local)
    }

    fn payload_in<Tz: TimeZone>(&self, editing: bool, timezone: &Tz) -> Result<Value> {
        let title = self.title.trim();
        if title.is_empty() {
            bail!("일정 제목을 입력해 주세요");
        }
        let start_date = NaiveDate::parse_from_str(&self.start_date, "%Y-%m-%d")
            .context("시작 날짜를 YYYY-MM-DD 형식으로 입력해 주세요")?;
        let end_date = NaiveDate::parse_from_str(&self.end_date, "%Y-%m-%d")
            .context("종료 날짜를 YYYY-MM-DD 형식으로 입력해 주세요")?;
        let (mut start, mut end) = if self.all_day {
            if end_date < start_date {
                bail!("종료 날짜는 시작 날짜보다 빠를 수 없습니다");
            }
            let exclusive_end = end_date
                .checked_add_days(Days::new(1))
                .context("종료 날짜가 범위를 벗어났습니다")?;
            (
                json!({ "date": start_date.to_string() }),
                json!({ "date": exclusive_end.to_string() }),
            )
        } else {
            let parse_time = |value: &str| {
                NaiveTime::parse_from_str(value, "%H:%M")
                    .or_else(|_| NaiveTime::parse_from_str(value, "%H:%M:%S%.f"))
            };
            let start_time = parse_time(&self.start_time)
                .context("시작 시간을 24시간제 HH:MM 형식으로 입력해 주세요")?;
            let end_time = parse_time(&self.end_time)
                .context("종료 시간을 24시간제 HH:MM 형식으로 입력해 주세요")?;
            let start = timezone
                .from_local_datetime(&start_date.and_time(start_time))
                .single()
                .context("시작 시간이 시간대 변경과 겹칩니다. 다른 시간을 선택해 주세요")?;
            let end = timezone
                .from_local_datetime(&end_date.and_time(end_time))
                .single()
                .context("종료 시간이 시간대 변경과 겹칩니다. 다른 시간을 선택해 주세요")?;
            if end <= start {
                bail!("종료 일시는 시작 일시보다 늦어야 합니다");
            }
            (
                json!({ "dateTime": start.to_rfc3339() }),
                json!({ "dateTime": end.to_rfc3339() }),
            )
        };
        if editing {
            // PATCH merges nested fields. Remove the previous representation when
            // switching between all-day and timed events; keep unrelated fields.
            let old_field = if self.all_day { "dateTime" } else { "date" };
            start[old_field] = Value::Null;
            end[old_field] = Value::Null;
            start["timeZone"] = Value::Null;
            end["timeZone"] = Value::Null;
        }
        if !["default", "public", "private", "confidential"].contains(&self.visibility.as_str()) {
            bail!("공개 범위를 선택해 주세요");
        }
        if !EVENT_COLORS.iter().any(|color| color.0 == self.color_id) {
            bail!("일정 색상을 다시 선택해 주세요");
        }
        if !self.default_reminders {
            if self.reminders.len() > 5 {
                bail!("알림은 최대 5개까지 설정할 수 있습니다");
            }
            for reminder in &self.reminders {
                if !["popup", "email"].contains(&reminder.method.as_str())
                    || reminder.minutes > 40320
                {
                    bail!("알림은 팝업/이메일 방식으로 최대 4주 전까지 설정할 수 있습니다");
                }
            }
        }
        let reminders = if self.default_reminders {
            json!({ "useDefault": true, "overrides": [] })
        } else {
            json!({ "useDefault": false, "overrides": self.reminders })
        };
        let mut payload = json!({ "summary": title, "start": start, "end": end,
            "description": self.description, "location": self.location.trim(),
            "visibility": self.visibility, "transparency": if self.busy { "opaque" } else { "transparent" },
            "reminders": reminders,
        });
        if !self.color_id.is_empty() {
            payload["colorId"] = json!(self.color_id);
        } else if editing {
            payload["colorId"] = Value::Null;
        }
        Ok(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn draft() -> EventDraft {
        let mut draft = EventDraft::new(NaiveDate::from_ymd_opt(2026, 11, 11).unwrap());
        draft.title = "  회의  ".into();
        draft
    }
    #[test]
    fn timed_event_has_local_offset_and_end_time() {
        let value = draft()
            .payload_in(false, &chrono::FixedOffset::east_opt(9 * 3600).unwrap())
            .unwrap();
        assert_eq!(value["summary"], "회의");
        assert_eq!(value["start"]["dateTime"], "2026-11-11T09:00:00+09:00");
        assert_eq!(value["end"]["dateTime"], "2026-11-11T10:00:00+09:00");
        assert!(value["start"].get("date").is_none());
    }
    #[test]
    fn overnight_and_multi_day_events_are_valid() {
        let mut draft = draft();
        draft.start_time = "23:30".into();
        draft.end_date = "2026-11-12".into();
        draft.end_time = "00:30".into();
        let value = draft.payload_in(false, &chrono::Utc).unwrap();
        assert_eq!(value["end"]["dateTime"], "2026-11-12T00:30:00+00:00");
        draft.all_day = true;
        let value = draft.payload(false).unwrap();
        assert_eq!(value["start"]["date"], "2026-11-11");
        assert_eq!(value["end"]["date"], "2026-11-13");
    }
    #[test]
    fn edits_remove_the_old_time_representation() {
        let mut draft = draft();
        let value = draft.payload(true).unwrap();
        assert_eq!(value["start"].get("date"), Some(&Value::Null));
        assert!(value["start"]["dateTime"].is_string());
        draft.all_day = true;
        let value = draft.payload(true).unwrap();
        assert_eq!(value["start"].get("dateTime"), Some(&Value::Null));
        assert_eq!(value["end"]["date"], "2026-11-12");
    }
    #[test]
    fn details_and_multiple_reminders_are_saved() {
        let mut draft = draft();
        draft.description = "회의 자료\nhttps://example.com/agenda".into();
        draft.location = "  3층 회의실  ".into();
        draft.color_id = "9".into();
        draft.visibility = "private".into();
        draft.busy = false;
        draft.default_reminders = false;
        draft.reminders = vec![
            Reminder {
                method: "popup".into(),
                minutes: 10,
            },
            Reminder {
                method: "email".into(),
                minutes: 1440,
            },
        ];
        let value = draft.payload(true).unwrap();
        assert_eq!(value["description"], draft.description);
        assert_eq!(value["location"], "3층 회의실");
        assert_eq!(value["colorId"], "9");
        assert_eq!(value["visibility"], "private");
        assert_eq!(value["transparency"], "transparent");
        assert_eq!(value["reminders"]["useDefault"], false);
        assert_eq!(value["reminders"]["overrides"][1]["minutes"], 1440);
        draft.color_id.clear();
        draft.reminders.clear();
        let value = draft.payload(true).unwrap();
        assert_eq!(value.get("colorId"), Some(&Value::Null));
        assert_eq!(value["reminders"]["overrides"], json!([]));
    }
    #[test]
    fn reminder_limits_are_validated_and_defaults_are_supported() {
        let mut draft = draft();
        assert_eq!(
            draft.payload(false).unwrap()["reminders"]["useDefault"],
            true
        );
        draft.default_reminders = false;
        draft.reminders = vec![Reminder {
            method: "popup".into(),
            minutes: 40321,
        }];
        assert!(draft.payload(false).is_err());
        draft.reminders[0].minutes = 40320;
        assert!(draft.payload(false).is_ok());
        draft.reminders = vec![
            Reminder {
                method: "popup".into(),
                minutes: 10
            };
            6
        ];
        assert!(draft.payload(false).is_err());
    }

    #[test]
    fn invalid_dates_times_and_reversed_ranges_are_rejected() {
        let mut draft = draft();
        draft.end_time = "08:00".into();
        assert!(draft.payload(false).is_err());
        draft.end_time = "09:00".into();
        assert!(draft.payload(false).is_err());
        draft.end_time = "25:00".into();
        assert!(draft.payload(false).is_err());
        draft.all_day = true;
        draft.end_date = "2026-02-30".into();
        assert!(draft.payload(false).is_err());
        draft.end_date = "2026-11-10".into();
        assert!(draft.payload(false).is_err());
        draft.end_date = draft.start_date.clone();
        draft.title.clear();
        assert!(draft.payload(false).is_err());
    }
}
