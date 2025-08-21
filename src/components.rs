use crate::models::{RateRecord, RateRecordWithLocalTime, ServerEvent};

use dioxus::{logger::tracing, prelude::*};
use futures::StreamExt;

#[cfg(feature = "web")]
use gloo_net::eventsource::futures::EventSource;

// const FAVICON: Asset = asset!("/assets/favicon.ico");
const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

struct RenderedRecord {
    rate: String,
    update_time: String,
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
        main { class: "@container", Main {} }
    )
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

    #[cfg(feature = "web")]
    use_future(move || {
        records_local.set(
            records
                .iter()
                .map(|record| RateRecordWithLocalTime::from(record.clone()))
                .collect::<Vec<_>>(),
        );

        async move {
            if let Ok(mut es) = EventSource::new("/sse")
                && let Ok(mut stream) = es.subscribe("message")
            {
                while let Some(msg) = stream.next().await {
                    match msg {
                        Ok((_type, msg)) => {
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
                                                .iter()
                                                .map(|record| {
                                                    RateRecordWithLocalTime::from(record.clone())
                                                })
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
                        Err(err) => {
                            tracing::error!("Failed to connect to event source: {}", err);
                        }
                    }
                }
                tracing::warn!("Event Source Closed.")
            } else {
                tracing::error!("Failed to connect to event source.")
            }
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
                document::Title { "Rate: {current.rate}" }
                div { class: "card my-4",
                    div { class: "card-body text-center",
                        h3 { class: "card-title", "Latest Rate" }
                        div { class: "text-4xl font-bold text-base-content", "{current.rate}" }
                        p { class: "text-gray-500", "Date: {current.update_time}" }
                    }
                }
            )
        }
        None => {
            rsx!(
                document::Title { "Rated" }
                div { class: "card my-4",
                    div { class: "card-body text-center",
                        h3 { class: "card-title", "Latest Rate" }
                        div { class: "text-4xl font-bold text-base-content", "No Rate Available" }
                        p { class: "text-gray-500", "Date" }
                    }
                }
            )
        }
    };

    rsx!(
        div { class: "mx-auto max-w-full px-4 @4xl:max-w-4xl", {inner} }
    )
}

#[server]
async fn get_rate() -> ServerFnResult<Vec<RateRecord>> {
    let FromContext::<crate::server::models::AppState>(state) = extract().await?;

    Ok(state.get_history())
}
