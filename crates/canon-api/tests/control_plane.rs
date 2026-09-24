//! End-to-end control-plane test: a real WebSocket client drives the axum server over a
//! bound TCP listener, exercising the whole ws+json path (yak canon-b46b, canon-9487).
//!
//! The player is real; only the Tidal network is stubbed, by a mock [`Connector`] and a
//! describing source, so this proves the transport, the command translation, the snapshot
//! stream, and the connection verbs together — everything except the live Tidal calls (which
//! need a human at a browser and are verified by `canon login tidal`).

use std::sync::Arc;

use async_trait::async_trait;
use canon_api::{AppState, serve};
use std::sync::atomic::{AtomicBool, Ordering};

use canon_core::{
    Account, Capabilities, Capability, ConnectionInfo, Connector, DeviceCode, Error, FlowKind,
    Health, LoginFlow, LoginStatus, Method, PlayerHandle, Quality, ResolvedStream, Result, Service,
    Settings, SettingsStore, Source, SourceRef, SourceTrack, Sources,
};
use canon_library::{Library, Store};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

/// A canned connector with one device-code login: no network, deterministic answers, so the
/// ws plumbing is what's under test. Signing in grants browsing only.
#[derive(Default)]
struct MockConnector {
    signed_in: AtomicBool,
}

#[async_trait]
impl Connector for MockConnector {
    fn service(&self) -> Service {
        Service::Tidal
    }
    fn methods(&self) -> Vec<Method> {
        vec![Method {
            id: "tidal.device".into(),
            service: Service::Tidal,
            label: "Tidal (code on another device)".into(),
            flow: FlowKind::DeviceCode,
            grants: BROWSING,
            note: String::new(),
        }]
    }
    async fn connections(&self) -> Vec<ConnectionInfo> {
        let signed_in = self.signed_in.load(Ordering::SeqCst);
        vec![ConnectionInfo {
            id: "tidal.device".into(),
            service: Service::Tidal,
            label: "Tidal (code on another device)".into(),
            grants: BROWSING,
            verified: None,
            health: if signed_in {
                Health::Ok
            } else {
                Health::NeedsLogin
            },
            account: signed_in.then(|| Account {
                service: Service::Tidal,
                user_id: "42".into(),
                username: Some("canon-tester".into()),
                attributes: Default::default(),
            }),
        }]
    }
    async fn begin(&self, _method: &str) -> Result<LoginFlow> {
        Ok(LoginFlow::DeviceCode {
            code: DeviceCode {
                user_code: "AB-CD".into(),
                verification_uri: "link.tidal.com".into(),
                verification_uri_complete: Some("link.tidal.com/AB-CD".into()),
                expires_in: 300,
                interval: 2,
            },
        })
    }
    async fn complete(&self, _method: &str, _input: Option<String>) -> Result<LoginStatus> {
        self.signed_in.store(true, Ordering::SeqCst);
        Ok(LoginStatus::Authorized)
    }
    async fn disconnect(&self, _method: &str) -> Result<()> {
        self.signed_in.store(false, Ordering::SeqCst);
        Ok(())
    }
    fn grants(&self, capability: Capability) -> bool {
        self.signed_in.load(Ordering::SeqCst) && BROWSING.has(capability)
    }
    fn source(&self, _need: Capability) -> Option<Arc<dyn Source>> {
        None
    }
    fn catalog(&self) -> Option<Arc<dyn canon_core::Catalog>> {
        None
    }
    fn hint(&self, _capability: Capability) -> String {
        "sign in with the browser login".into()
    }
}

const BROWSING: Capabilities = Capabilities {
    catalog: true,
    library_read: true,
    library_write: false,
    recommendations: false,
    stream: None,
};

/// A Tidal source that can describe any track (so the library can resolve it) but not play one:
/// the player here is the bare actor, which never opens a stream.
struct Describer;

#[async_trait]
impl Source for Describer {
    fn service(&self) -> Service {
        Service::Tidal
    }
    async fn open(
        &self,
        _source: &SourceRef,
        _quality: Quality,
        _start: std::time::Duration,
    ) -> Result<ResolvedStream> {
        Err(Error::Unsupported("describe only".into()))
    }
    async fn describe(&self, source: &SourceRef) -> Result<SourceTrack> {
        Ok(SourceTrack {
            source: source.clone(),
            title: "Army of Me".into(),
            artists: Vec::new(),
            album: None,
            disc: None,
            position: None,
            duration_ms: Some(234_000),
            isrc: None,
        })
    }
}

/// Settings held in memory: the ws plumbing and the schema are what's under test here.
#[derive(Default)]
struct MemorySettings(std::sync::Mutex<Settings>);

#[async_trait]
impl SettingsStore for MemorySettings {
    fn get(&self) -> Settings {
        self.0.lock().unwrap().clone()
    }
    async fn set(&self, settings: Settings) -> Result<()> {
        *self.0.lock().unwrap() = settings;
        Ok(())
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
    let state = Arc::new(
        AppState::new(control)
            .with_settings(Arc::new(MemorySettings::default()))
            .with_library(
                Library::new(Store::open_in_memory().unwrap()),
                Sources::new()
                    .with(Arc::new(Describer))
                    .with_connector(Arc::new(MockConnector::default())),
            ),
    );
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

    // 3. the connection verbs, correlated by id: what each login grants, before and after.
    send(&mut ws, serde_json::json!({"id": 2, "op": "services"})).await;
    let listed = next_matching(&mut ws, |v| v["id"] == 2).await;
    assert_eq!(listed["result"]["kind"], "services");
    let tidal = &listed["result"]["services"][0];
    assert_eq!(tidal["methods"][0]["id"], "tidal.device");
    assert_eq!(tidal["methods"][0]["flow"], "device_code");
    assert_eq!(
        tidal["methods"][0]["grants"]["stream"],
        serde_json::Value::Null
    );
    assert_eq!(tidal["connections"][0]["health"]["state"], "needs_login");

    send(
        &mut ws,
        serde_json::json!({"id": 3, "op": "connect", "method": "tidal.device"}),
    )
    .await;
    let begun = next_matching(&mut ws, |v| v["id"] == 3).await;
    assert_eq!(begun["result"]["kind"], "connecting");
    assert_eq!(begun["result"]["login"]["flow"], "device_code");
    assert_eq!(begun["result"]["login"]["code"]["user_code"], "AB-CD");

    send(
        &mut ws,
        serde_json::json!({"id": 4, "op": "connect_complete", "method": "tidal.device"}),
    )
    .await;
    let polled = next_matching(&mut ws, |v| v["id"] == 4).await;
    assert_eq!(polled["result"]["kind"], "login");
    assert_eq!(polled["result"]["status"], "authorized");

    send(&mut ws, serde_json::json!({"id": 15, "op": "services"})).await;
    let listed = next_matching(&mut ws, |v| v["id"] == 15).await;
    let connection = &listed["result"]["services"][0]["connections"][0];
    assert_eq!(connection["health"]["state"], "ok");
    assert_eq!(connection["account"]["user_id"], "42");

    // 4. the sink list: always at least the local output, with the reserved `local` id a client
    //    selects to come back from a network renderer.
    send(&mut ws, serde_json::json!({"id": 6, "op": "list_sinks"})).await;
    let sinks = next_matching(&mut ws, |v| v["id"] == 6).await;
    assert_eq!(sinks["ok"], true);
    assert_eq!(sinks["result"]["kind"], "sinks");
    let listed = sinks["result"]["sinks"].as_array().expect("sinks array");
    assert_eq!(listed[0]["id"], "local");
    assert_eq!(listed[0]["kind"], "local");

    // 5. the queue is server state: enqueueing is a snapshot transition, the contents come on
    //    request, and a command the player cannot apply is an error reply, not silence.
    send(
        &mut ws,
        serde_json::json!({"id": 7, "op": "enqueue", "service": "tidal", "track_id": "33348478"}),
    )
    .await;
    let queued = next_matching(&mut ws, |v| {
        v["type"] == "snapshot" && v["snapshot"]["queue"]["len"] == 1
    })
    .await;
    assert!(queued["snapshot"]["queue"]["revision"].as_u64().unwrap() >= 1);
    send(&mut ws, serde_json::json!({"id": 8, "op": "queue"})).await;
    let queue = next_matching(&mut ws, |v| v["id"] == 8).await;
    assert_eq!(queue["result"]["kind"], "queue");
    assert_eq!(queue["result"]["index"], 0);
    assert_eq!(queue["result"]["tracks"][0]["sources"][0]["id"], "33348478");
    assert_eq!(
        queue["result"]["tracks"][0]["meta"]["title"], "Army of Me",
        "described by the source on the way into the library"
    );
    send(&mut ws, serde_json::json!({"id": 9, "op": "next"})).await;
    let refused = next_matching(&mut ws, |v| v["id"] == 9).await;
    assert_eq!(refused["ok"], false);
    assert!(refused["error"].as_str().unwrap().contains("no next track"));

    //    The same service id enqueued again is the same library entity, not a fresh one.
    send(
        &mut ws,
        serde_json::json!({"id": 13, "op": "enqueue", "service": "tidal", "track_id": "33348478"}),
    )
    .await;
    next_matching(&mut ws, |v| {
        v["type"] == "snapshot" && v["snapshot"]["queue"]["len"] == 2
    })
    .await;
    send(&mut ws, serde_json::json!({"id": 14, "op": "queue"})).await;
    let twice = next_matching(&mut ws, |v| v["id"] == 14).await;
    let tracks = twice["result"]["tracks"].as_array().expect("tracks");
    assert_eq!(tracks[0]["id"], tracks[1]["id"]);

    // 6. settings: replaced whole, read back, and a stale key is refused, not silently ignored.
    send(
        &mut ws,
        serde_json::json!({"id": 10, "op": "set_settings",
            "settings": {"outputs": {"Tunes": {"mode": "standard"}}}}),
    )
    .await;
    let set = next_matching(&mut ws, |v| v["id"] == 10).await;
    assert_eq!(set["ok"], true);
    send(&mut ws, serde_json::json!({"id": 11, "op": "settings"})).await;
    let got = next_matching(&mut ws, |v| v["id"] == 11).await;
    assert_eq!(
        got["result"]["settings"]["outputs"]["Tunes"]["mode"],
        "standard"
    );
    send(
        &mut ws,
        serde_json::json!({"id": 12, "op": "set_settings", "settings": {"crossfade": 3}}),
    )
    .await;
    let stale = next_matching(&mut ws, |v| v["type"] == "reply" && v["ok"] == false).await;
    assert!(
        stale["error"].as_str().unwrap().contains("unknown field"),
        "{stale}"
    );

    // 7. a malformed frame is a non-fatal error reply (no id echoed).
    send(&mut ws, serde_json::json!({"op": "nonsense"})).await;
    let err = next_matching(&mut ws, |v| v["type"] == "reply" && v["ok"] == false).await;
    assert!(err["error"].as_str().unwrap().contains("bad request"));

    // 8. an unknown login method is a clean error, not a panic.
    send(
        &mut ws,
        serde_json::json!({"id": 5, "op": "connect", "method": "spotify.web"}),
    )
    .await;
    let unknown = next_matching(&mut ws, |v| v["id"] == 5).await;
    assert_eq!(unknown["ok"], false);
    assert!(unknown["error"].as_str().unwrap().contains("spotify.web"));
}
