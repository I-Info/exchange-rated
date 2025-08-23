use crate::models::{RateRecord, RateRecordWithLocalTime, ServerEvent};

use dioxus::{logger::tracing, prelude::*};
use futures::StreamExt;
use serde::Serialize;

// const FAVICON: Asset = asset!("/assets/favicon.ico");
const TAILWIND_CSS: Asset = asset!("./assets/tailwind.css");
const APEXCHART_CSS: Asset = asset!("./node_modules/apexcharts/dist/apexcharts.css");
const APEXCHART_JS: Asset = asset!("./node_modules/apexcharts/dist/apexcharts.min.js");
const LODASH_JS: Asset = asset!("./node_modules/lodash/lodash.min.js");

struct RenderedRecord {
    rate: String,
    update_time: String,
}

#[derive(Clone, Copy, PartialEq)]
enum ConnectionStatus {
    Connecting,
    Connected,
    Error,
}

impl ConnectionStatus {
    fn badge_class(&self) -> &'static str {
        match self {
            ConnectionStatus::Connecting => "badge badge-warning",
            ConnectionStatus::Connected => "badge badge-success",
            ConnectionStatus::Error => "badge badge-error",
        }
    }

    fn text(&self) -> &'static str {
        match self {
            ConnectionStatus::Connecting => "Connecting",
            ConnectionStatus::Connected => "Live",
            ConnectionStatus::Error => "Error",
        }
    }
}

impl PartialEq for RenderedRecord {
    fn eq(&self, other: &Self) -> bool {
        self.update_time == other.update_time
    }
}

#[component]
pub fn App() -> Element {
    rsx!(
        document::Link { rel: "stylesheet", href: TAILWIND_CSS }
        document::Link { rel: "stylesheet", href: APEXCHART_CSS }
        script { src: LODASH_JS }
        script { src: APEXCHART_JS }
        main { class: "@container flex min-h-screen flex-col", Main {} }

    )
}

#[derive(Debug, Clone, Serialize)]
struct ChartData {
    x: i64, // timestamp
    y: f64, // rate
}

#[component]
fn Main() -> Element {
    let records = use_server_future(get_rate)?.unwrap()?;

    let mut records_local: Signal<Vec<RateRecordWithLocalTime>> = use_signal(|| {
        records
            .iter()
            .map(|record| RateRecordWithLocalTime::from(record.clone()))
            .collect::<Vec<_>>()
    });

    let mut connection_status = use_signal(|| ConnectionStatus::Connecting);

    #[cfg(feature = "web")]
    use_future(move || async move {
        use gloo_net::eventsource::futures::EventSource;
        use gloo_timers::future::TimeoutFuture;

        let mut retry_count = 0u32;
        const MAX_RETRIES: u32 = 10;

        loop {
            if let Ok(mut es) = EventSource::new("/sse")
                && let Ok(mut stream) = es.subscribe("message")
            {
                connection_status.set(ConnectionStatus::Connected);
                retry_count = 0; // Reset retry count on successful connection
                tracing::info!("SSE connected successfully");

                while let Some(Ok((_type, msg))) = stream.next().await {
                    if let Some(raw) = msg.data().as_string() {
                        match serde_json::from_str::<ServerEvent>(&raw) {
                            Ok(ServerEvent::RateUpdate(record)) => {
                                let mut r = records_local.write();
                                r.push(record.into());
                                tracing::debug!("Rate Update received.");
                            }
                            Ok(ServerEvent::History(records)) => {
                                records_local.set(
                                    records
                                        .into_iter()
                                        .map(|record| RateRecordWithLocalTime::from(record))
                                        .collect::<Vec<_>>(),
                                );
                                tracing::debug!("History received.");
                            }
                            Err(err) => {
                                tracing::error!("Failed to parse server event: {}", err);
                            }
                        }
                    }
                }

                tracing::warn!("Event Source connection closed, attempting reconnect...");
            } else {
                connection_status.set(ConnectionStatus::Error);
                tracing::error!(
                    "Failed to connect to event source, retry {} of {}",
                    retry_count + 1,
                    MAX_RETRIES
                );
            }

            retry_count += 1;
            if retry_count >= MAX_RETRIES {
                tracing::error!("Max reconnection attempts reached, giving up");
                connection_status.set(ConnectionStatus::Error);
                break;
            }

            // Exponential backoff with jitter: base delay of 500ms, max 30s
            let delay_ms = std::cmp::min(500 * 2u32.pow(retry_count), 30000);
            let jitter = (web_sys::js_sys::Math::random() * 0.1 + 0.9) as f64; // 10% jitter
            let final_delay = (delay_ms as f64 * jitter) as u32;
            tracing::info!("Reconnecting in {}ms...", final_delay);
            connection_status.set(ConnectionStatus::Connecting);
            TimeoutFuture::new(final_delay).await;
        }
    });

    let current = use_memo(move || {
        records_local
            .read_unchecked()
            .last()
            .map(|record| RenderedRecord {
                rate: record.rate.clone(),
                update_time: record.timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
            })
    });

    let inner = match &*current.read_unchecked() {
        Some(current) => {
            rsx!(
                document::Title { "AUD/CNY: {current.rate}" }
                div { class: "text-4xl font-bold text-base-content", "{current.rate}" }
                p { class: "text-gray-500", "Date: {current.update_time}" }
            )
        }
        None => {
            rsx!(
                div { class: "text-4xl font-bold text-base-content", "No Rate Available" }
                p { class: "text-gray-500", "Date: N/A" }
            )
        }
    };

    rsx!(
        document::Title { "Rated" }
        div { class: "mx-auto w-full flex-1 px-4 @4xl:max-w-4xl",
            div { class: "card my-4 w-full",
                div { class: "card-body text-center",
                    div { class: "mb-4 flex items-center @sm:grid @sm:grid-cols-[1fr_auto_1fr]",
                        h3 { class: "text-left card-title @sm:col-start-2 @sm:text-center",
                            "Latest Rate"
                        }
                        span { class: "ml-auto @sm:col-start-3 @sm:ml-0 @sm:justify-self-end {connection_status.read().badge_class()}",
                            {connection_status.read().text()}
                        }
                    }
                    {inner}
                }
            }
            Chart { records: records_local }
        }
        footer { class: "flex items-center gap-4 rounded-t-box bg-base-100/80 px-6 py-4 shadow-sm shadow-base-300/20",
            aside { class: "items-center",
                p { class: "flex gap-1 text-base-content/50",
                    a {
                        class: "link font-medium link-hover text-base-content",
                        href: "https://github.com/I-Info/exchange-rated",
                        "Rated "
                    }
                    "©2025"
                }
            }
            p { class: "ml-auto flex gap-1 text-base-content/50",
                span { class: "hidden @lg:inline", "Powered by " }
                a {
                    class: "link hidden @lg:inline link-hover text-base-content",
                    href: "https://dioxuslabs.com",
                    "@Dioxus"
                }
                span { class: "hidden @lg:inline", ", developed by " }
                span { class: "@lg:hidden", "by " }
                a {
                    class: "link inline link-hover text-base-content",
                    href: "https://github.com/I-Info",
                    "@IInfo"
                }
            }
        }
    )
}

#[component]
pub fn Chart(records: Signal<Vec<RateRecordWithLocalTime>>) -> Element {
    let mut chart_element: Signal<Option<Event<MountedData>>> = use_signal(|| None);
    #[cfg(feature = "web")]
    let mut chart_instance: Signal<Option<web_sys::wasm_bindgen::JsValue>> = use_signal(|| None);

    #[cfg(feature = "web")]
    use_effect(move || {
        use dioxus::web::WebEventExt;
        use gloo_utils::format::JsValueSerdeExt;
        use web_sys::js_sys::{Array, Function, Reflect};
        use web_sys::wasm_bindgen::{JsCast, JsValue};

        let el = if let Some(event) = &*chart_element.read_unchecked() {
            event.as_web_event()
        } else {
            return;
        };

        let window = web_sys::window().unwrap();
        let apexcharts = Reflect::get(&window, &JsValue::from_str("ApexCharts")).unwrap();
        let apexcharts_fn: &Function = apexcharts.dyn_ref().unwrap();
        let param_array = Array::new();
        param_array.push(&el);
        param_array.push(
            &JsValue::from_serde(&serde_json::json!({
                "series": [],
                "chart": {
                    "type": "area",
                    "height": 500,
                    "zoom": {
                        "autoScaleYaxis": true
                    }
                },
                "xaxis": {
                    "type": "datetime",
                    "labels": {
                        "datetimeUTC": false,
                        "style": {
                            "colors": "var(--color-base-content)",
                            "fontSize": "13px"
                        }
                    }
                },
                "yaxis": {
                  "labels": {
                    "style": {
                      "colors": "var(--color-base-content)",
                      "fontSize": "13px"
                    }
                  }
                },
                "dataLabels": {
                    "enabled": false
                },
                "stroke": {
                    "curve": "smooth",
                    "width": 2,
                },
                "tooltip": {
                    "x": {
                        "format": "MMM dd HH:mm:ss"
                    },
                },
                "fill": {
                    "type": "gradient",
                    "gradient": {
                      "opacityFrom": 0.9,
                      "opacityTo": 0.1,
                    }
                }
            }))
            .unwrap(),
        );
        let instance = Reflect::construct(apexcharts_fn, &param_array).unwrap();
        tracing::info!("ApexCharts instance created: {:?}", instance);

        let render_fn = Reflect::get(&instance, &JsValue::from_str("render")).unwrap();
        let render_fn: &Function = render_fn.dyn_ref().unwrap();
        render_fn.call0(&instance).unwrap();

        chart_instance.set(Some(instance));
    });

    #[cfg(feature = "web")]
    use_effect(move || {
        use gloo_utils::format::JsValueSerdeExt;
        use web_sys::{
            js_sys::{Function, Reflect},
            wasm_bindgen::{JsCast, JsValue},
        };

        let instance = if let Some(instance) = chart_instance() {
            instance
        } else {
            return;
        };

        let records = records
            .read_unchecked()
            .iter()
            .map(|record| ChartData {
                x: record.timestamp.timestamp_millis(),
                y: record.rate.parse::<f64>().unwrap_or(0.0),
            })
            .collect::<Vec<_>>();

        let update_series_fn = Reflect::get(&instance, &JsValue::from_str("updateSeries")).unwrap();
        let update_series_fn: &Function = update_series_fn.dyn_ref().unwrap();

        update_series_fn
            .call1(
                &instance,
                &JsValue::from_serde(&serde_json::json!([
                    { "data": records, "name": "Rate" }
                ]))
                .unwrap(),
            )
            .unwrap();
    });

    rsx! {
        div {
            id: "chart-container",
            class: "h-[515px]",
            onmounted: move |element| {
                chart_element.set(Some(element));
            },
        }
    }
}

#[server]
async fn get_rate() -> ServerFnResult<Vec<RateRecord>> {
    let FromContext::<crate::server::models::AppState>(state) = extract().await?;

    Ok(state.get_history())
}
