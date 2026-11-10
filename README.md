# smsgora

[![crates.io](https://img.shields.io/crates/v/smsgora.svg)](https://crates.io/crates/smsgora)
[![docs.rs](https://docs.rs/smsgora/badge.svg)](https://docs.rs/smsgora)
[![tests](https://github.com/TekservixHQ/smsgora-rust/actions/workflows/tests.yml/badge.svg)](https://github.com/TekservixHQ/smsgora-rust/actions/workflows/tests.yml)

Rust client for the [SMS Gora SMS Gateway](https://tekservix.com/smsgora/) API.

SMS Gora turns Android phones with ordinary SIM cards into an SMS gateway. Your server calls the API, a linked phone sends the text, and replies and delivery reports come back to you. There's no sender ID registration and no per-message fee from us: your mobile operator bills the texts like any SMS.

- async (reqwest + any Tokio app: Axum, Actix Web, Rocket, Loco)
- Results are `serde_json::Value`, so nothing breaks when the API adds fields
- Webhook signature check built in, `#![forbid(unsafe_code)]`

## Install

```bash
cargo add smsgora
```

Rustls is used by default. For the system's TLS instead: `cargo add smsgora --no-default-features --features native-tls`.

## Quick start

Create an API key in the SMS Gora dashboard (SMS Gora -> API keys) and keep it on your server.

```rust
use smsgora::{Client, SendSms};

#[tokio::main]
async fn main() -> Result<(), smsgora::Error> {
    let sms = Client::new(std::env::var("SMSGORA_KEY").expect("SMSGORA_KEY"));

    let result = sms.send_sms(SendSms::new(["+15551234567"], "Your verification code is 482913")).await?;
    println!("{}", result["message_ids"][0]); // 1873
    Ok(())
}
```

## Sending

```rust
// Several recipients (up to 1,000 per call)
sms.send_sms(SendSms::new(["+15551234567", "+447700900123"], "Your order has shipped")).await?;

// From a chosen phone and SIM, scheduled for later
sms.send_sms(
    SendSms::new(["+15551234567"], "Reminder: your appointment is tomorrow at 10:00")
        .device_id(42)
        .sim_slot(1) // 0 = SIM 1, 1 = SIM 2
        .schedule_at("2026-12-01T08:00:00Z"),
).await?;

// Different texts for different people in one call (up to 5,000 recipients)
use smsgora::BulkMessage;
sms.send_bulk_sms(42, &[
    BulkMessage::new(["+15551234567"]).with_message("Hi Aline, parcel #1042 is ready."),
    BulkMessage::new(["+15557654321"]),
], Some("Hello, your parcel is ready for pickup.")).await?;
```

## Delivery status and incoming SMS

```rust
let msg = sms.get_message(1873).await?;
println!("{}", msg["status"]); // queued -> dispatched -> sent -> delivered (or failed)

sms.cancel_message(1874).await?; // works while queued or scheduled

let inbox   = sms.list_received_sms(None, &[("since", "2026-11-01"), ("limit", "50")]).await?;
let phones  = sms.list_devices().await?;
let account = sms.get_account().await?;
```

Want typed results? `serde_json::from_value::<MyStruct>(value)`.

Anything else in the [API reference](https://tekservix.com/smsgora/docs/) is one call away:

```rust
let batch = sms.request(smsgora::Method::GET, "/batches/311", None).await?;
```

## Webhooks

Add your HTTPS URL in SMS Gora -> Webhooks (or with `create_webhook`) and keep the secret. Each request carries `X-SmsGora-Signature: sha256=...`, an HMAC-SHA256 of the raw body. Axum example:

```rust
use axum::{body::Bytes, http::{HeaderMap, StatusCode}};

async fn sms_webhook(headers: HeaderMap, body: Bytes) -> StatusCode {
    let sig = headers.get("x-smsgora-signature").and_then(|v| v.to_str().ok()).unwrap_or("");
    let secret = std::env::var("SMSGORA_WEBHOOK_SECRET").unwrap_or_default();
    if !smsgora::verify_webhook(&body, sig, &secret) {
        return StatusCode::UNAUTHORIZED;
    }
    let event: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
    if event["event"] == "sms.received" {
        // event["data"]["from"], event["data"]["message"]: queue slow work, answer fast
    }
    StatusCode::OK // answer within 10 seconds
}
```

## Errors

API errors come back as `Error::Api { status, message, errors, body, retry_after }`.

```rust
match sms.send_sms(SendSms::new(["+15551234567"], "Hi")).await {
    Err(e) if e.status() == Some(429) => {
        // 300 requests/minute per key, or plan limit
        let wait = e.retry_after().unwrap_or(5.0);
    }
    other => { other?; }
}
```

Never put the key in a browser (WASM) or mobile app: anyone could read it. Call the API from your server.

## Migrating from TextBee

SMS Gora accepts TextBee-style requests, so most integrations switch by changing the base URL and key. See the [compatibility notes](https://tekservix.com/smsgora/docs/).

## More from Tekservix

- [WAservix WhatsApp API Gateway](https://tekservix.com/waservix/): send WhatsApp messages from your own number (`cargo add waservix`)
- [AI Sales Agent](https://tekservix.com/ai-sales-agent): answers customers on WhatsApp and SMS using your website's content
- [Website to App](https://tekservix.com/mobile-apps): turn your website into Android and iOS apps
- SMS Gora is also available for Node.js, Python, PHP, Go, Java/Kotlin, .NET, Ruby and Swift: see [github.com/TekservixHQ](https://github.com/TekservixHQ)

## License

MIT
