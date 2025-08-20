use super::models::*;
use crate::models::{RateRecord, ServerEvent};

use std::sync::{Arc, LazyLock, RwLock};
use std::time::Duration;
use std::{collections::VecDeque, io::Write};

use chrono::{DateTime, TimeZone, Timelike, Utc};
use chrono_tz::Asia::Shanghai;
use regex::Regex;
use reqwest::header::AUTHORIZATION;
use reqwest::{
    Client,
    header::{HeaderMap, HeaderValue, IF_MODIFIED_SINCE, LAST_MODIFIED},
};
use sqlx::Row;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::sleep;

static URL: &str = "https://www.boc.cn/sourcedb/whpj/mfx_1620.html";

static RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?s)<td class="mc">\s*澳大利亚元\s*</td>\s*<td[^>]*>(.*?)</td>\s*<td[^>]*>(.*?)</td>"#,
    )
    .unwrap()
});

const RETRY: u32 = 3u32;

impl AppState {
    pub fn new(db_pool: sqlx::SqlitePool) -> Self {
        let (broadcast_tx, _rx) = broadcast::channel(100);
        Self {
            broadcast_tx,
            db_pool,
            rate_history: Arc::new(RwLock::new(VecDeque::new())),
            extreme_values: Arc::new(RwLock::new(None)),
        }
    }

    pub fn clean(&self) {
        let mut history = self.rate_history.write().unwrap();
        history.retain(|record| record.timestamp > Utc::now() - chrono::Duration::days(5));
    }

    pub fn add_rate(&self, record: &RateRecord) -> bool {
        {
            let history = self.rate_history.read().unwrap();

            if let Some(last_record) = history.back() {
                if last_record.timestamp == record.timestamp {
                    let mut stdout = std::io::stdout();
                    stdout.write_all(b"+").unwrap();
                    stdout.flush().unwrap();
                    return false;
                }
            }
        }

        println!(
            "+\n[{}] New Rate: {}",
            record.timestamp.format("%Y-%m-%d %H:%M:%S"),
            record.rate
        );

        {
            let mut history = self.rate_history.write().unwrap();
            history.push_back(record.clone());
        };

        // Broadcast the update to all connected WebSocket clients
        let message = ServerEvent::RateUpdate {
            record: record.clone(),
        };
        let _ = self.broadcast_tx.send(message);

        true
    }

    pub fn get_latest_rate(&self) -> Option<RateRecord> {
        let history = self.rate_history.read().unwrap();
        history.back().cloned()
    }

    pub fn get_history(&self) -> Vec<RateRecord> {
        let history = self.rate_history.read().unwrap();
        history.iter().rev().cloned().collect()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ServerEvent> {
        self.broadcast_tx.subscribe()
    }

    pub async fn load_from_db(&self) -> Result<(), sqlx::Error> {
        let five_days_ago = Utc::now() - chrono::Duration::days(5);

        let rows = sqlx::query(
            "SELECT rate, timestamp FROM rate_records WHERE timestamp > $1 ORDER BY timestamp DESC",
        )
        .bind(five_days_ago)
        .fetch_all(&self.db_pool)
        .await?;

        let mut history = self.rate_history.write().unwrap();
        history.clear();

        for row in rows.into_iter().rev() {
            let rate: String = row.get("rate");
            let timestamp: DateTime<Utc> = row.get("timestamp");

            history.push_back(RateRecord { rate, timestamp });
        }

        println!("Loaded {} records from database", history.len());
        Ok(())
    }

    pub async fn get_history_extremes(&self) -> Result<Option<ExtremeValues>, sqlx::Error> {
        let five_days_ago = Utc::now() - chrono::Duration::days(5);

        if let Some(extreme_values) = &*self.extreme_values.read().unwrap() {
            if extreme_values.high_timestamp >= five_days_ago
                && extreme_values.low_timestamp >= five_days_ago
            {
                return Ok(Some(extreme_values.clone()));
            }
        }

        let rows = sqlx::query(
            "SELECT rate, timestamp FROM rate_records WHERE timestamp >= ? ORDER BY timestamp;",
        )
        .bind(five_days_ago)
        .fetch_all(&self.db_pool)
        .await?;

        if rows.is_empty() {
            return Ok(None);
        }

        let mut high_rate = 0.0f64;
        let mut low_rate = f64::MAX;
        let mut high_timestamp = five_days_ago;
        let mut low_timestamp = five_days_ago;

        for row in rows {
            let rate_str: String = row.get("rate");
            let timestamp: DateTime<Utc> = row.get("timestamp");

            if let Ok(rate) = rate_str.parse::<f64>() {
                if rate > high_rate {
                    high_rate = rate;
                    high_timestamp = timestamp;
                }
                if rate < low_rate {
                    low_rate = rate;
                    low_timestamp = timestamp;
                }
            }
        }

        if low_rate == f64::MAX {
            return Ok(None);
        }

        let values = Some(ExtremeValues {
            high_rate,
            low_rate,
            high_timestamp,
            low_timestamp,
        });

        {
            let mut cache = self.extreme_values.write().unwrap();
            *cache = values.clone();
        }

        println!("*\nLoaded history extremes: {values:?}");
        Ok(values)
    }

    pub async fn check_extremes(
        &self,
        current_record: &RateRecord,
    ) -> Result<Option<Extreme>, Box<dyn std::error::Error + Send + Sync>> {
        let current_rate = current_record.rate.parse::<f64>()?;

        let extremes = match self.get_history_extremes().await? {
            Some(extremes) => extremes,
            None => return Ok(None),
        };

        if current_rate < extremes.low_rate {
            let mut cache = self.extreme_values.write().unwrap();
            *cache = Some(ExtremeValues {
                low_rate: current_rate,
                low_timestamp: current_record.timestamp,
                ..extremes
            });
            return Ok(Some(Extreme::Low(
                extremes.low_rate,
                current_record.timestamp,
            )));
        } else if current_rate > extremes.high_rate {
            let mut cache = self.extreme_values.write().unwrap();
            *cache = Some(ExtremeValues {
                high_rate: current_rate,
                high_timestamp: current_record.timestamp,
                ..extremes
            });
            return Ok(Some(Extreme::High(
                extremes.high_rate,
                current_record.timestamp,
            )));
        }

        Ok(None)
    }
}

pub struct Ntfy {
    client: Client,
    config: NtfyConfig,
    last_sent: Arc<RwLock<Option<DateTime<Utc>>>>,
}

impl Ntfy {
    pub fn new(client: Client, config: NtfyConfig) -> Self {
        Self {
            client,
            config,
            last_sent: Arc::new(RwLock::new(None)),
        }
    }

    pub fn send(
        &self,
        current_record: &RateRecord,
        pre_extreme: Extreme,
    ) -> Option<JoinHandle<()>> {
        // Skip alerts during non-trading hours (Beijing time 9:30-23:00)
        let beijing_time = Shanghai.from_utc_datetime(&current_record.timestamp.naive_utc());
        let hour = beijing_time.hour();
        let minute = beijing_time.minute();
        if !((10..23).contains(&hour) || (hour == 9 && minute >= 30)) {
            return None;
        }

        let now = Utc::now();
        let should_alert = {
            match *self.last_sent.read().unwrap() {
                None => true,
                Some(last) => now.signed_duration_since(last).num_minutes() >= 20,
            }
        };

        if !should_alert {
            return None;
        }

        let (dropping, pre, pre_timestamp) = match pre_extreme {
            Extreme::High(pre, pre_timestamp) => (false, pre, pre_timestamp),
            Extreme::Low(pre, pre_timestamp) => (true, pre, pre_timestamp),
        };
        let subject = format!(
            "澳元汇率5日新{}: {}",
            if dropping { "低" } else { "高" },
            current_record.rate
        );
        let body = format!(
            "当前汇率: {}\n时间: {}\n\n历史: {} ({})",
            current_record.rate,
            current_record
                .timestamp
                .with_timezone(&Shanghai)
                .format("%Y-%m-%d %H:%M:%S"),
            pre,
            pre_timestamp
                .with_timezone(&Shanghai)
                .format("%Y-%m-%d %H:%M:%S")
        );

        let handle = tokio::spawn(ntfy_send(
            self.client.clone(),
            self.config.url.clone(),
            subject,
            body,
            self.config.auth.clone(),
        ));

        {
            *self.last_sent.write().unwrap() = Some(now);
        }

        Some(handle)
    }
}

pub async fn ntfy_send(
    client: Client,
    url: String,
    subject: String,
    body: String,
    auth: Option<String>,
) {
    let mut builder = client
        .post(url)
        .header("X-Title", subject)
        .body::<String>(body);

    if let Some(auth) = auth {
        builder = builder.header(AUTHORIZATION, auth);
    }

    match builder.send().await {
        Ok(response) => {
            if response.status().is_success() {
                println!("*\nNtfy sent successfully");
            } else {
                eprintln!("Ntfy failed with status: {}", response.status());
            }
        }
        Err(err) => {
            eprintln!("Ntfy failed with error: {err}");
        }
    }
}

pub async fn persist_record(db_pool: sqlx::SqlitePool, record: RateRecord) {
    for r in 0..RETRY {
        match sqlx::query("INSERT OR IGNORE INTO rate_records (rate, timestamp) VALUES (?, ?);")
            .bind(&record.rate)
            .bind(record.timestamp)
            .execute(&db_pool)
            .await
        {
            Ok(_) => return,
            Err(e) => {
                eprintln!("Warning: Failed to persist rate record: {e} ");
                if r == RETRY - 1 {
                    eprintln!("Failed to persist rate record after {RETRY} retries: {e}");
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

pub async fn fetch_sell_rate(
    client: &Client,
    cache_info: &mut Option<CacheInfo>,
) -> Option<RateRecord> {
    let mut headers = HeaderMap::new();
    // 添加缓存头部
    if let Some(cache) = cache_info {
        // println!("使用缓存：{:?}", &cache);
        // if let Ok(header_value) = HeaderValue::from_str(&cache.etag) {
        //     headers.insert(IF_NONE_MATCH, header_value);
        // } else {
        //     eprintln!("*\n警告: ETag 格式无效: {}", &cache.etag);
        // }
        if let Ok(header_value) = HeaderValue::from_str(&cache.last_modified) {
            headers.insert(IF_MODIFIED_SINCE, header_value);
        } else {
            eprintln!("*\n警告: Last-Modified 格式无效: {}", &cache.last_modified);
        }
    }

    let response = match client.get(URL).headers(headers).send().await {
        Ok(resp) => resp,
        Err(e) => {
            eprintln!("*\nHTTP 请求失败: {e:?}");
            return None;
        }
    };

    let status = response.status().as_u16();
    // println!("HTTP 响应状态: {}", status);

    // 检查是否返回304 Not Modified，跳过
    if status == 304 {
        let mut stdout = std::io::stdout();
        stdout.write_all(b".").unwrap();
        stdout.flush().unwrap();
        return None;
    }

    // 更新缓存信息
    // let etag = response
    //     .headers()
    //     .get(ETAG)
    //     .and_then(|v| v.to_str().ok())
    //     .map(|s| s.to_string());
    let last_modified = response
        .headers()
        .get(LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let html = match response.text().await {
        Ok(content) => content,
        Err(e) => {
            eprintln!("*\n读取响应内容失败: {e}");
            return None;
        }
    };

    // 匹配 "澳大利亚元" 行后的两个 <td>，第2个是卖出价
    if let Some(caps) = &RE.captures(&html) {
        let rate = caps.get(2)?.as_str().trim().to_string();
        // println!("成功提取汇率: {}", rate);
        // 更新缓存
        match &last_modified {
            Some(last_modified) => {
                *cache_info = Some(CacheInfo {
                    // etag: etag,
                    last_modified: last_modified.clone(),
                });
                // println!("{:?}", cache_info);
            }
            _ => {
                eprintln!("*\n错误: 缺少必要的缓存信息");
            }
        }
        match last_modified {
            Some(last_modified) => match DateTime::parse_from_rfc2822(&last_modified) {
                Ok(timestamp) => Some(RateRecord {
                    rate,
                    timestamp: timestamp.to_utc(),
                }),
                Err(_) => {
                    eprintln!("*\n错误: 无法解析最后修改时间，使用当前时间");
                    Some(RateRecord {
                        rate,
                        timestamp: Utc::now(),
                    })
                }
            },
            None => {
                eprintln!("*\n错误: 缺少最后修改时间信息，使用当前时间");
                Some(RateRecord {
                    rate,
                    timestamp: Utc::now(),
                })
            }
        }
    } else {
        eprintln!("*\n错误: 在页面内容中找不到澳大利亚元汇率信息");
        None
    }
}

pub async fn rate_fetcher(state: AppState, ntfy: Option<Ntfy>) {
    let client = Client::builder()
        .user_agent(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:141.0) Gecko/20100101 Firefox/141.0",
        )
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let mut cache_info: Option<CacheInfo> = None;

    loop {
        if let Some(current_rate) = fetch_sell_rate(&client, &mut cache_info).await {
            if state.add_rate(&current_rate) {
                // Check for extreme value alerts
                match (state.check_extremes(&current_rate).await, &ntfy) {
                    (Ok(Some(old)), Some(ntfy)) => {
                        _ = ntfy.send(&current_rate, old);
                    }
                    (Err(e), _) => eprintln!("X Failed to check extremes: {e}"),
                    _ => (),
                };

                tokio::spawn(persist_record(state.db_pool.clone(), current_rate));
            }
        }
        sleep(Duration::from_millis(rand::random_range(1000..5000))).await;
    }
}

pub async fn cleaner(state: AppState) {
    let mut interval = tokio::time::interval(Duration::from_secs(3600));
    // Ignore the first tick, which is triggered immediately after the interval is created
    interval.tick().await;
    loop {
        interval.tick().await;
        println!("*\nCleaner triggered.");
        state.clean();
    }
}

pub fn prepare_rate_display(history: &[RateRecord]) -> Vec<RateDisplay> {
    history
        .iter()
        .enumerate()
        .map(|(index, record)| {
            let (change_text, change_indicator) = if index < history.len() - 1 {
                let previous_rate: f64 = history[index + 1].rate.parse().unwrap_or(0.0);
                let current_rate: f64 = record.rate.parse().unwrap_or(0.0);
                let change = current_rate - previous_rate;

                let indicator = if change > 0.0 {
                    "<span class='trend-up'>↗</span>".to_string()
                } else if change < 0.0 {
                    "<span class='trend-down'>↘</span>".to_string()
                } else {
                    "<span class='trend-flat'>→</span>".to_string()
                };

                let change_text = if change > 0.0 {
                    format!("<span class='change-up'>+{change:.2}</span>")
                } else if change < 0.0 {
                    format!("<span class='change-down'>{change:.2}</span>")
                } else {
                    "<span class='change-flat'>0.00</span>".to_string()
                };

                (change_text, indicator)
            } else {
                (String::new(), String::new())
            };

            RateDisplay {
                rate: record.rate.clone(),
                timestamp: record.timestamp,
                change_text,
                change_indicator,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::services::Ntfy;
    use crate::server::utils::load_ntfy_config;
    use chrono::Utc;
    use dotenv::dotenv;
    use std::time::Duration;

    #[tokio::test]
    async fn test_ntfy() {
        dotenv().ok();
        let ntfy_config = load_ntfy_config();

        let ntfy = if let Some(config) = ntfy_config {
            // Initialize Ntfy client with the provided configuration
            let mut client_builder =
                reqwest::Client::builder().timeout(std::time::Duration::from_secs(10));
            if let Some(auth) = &config.auth {
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert(
                    reqwest::header::AUTHORIZATION,
                    reqwest::header::HeaderValue::from_str(auth).unwrap(),
                );
                client_builder = client_builder.default_headers(headers);
            }
            let client = client_builder.build().unwrap();
            Some(Ntfy::new(client, config))
        } else {
            None
        };

        if let Some(ntfy) = ntfy {
            if let Some(handle) = ntfy.send(
                &RateRecord {
                    rate: "455.50".into(),
                    timestamp: Utc::now(),
                },
                Extreme::High(454.1, Utc::now() - Duration::from_secs(3600)),
            ) {
                handle.await.unwrap();
            }
        }
    }
}
