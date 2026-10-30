use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use async_trait::async_trait;
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::{Value as Json, json};
use tokio::{task::JoinHandle, time::Instant};
use types::Watermark;

use crate::{Error, Pause, PauseServer};

const ROUND_DELAY: Duration = Duration::from_millis(500);

/// Finds the leader among the replicas, retrying `503`s and connection failures for `retry_for`.
pub struct HttpPauseServer {
    endpoints: Arc<Endpoints>,
    /// Each held token's renewal, stopped by `resume`.
    renewals: Mutex<BTreeMap<String, JoinHandle<()>>>,
}

struct Endpoints {
    client: reqwest::Client,
    urls: Vec<String>,
    retry_for: Duration,
}

#[derive(Deserialize)]
struct PauseResponse {
    token: String,
    ttl_seconds: u64,
    watermark: Vec<PipelineOffsets>,
}

#[derive(Deserialize)]
struct PipelineOffsets {
    offsets: Vec<KafkaOffset>,
}

#[derive(Deserialize)]
struct KafkaOffset {
    partition: KafkaPartition,
    offset: KafkaOffsetValue,
}

#[derive(Deserialize)]
struct KafkaPartition {
    kafka_topic: String,
    kafka_partition: i32,
}

#[derive(Deserialize)]
struct KafkaOffsetValue {
    kafka_offset: i64,
}

impl Endpoints {
    /// The leader's answer, or an error once no replica answered within `retry_for`.
    async fn post(&self, path: &str, body: &Json) -> Result<(StatusCode, String), Error> {
        let deadline = Instant::now() + self.retry_for;
        loop {
            let mut last = String::from("no pause server URLs");
            for url in &self.urls {
                let url = format!("{}/{path}", url.trim_end_matches('/'));
                match self.client.post(&url).json(body).send().await {
                    Ok(response) if response.status() != StatusCode::SERVICE_UNAVAILABLE => {
                        let status = response.status();
                        let text = response
                            .text()
                            .await
                            .map_err(|e| Error::Pause(e.to_string()))?;
                        return Ok((status, text));
                    }
                    Ok(response) => last = format!("{url}: {}", response.status()),
                    Err(err) => last = format!("{url}: {err}"),
                }
            }
            if Instant::now() >= deadline {
                return Err(Error::Pause(format!(
                    "no leader answered within {:?}; last: {last}",
                    self.retry_for
                )));
            }
            tokio::time::sleep(ROUND_DELAY).await;
        }
    }
}

impl HttpPauseServer {
    pub fn new(config: &config::PauseConfig) -> Self {
        Self {
            endpoints: Arc::new(Endpoints {
                client: reqwest::Client::new(),
                urls: config.urls.clone(),
                retry_for: config.retry_for,
            }),
            renewals: Mutex::default(),
        }
    }

    /// Renews the token every third of its ttl until stopped, so a slow clone keeps its pause.
    fn renew(&self, token: &str, ttl: Duration) {
        let (endpoints, body) = (self.endpoints.clone(), json!({ "token": token }));
        let every = (ttl / 3).max(Duration::from_secs(1));
        let task = tokio::spawn(async move {
            loop {
                tokio::time::sleep(every).await;
                match endpoints.post("renew", &body).await {
                    Ok((status, _)) if status.is_success() => {}
                    _ => break,
                }
            }
        });
        let previous = self
            .renewals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(token.to_owned(), task);
        if let Some(previous) = previous {
            previous.abort();
        }
    }
}

impl Drop for HttpPauseServer {
    fn drop(&mut self) {
        let renewals = self
            .renewals
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner);
        for task in renewals.values() {
            task.abort();
        }
    }
}

#[async_trait]
impl PauseServer for HttpPauseServer {
    async fn pause(&self, table: &str) -> Result<Pause, Error> {
        let (status, body) = self
            .endpoints
            .post("pause", &json!({ "table": table }))
            .await?;
        match status {
            StatusCode::NOT_FOUND => return Err(Error::UnknownTable(table.to_owned())),
            status if !status.is_success() => {
                return Err(Error::Pause(format!("{status}: {}", body.trim())));
            }
            _ => {}
        }
        let response: PauseResponse = serde_json::from_str(&body)
            .map_err(|err| Error::Pause(format!("unexpected /pause response: {err}")))?;
        let mut watermark = Watermark::new();
        for offset in response.watermark.iter().flat_map(|p| &p.offsets) {
            watermark.set(
                &offset.partition.kafka_topic,
                offset.partition.kafka_partition,
                offset.offset.kafka_offset,
            );
        }
        self.renew(&response.token, Duration::from_secs(response.ttl_seconds));
        Ok(Pause {
            token: response.token,
            watermark,
        })
    }

    async fn resume(&self, pause: &Pause) -> Result<(), Error> {
        let renewal = self
            .renewals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&pause.token);
        if let Some(renewal) = renewal {
            renewal.abort();
        }
        let (status, body) = self
            .endpoints
            .post("resume", &json!({ "token": pause.token }))
            .await?;
        match status.is_success() {
            true => Ok(()),
            false => Err(Error::Pause(format!("{status}: {}", body.trim()))),
        }
    }
}
