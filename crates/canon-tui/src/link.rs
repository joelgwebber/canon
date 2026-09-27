//! The WebSocket to the daemon, as two channels: requests out, server messages in.
//!
//! A task owns the socket, so the app never awaits the network: it queues requests and applies
//! whatever arrives, in the order it arrived. A frame the client can't read is reported rather
//! than dropped silently, since it means the daemon speaks a protocol this client doesn't.

use canon_api::{ClientEnvelope, ServerMessage};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

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

/// Connect to the daemon's control plane at `addr` (`host:port`).
///
/// # Errors
/// The daemon couldn't be reached.
pub async fn connect(addr: &str) -> Result<Link, String> {
    let url = format!("ws://{addr}/ws");
    let (socket, _) = tokio_tungstenite::connect_async(&url)
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
