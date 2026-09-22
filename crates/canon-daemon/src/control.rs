//! `canon control` — an interactive keypress client over the ws control plane (yak
//! canon-bb21). The hands-on way to stress-test transport + queue + seek: it enqueues
//! any track ids you pass, shows a live status line, and turns single keypresses into
//! control commands.
//!
//! Keys:
//!
//! ```text
//! space play/pause   n next   p prev   [ seek -10s   ] seek +10s
//! + / - volume       m mute   s stop   c clear       1-9 enqueue palette[n]
//! q / Esc quit
//! ```

use std::io::Write;

use crossterm::event::{Event, EventStream, KeyCode, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Restores the terminal out of raw mode on drop, even on error/panic.
struct RawGuard;
impl Drop for RawGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        // Move to a fresh line so the shell prompt isn't glued to the status line.
        print!("\r\n");
        let _ = std::io::stdout().flush();
    }
}

pub async fn run(addr: &str, palette: Vec<String>) -> Result<(), BoxError> {
    let url = format!("ws://{addr}/ws");
    let (ws, _resp) = tokio_tungstenite::connect_async(&url).await?;
    let (mut write, mut read) = ws.split();

    // Enqueue whatever the user passed, so there's something to drive.
    for id in &palette {
        write.send(Message::Text(enqueue(id))).await?;
    }

    println!("connected to {url}");
    println!(
        "keys: [space] play/pause  n next  p prev  [ / ] seek 10s  + / - vol  m mute  s stop  c clear  1-9 enqueue  q quit"
    );
    if !palette.is_empty() {
        println!("palette: {}", palette.join(", "));
    }
    println!();

    enable_raw_mode()?;
    let _guard = RawGuard;
    let mut keys = EventStream::new();

    // Latest known state, for context-sensitive keys (play/pause toggle, relative seek).
    let mut state = String::from("idle");
    let mut position_ms: u64 = 0;
    let mut volume: f32 = 1.0;
    let mut muted = false;

    loop {
        tokio::select! {
            message = read.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
                            && value["type"] == "snapshot"
                        {
                            let snap = &value["snapshot"];
                            state = snap["state"].as_str().unwrap_or("?").to_string();
                            position_ms = snap["position_ms"].as_u64().unwrap_or(0);
                            volume = snap["volume"].as_f64().unwrap_or(1.0) as f32;
                            muted = snap["muted"].as_bool().unwrap_or(false);
                            render(snap, muted);
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
            event = keys.next() => {
                let Some(Ok(Event::Key(key))) = event else { continue; };
                // Ctrl-C quits too.
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    break;
                }
                let outgoing = match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char(' ') => Some(if state == "playing" { op("pause") } else { op("play") }),
                    KeyCode::Char('n') => Some(op("next")),
                    KeyCode::Char('p') => Some(op("previous")),
                    KeyCode::Char(']') | KeyCode::Right => Some(seek(position_ms.saturating_add(10_000))),
                    KeyCode::Char('[') | KeyCode::Left => Some(seek(position_ms.saturating_sub(10_000))),
                    KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Up => {
                        volume = (volume + 0.1).min(1.0);
                        Some(set_volume(volume))
                    }
                    KeyCode::Char('-') | KeyCode::Down => {
                        volume = (volume - 0.1).max(0.0);
                        Some(set_volume(volume))
                    }
                    KeyCode::Char('m') => { muted = !muted; Some(set_muted(muted)) }
                    KeyCode::Char('s') => Some(op("stop")),
                    KeyCode::Char('c') => Some(op("clear")),
                    KeyCode::Char(d @ '1'..='9') => {
                        let idx = d as usize - '1' as usize;
                        palette.get(idx).map(|id| enqueue(id))
                    }
                    _ => None,
                };
                if let Some(text) = outgoing
                    && write.send(Message::Text(text)).await.is_err()
                {
                    break;
                }
            }
        }
    }

    let _ = write.send(Message::Close(None)).await;
    Ok(())
}

/// Draw the one-line live status (raw mode: \r to line start, clear to end of line).
fn render(snap: &serde_json::Value, muted: bool) {
    let state = snap["state"].as_str().unwrap_or("?");
    let pos = snap["position_ms"].as_u64().unwrap_or(0);
    let dur = snap["duration_ms"].as_u64();
    let vol = snap["volume"].as_f64().unwrap_or(1.0);
    let title = snap["track"]["meta"]["title"].as_str().unwrap_or("—");
    let artists = snap["track"]["meta"]["artists"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let dur_str = dur.map(clock).unwrap_or_else(|| "--:--".into());
    let vol_str = if muted {
        "muted".to_string()
    } else {
        format!("{:.0}%", vol * 100.0)
    };
    let queue_str = match (
        snap["queue"]["index"].as_u64(),
        snap["queue"]["len"].as_u64(),
    ) {
        (Some(index), Some(len)) => format!("  [{}/{}]", index + 1, len),
        _ => String::new(),
    };
    print!(
        "\r\x1b[2K{state:8} {title} — {artists}   {}/{}   vol {vol_str}{queue_str}",
        clock(pos),
        dur_str
    );
    let _ = std::io::stdout().flush();
}

fn clock(ms: u64) -> String {
    let secs = ms / 1000;
    format!("{}:{:02}", secs / 60, secs % 60)
}

fn op(name: &str) -> String {
    serde_json::json!({ "op": name }).to_string()
}
fn seek(position_ms: u64) -> String {
    serde_json::json!({ "op": "seek", "position_ms": position_ms }).to_string()
}
fn set_volume(volume: f32) -> String {
    serde_json::json!({ "op": "set_volume", "volume": volume }).to_string()
}
fn set_muted(muted: bool) -> String {
    serde_json::json!({ "op": "set_muted", "muted": muted }).to_string()
}
fn enqueue(track_id: &str) -> String {
    serde_json::json!({ "op": "enqueue", "service": "tidal", "track_id": track_id }).to_string()
}
