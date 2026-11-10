use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use smsgora::{verify_webhook, BulkMessage, Client, Error, SendSms, Value};

#[derive(Clone, Debug, Default)]
struct Seen {
    method: String,
    path: String,
    api_key: String,
    body: Value,
}

/// Tiny HTTP server so the tests need no mock crates.
fn server() -> (String, Arc<Mutex<Seen>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Seen::default()));
    let shared = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                continue;
            }
            let mut parts = line.split_whitespace();
            let method = parts.next().unwrap_or("").to_string();
            let path = parts.next().unwrap_or("").to_string();
            let (mut len, mut key) = (0usize, String::new());
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).unwrap();
                if h == "\r\n" || h.is_empty() {
                    break;
                }
                let (name, value) = h.split_once(':').unwrap();
                match name.to_ascii_lowercase().as_str() {
                    "content-length" => len = value.trim().parse().unwrap(),
                    "x-api-key" => key = value.trim().to_string(),
                    _ => {}
                }
            }
            let mut body = vec![0u8; len];
            reader.read_exact(&mut body).unwrap();
            let body: Value = if body.is_empty() { Value::Null } else { serde_json::from_slice(&body).unwrap() };
            *shared.lock().unwrap() = Seen { method, path: path.clone(), api_key: key, body };

            let (status, extra, reply) = if path.starts_with("/limited") {
                ("429 Too Many Requests", "Retry-After: 12\r\n", r#"{"success":false,"message":"Too many requests","errors":{"recipients":["bad"]}}"#)
            } else if path.starts_with("/broken") {
                ("502 Bad Gateway", "", "<html>bad gateway</html>")
            } else {
                ("200 OK", "", r#"{"success":true,"data":{"message_ids":[1873]}}"#)
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
        }
    });
    (base, seen)
}

#[tokio::test]
async fn send_sms_and_paths() {
    let (base, seen) = server();
    let sms = Client::builder("sgk_test").base_url(format!("{base}/api/")).build();

    let res = sms.send_sms(SendSms::new(["+15551234567"], "Hi")).await.unwrap();
    assert_eq!(res["message_ids"][0], 1873);
    {
        let s = seen.lock().unwrap();
        assert_eq!((s.method.as_str(), s.path.as_str(), s.api_key.as_str()), ("POST", "/api/send-sms", "sgk_test"));
        assert_eq!(s.body, serde_json::json!({"recipients": ["+15551234567"], "message": "Hi"}));
    }

    sms.send_sms(SendSms::new(vec!["+1".to_string(), "+2".to_string()], "x").device_id(42).sim_slot(1).schedule_at("2026-12-01T08:00:00Z"))
        .await
        .unwrap();
    {
        let s = seen.lock().unwrap();
        assert_eq!(s.path, "/api/devices/42/send-sms");
        assert_eq!(s.body["sim_slot"], 1);
        assert_eq!(s.body["schedule_at"], "2026-12-01T08:00:00Z");
    }

    sms.send_bulk_sms(42, &[BulkMessage::new(["+1"]).with_message("a"), BulkMessage::new(["+2"])], Some("t")).await.unwrap();
    {
        let s = seen.lock().unwrap();
        assert_eq!(s.path, "/api/devices/42/send-bulk-sms");
        assert_eq!(s.body["messageTemplate"], "t");
        assert!(s.body["messages"][1].get("message").is_none());
    }

    sms.list_received_sms(None, &[("phone", "+1555"), ("since", "2026-11-01")]).await.unwrap();
    assert_eq!(seen.lock().unwrap().path, "/api/received-sms?phone=%2B1555&since=2026-11-01");
    sms.list_received_sms(Some(3), &[]).await.unwrap();
    assert_eq!(seen.lock().unwrap().path, "/api/devices/3/get-received-sms");
    sms.cancel_message(7).await.unwrap();
    assert_eq!(seen.lock().unwrap().path, "/api/messages/7/cancel");
    sms.create_webhook("https://my.app/hook", None, None).await.unwrap();
    assert_eq!(seen.lock().unwrap().body["events"], serde_json::json!(["sms.received"]));
    sms.delete_webhook(9).await.unwrap();
    assert_eq!(seen.lock().unwrap().method, "DELETE");

    assert!(matches!(sms.send_sms(SendSms::new(Vec::<String>::new(), "x")).await, Err(Error::Invalid(_))));
}

#[tokio::test]
async fn errors() {
    let (base, _) = server();
    let err = Client::builder("k").base_url(format!("{base}/limited")).build().get_account().await.unwrap_err();
    assert_eq!(err.status(), Some(429));
    assert_eq!(err.retry_after(), Some(12.0));
    match &err {
        Error::Api { message, errors, .. } => {
            assert_eq!(message, "Too many requests");
            assert_eq!(errors.as_ref().unwrap()["recipients"][0], "bad");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(err.to_string(), "SmsGora: Too many requests (HTTP 429)");

    let err = Client::builder("k").base_url(format!("{base}/broken")).build().list_devices().await.unwrap_err();
    match err {
        Error::Api { status, message, body, .. } => {
            assert_eq!((status, message.as_str()), (502, "HTTP 502"));
            assert_eq!(body, Value::String("<html>bad gateway</html>".into()));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn webhook_signature() {
    let body = br#"{"event":"sms.received","data":{"from":"+15551234567"}}"#;
    let sig = "sha256=e125c07dfbecb498e8979383d7b9f0815efb128755409794b486d4bbb7199583";
    assert!(verify_webhook(body, sig, "whsec_test"));
    assert!(!verify_webhook(body, sig, "other"));
    assert!(!verify_webhook(b"{}", sig, "whsec_test"));
    assert!(!verify_webhook(body, "", "whsec_test"));
}
