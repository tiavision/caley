use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct Calendar {
    pub name: String,
    pub href: String,
    pub color: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Alarm {
    pub trigger_type: String,     // "start" | "end" | "absolute"
    pub minutes: Option<i64>,     // for relative: negative = before, positive = after
    pub datetime: Option<String>, // for absolute: UTC ISO 8601 (e.g. "20260514T090000Z")
}

#[derive(Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(rename_all = "SCREAMING-KEBAB-CASE")]
pub enum PartStat {
    #[default]
    NeedsAction,
    Accepted,
    Declined,
    Tentative,
}

impl PartStat {
    pub fn from_ical(s: &str) -> Self {
        match s {
            "ACCEPTED" => Self::Accepted,
            "DECLINED" => Self::Declined,
            "TENTATIVE" => Self::Tentative,
            _ => Self::NeedsAction,
        }
    }

    pub fn to_ical(&self) -> &'static str {
        match self {
            Self::NeedsAction => "NEEDS-ACTION",
            Self::Accepted => "ACCEPTED",
            Self::Declined => "DECLINED",
            Self::Tentative => "TENTATIVE",
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Attendee {
    pub cn: String,
    pub email: String,
    pub partstat: PartStat,
}

#[derive(Serialize)]
pub struct Event {
    pub uid: String,
    pub summary: String,
    pub description: String,
    pub location: String,
    pub dtstart: String,
    pub dtend: Option<String>,
    pub date: NaiveDate,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub is_all_day: bool,
    pub tz: String,
    pub is_recurring: bool,
    pub recurrence: Option<RecurrenceInfo>,
    pub master_dtstart: String,
    pub alarms: Vec<Alarm>,
    pub attendees: Vec<Attendee>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RecurrenceInfo {
    pub freq: String,
    pub interval: u32,
    pub until: Option<String>,
    pub count: Option<u32>,
    pub by_day: Vec<String>,
    pub by_month_day: Vec<u32>,
    pub by_month: Vec<u32>,
    pub by_set_pos: Vec<i32>,
}

pub struct SyncResult {
    pub new_token: String,
    pub changed: Vec<(String, String, String)>, // (href, etag, raw_ical)
    pub deleted: Vec<String>,
}
