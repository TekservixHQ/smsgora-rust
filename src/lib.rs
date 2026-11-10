//! Rust client for the [SMS Gora SMS Gateway](https://tekservix.com/smsgora/) API.
//!
//! SMS Gora turns Android phones with ordinary SIM cards into an SMS gateway: your server
//! calls the API, a linked phone sends the text, and replies and delivery reports come back.
//!
//! ```no_run
//! # async fn run() -> Result<(), smsgora::Error> {
//! let sms = smsgora::Client::new(std::env::var("SMSGORA_KEY").unwrap());
//! let result = sms.send_sms(smsgora::SendSms::new(["+15551234567"], "Your code is 482913")).await?;
//! println!("{}", result["message_ids"][0]); // 1873
//! # Ok(()) }
//! ```
//!
//! Keep the API key on your server. API reference: <https://tekservix.com/smsgora/docs/>

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::fmt;
use std::time::Duration;

pub use reqwest::Method;
pub use serde_json::{json, Value};

/// Crate version, sent in the User-Agent header.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Production API base URL.
pub const DEFAULT_BASE_URL: &str = "https://tekservix.com/smsgora/api/v1";

/// Errors returned by the client.
#[derive(Debug)]
pub enum Error {
    /// The API answered with an error.
    Api {
        /// HTTP status.
        status: u16,
        /// Message from the API, or `HTTP <status>`.
        message: String,
        /// Field errors, when the API sends them.
        errors: Option<Value>,
        /// Parsed JSON body, or the raw text as a JSON string.
        body: Value,
        /// Seconds to wait before retrying (sent with 429).
        retry_after: Option<f64>,
    },
    /// Network or HTTP failure before an API answer arrived.
    Http(reqwest::Error),
    /// Bad argument or invalid request body.
    Invalid(String),
}

impl Error {
    /// HTTP status for API errors.
    pub fn status(&self) -> Option<u16> {
        match self {
            Error::Api { status, .. } => Some(*status),
            Error::Http(e) => e.status().map(|s| s.as_u16()),
            Error::Invalid(_) => None,
        }
    }

    /// Seconds to wait before retrying, when the API sent `Retry-After`.
    pub fn retry_after(&self) -> Option<f64> {
        match self {
            Error::Api { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Api { status, message, .. } => write!(f, "SmsGora: {message} (HTTP {status})"),
            Error::Http(e) => write!(f, "SmsGora: {e}"),
            Error::Invalid(m) => write!(f, "SmsGora: {m}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Http(e) => Some(e),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Error::Http(e)
    }
}

/// Parameters for [`Client::send_sms`].
#[derive(Debug, Clone)]
pub struct SendSms {
    recipients: Vec<String>,
    message: String,
    device_id: Option<i64>,
    sim_slot: Option<u8>,
    schedule_at: Option<String>,
}

impl SendSms {
    /// Text `message` to one or more numbers (up to 1,000), in international format.
    pub fn new<I, S>(recipients: I, message: impl Into<String>) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        SendSms {
            recipients: recipients.into_iter().map(Into::into).collect(),
            message: message.into(),
            device_id: None,
            sim_slot: None,
            schedule_at: None,
        }
    }

    /// Send from this phone instead of your best online one.
    pub fn device_id(mut self, id: i64) -> Self {
        self.device_id = Some(id);
        self
    }

    /// 0 = SIM 1, 1 = SIM 2.
    pub fn sim_slot(mut self, slot: u8) -> Self {
        self.sim_slot = Some(slot);
        self
    }

    /// Send later. ISO 8601 time, e.g. `2026-12-01T08:00:00Z`.
    pub fn schedule_at(mut self, when: impl Into<String>) -> Self {
        self.schedule_at = Some(when.into());
        self
    }
}

/// One entry for [`Client::send_bulk_sms`]. Leave `message` as `None` to use the shared template.
#[derive(Debug, Clone)]
pub struct BulkMessage {
    /// Numbers for this entry.
    pub recipients: Vec<String>,
    /// Own text for these numbers.
    pub message: Option<String>,
}

impl BulkMessage {
    /// Entry using the shared template.
    pub fn new<I, S>(recipients: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        BulkMessage { recipients: recipients.into_iter().map(Into::into).collect(), message: None }
    }

    /// Entry with its own text.
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }
}

/// Builder for [`Client`] with a custom base URL, timeout or reqwest client.
#[derive(Debug)]
pub struct ClientBuilder {
    api_key: String,
    base_url: String,
    timeout: Duration,
    http: Option<reqwest::Client>,
}

impl ClientBuilder {
    /// Change only for testing.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Timeout per request (default 30 seconds). Ignored when you pass your own client.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Use your own reqwest client (proxy, connection pool, middleware).
    pub fn http_client(mut self, http: reqwest::Client) -> Self {
        self.http = Some(http);
        self
    }

    /// Build the client.
    pub fn build(self) -> Client {
        let http = self.http.unwrap_or_else(|| {
            reqwest::Client::builder()
                .timeout(self.timeout)
                .user_agent(format!("smsgora-rust/{VERSION}"))
                .build()
                .expect("SmsGora: failed to build HTTP client")
        });
        Client { http, api_key: self.api_key, base_url: self.base_url.trim_end_matches('/').to_string() }
    }
}

/// SMS Gora API client. Cheap to clone; reuse one per app.
#[derive(Debug, Clone)]
pub struct Client {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl Client {
    /// Client with your API key (starts with `sgk_`, from SMS Gora -> API keys).
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::builder(api_key).build()
    }

    /// Builder for a custom base URL, timeout or reqwest client.
    pub fn builder(api_key: impl Into<String>) -> ClientBuilder {
        let api_key = api_key.into();
        assert!(!api_key.is_empty(), "SmsGora: api_key is required (create one in SMS Gora -> API keys)");
        ClientBuilder { api_key, base_url: DEFAULT_BASE_URL.to_string(), timeout: Duration::from_secs(30), http: None }
    }

    /// Call any documented endpoint, e.g. `request(Method::GET, "/devices", None)`.
    /// Returns the `data` field of the answer.
    pub async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value, Error> {
        let mut req = self
            .http
            .request(method, format!("{}{}", self.base_url, path))
            .header("x-api-key", &self.api_key)
            .header("Accept", "application/json");
        if let Some(body) = body {
            req = req.header("Content-Type", "application/json").body(body.to_string());
        }
        let res = req.send().await?;
        let status = res.status().as_u16();
        let retry_after = res
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<f64>().ok());
        let text = res.text().await?;
        let json: Option<Value> = if text.is_empty() { None } else { serde_json::from_str(&text).ok() };

        let failed = json.as_ref().and_then(|j| j.get("success")).and_then(Value::as_bool) == Some(false);
        if status >= 400 || failed {
            let message = json
                .as_ref()
                .and_then(|j| j.get("message"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("HTTP {status}"));
            let errors = json.as_ref().and_then(|j| j.get("errors")).cloned();
            return Err(Error::Api { status, message, errors, body: json.unwrap_or(Value::String(text)), retry_after });
        }
        Ok(match json {
            Some(Value::Object(mut o)) if o.contains_key("data") => o.remove("data").unwrap_or(Value::Null),
            Some(v) => v,
            None => Value::Null,
        })
    }

    /// Send an SMS. Returns `{"message_ids": [...]}`.
    pub async fn send_sms(&self, params: SendSms) -> Result<Value, Error> {
        if params.recipients.is_empty() {
            return Err(Error::Invalid("send_sms: at least one recipient is required".into()));
        }
        let mut body = json!({ "recipients": params.recipients, "message": params.message });
        if let Some(slot) = params.sim_slot {
            body["sim_slot"] = json!(slot);
        }
        if let Some(when) = params.schedule_at {
            body["schedule_at"] = json!(when);
        }
        let path = match params.device_id {
            Some(id) => format!("/devices/{id}/send-sms"),
            None => "/send-sms".to_string(),
        };
        self.request(Method::POST, &path, Some(body)).await
    }

    /// Different texts for different people in one call (up to 5,000 recipients).
    pub async fn send_bulk_sms(&self, device_id: i64, messages: &[BulkMessage], message_template: Option<&str>) -> Result<Value, Error> {
        let items: Vec<Value> = messages
            .iter()
            .map(|m| {
                let mut item = json!({ "recipients": m.recipients });
                if let Some(text) = &m.message {
                    item["message"] = json!(text);
                }
                item
            })
            .collect();
        let mut body = json!({ "messages": items });
        if let Some(t) = message_template {
            body["messageTemplate"] = json!(t);
        }
        self.request(Method::POST, &format!("/devices/{device_id}/send-bulk-sms"), Some(body)).await
    }

    /// Status is queued, dispatched, sent, delivered, failed or cancelled.
    pub async fn get_message(&self, id: i64) -> Result<Value, Error> {
        self.request(Method::GET, &format!("/messages/{id}"), None).await
    }

    /// Cancel a queued or scheduled message.
    pub async fn cancel_message(&self, id: i64) -> Result<Value, Error> {
        self.request(Method::POST, &format!("/messages/{id}/cancel"), None).await
    }

    /// Filters: direction (in/out), status, device_id, phone, since, after_id, page, limit.
    pub async fn list_messages(&self, filters: &[(&str, &str)]) -> Result<Value, Error> {
        self.request(Method::GET, &format!("/messages{}", query(filters)), None).await
    }

    /// Incoming SMS for all phones, or one phone with `device_id`.
    pub async fn list_received_sms(&self, device_id: Option<i64>, filters: &[(&str, &str)]) -> Result<Value, Error> {
        let path = match device_id {
            Some(id) => format!("/devices/{id}/get-received-sms"),
            None => "/received-sms".to_string(),
        };
        self.request(Method::GET, &format!("{path}{}", query(filters)), None).await
    }

    /// One bulk batch.
    pub async fn get_batch(&self, id: i64) -> Result<Value, Error> {
        self.request(Method::GET, &format!("/batches/{id}"), None).await
    }

    /// Your linked Android phones.
    pub async fn list_devices(&self) -> Result<Value, Error> {
        self.request(Method::GET, "/devices", None).await
    }

    /// One phone.
    pub async fn get_device(&self, id: i64) -> Result<Value, Error> {
        self.request(Method::GET, &format!("/devices/{id}"), None).await
    }

    /// Account, plan and usage.
    pub async fn get_account(&self) -> Result<Value, Error> {
        self.request(Method::GET, "/account", None).await
    }

    /// Your webhooks.
    pub async fn list_webhooks(&self) -> Result<Value, Error> {
        self.request(Method::GET, "/webhooks", None).await
    }

    /// Register a webhook (events default to `sms.received`). The answer includes the secret, returned once.
    pub async fn create_webhook(&self, url: &str, events: Option<&[&str]>, device_id: Option<i64>) -> Result<Value, Error> {
        let events = events.unwrap_or(&["sms.received"]);
        self.request(Method::POST, "/webhooks", Some(json!({ "url": url, "events": events, "device_id": device_id })))
            .await
    }

    /// Delete a webhook.
    pub async fn delete_webhook(&self, id: i64) -> Result<Value, Error> {
        self.request(Method::DELETE, &format!("/webhooks/{id}"), None).await
    }
}

/// Check the `X-SmsGora-Signature` header (`sha256=<hex HMAC-SHA256 of the raw body>`).
/// Pass the raw request body exactly as received, before JSON parsing.
pub fn verify_webhook(raw_body: &[u8], signature: &str, secret: &str) -> bool {
    if raw_body.is_empty() || signature.is_empty() || secret.is_empty() {
        return false;
    }
    let expected = format!("sha256={}", to_hex(&hmac_sha256(secret.as_bytes(), raw_body)));
    constant_time_eq(expected.as_bytes(), signature.as_bytes())
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(message);
    let inner_hash = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner_hash);
    let mut out = [0u8; 32];
    out.copy_from_slice(&outer.finalize());
    out
}

fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(DIGITS[(b >> 4) as usize] as char);
        s.push(DIGITS[(b & 0x0f) as usize] as char);
    }
    s
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn query(params: &[(&str, &str)]) -> String {
    let pairs: Vec<String> = params
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect();
    if pairs.is_empty() {
        String::new()
    } else {
        format!("?{}", pairs.join("&"))
    }
}

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn hmac_matches_reference() {
        assert_eq!(
            to_hex(&hmac_sha256(&[b'k'; 100], "abc".repeat(50).as_bytes())),
            "b2f2222673f70e4563af636099533dc20d077b754c16c8c3962f57e789350dd0"
        );
    }

    #[test]
    fn query_encoding() {
        assert_eq!(query(&[("phone", "+1555"), ("since", "2026-11-01"), ("x", "")]), "?phone=%2B1555&since=2026-11-01");
        assert_eq!(query(&[]), "");
    }
}
