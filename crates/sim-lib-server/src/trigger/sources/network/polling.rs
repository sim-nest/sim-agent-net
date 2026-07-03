#[cfg(any(feature = "trigger-telegram", feature = "trigger-matrix"))]
use std::collections::VecDeque;
#[cfg(any(feature = "trigger-telegram", feature = "trigger-matrix"))]
use std::time::Duration;

#[cfg(any(feature = "trigger-telegram", feature = "trigger-matrix"))]
use sim_kernel::{Cx, Error, Result};

#[cfg(any(feature = "trigger-telegram", feature = "trigger-matrix"))]
use super::TriggerSource;
#[cfg(any(feature = "trigger-telegram", feature = "trigger-matrix"))]
use super::common::{connect_with_timeout, io_to_host};

#[cfg(feature = "trigger-telegram")]
pub(crate) struct TelegramSource {
    client: JsonPollingSource,
    chat_id: String,
    bot: String,
    offset: i64,
}

#[cfg(feature = "trigger-telegram")]
impl TelegramSource {
    pub(crate) fn new(bot: String, chat_id: String, base_url: String) -> Result<Self> {
        Ok(Self {
            client: JsonPollingSource::new(base_url)?,
            chat_id,
            bot,
            offset: 0,
        })
    }
}

#[cfg(feature = "trigger-telegram")]
impl TriggerSource for TelegramSource {
    fn next_event(&mut self, _cx: &mut Cx, timeout: Duration) -> Result<Option<Vec<u8>>> {
        if let Some(event) = self.client.pending.pop_front() {
            return Ok(Some(event));
        }
        let path = format!(
            "{}/bot{}/getUpdates?timeout={}&offset={}",
            self.client.base_path,
            self.bot,
            timeout.as_secs(),
            self.offset
        );
        let body = self.client.get(&path, timeout)?;
        let value: serde_json::Value =
            serde_json::from_slice(&body).map_err(|error| Error::HostError(error.to_string()))?;
        let mut max_update = self.offset;
        for item in value
            .get("result")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            let update_id = item
                .get("update_id")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(max_update);
            max_update = max_update.max(update_id + 1);
            if item
                .pointer("/message/chat/id")
                .map(json_id_text)
                .as_deref()
                == Some(self.chat_id.as_str())
            {
                self.client.pending.push_back(
                    serde_json::to_vec(item)
                        .map_err(|error| Error::HostError(error.to_string()))?,
                );
            }
        }
        self.offset = max_update;
        Ok(self.client.pending.pop_front())
    }

    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(feature = "trigger-matrix")]
pub(crate) struct MatrixSource {
    client: JsonPollingSource,
    room_id: String,
    since: Option<String>,
}

#[cfg(feature = "trigger-matrix")]
impl MatrixSource {
    pub(crate) fn new(room_id: String, base_url: String) -> Result<Self> {
        Ok(Self {
            client: JsonPollingSource::new(base_url)?,
            room_id,
            since: None,
        })
    }
}

#[cfg(feature = "trigger-matrix")]
impl TriggerSource for MatrixSource {
    fn next_event(&mut self, _cx: &mut Cx, timeout: Duration) -> Result<Option<Vec<u8>>> {
        if let Some(event) = self.client.pending.pop_front() {
            return Ok(Some(event));
        }
        let mut path = format!(
            "{}/sync?timeout={}",
            self.client.base_path,
            timeout.as_millis()
        );
        if let Some(since) = &self.since {
            path.push_str("&since=");
            path.push_str(since);
        }
        let body = self.client.get(&path, timeout)?;
        let value: serde_json::Value =
            serde_json::from_slice(&body).map_err(|error| Error::HostError(error.to_string()))?;
        self.since = value
            .get("next_batch")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned);
        if let Some(events) = value.pointer(&format!(
            "/rooms/join/{}/timeline/events",
            self.room_id.replace('/', "~1")
        )) && let Some(events) = events.as_array()
        {
            for item in events {
                self.client.pending.push_back(
                    serde_json::to_vec(item)
                        .map_err(|error| Error::HostError(error.to_string()))?,
                );
            }
        }
        Ok(self.client.pending.pop_front())
    }

    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(any(feature = "trigger-telegram", feature = "trigger-matrix"))]
struct JsonPollingSource {
    host: String,
    port: u16,
    base_path: String,
    pending: VecDeque<Vec<u8>>,
}

#[cfg(any(feature = "trigger-telegram", feature = "trigger-matrix"))]
impl JsonPollingSource {
    fn new(base_url: String) -> Result<Self> {
        #[cfg(feature = "server-net-http")]
        {
            let parsed = crate::http::parse_url(&base_url, "http", "")?;
            Ok(Self {
                host: parsed.host,
                port: parsed.port,
                base_path: parsed.path.trim_end_matches('/').to_owned(),
                pending: VecDeque::new(),
            })
        }
        #[cfg(not(feature = "server-net-http"))]
        {
            let _ = base_url;
            Err(Error::Eval(
                "http polling triggers require feature server-net-http".to_owned(),
            ))
        }
    }

    fn get(&self, path: &str, timeout: Duration) -> Result<Vec<u8>> {
        #[cfg(feature = "server-net-http")]
        {
            let mut stream = connect_with_timeout(&self.host, self.port, timeout)?;
            stream.set_read_timeout(Some(timeout)).map_err(io_to_host)?;
            stream
                .set_write_timeout(Some(timeout))
                .map_err(io_to_host)?;
            crate::http::write_request(
                &mut stream,
                &crate::http::HttpRequest {
                    method: "GET".to_owned(),
                    path: path.to_owned(),
                    headers: vec![("Host".to_owned(), self.host.clone())],
                    body: Vec::new(),
                },
            )?;
            let response = crate::http::read_response(&mut stream)?;
            if response.status != 200 {
                return Err(Error::Eval(format!("http status {}", response.status)));
            }
            Ok(response.body)
        }
        #[cfg(not(feature = "server-net-http"))]
        {
            let _ = (path, timeout);
            Err(Error::Eval(
                "http polling triggers require feature server-net-http".to_owned(),
            ))
        }
    }
}

#[cfg(feature = "trigger-telegram")]
fn json_id_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Number(number) => number.to_string(),
        _ => String::new(),
    }
}
