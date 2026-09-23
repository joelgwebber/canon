//! End-to-end control-plane test: a real WebSocket client drives the axum server over a
//! bound TCP listener, exercising the whole ws+json path (yak canon-b46b, canon-9487).
//!
//! The player is real; only the Tidal network is stubbed by a mock [`ServiceSession`],
//! so this proves the transport, the command translation, the snapshot stream, and the
//! session verbs together — everything except the live Tidal calls (which need a human
//! at a browser and are verified by `canon login tidal`).

use std::sync::Arc;

use async_trait::async_trait;
use canon_api::{AppState, serve};
use canon_core::{Account, DeviceCode, LoginStatus, PlayerHandle, Result, Service, ServiceSession};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

/// A canned session: no network, deterministic answers, so the ws plumbing is what's
/// under test.
struct MockSession;

#[async_trait]
impl ServiceSession for MockSession {
    fn service(&self) -> Service {
        Service::Tidal
    }
    fn is_authenticated(&self) -> bool {
        false
    }
    async fn begin_login(&self) -> Result<DeviceCode> {
        Ok(DeviceCode {
            user_code: "AB-CD".into(),
            verification_uri: "link.tidal.com".into(),
            verification_uri_complete: Some("link.tidal.com/AB-CD".into()),
            expires_in: 300,
            interval: 2,
        })
    }
    async fn poll_login(&self) -> Result<LoginStatus> {
        Ok(LoginStatus::Authorized)
    }
    async fn account(&self) -> Result<Account> {
        Ok(Account {
            service: Service::Tidal,
            user_id: "42".into(),
            username: Some("canon-tester".into()),
            attributes: Default::default(),
        })
    }
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Read frames until a JSON text frame arrives, and parse it.
async fn next_json(ws: &mut Ws) -> serde_json::Value {
    loop {
        let message = ws.next().await.expect("socket open").expect("frame ok");
        if let Message::Text(text) = message {
            return serde_json::from_str(&text).expect("valid json frame");
        }
    }
}

/// Read frames until one satisfies `pred` (tolerating snapshot/reply interleaving).
async fn next_matching(
    ws: &mut Ws,
    mut pred: impl FnMut(&serde_json::Value) -> bool,
) -> serde_json::Value {
    for _ in 0..16 {
        let value = next_json(ws).await;
        if pred(&value) {
            return value;
        }
    }
    panic!("expected frame not seen within budget");
}

async fn send(ws: &mut Ws, json: serde_json::Value) {
    ws.send(Message::Text(json.to_string()))
        .await
        .expect("send");
}

#[tokio::test]
async fn ws_control_plane_end_to_end() {
    // Real player, mock session, ephemeral port.
    let player = PlayerHandle::spawn();
    let control: Arc<dyn canon_core::ControlPlane> = Arc::new(player);
    let state = Arc::new(AppState::new(control).with_session(Arc::new(MockSession)));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { serve(state, listener).await.unwrap() });

    let (mut ws, _resp) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .expect("connect");

    // 1. hello, then the immediate idle snapshot.
    let hello = next_json(&mut ws).await;
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["protocol"], 1);

    let snapshot = next_matching(&mut ws, |v| v["type"] == "snapshot").await;
    assert_eq!(snapshot["snapshot"]["state"], "idle");
    assert_eq!(snapshot["snapshot"]["seq"], 0);

    // 2. a transport command: ack + a snapshot reflecting the new volume.
    send(
        &mut ws,
        serde_json::json!({"id": 1, "op": "set_volume", "volume": 0.4}),
    )
    .await;
    let ack = next_matching(&mut ws, |v| v["type"] == "reply" && v["id"] == 1).await;
    assert_eq!(ack["ok"], true);
    assert_eq!(ack["result"]["kind"], "ack");
    let vol = next_matching(&mut ws, |v| {
        v["type"] == "snapshot" && (v["snapshot"]["volume"].as_f64().unwrap() - 0.4).abs() < 1e-6
    })
    .await;
    assert!(vol["snapshot"]["seq"].as_u64().unwrap() >= 1);

    // 3. the session/auth verbs, correlated by id.
    send(&mut ws, serde_json::json!({"id": 2, "op": "login_begin"})).await;
    let begun = next_matching(&mut ws, |v| v["id"] == 2).await;
    assert_eq!(begun["ok"], true);
    assert_eq!(begun["result"]["kind"], "device_code");
    assert_eq!(begun["result"]["user_code"], "AB-CD");

    send(&mut ws, serde_json::json!({"id": 3, "op": "login_poll"})).await;
    let polled = next_matching(&mut ws, |v| v["id"] == 3).await;
    assert_eq!(polled["result"]["kind"], "login");
    assert_eq!(polled["result"]["status"], "authorized");

    send(&mut ws, serde_json::json!({"id": 4, "op": "account"})).await;
    let account = next_matching(&mut ws, |v| v["id"] == 4).await;
    assert_eq!(account["result"]["kind"], "account");
    assert_eq!(account["result"]["user_id"], "42");
    assert_eq!(account["result"]["username"], "canon-tester");

    // 4. the sink list: always at least the local output, with the reserved `local` id a client
    //    selects to come back from a network renderer.
    send(&mut ws, serde_json::json!({"id": 6, "op": "list_sinks"})).await;
    let sinks = next_matching(&mut ws, |v| v["id"] == 6).await;
    assert_eq!(sinks["ok"], true);
    assert_eq!(sinks["result"]["kind"], "sinks");
    let listed = sinks["result"]["sinks"].as_array().expect("sinks array");
    assert_eq!(listed[0]["id"], "local");
    assert_eq!(listed[0]["kind"], "local");

    // 5. a malformed frame is a non-fatal error reply (no id echoed).
    send(&mut ws, serde_json::json!({"op": "nonsense"})).await;
    let err = next_matching(&mut ws, |v| v["type"] == "reply" && v["ok"] == false).await;
    assert!(err["error"].as_str().unwrap().contains("bad request"));

    // 6. an unknown service is a clean error, not a panic.
    send(
        &mut ws,
        serde_json::json!({"id": 5, "op": "account", "service": "spotify"}),
    )
    .await;
    let no_session = next_matching(&mut ws, |v| v["id"] == 5).await;
    assert_eq!(no_session["ok"], false);
    assert!(no_session["error"].as_str().unwrap().contains("spotify"));
}
