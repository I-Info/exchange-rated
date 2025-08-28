use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ServerEvent {
    RateUpdate(RateRecord),
    History(Vec<RateRecord>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RateRecord {
    pub rate: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct RateRecordWithLocalTime {
    pub rate: String,
    pub timestamp: DateTime<Local>,
}

impl PartialEq for RateRecordWithLocalTime {
    fn eq(&self, other: &Self) -> bool {
        self.timestamp == other.timestamp
    }
}

impl Eq for RateRecordWithLocalTime {}

impl From<RateRecord> for RateRecordWithLocalTime {
    fn from(record: RateRecord) -> Self {
        RateRecordWithLocalTime {
            rate: record.rate,
            timestamp: record.timestamp.with_timezone(&Local),
        }
    }
}

impl PartialEq for RateRecord {
    fn eq(&self, other: &Self) -> bool {
        self.timestamp == other.timestamp
    }
}

impl Eq for RateRecord {}
