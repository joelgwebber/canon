//! `canon control` — a line-oriented client for a running daemon's control plane.
//!
//! One command per line on stdin, so the same client serves a human at a terminal and a
//! script (or an agent) driving canon through a pipe:
//!
//! ```text
//! printf 'sinks\nsink Tunes\nenqueue 520285418\nsleep 40\n' | canon control
//! ```
//!
//! Deliberately not a TUI. Raw mode buys single-keypress transport at the cost of being
//! undriveable by anything that is not a human with a terminal, and on-metal verification
//! of the network sinks needs exactly that: select a renderer, play something, watch the
//! published position for a while, with the output in a file afterwards. A real TUI can
//! come later on top of the same control plane.
//!
//! ```text
//! play | pause | stop | next | prev | clear   transport
//! seek 90 | seek 1:30 | seek +10 | seek -10   absolute (s or m:ss) or relative
//! vol 60 | vol +10 | mute | unmute            volume, as a percentage
//! enqueue <track-id>                          append to the server-owned queue
//! sinks | sink <name[@protocol]-or-id>        list outputs, select one by name
//! queue                                       list the queue, marking the current entry
//! status | sleep <secs> | help | quit
//! ```

use std::time::{Duration, Instant};

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type BoxError = Box<dyn std::error::Error + Send + Sync>;
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// How often a position-only refresh is echoed while playing. Snapshots arrive several
/// times a second; that is the right rate for a progress bar and the wrong one for a log.
const POSITION_ECHO: Duration = Duration::from_secs(1);

pub async fn run(
    addr: &str,
    palette: Vec<String>,
    json_out: bool,
    quiet: bool,
) -> Result<(), BoxError> {
    let url = format!("ws://{addr}/ws");
    let (ws, _resp) = tokio_tungstenite::connect_async(&url).await?;
    let (write, read) = ws.split();
    let mut client = Client {
        write,
        read,
        json_out,
        quiet,
        next_id: 1,
        state: String::from("idle"),
        position_ms: 0,
        volume: 1.0,
        seq: u64::MAX,
        last_snapshot: None,
        last_echo: Instant::now() - POSITION_ECHO,
    };

    if !json_out {
        println!("connected to {url} — `help` for commands, `quit` to exit");
    }
    for id in &palette {
        client.send(enqueue(id)).await?;
    }

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    loop {
        tokio::select! {
            line = lines.next_line() => match line? {
                Some(line) => match client.command(line.trim()).await? {
                    Flow::Continue => {}
                    Flow::Quit => break,
                },
                None => break, // stdin closed: a piped script has run out of commands
            },
            message = client.read.next() => match message {
                Some(Ok(Message::Text(text))) => client.absorb(&text),
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(e.into()),
            },
        }
    }

    let _ = client.write.send(Message::Close(None)).await;
    Ok(())
}

enum Flow {
    Continue,
    Quit,
}

struct Client {
    write: SplitSink<Socket, Message>,
    read: SplitStream<Socket>,
    json_out: bool,
    quiet: bool,
    next_id: u64,
    state: String,
    position_ms: u64,
    volume: f32,
    /// Last snapshot `seq` seen, so transitions can be told from position refreshes.
    seq: u64,
    last_snapshot: Option<Value>,
    last_echo: Instant,
}

impl Client {
    async fn send(&mut self, frame: Value) -> Result<(), BoxError> {
        self.write.send(Message::Text(frame.to_string())).await?;
        Ok(())
    }

    /// Send a request and wait for its reply, still absorbing snapshots that arrive first.
    async fn request(&mut self, mut frame: Value) -> Result<Option<Value>, BoxError> {
        let id = self.next_id;
        self.next_id += 1;
        frame["id"] = json!(id);
        self.send(frame).await?;

        while let Some(message) = self.read.next().await {
            let Ok(Message::Text(text)) = message else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if value["type"] == "reply" && value["id"] == json!(id) {
                if self.json_out {
                    println!("{text}");
                }
                if value["ok"] == json!(false) {
                    let why = value["error"].as_str().unwrap_or("request failed");
                    eprintln!("error: {why}");
                    return Ok(None);
                }
                return Ok(Some(value["result"].clone()));
            }
            self.absorb(&text);
        }
        Err("connection closed while awaiting a reply".into())
    }

    /// Fold one server frame into the local view, echoing what is worth seeing.
    fn absorb(&mut self, text: &str) {
        if self.json_out {
            println!("{text}");
        }
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            return;
        };
        if value["type"] != "snapshot" {
            return;
        }
        let snap = &value["snapshot"];
        let seq = snap["seq"].as_u64().unwrap_or(0);
        let transition = seq != self.seq;

        self.seq = seq;
        self.last_snapshot = Some(snap.clone());
        self.state = snap["state"].as_str().unwrap_or("?").to_string();
        self.position_ms = snap["position_ms"].as_u64().unwrap_or(0);
        #[allow(clippy::cast_possible_truncation)]
        {
            self.volume = snap["volume"].as_f64().unwrap_or(1.0) as f32;
        }

        if self.json_out || self.quiet {
            return;
        }
        // A transition is news and always prints. A position refresh is not, so it is
        // rate-limited — and only while playing, so an idle client stays silent.
        let echo =
            transition || (self.state == "playing" && self.last_echo.elapsed() >= POSITION_ECHO);
        if echo {
            println!("{}", describe(snap));
            self.last_echo = Instant::now();
        }
    }

    async fn command(&mut self, line: &str) -> Result<Flow, BoxError> {
        let (verb, rest) = match line.split_once(char::is_whitespace) {
            Some((verb, rest)) => (verb, rest.trim()),
            None => (line, ""),
        };
        match verb {
            "" | "#" => {}
            "quit" | "exit" | "q" => return Ok(Flow::Quit),
            "help" | "?" => print!("{HELP}"),

            "play" => self.send(op("play")).await?,
            "pause" => self.send(op("pause")).await?,
            "stop" => self.send(op("stop")).await?,
            "next" => self.send(op("next")).await?,
            "prev" | "previous" => self.send(op("previous")).await?,
            "clear" => self.send(op("clear")).await?,
            "mute" => self.send(json!({"op": "set_muted", "muted": true})).await?,
            "unmute" => {
                self.send(json!({"op": "set_muted", "muted": false}))
                    .await?
            }

            "seek" => match parse_seek(rest, self.position_ms) {
                Some(position_ms) => {
                    self.send(json!({"op": "seek", "position_ms": position_ms}))
                        .await?;
                }
                None => eprintln!("usage: seek <secs | m:ss | +secs | -secs>"),
            },
            "vol" | "volume" => match parse_volume(rest, self.volume) {
                Some(volume) => {
                    self.send(json!({"op": "set_volume", "volume": volume}))
                        .await?
                }
                None => eprintln!("usage: vol <0-100 | +n | -n>"),
            },
            "enqueue" | "add" => {
                if rest.is_empty() {
                    eprintln!("usage: enqueue <track-id>");
                } else {
                    self.send(enqueue(rest)).await?;
                }
            }

            "sinks" => self.list_sinks().await?,
            "queue" => self.show_queue().await?,
            "sink" => {
                if rest.is_empty() {
                    eprintln!("usage: sink <name[@cast|@dlna] | id>");
                } else {
                    self.select_sink(rest).await?;
                }
            }

            // The server pushes a snapshot on every change, so the newest one in hand is
            // current; asking for it again would only add a round trip.
            "status" => match &self.last_snapshot {
                Some(snapshot) => println!("{}", describe(snapshot)),
                None => println!("no snapshot yet"),
            },
            // Keeps absorbing (and echoing) server frames while it waits, so a script can
            // say "play this and show me the next 40 seconds".
            "sleep" | "watch" => match rest.parse::<f64>() {
                Ok(secs) => self.pump_for(Duration::from_secs_f64(secs)).await,
                Err(_) => eprintln!("usage: sleep <secs>"),
            },

            other => eprintln!("unknown command: {other} (try `help`)"),
        }
        Ok(Flow::Continue)
    }

    async fn show_queue(&mut self) -> Result<(), BoxError> {
        let Some(result) = self.request(op("queue")).await? else {
            return Ok(());
        };
        if self.json_out {
            return Ok(()); // the raw reply was already printed
        }
        let index = result["index"].as_u64();
        let tracks = result["tracks"].as_array().cloned().unwrap_or_default();
        if tracks.is_empty() {
            println!("queue is empty");
        }
        for (i, track) in tracks.iter().enumerate() {
            let marker = if index == Some(i as u64) { ">" } else { " " };
            let title = track["meta"]["title"].as_str().filter(|t| !t.is_empty());
            let source = track["sources"][0]["id"].as_str().unwrap_or("?");
            println!(
                "{marker} {:>3}. {}",
                i + 1,
                title.map_or_else(|| format!("(tidal {source})"), str::to_string)
            );
        }
        Ok(())
    }

    async fn list_sinks(&mut self) -> Result<(), BoxError> {
        let Some(result) = self.request(op("list_sinks")).await? else {
            return Ok(());
        };
        if self.json_out {
            return Ok(()); // the raw reply was already printed
        }
        for sink in result["sinks"].as_array().into_iter().flatten() {
            // The preferred protocol first; any others a speaker also answers on after it, which
            // is what `sink <name>@<protocol>` selects.
            let others: Vec<&str> = sink["protocols"]
                .as_array()
                .into_iter()
                .flatten()
                .skip(1)
                .filter_map(|endpoint| endpoint["kind"].as_str())
                .collect();
            let also = if others.is_empty() {
                String::new()
            } else {
                format!("  (also {})", others.join(", "))
            };
            println!(
                "{:<10} {:<24} {}{also}",
                sink["kind"].as_str().unwrap_or("?"),
                sink["name"].as_str().unwrap_or("?"),
                sink["id"].as_str().unwrap_or("?"),
            );
        }
        Ok(())
    }

    /// Select an output by id, or by a case-insensitive prefix of its name, optionally pinned to
    /// one of its protocols: `Tunes` takes the speaker's preferred protocol, `Tunes@dlna` drives
    /// it over DLNA.
    ///
    /// Renderer ids are mDNS service names and UPnP UDNs — unguessable and unmemorable — so
    /// selecting by the name the device advertises ("Tunes") is the only usable form from a script.
    async fn select_sink(&mut self, wanted: &str) -> Result<(), BoxError> {
        let Some(result) = self.request(op("list_sinks")).await? else {
            return Ok(());
        };
        let sinks: Vec<&Value> = result["sinks"].as_array().into_iter().flatten().collect();
        let (name, protocol) = match wanted.rsplit_once('@') {
            Some((name, protocol)) => (name, Some(protocol_kind(protocol))),
            None => (wanted, None),
        };
        let matched = sinks
            .iter()
            .find(|s| s["id"].as_str() == Some(wanted))
            .or_else(|| {
                sinks.iter().find(|s| {
                    s["name"]
                        .as_str()
                        .is_some_and(|n| n.to_lowercase().starts_with(&name.to_lowercase()))
                })
            });
        let Some(sink) = matched else {
            eprintln!("no sink matching {wanted:?} — `sinks` to list what was discovered");
            return Ok(());
        };
        let id = match protocol {
            None => sink["id"].as_str().unwrap_or_default().to_string(),
            Some(kind) => {
                let endpoint = sink["protocols"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|endpoint| endpoint["kind"].as_str() == Some(kind));
                let Some(endpoint) = endpoint else {
                    eprintln!("{name:?} is not reachable over {kind}");
                    return Ok(());
                };
                endpoint["id"].as_str().unwrap_or_default().to_string()
            }
        };
        if !self.json_out {
            println!("selecting {}", sink["name"].as_str().unwrap_or(&id));
        }
        self.send(json!({"op": "select_sink", "sink": id})).await?;
        Ok(())
    }

    async fn pump_for(&mut self, duration: Duration) {
        let deadline = tokio::time::Instant::now() + duration;
        loop {
            tokio::select! {
                () = tokio::time::sleep_until(deadline) => return,
                message = self.read.next() => match message {
                    Some(Ok(Message::Text(text))) => self.absorb(&text),
                    Some(Ok(_)) => {}
                    _ => return,
                },
            }
        }
    }
}

const HELP: &str = "\
  play | pause | stop | next | prev | clear   transport
  seek 90 | seek 1:30 | seek +10 | seek -10   absolute (s or m:ss) or relative
  vol 60 | vol +10 | mute | unmute            volume, as a percentage
  enqueue <track-id>                          append to the server-owned queue
  sinks | sink <name[@protocol]-or-id>        list outputs, select one by name
  queue                                       list the queue, marking the current entry
  status | sleep <secs> | help | quit
";

fn describe(snap: &Value) -> String {
    let state = snap["state"].as_str().unwrap_or("?");
    let position = clock(snap["position_ms"].as_u64().unwrap_or(0));
    let duration = snap["duration_ms"]
        .as_u64()
        .map_or_else(|| "--:--".to_string(), clock);
    let title = snap["track"]["meta"]["title"].as_str().unwrap_or("—");
    let artists = snap["track"]["meta"]["artists"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(Value::as_str)
        .unwrap_or("");
    let queue = match (
        snap["queue"]["index"].as_u64(),
        snap["queue"]["len"].as_u64(),
    ) {
        (Some(index), Some(len)) if len > 0 => format!("  [{}/{}]", index + 1, len),
        _ => String::new(),
    };
    format!("{state:8} {position}/{duration}  {title} — {artists}{queue}")
}

fn clock(ms: u64) -> String {
    let secs = ms / 1000;
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// `90` / `1:30` absolute, `+10` / `-10` relative to `current_ms`. Seconds throughout.
fn parse_seek(spec: &str, current_ms: u64) -> Option<u64> {
    if let Some(delta) = spec.strip_prefix('+') {
        let secs: f64 = delta.trim().parse().ok()?;
        return Some(current_ms.saturating_add(to_ms(secs)?));
    }
    if let Some(delta) = spec.strip_prefix('-') {
        let secs: f64 = delta.trim().parse().ok()?;
        return Some(current_ms.saturating_sub(to_ms(secs)?));
    }
    if let Some((minutes, seconds)) = spec.split_once(':') {
        let minutes: u64 = minutes.trim().parse().ok()?;
        let seconds: f64 = seconds.trim().parse().ok()?;
        return Some(minutes * 60_000 + to_ms(seconds)?);
    }
    to_ms(spec.parse().ok()?)
}

fn to_ms(secs: f64) -> Option<u64> {
    if secs < 0.0 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Some((secs * 1000.0) as u64)
}

/// `60` absolute, `+10` / `-10` relative. Percentages in, 0.0–1.0 out.
fn parse_volume(spec: &str, current: f32) -> Option<f32> {
    let spec = spec.trim();
    let relative = spec.starts_with('+') || spec.starts_with('-');
    // A sign is part of the number, so parsing the whole spec already carries it.
    let percent: f32 = spec.parse().ok()?;
    let volume = if relative {
        current + percent / 100.0
    } else {
        percent / 100.0
    };
    Some(volume.clamp(0.0, 1.0))
}

/// The wire name of a protocol as a user types it (`cast` is the everyday word for Chromecast).
fn protocol_kind(typed: &str) -> &str {
    match typed.to_ascii_lowercase().as_str() {
        "cast" | "chromecast" => "chromecast",
        "dlna" | "upnp" => "dlna",
        _ => typed,
    }
}

fn op(name: &str) -> Value {
    json!({ "op": name })
}

fn enqueue(track_id: &str) -> Value {
    json!({ "op": "enqueue", "service": "tidal", "track_id": track_id })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seek_accepts_absolute_seconds_and_minutes() {
        assert_eq!(parse_seek("90", 0), Some(90_000));
        assert_eq!(parse_seek("1:30", 0), Some(90_000));
        assert_eq!(parse_seek("2:05.5", 0), Some(125_500));
    }

    #[test]
    fn seek_accepts_relative_offsets() {
        assert_eq!(parse_seek("+10", 30_000), Some(40_000));
        assert_eq!(parse_seek("-10", 30_000), Some(20_000));
    }

    /// Seeking back past the start of a track must clamp, not wrap to the end of time.
    #[test]
    fn relative_seek_clamps_at_zero() {
        assert_eq!(parse_seek("-30", 10_000), Some(0));
    }

    #[test]
    fn seek_rejects_nonsense() {
        assert_eq!(parse_seek("", 0), None);
        assert_eq!(parse_seek("soon", 0), None);
    }

    #[test]
    fn volume_is_a_percentage() {
        assert_eq!(parse_volume("60", 1.0), Some(0.6));
        assert_eq!(parse_volume("+10", 0.5), Some(0.6));
        assert_eq!(parse_volume("-10", 0.5), Some(0.4));
    }

    #[test]
    fn volume_clamps_to_the_usable_range() {
        assert_eq!(parse_volume("+50", 0.8), Some(1.0));
        assert_eq!(parse_volume("-50", 0.2), Some(0.0));
        assert_eq!(parse_volume("300", 0.5), Some(1.0));
    }
}
