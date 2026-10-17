//! The WebSocket to the daemon, as two channels: requests out, server messages in.
//!
//! A task owns the socket, so the app never awaits the network: it queues requests and applies
//! whatever arrives, in the order it arrived. A frame the client can't read is reported rather
//! than dropped silently, since it means the daemon speaks a protocol this client doesn't.

use canon_core::PlayerId;
use canon_protocol::{ClientEnvelope, ServerMessage};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::handshake::client::Request;

/// What the connection reports.
#[derive(Debug)]
pub enum Incoming {
    Message(Box<ServerMessage>),
    /// A frame that didn't parse as a server message.
    Unreadable(String),
    /// The connection is gone, and why.
    Closed(String),
}

/// A live connection: send requests on `outgoing`, receive on `incoming`.
pub struct Link {
    pub outgoing: mpsc::UnboundedSender<ClientEnvelope>,
    pub incoming: mpsc::UnboundedReceiver<Incoming>,
}

/// How a client reaches a server: where it is, which player it drives, and the player's token
/// when the server wants one (canon-65d2).
#[derive(Debug, Clone, Default)]
pub struct Endpoint {
    /// `host:port`.
    pub addr: String,
    /// The player to drive; the server's default when `None` (or the token's, when one is given).
    pub player: Option<PlayerId>,
    /// A paired player's token, sent as `Authorization: Bearer`.
    pub token: Option<String>,
}

impl Endpoint {
    /// The WebSocket handshake request for this endpoint.
    ///
    /// # Errors
    /// The address doesn't make a URL, or the token doesn't make a header.
    pub fn request(&self) -> Result<Request, String> {
        let url = match &self.player {
            Some(player) => format!("ws://{}/ws?player={player}", self.addr),
            None => format!("ws://{}/ws", self.addr),
        };
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|e| format!("{url}: {e}"))?;
        if let Some(token) = &self.token {
            let value = format!("Bearer {token}")
                .parse()
                .map_err(|_| "the token isn't usable in a header".to_owned())?;
            request.headers_mut().insert("authorization", value);
        }
        Ok(request)
    }

    /// The WebSocket URL, for messages.
    #[must_use]
    pub fn url(&self) -> String {
        format!("ws://{}/ws", self.addr)
    }
}

/// Connect to the daemon's control plane at `endpoint`.
///
/// # Errors
/// The daemon couldn't be reached, or refused the player or its token.
pub async fn connect(endpoint: &Endpoint) -> Result<Link, String> {
    let url = endpoint.url();
    let (socket, _) = tokio_tungstenite::connect_async(endpoint.request()?)
        .await
        .map_err(|e| format!("connect to {url}: {e}"))?;
    let (mut write, mut read) = socket.split();
    let (outgoing, mut requests) = mpsc::unbounded_channel::<ClientEnvelope>();
    let (deliver, incoming) = mpsc::unbounded_channel();

    tokio::spawn(async move {
        while let Some(request) = requests.recv().await {
            let Ok(text) = serde_json::to_string(&request) else {
                continue;
            };
            if write.send(Message::Text(text)).await.is_err() {
                break;
            }
        }
        let _ = write.close().await;
    });
    tokio::spawn(async move {
        let why = loop {
            match read.next().await {
                Some(Ok(Message::Text(text))) => {
                    let incoming = match serde_json::from_str::<ServerMessage>(&text) {
                        Ok(message) => Incoming::Message(Box::new(message)),
                        Err(e) => Incoming::Unreadable(format!("{e}: {text}")),
                    };
                    if deliver.send(incoming).is_err() {
                        return;
                    }
                }
                Some(Ok(Message::Close(_))) | None => {
                    break "the daemon closed the connection".into();
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => break e.to_string(),
            }
        };
        let _ = deliver.send(Incoming::Closed(why));
    });
    Ok(Link { outgoing, incoming })
}
