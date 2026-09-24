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
//! play|add|playnext <item>...                 queue now, at the end, or next
//! jump N | rm N | mv FROM TO | shuffle | repeat off|all|one
//! search <words> | album <item> | artist <item> | playfrom #n
//! save [item] | unsave [item] | library [tracks|albums|artists] [words]
//! radio [item] | similar <artist> | mixes | mix #n | autoplay on|off
//! pl [list] | pl new|fromqueue <name> | pl use #n | pl show|play|add|rm|mv|rename|delete
//! services | connect <method> [redirect-url] | disconnect <method> | spotify-app <client-id>
//! import [service]
//! sinks | sink <name[@protocol]-or-id>        list outputs, select one by name
//! queue                                       list the queue, marking the current entry
//! settings | mode <output> flow|standard       show settings; set how an output gets tracks
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
        listing: Vec::new(),
        playlist: None,
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
    /// The items of the last numbered listing, which `#n` names.
    listing: Vec<Value>,
    /// The playlist `pl` verbs act on: its id and name.
    playlist: Option<(String, String)>,
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

            "play" if rest.is_empty() => self.send(op("play")).await?,
            "play" => self.queue_add(rest, "now").await?,
            "add" | "enqueue" => self.queue_add(rest, "end").await?,
            "playnext" | "next-up" => self.queue_add(rest, "next").await?,
            "jump" => match position(rest) {
                Some(index) => {
                    self.command_request(json!({"op": "jump", "index": index}))
                        .await?
                }
                None => eprintln!("usage: jump <queue position>"),
            },
            "rm" | "remove" => match position(rest) {
                Some(index) => {
                    self.command_request(json!({"op": "remove", "index": index}))
                        .await?
                }
                None => eprintln!("usage: rm <queue position>"),
            },
            "mv" | "move" => match rest
                .split_once(' ')
                .map(|(a, b)| (position(a), position(b)))
            {
                Some((Some(from), Some(to))) => {
                    self.command_request(json!({"op": "move", "from": from, "to": to}))
                        .await?
                }
                _ => eprintln!("usage: mv <from position> <to position>"),
            },
            "search" | "find" => {
                if rest.is_empty() {
                    eprintln!("usage: search <words>");
                } else {
                    self.search(rest).await?;
                }
            }
            "album" | "artist" => match item(rest, &self.listing) {
                Some(mut item) => {
                    // A bare service id here names an album or artist, not a track.
                    if item.get("service").is_some() {
                        item["kind"] = json!(verb);
                    }
                    if verb == "album" {
                        self.show_album(item).await?;
                    } else {
                        self.show_artist(item).await?;
                    }
                }
                None => eprintln!("usage: {verb} <#n | canon id | tidal id>"),
            },
            "playfrom" => match rest.strip_prefix('#').and_then(|n| n.parse::<usize>().ok()) {
                Some(n) if (1..=self.listing.len()).contains(&n) => {
                    let items = self.listing.clone();
                    self.command_request(
                        json!({"op": "queue_add", "items": items, "at": "now", "start": n - 1}),
                    )
                    .await?;
                }
                _ => eprintln!("usage: playfrom #n  (plays the whole last listing from entry n)"),
            },
            "radio" => {
                let seed = if rest.is_empty() {
                    self.current_track()
                } else {
                    item(rest, &self.listing)
                };
                match seed {
                    Some(seed) => {
                        self.show_tracks("radio", json!({"op": "radio", "item": seed}))
                            .await?
                    }
                    None => eprintln!("usage: radio [item]  (no item: the current track)"),
                }
            }
            "similar" => match item(rest, &self.listing) {
                Some(mut artist) => {
                    if artist.get("service").is_some() {
                        artist["kind"] = json!("artist");
                    }
                    let Some(found) = self
                        .request(json!({"op": "similar", "item": artist}))
                        .await?
                    else {
                        return Ok(Flow::Continue);
                    };
                    let mut listing = Numbered::default();
                    listing.section("similar artists", &found["artists"], |a| {
                        a["name"].as_str().unwrap_or("?").to_string()
                    });
                    self.show(listing);
                }
                None => eprintln!("usage: similar <artist item>"),
            },
            "mixes" => {
                let Some(found) = self.request(op("mixes")).await? else {
                    return Ok(Flow::Continue);
                };
                let mut listing = Numbered::default();
                for mix in found["mixes"].as_array().into_iter().flatten() {
                    listing.entry(
                        json!({"service": mix["service"], "mix": mix["mix"]}),
                        format!(
                            "{} — {}",
                            mix["name"].as_str().unwrap_or("?"),
                            mix["description"].as_str().unwrap_or("")
                        ),
                    );
                }
                self.show(listing);
            }
            "mix" => match item(rest, &self.listing) {
                Some(mix) if mix.get("mix").is_some() => {
                    let request =
                        json!({"op": "mix", "service": mix["service"], "mix": mix["mix"]});
                    self.show_tracks("mix", request).await?;
                }
                _ => eprintln!("usage: mix #n  (`mixes` lists them)"),
            },
            "autoplay" => match rest {
                "on" | "off" => self.set_autoplay(rest == "on").await?,
                _ => eprintln!("usage: autoplay on|off"),
            },
            "spotify-app" => match rest {
                "" => eprintln!("usage: spotify-app <client-id>"),
                id => self.set_spotify_app(id).await?,
            },
            "save" | "unsave" => {
                // No item: the track playing now.
                let target = if rest.is_empty() {
                    self.current_track()
                } else {
                    item(rest, &self.listing)
                };
                match target {
                    Some(target) => {
                        self.command_request(json!({"op": verb, "item": target}))
                            .await?;
                    }
                    None => eprintln!("usage: {verb} [item]  (no item: the current track)"),
                }
            }
            "library" | "lib" => {
                let (kind, query) = match rest.split_once(' ').unwrap_or((rest, "")) {
                    ("tracks" | "track" | "", query) => ("track", query),
                    ("albums" | "album", query) => ("album", query),
                    ("artists" | "artist", query) => ("artist", query),
                    _ => ("track", rest),
                };
                self.show_library(kind, query.trim()).await?;
            }
            "pl" | "playlist" => self.playlist_command(rest).await?,
            "services" => self.show_services().await?,
            "connect" => match rest.split_once(' ') {
                Some((method, redirect)) => {
                    let request = json!({"op": "connect_complete", "method": method,
                                         "redirect": redirect.trim()});
                    if let Some(done) = self.request(request).await? {
                        println!("{method}: {}", done["status"].as_str().unwrap_or("?"));
                    }
                }
                None if !rest.is_empty() => self.connect(rest).await?,
                None => {
                    eprintln!("usage: connect <method> [redirect-url]  (`services` lists methods)")
                }
            },
            "disconnect" if !rest.is_empty() => {
                self.command_request(json!({"op": "disconnect", "method": rest}))
                    .await?;
            }
            "import" => {
                let mut request = op("import");
                if !rest.is_empty() {
                    request["service"] = json!(rest);
                }
                if let Some(report) = self.request(request).await?
                    && !self.json_out
                {
                    println!(
                        "imported {} tracks, {} albums, {} artists, {} playlists",
                        report["tracks"], report["albums"], report["artists"], report["playlists"]
                    );
                }
            }
            "shuffle" => self.command_request(op("shuffle")).await?,
            "repeat" => match rest {
                "off" | "all" | "one" => {
                    self.command_request(json!({"op": "repeat", "mode": rest}))
                        .await?
                }
                _ => eprintln!("usage: repeat off|all|one"),
            },
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

            "sinks" => self.list_sinks().await?,
            "queue" => self.show_queue().await?,
            "settings" => self.show_settings().await?,
            "mode" => match rest.rsplit_once(' ') {
                Some((output, mode)) if matches!(mode, "flow" | "standard") => {
                    self.set_mode(output.trim(), mode).await?;
                }
                _ => eprintln!("usage: mode <output> flow|standard"),
            },
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

    /// Queue what `spec` names (see [`item`]) `at` the end, next, or now.
    async fn queue_add(&mut self, spec: &str, at: &str) -> Result<(), BoxError> {
        let items: Option<Vec<Value>> = spec
            .split_whitespace()
            .map(|word| item(word, &self.listing))
            .collect();
        match items {
            Some(items) if !items.is_empty() => {
                self.command_request(json!({"op": "queue_add", "items": items, "at": at}))
                    .await
            }
            _ => {
                eprintln!(
                    "usage: play|add|playnext <item>...  (a Tidal track id, album:<id>, a canon \
                     id, or #n from the last listing)"
                );
                Ok(())
            }
        }
    }

    async fn search(&mut self, query: &str) -> Result<(), BoxError> {
        let Some(found) = self
            .request(json!({"op": "search", "query": query}))
            .await?
        else {
            return Ok(());
        };
        let mut listing = Numbered::default();
        listing.section("tracks", &found["tracks"], track_line);
        listing.section("albums", &found["albums"], album_line);
        listing.section("artists", &found["artists"], |a| {
            a["name"].as_str().unwrap_or("?").to_string()
        });
        self.show(listing);
        Ok(())
    }

    async fn show_album(&mut self, item: Value) -> Result<(), BoxError> {
        let Some(detail) = self.request(json!({"op": "album", "item": item})).await? else {
            return Ok(());
        };
        let mut listing = Numbered::default();
        listing.heading(album_line(&detail["album"]));
        let tracks: Vec<Value> = detail["tracks"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|entry| entry["track"].clone())
            .collect();
        listing.section("", &Value::Array(tracks), track_line);
        self.show(listing);
        Ok(())
    }

    async fn show_artist(&mut self, item: Value) -> Result<(), BoxError> {
        let Some(detail) = self.request(json!({"op": "artist", "item": item})).await? else {
            return Ok(());
        };
        let mut listing = Numbered::default();
        listing.heading(detail["artist"]["name"].as_str().unwrap_or("?").to_string());
        listing.section("top tracks", &detail["top_tracks"], track_line);
        listing.section("releases", &detail["albums"], album_line);
        self.show(listing);
        Ok(())
    }

    /// `pl <verb> ...`: playlists, acting on the one last created or chosen with `pl use`.
    async fn playlist_command(&mut self, line: &str) -> Result<(), BoxError> {
        let (verb, rest) = line.split_once(' ').unwrap_or((line, ""));
        let rest = rest.trim();
        match verb {
            "" | "list" | "ls" => {
                let mut request = json!({"op": "library", "kind": "playlist"});
                if !rest.is_empty() {
                    request["query"] = json!(rest);
                }
                let Some(page) = self.request(request).await? else {
                    return Ok(());
                };
                let mut listing = Numbered::default();
                if page["total"] == 0 {
                    listing.heading("no playlists".to_string());
                }
                listing.section("playlists", &page["playlists"], |p| {
                    format!(
                        "{} ({} tracks)",
                        p["name"].as_str().unwrap_or("?"),
                        p["track_count"]
                    )
                });
                self.show(listing);
            }
            "new" | "fromqueue" if !rest.is_empty() => {
                let items = if verb == "fromqueue" {
                    let queue = self.request(op("queue")).await?.unwrap_or_default();
                    queue["tracks"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|t| json!({ "entity": t["id"] }))
                        .collect()
                } else {
                    Vec::new()
                };
                let request = json!({"op": "playlist_create", "name": rest, "items": items});
                if let Some(created) = self.request(request).await? {
                    self.choose_playlist(&created["playlist"]);
                }
            }
            "use" => match item(rest, &self.listing) {
                Some(found) if found.get("entity").is_some() => {
                    let request = json!({"op": "playlist", "playlist": found["entity"]});
                    if let Some(detail) = self.request(request).await? {
                        self.choose_playlist(&detail["playlist"]);
                    }
                }
                _ => eprintln!("usage: pl use <#n | playlist id>  (`pl` lists them)"),
            },
            _ => {
                let Some((id, _)) = self.playlist.clone() else {
                    eprintln!("no playlist chosen: `pl new <name>` or `pl use #n` first");
                    return Ok(());
                };
                self.chosen_playlist_command(&id, verb, rest).await?;
            }
        }
        Ok(())
    }

    async fn chosen_playlist_command(
        &mut self,
        id: &str,
        verb: &str,
        rest: &str,
    ) -> Result<(), BoxError> {
        match verb {
            "show" => {
                let request = json!({"op": "playlist", "playlist": id});
                let Some(detail) = self.request(request).await? else {
                    return Ok(());
                };
                let mut listing = Numbered::default();
                listing.heading(
                    detail["playlist"]["name"]
                        .as_str()
                        .unwrap_or("?")
                        .to_string(),
                );
                listing.section("", &detail["tracks"], track_line);
                self.show(listing);
            }
            "play" => {
                let request = json!({"op": "queue_add", "items": [{ "entity": id }], "at": "now"});
                self.command_request(request).await?;
            }
            "add" => {
                let items: Option<Vec<Value>> = rest
                    .split_whitespace()
                    .map(|word| item(word, &self.listing))
                    .collect();
                match items {
                    Some(items) if !items.is_empty() => {
                        let request = json!({"op": "playlist_add", "playlist": id, "items": items});
                        self.command_request(request).await?;
                    }
                    _ => eprintln!("usage: pl add <item>..."),
                }
            }
            "rm" => match position(rest) {
                Some(index) => {
                    let request = json!({"op": "playlist_remove", "playlist": id, "index": index});
                    self.command_request(request).await?;
                }
                None => eprintln!("usage: pl rm <position>"),
            },
            "mv" => match rest
                .split_once(' ')
                .map(|(a, b)| (position(a), position(b)))
            {
                Some((Some(from), Some(to))) => {
                    let request =
                        json!({"op": "playlist_move", "playlist": id, "from": from, "to": to});
                    self.command_request(request).await?;
                }
                _ => eprintln!("usage: pl mv <from> <to>"),
            },
            "rename" if !rest.is_empty() => {
                let request = json!({"op": "playlist_rename", "playlist": id, "name": rest});
                if self.request(request).await?.is_some() {
                    self.playlist = Some((id.to_string(), rest.to_string()));
                }
            }
            "delete" => {
                let request = json!({"op": "playlist_delete", "playlist": id});
                if self.request(request).await?.is_some() {
                    self.playlist = None;
                }
            }
            _ => eprintln!(
                "usage: pl [list [words]] | new <name> | fromqueue <name> | use <#n> | show | \
                 play | add <item>... | rm N | mv A B | rename <name> | delete"
            ),
        }
        Ok(())
    }

    fn choose_playlist(&mut self, playlist: &Value) {
        let id = playlist["id"].as_str().unwrap_or_default().to_string();
        let name = playlist["name"].as_str().unwrap_or("?").to_string();
        if !self.json_out {
            println!("playlist: {name} ({} tracks)", playlist["track_count"]);
        }
        self.playlist = Some((id, name));
    }

    /// Every service's login methods, what each grants, and how each connection stands.
    async fn show_services(&mut self) -> Result<(), BoxError> {
        let Some(found) = self.request(op("services")).await? else {
            return Ok(());
        };
        if self.json_out {
            return Ok(());
        }
        for service in found["services"].as_array().into_iter().flatten() {
            println!("{}", service["service"].as_str().unwrap_or("?"));
            for connection in service["connections"].as_array().into_iter().flatten() {
                let method = connection["id"].as_str().unwrap_or("?");
                let health = &connection["health"];
                let state = match health["state"].as_str() {
                    Some("ok") => format!(
                        "signed in as {}",
                        connection["account"]["user_id"].as_str().unwrap_or("?")
                    ),
                    Some("needs_login") => "not signed in".to_string(),
                    Some("degraded") => format!(
                        "signed in, degraded: {}",
                        health["why"].as_str().unwrap_or("?")
                    ),
                    _ => format!("failing: {}", health["why"].as_str().unwrap_or("?")),
                };
                println!(
                    "  {method:<16} {:<32} {state}",
                    connection["label"].as_str().unwrap_or("")
                );
                println!("  {:<16} grants: {}", "", grants(&connection["grants"]));
                if connection["verified"].is_object() {
                    println!("  {:<16} verified: {}", "", grants(&connection["verified"]));
                }
            }
        }
        Ok(())
    }

    /// Start signing in with `method`: show a device code and wait for approval, or print the
    /// URL of a browser login.
    async fn connect(&mut self, method: &str) -> Result<(), BoxError> {
        let Some(begun) = self
            .request(json!({"op": "connect", "method": method}))
            .await?
        else {
            return Ok(());
        };
        let login = &begun["login"];
        if login["flow"] == "browser" {
            println!(
                "Open this URL in a browser and log in:\n\n  {}\n\nThen copy the URL of the \
                 page you land on and run:\n\n  connect {method} <that URL>",
                login["url"].as_str().unwrap_or("?")
            );
            return Ok(());
        }
        let code = &login["code"];
        println!(
            "Visit {} and enter {}",
            code["verification_uri_complete"]
                .as_str()
                .or(code["verification_uri"].as_str())
                .unwrap_or("?"),
            code["user_code"].as_str().unwrap_or("?")
        );
        let mut interval = code["interval"].as_u64().unwrap_or(5).max(1);
        let deadline =
            Instant::now() + Duration::from_secs(code["expires_in"].as_u64().unwrap_or(300));
        while Instant::now() < deadline {
            tokio::time::sleep(Duration::from_secs(interval)).await;
            let request = json!({"op": "connect_complete", "method": method});
            let Some(polled) = self.request(request).await? else {
                return Ok(());
            };
            match polled["status"].as_str() {
                Some("authorized") => {
                    println!("{method}: signed in");
                    return Ok(());
                }
                Some("slow_down") => interval += 2,
                _ => {}
            }
        }
        eprintln!("the code expired before it was approved");
        Ok(())
    }

    /// The track playing now, as an item.
    fn current_track(&self) -> Option<Value> {
        self.last_snapshot
            .as_ref()
            .and_then(|snap| snap["track"]["id"].as_str())
            .map(|id| json!({ "entity": id }))
    }

    /// A request answered with a list of tracks, shown numbered.
    async fn show_tracks(&mut self, title: &str, request: Value) -> Result<(), BoxError> {
        let Some(found) = self.request(request).await? else {
            return Ok(());
        };
        let mut listing = Numbered::default();
        listing.section(title, &found["tracks"], track_line);
        self.show(listing);
        Ok(())
    }

    async fn set_autoplay(&mut self, on: bool) -> Result<(), BoxError> {
        let Some(current) = self.request(op("settings")).await? else {
            return Ok(());
        };
        let mut settings = current["settings"].clone();
        settings["queue"]["autoplay"] = json!(on);
        if self
            .request(json!({"op": "set_settings", "settings": settings}))
            .await?
            .is_some()
            && !self.json_out
        {
            println!("autoplay {}", if on { "on" } else { "off" });
        }
        Ok(())
    }

    /// Set the client id of the user's Spotify app; the next `connect spotify.web` uses it.
    async fn set_spotify_app(&mut self, client_id: &str) -> Result<(), BoxError> {
        let Some(current) = self.request(op("settings")).await? else {
            return Ok(());
        };
        let mut settings = current["settings"].clone();
        settings["spotify"]["client_id"] = json!(client_id);
        if self
            .request(json!({"op": "set_settings", "settings": settings}))
            .await?
            .is_some()
            && !self.json_out
        {
            println!("spotify app {client_id}: `connect spotify.web` to sign in");
        }
        Ok(())
    }

    async fn show_library(&mut self, kind: &str, query: &str) -> Result<(), BoxError> {
        let mut request = json!({"op": "library", "kind": kind});
        if !query.is_empty() {
            request["query"] = json!(query);
        }
        let Some(page) = self.request(request).await? else {
            return Ok(());
        };
        let mut listing = Numbered::default();
        let total = page["total"].as_u64().unwrap_or(0);
        listing.heading(format!("{total} saved {kind}s"));
        listing.section("", &page["tracks"], track_line);
        listing.section("", &page["albums"], album_line);
        listing.section("", &page["artists"], |a| {
            a["name"].as_str().unwrap_or("?").to_string()
        });
        self.show(listing);
        Ok(())
    }

    /// Print a numbered listing (unless the raw reply was already printed) and make it the one
    /// `#n` refers to.
    fn show(&mut self, listing: Numbered) {
        if !self.json_out {
            print!("{}", listing.text);
        }
        self.listing = listing.items;
    }

    /// A request whose only answer is an ack: errors are printed by `request`.
    async fn command_request(&mut self, frame: Value) -> Result<(), BoxError> {
        self.request(frame).await.map(|_| ())
    }

    async fn show_settings(&mut self) -> Result<(), BoxError> {
        let Some(result) = self.request(op("settings")).await? else {
            return Ok(());
        };
        if !self.json_out {
            println!("{}", serde_json::to_string_pretty(&result["settings"])?);
        }
        Ok(())
    }

    /// Set one output's delivery mode: read the settings, change that one field, write them back.
    /// The output is named like `sink` names it (a name prefix, or an id).
    async fn set_mode(&mut self, wanted: &str, mode: &str) -> Result<(), BoxError> {
        let Some(sinks) = self.request(op("list_sinks")).await? else {
            return Ok(());
        };
        let output = sinks["sinks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|s| s["kind"] != "local")
            .find(|s| {
                s["id"].as_str() == Some(wanted)
                    || s["name"]
                        .as_str()
                        .is_some_and(|n| n.to_lowercase().starts_with(&wanted.to_lowercase()))
            });
        let Some(output) = output else {
            eprintln!("no network output matching {wanted:?} — `sinks` to list them");
            return Ok(());
        };
        let id = output["id"].as_str().unwrap_or_default().to_string();
        let Some(current) = self.request(op("settings")).await? else {
            return Ok(());
        };
        let mut settings = current["settings"].clone();
        if !settings["outputs"].is_object() {
            settings["outputs"] = json!({});
        }
        settings["outputs"][&id] = json!({ "mode": mode });
        if self
            .request(json!({"op": "set_settings", "settings": settings}))
            .await?
            .is_some()
            && !self.json_out
        {
            println!(
                "{} → {mode} (from the next track or seek)",
                output["name"].as_str().unwrap_or(&id)
            );
        }
        Ok(())
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
  play|add|playnext <item>...                 queue now (replacing), at the end, or next;
                                              an item is a Tidal track id, album:<id>,
                                              a canon id, or #n from the last listing
  search <words> | album <item> | artist <item>
                                              browse Tidal; results are numbered #n
  playfrom #n                                 play the whole last listing from entry n
  pl [list] | pl new|fromqueue <name> | pl use #n
  pl show | play | add <item>... | rm N | mv A B | rename <name> | delete
                                              playlists: the `pl` verbs act on the chosen one
  radio [item] | similar <artist>             recommendations (no item: the current track)
  mixes | mix #n                              your Tidal mixes (play #n plays one)
  autoplay on|off                             keep playing radio when the queue runs out
  save [item] | unsave [item]                 your library (no item: the current track)
  library [tracks|albums|artists] [words]     list what you've saved, newest first
  import [service]                            bring in your favorites and playlists
                                              (default Tidal; `import spotify`)
  services                                    login methods, what each grants, and their state
  spotify-app <client-id>                     your Spotify developer app, for spotify.web
  connect <method> [redirect-url]             sign in (a browser login finishes with the URL)
  disconnect <method>                         sign out of one login method
  jump N | rm N | mv FROM TO                  queue positions as `queue` numbers them
  shuffle | repeat off|all|one                reorder what's next; what follows the end
  sinks | sink <name[@protocol]-or-id>        list outputs, select one by name
  queue                                       list the queue, marking the current entry
  settings | mode <output> flow|standard       show settings; set how an output gets tracks
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
    let repeat = match snap["queue"]["repeat"].as_str() {
        Some("all") => "  (repeat all)",
        Some("one") => "  (repeat one)",
        _ => "",
    };
    format!("{state:8} {position}/{duration}  {title} — {artists}{queue}{repeat}")
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

/// A listing being built: its text, and the item each number stands for.
#[derive(Default)]
struct Numbered {
    text: String,
    items: Vec<Value>,
}

impl Numbered {
    fn heading(&mut self, line: String) {
        self.text.push_str(&line);
        self.text.push('\n');
    }

    /// One numbered line standing for `item`.
    fn entry(&mut self, item: Value, line: String) {
        self.items.push(item);
        let n = self.items.len();
        self.text.push_str(&format!("  #{n:<3} {line}\n"));
    }

    fn section(&mut self, title: &str, entries: &Value, line: impl Fn(&Value) -> String) {
        let entries = entries.as_array().cloned().unwrap_or_default();
        if entries.is_empty() {
            return;
        }
        if !title.is_empty() {
            self.text.push_str(title);
            self.text.push('\n');
        }
        for entry in entries {
            self.entry(json!({ "entity": entry["id"] }), line(&entry));
        }
    }
}

/// What a connection grants, as a short list.
fn grants(grants: &Value) -> String {
    let mut granted: Vec<String> = [
        ("catalog", "browse"),
        ("library_read", "library"),
        ("library_write", "library changes"),
        ("recommendations", "recommendations"),
    ]
    .iter()
    .filter(|(key, _)| grants[*key] == true)
    .map(|(_, name)| (*name).to_string())
    .collect();
    match grants["stream"].as_str() {
        Some(quality) => granted.push(format!("streaming up to {quality}")),
        None => granted.push("no streaming".to_string()),
    }
    granted.join(", ")
}

fn names(credits: &Value) -> String {
    credits
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| a["name"].as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn track_line(track: &Value) -> String {
    let duration = track["duration_ms"]
        .as_u64()
        .map_or_else(String::new, |ms| format!("  {}", clock(ms)));
    let album = track["album"]["name"]
        .as_str()
        .map_or_else(String::new, |album| format!(" · {album}"));
    format!(
        "{} — {}{album}{duration}",
        track["title"].as_str().unwrap_or("?"),
        names(&track["artists"])
    )
}

fn album_line(album: &Value) -> String {
    let year = album["release_date"]
        .as_str()
        .and_then(|date| date.get(..4))
        .map_or_else(String::new, |year| format!(" ({year})"));
    let credit = album["credit"].as_str().filter(|c| !c.is_empty());
    format!(
        "{}{}{year}",
        album["title"].as_str().unwrap_or("?"),
        credit.map_or_else(String::new, |c| format!(" — {c}"))
    )
}

/// One queue item as a user types it: `#3` (the third entry of the last listing), `album:<id>`
/// (a Tidal album), a canon id (a UUID), or a bare Tidal track id.
fn item(word: &str, listing: &[Value]) -> Option<Value> {
    if let Some(n) = word.strip_prefix('#') {
        let n: usize = n.parse().ok()?;
        return listing.get(n.checked_sub(1)?).cloned();
    }
    if let Some(id) = word.strip_prefix("album:") {
        return Some(json!({"service": "tidal", "id": id, "kind": "album"}));
    }
    if word.len() == 36 && word.matches('-').count() == 4 {
        return Some(json!({ "entity": word }));
    }
    word.chars()
        .all(|c| c.is_ascii_digit())
        .then(|| json!({"service": "tidal", "id": word}))
}

/// A 1-based queue position as `queue` shows it, as the 0-based index the server takes.
fn position(typed: &str) -> Option<usize> {
    typed.trim().parse::<usize>().ok()?.checked_sub(1)
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
    fn items_are_typed_as_ids_albums_entities_or_listing_numbers() {
        let listing = vec![json!({"entity": "x"})];
        assert_eq!(
            item("33348478", &listing),
            Some(json!({"service": "tidal", "id": "33348478"}))
        );
        assert_eq!(
            item("album:55391786", &listing),
            Some(json!({"service": "tidal", "id": "55391786", "kind": "album"}))
        );
        let uuid = "5c7ae60d-789c-45d7-84c2-37d49f8b18cb";
        assert_eq!(item(uuid, &listing), Some(json!({ "entity": uuid })));
        assert_eq!(item("#1", &listing), Some(json!({"entity": "x"})));
        assert_eq!(item("#2", &listing), None);
        assert_eq!(item("#0", &listing), None);
        assert_eq!(item("army", &listing), None);
        assert_eq!(position("1"), Some(0));
        assert_eq!(position("0"), None);
    }

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
