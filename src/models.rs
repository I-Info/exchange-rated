use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ServerEvent {
    #[serde(rename = "rate_update")]
    RateUpdate { record: RateRecord },
    #[serde(rename = "history")]
    History { records: Vec<RateRecord> },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RateRecord {
    pub rate: String,
    pub timestamp: DateTime<Utc>,
}
