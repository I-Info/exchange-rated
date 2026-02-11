use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ServerEvent {
    RateUpdate(RateRecord),
    CibRateUpdate(CibRateRecord),
    History(Vec<RateRecord>),
    CibHistory(Vec<CibRateRecord>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RateRecord {
    pub rate: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CibRateRecord {
    pub rate: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candle {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub start_timestamp: DateTime<Utc>,
    pub end_timestamp: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IndicatorPoint {
    pub timestamp: DateTime<Utc>,
    pub value: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MacdRsiSeries {
    pub macd: Vec<IndicatorPoint>,
    pub signal: Vec<IndicatorPoint>,
    pub histogram: Vec<IndicatorPoint>,
    pub rsi: Vec<IndicatorPoint>,
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
