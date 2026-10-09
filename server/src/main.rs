//! Backroom server: relays voice between people in the same voice channel,
//! keeps text chat history, and logs what happens.
//!
//! One WebSocket per person carries JSON control messages and binary Opus voice frames.

#[macro_use]
mod log;
mod config;
mod selfupdate;

use config::Config;
use futures_util::{SinkExt, StreamExt};
use log::Level;
use proto::*;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::BufRead;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::{Bytes, Message, Utf8Bytes};

const OUT_QUEUE: usize = 512;
const HELLO_TIMEOUT: Duration = Duration::from_secs(15);
const IDLE_TIMEOUT: Duration = Duration::from_secs(45);

struct Client {
    id: u32,
    name: String,
    ip: String,
    tx: mpsc::Sender<Message>,
    authed: bool,
    signed_in_at: Instant,
    voice: Option<String>,
    voice_order: u64,
    muted: bool,
    deafened: bool,
    talking: bool,
    voice_window: (Instant, u32),
    dropped_while_muted: bool,
    chat_times: VecDeque<Instant>,
    report_times: VecDeque<Instant>,
    upload_times: VecDeque<Instant>,
}

struct State {
    cfg: Arc<Config>,
    clients: HashMap<u32, Client>,
    history: BTreeMap<String, Vec<ChatMessage>>,
    history_dirty: bool,
    next_id: u32,
    next_voice_order: u64,
    started: Instant,
    /// A newer server is installed and waiting for a restart.
    pending_update: Option<String>,
    /// Restart as soon as possible (typed "update").
    restart_now: bool,
    last_chat: Option<Instant>,
}

type Shared = Arc<Mutex<State>>;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn duration(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m {}s", s / 60, s % 60)
    } else {
        format!("{}h {}m", s / 3600, (s / 60) % 60)
    }
}

fn clean(s: &str, max: usize) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .chars()
        .take(max)
        .collect()
}

fn random_id() -> String {
    let mut b = [0u8; 12];
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn memory_mb() -> Option<f64> {
    memory_stats::memory_stats().map(|m| m.physical_mem as f64 / 1_048_576.0)
}

// ---------------------------------------------------------------- state helpers

impl State {
    fn who(&self, id: u32) -> String {
        match self.clients.get(&id) {
            Some(c) if c.authed => c.name.clone(),
            Some(c) => format!("unknown visitor ({})", c.ip),
            None => "someone".into(),
        }
    }

    fn online_count(&self) -> usize {
        self.clients.values().filter(|c| c.authed).count()
    }

    fn voice_count(&self, ch: &str) -> usize {
        self.clients
            .values()
            .filter(|c| c.authed && c.voice.as_deref() == Some(ch))
            .count()
    }

    fn voice_state(&self) -> Vec<VoiceChannelState> {
        self.cfg
            .voice_channels
            .iter()
            .map(|ch| {
                let mut members: Vec<&Client> = self
                    .clients
                    .values()
                    .filter(|c| c.authed && c.voice.as_deref() == Some(ch.as_str()))
                    .collect();
                members.sort_by_key(|c| c.voice_order);
                VoiceChannelState {
                    name: ch.clone(),
                    members: members
                        .iter()
                        .map(|c| VoiceMember {
                            id: c.id,
                            name: c.name.clone(),
                            muted: c.muted,
                            deafened: c.deafened,
                        })
                        .collect(),
                }
            })
            .collect()
    }

    fn users(&self) -> Vec<User> {
        let mut u: Vec<User> = self
            .clients
            .values()
            .filter(|c| c.authed)
            .map(|c| User {
                id: c.id,
                name: c.name.clone(),
            })
            .collect();
        u.sort_by_key(|x| x.id);
        u
    }

    fn send(&self, id: u32, msg: &ServerMsg) {
        if let Some(c) = self.clients.get(&id) {
            let text = serde_json::to_string(msg).expect("serialize");
            if c.tx.try_send(Message::Text(text.into())).is_err() {
                warn!(
                    "conn",
                    "{} isn't keeping up; a message to them was dropped", c.name
                );
            }
        }
    }

    fn broadcast(&self, msg: &ServerMsg) {
        let text: Utf8Bytes = serde_json::to_string(msg).expect("serialize").into();
        for c in self.clients.values().filter(|c| c.authed) {
            if c.tx.try_send(Message::Text(text.clone())).is_err() {
                warn!(
                    "conn",
                    "{} isn't keeping up; a message to them was dropped", c.name
                );
            }
        }
    }

    fn broadcast_voice(&self) {
        self.broadcast(&ServerMsg::VoiceState {
            state: self.voice_state(),
        });
    }
    fn broadcast_presence(&self) {
        self.broadcast(&ServerMsg::Presence {
            users: self.users(),
        });
    }

    fn status_text(&self) -> String {
        let users: Vec<&Client> = self.clients.values().filter(|c| c.authed).collect();
        let mem = memory_mb()
            .map(|m| format!("{m:.1} MB"))
            .unwrap_or_else(|| "unknown".into());
        let mut names: Vec<&str> = users.iter().map(|c| c.name.as_str()).collect();
        names.sort_unstable();
        let mut out = format!(
            "Backroom server {} | up {} | log level {} | memory {mem}\nOnline ({}): {}",
            proto::update::CURRENT,
            duration(self.started.elapsed()),
            log::level().name(),
            users.len(),
            if names.is_empty() {
                "nobody".to_string()
            } else {
                names.join(", ")
            }
        );
        for vc in self.voice_state() {
            if !vc.members.is_empty() {
                let list: Vec<String> = vc
                    .members
                    .iter()
                    .map(|m| {
                        let flag = if m.deafened {
                            " (deafened)"
                        } else if m.muted {
                            " (muted)"
                        } else {
                            ""
                        };
                        format!("{}{flag}", m.name)
                    })
                    .collect();
                out.push_str(&format!("\n  {}: {}", vc.name, list.join(", ")));
            }
        }
        out
    }
}

// ---------------------------------------------------------------- message handling

fn handle_text(shared: &Shared, id: u32, text: &str) {
    let msg: ClientMsg = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(_) => {
            let st = shared.lock().unwrap();
            debug!("conn", "Ignored an unreadable message from {}", st.who(id));
            return;
        }
    };
    let mut st = shared.lock().unwrap();
    let authed = st.clients.get(&id).map(|c| c.authed).unwrap_or(false);
    if !authed && !matches!(msg, ClientMsg::Hello { .. } | ClientMsg::Ping { .. }) {
        return;
    }
    match msg {
        ClientMsg::Hello {
            name,
            password,
            version,
        } => hello(&mut st, id, &name, &password, version),
        ClientMsg::Chat { channel, text } => chat(&mut st, id, &channel, &text),
        ClientMsg::JoinVoice {
            channel,
            muted,
            deafened,
        } => join_voice(&mut st, id, &channel, muted, deafened),
        ClientMsg::LeaveVoice => leave_voice(&mut st, id, None),
        ClientMsg::Status { muted, deafened } => {
            let Some(c) = st.clients.get_mut(&id) else {
                return;
            };
            if deafened != c.deafened {
                debug!(
                    "voice",
                    "{} {}",
                    c.name,
                    if deafened { "deafened" } else { "undeafened" }
                );
            } else if muted != c.muted {
                debug!(
                    "voice",
                    "{} {}",
                    c.name,
                    if muted { "muted" } else { "unmuted" }
                );
            }
            c.muted = muted;
            c.deafened = deafened;
            c.dropped_while_muted = false;
            if c.voice.is_some() {
                st.broadcast_voice();
            }
        }
        ClientMsg::Ping { t } => st.send(id, &ServerMsg::Pong { t }),
        ClientMsg::Report { kind, message } => report(&mut st, id, &kind, &message),
        ClientMsg::GetAttachment { id: att } => request_attachment(&st, id, &att),
    }
}

fn hello(st: &mut State, id: u32, name: &str, password: &str, version: u32) {
    let ip = st
        .clients
        .get(&id)
        .map(|c| c.ip.clone())
        .unwrap_or_default();
    if st.clients.get(&id).map(|c| c.authed).unwrap_or(true) {
        return;
    }
    let name: String = name
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(32)
        .collect();
    if name.is_empty() {
        debug!("auth", "Sign-in from {ip} with an empty name");
        st.send(
            id,
            &ServerMsg::Error {
                code: "bad_name".into(),
                message: "Pick a name to join.".into(),
            },
        );
        return;
    }
    if version != PROTOCOL_VERSION {
        warn!("auth", "{name} ({ip}) has a different app version (protocol {version}, server {PROTOCOL_VERSION})");
        st.send(
            id,
            &ServerMsg::Error {
                code: "bad_version".into(),
                message:
                    "Your Backroom app is a different version than the server. Get the latest one."
                        .into(),
            },
        );
        return;
    }
    let ok = st.cfg.password.is_empty()
        || constant_time_eq(password.as_bytes(), st.cfg.password.as_bytes());
    if !ok {
        warn!("auth", "Wrong password from {ip} (name \"{name}\")");
        // Reply after a short delay so password guessing is slow.
        if let Some(c) = st.clients.get(&id) {
            let tx = c.tx.clone();
            let text = serde_json::to_string(&ServerMsg::Error {
                code: "bad_password".into(),
                message: "That password is not right.".into(),
            })
            .unwrap();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(800)).await;
                let _ = tx.send(Message::Text(text.into())).await;
                let _ = tx.send(Message::Close(None)).await;
            });
        }
        return;
    }
    {
        let c = st.clients.get_mut(&id).unwrap();
        c.authed = true;
        c.name = name.clone();
        c.signed_in_at = Instant::now();
    }
    info!(
        "conn",
        "{name} signed in from {ip} ({} online)",
        st.online_count()
    );
    let welcome = ServerMsg::Welcome {
        id,
        name,
        app_name: st.cfg.app_name.clone(),
        text_channels: st.cfg.text_channels.clone(),
        voice_channels: st.cfg.voice_channels.clone(),
        max_per_voice_channel: st.cfg.max_per_voice_channel,
        history: st.history.clone(),
        voice_state: st.voice_state(),
        users: st.users(),
        features: vec![FEATURE_ATTACHMENTS.to_string()],
        max_attachment_bytes: st.cfg.max_attachment_bytes,
    };
    st.send(id, &welcome);
    st.broadcast_presence();
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    // Compare full length regardless of where a difference is.
    let mut diff = (a.len() != b.len()) as u8;
    for i in 0..a.len().max(b.len()) {
        diff |= a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0);
    }
    diff == 0
}

fn chat(st: &mut State, id: u32, channel: &str, text: &str) {
    let text: String = text
        .replace("\r\n", "\n")
        .trim()
        .chars()
        .take(2000)
        .collect();
    let name = st.who(id);
    if !st.cfg.text_channels.iter().any(|c| c == channel) || text.is_empty() {
        debug!("chat", "Ignored an empty or invalid message from {name}");
        return;
    }
    let now = Instant::now();
    {
        let c = st.clients.get_mut(&id).unwrap();
        while c
            .chat_times
            .front()
            .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(5))
        {
            c.chat_times.pop_front();
        }
        if c.chat_times.len() >= 10 {
            warn!(
                "chat",
                "{name} is sending messages too fast; one was dropped"
            );
            st.send(
                id,
                &ServerMsg::Error {
                    code: "slow_down".into(),
                    message: "Slow down a little.".into(),
                },
            );
            return;
        }
        c.chat_times.push_back(now);
    }
    let message = ChatMessage {
        id: random_id(),
        channel: channel.to_string(),
        author: name.clone(),
        author_id: id.to_string(),
        text,
        ts: now_ms(),
        attachments: Vec::new(),
    };
    debug!(
        "chat",
        "{name} posted in #{channel} ({} characters)",
        message.text.chars().count()
    );
    post_message(st, message);
}

fn join_voice(st: &mut State, id: u32, channel: &str, muted: bool, deafened: bool) {
    let name = st.who(id);
    if !st.cfg.voice_channels.iter().any(|c| c == channel) {
        debug!(
            "voice",
            "{name} tried to join a voice channel that doesn't exist: {}",
            clean(channel, 40)
        );
        return;
    }
    let current = st.clients.get(&id).and_then(|c| c.voice.clone());
    if current.as_deref() == Some(channel) {
        return;
    }
    let count = st.voice_count(channel);
    if count >= st.cfg.max_per_voice_channel {
        let max = st.cfg.max_per_voice_channel;
        info!(
            "voice",
            "{name} couldn't join {channel}: it's full ({max} max)"
        );
        st.send(
            id,
            &ServerMsg::Error {
                code: "voice_full".into(),
                message: format!("{channel} is full ({max} people max)."),
            },
        );
        return;
    }
    st.next_voice_order += 1;
    let order = st.next_voice_order;
    let c = st.clients.get_mut(&id).unwrap();
    c.voice = Some(channel.to_string());
    c.voice_order = order;
    c.muted = muted;
    c.deafened = deafened;
    c.talking = false;
    match &current {
        Some(from) => info!(
            "voice",
            "{name} moved from {from} to {channel} ({} in channel)",
            count + 1
        ),
        None => info!(
            "voice",
            "{name} joined {channel} ({} in channel)",
            count + 1
        ),
    }
    st.send(
        id,
        &ServerMsg::VoiceJoined {
            channel: channel.to_string(),
        },
    );
    st.broadcast_voice();
}

fn leave_voice(st: &mut State, id: u32, reason: Option<&str>) {
    let Some(c) = st.clients.get_mut(&id) else {
        return;
    };
    let Some(ch) = c.voice.take() else { return };
    c.talking = false;
    let name = c.name.clone();
    let left = st.voice_count(&ch);
    match reason {
        Some(r) => info!("voice", "{name} left {ch} ({r}), {left} still there"),
        None => info!("voice", "{name} left {ch}, {left} still there"),
    }
    st.broadcast_voice();
}

fn report(st: &mut State, id: u32, kind: &str, message: &str) {
    let now = Instant::now();
    let name = st.who(id);
    {
        let Some(c) = st.clients.get_mut(&id) else {
            return;
        };
        while c
            .report_times
            .front()
            .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60))
        {
            c.report_times.pop_front();
        }
        if c.report_times.len() >= 30 {
            return;
        }
        c.report_times.push_back(now);
    }
    let message = clean(message, 300);
    match kind {
        "mic" => warn!("client", "{name}'s microphone: {message}"),
        "speaker" => warn!("client", "{name}'s speakers/headphones: {message}"),
        "audioQuality" => warn!("client", "{name}: {message}"),
        "error" => error!("client", "App error on {name}'s computer: {message}"),
        "info" => info!("client", "{name}: {message}"),
        _ => debug!(
            "client",
            "Unknown report \"{}\" from {name}: {message}",
            clean(kind, 30)
        ),
    }
}

fn handle_voice(shared: &Shared, id: u32, frame: &[u8]) {
    let Some((kind, seq, payload)) = parse_client_voice(frame) else {
        let st = shared.lock().unwrap();
        debug!(
            "voice",
            "Ignored a malformed voice frame from {}",
            st.who(id)
        );
        return;
    };
    let mut st = shared.lock().unwrap();
    let Some(c) = st.clients.get_mut(&id) else {
        return;
    };
    if !c.authed {
        return;
    }
    let Some(channel) = c.voice.clone() else {
        return;
    };
    if (c.muted || c.deafened) && kind == VOICE_DATA {
        if !c.dropped_while_muted {
            c.dropped_while_muted = true;
            debug!(
                "voice",
                "Dropped voice from {} because they're muted", c.name
            );
        }
        return;
    }
    // Rate limit: 20 ms frames are 50 a second; allow some burst.
    let now = Instant::now();
    if now.duration_since(c.voice_window.0) >= Duration::from_secs(1) {
        c.voice_window = (now, 0);
    }
    c.voice_window.1 += 1;
    if c.voice_window.1 > 80 {
        return;
    }
    if kind == VOICE_DATA && !c.talking {
        c.talking = true;
        trace!("voice", "{} started talking in {channel}", c.name);
    } else if kind == VOICE_END && c.talking {
        c.talking = false;
        trace!("voice", "{} stopped talking", c.name);
    }
    let out = Bytes::from(encode_server_voice(kind, id, seq, payload));
    for other in st.clients.values() {
        if other.id != id
            && other.authed
            && !other.deafened
            && other.voice.as_deref() == Some(channel.as_str())
        {
            // Voice is time-sensitive: if someone's queue is full, skipping a frame beats waiting.
            let _ = other.tx.try_send(Message::Binary(out.clone()));
        }
    }
}

// ---------------------------------------------------------------- connections

async fn handle_connection(stream: TcpStream, addr: SocketAddr, shared: Shared) {
    let _ = stream.set_nodelay(true);
    let mut forwarded: Option<String> = None;
    let ws = tokio_tungstenite::accept_hdr_async(stream, |req: &Request, resp: Response| {
        let h = req.headers();
        forwarded = h
            .get("cf-connecting-ip")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
            .or_else(|| {
                h.get("x-forwarded-for")
                    .and_then(|v| v.to_str().ok())
                    .map(|v| v.split(',').next().unwrap_or("").trim().to_string())
            })
            .filter(|s| !s.is_empty());
        Ok(resp)
    })
    .await;
    let ip = forwarded.unwrap_or_else(|| addr.ip().to_string());
    let ws = match ws {
        Ok(ws) => ws,
        Err(e) => {
            debug!("conn", "Connection from {ip} wasn't a Backroom app ({e})");
            return;
        }
    };
    let (mut sink, mut stream) = ws.split();
    let (tx, mut rx) = mpsc::channel::<Message>(OUT_QUEUE);

    let id = {
        let mut st = shared.lock().unwrap();
        st.next_id = st.next_id.wrapping_add(1).max(1);
        let id = st.next_id;
        st.clients.insert(
            id,
            Client {
                id,
                name: String::new(),
                ip: ip.clone(),
                tx,
                authed: false,
                signed_in_at: Instant::now(),
                voice: None,
                voice_order: 0,
                muted: false,
                deafened: false,
                talking: false,
                voice_window: (Instant::now(), 0),
                dropped_while_muted: false,
                chat_times: VecDeque::new(),
                report_times: VecDeque::new(),
                upload_times: VecDeque::new(),
            },
        );
        id
    };
    debug!("conn", "New connection from {ip}");

    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let closing = matches!(msg, Message::Close(_));
            if sink.send(msg).await.is_err() || closing {
                break;
            }
        }
        let _ = sink.close().await;
    });

    let connected_at = Instant::now();
    let mut last_heard = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    let mut ticks: u32 = 0;
    let reason: &str = loop {
        tokio::select! {
            msg = stream.next() => {
                last_heard = Instant::now();
                match msg {
                    Some(Ok(Message::Text(t))) => handle_text(&shared, id, t.as_str()),
                    Some(Ok(Message::Binary(b))) => {
                        if b.first() == Some(&ATTACH_UPLOAD) {
                            handle_upload(&shared, id, &b);
                        } else {
                            handle_voice(&shared, id, &b);
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break "closed",
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        debug!("conn", "Connection error for {}: {e}", shared.lock().unwrap().who(id));
                        break "connection error";
                    }
                }
            }
            _ = tick.tick() => {
                ticks += 1;
                let st = shared.lock().unwrap();
                let Some(c) = st.clients.get(&id) else { break "removed" };
                if !c.authed && connected_at.elapsed() > HELLO_TIMEOUT {
                    debug!("conn", "Closing connection from {ip}: never signed in");
                    break "never signed in";
                }
                if last_heard.elapsed() > IDLE_TIMEOUT {
                    info!("conn", "{} stopped responding; dropping the connection", st.who(id));
                    break "stopped responding";
                }
                if ticks % 4 == 0 {
                    let _ = c.tx.try_send(Message::Ping(Bytes::new()));
                }
            }
        }
    };

    {
        let mut st = shared.lock().unwrap();
        if let Some(c) = st.clients.get(&id) {
            let _ = c.tx.try_send(Message::Close(None));
        }
        let was = st
            .clients
            .get(&id)
            .map(|c| (c.authed, c.voice.is_some(), c.name.clone(), c.signed_in_at));
        if let Some((authed, in_voice, name, since)) = was {
            if in_voice {
                let r = if reason == "closed" {
                    "disconnected"
                } else {
                    reason
                };
                leave_voice(&mut st, id, Some(r));
            }
            st.clients.remove(&id);
            if authed {
                info!(
                    "conn",
                    "{name} disconnected after {} ({} online)",
                    duration(since.elapsed()),
                    st.online_count()
                );
                st.broadcast_presence();
            } else {
                debug!("conn", "Connection from {ip} closed");
            }
        }
    }
    let _ = tokio::time::timeout(Duration::from_secs(2), writer).await;
}

// ---------------------------------------------------------------- history

fn history_path(cfg: &Config) -> PathBuf {
    cfg.data_dir.join("messages.json")
}

fn load_history(cfg: &Config) -> BTreeMap<String, Vec<ChatMessage>> {
    let path = history_path(cfg);
    let mut history: BTreeMap<String, Vec<ChatMessage>> = BTreeMap::new();
    if path.exists() {
        match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
        {
            Ok(h) => history = h,
            Err(e) => error!(
                "chat",
                "Could not read chat history ({e}). Starting with empty history."
            ),
        }
    }
    history.retain(|ch, _| cfg.text_channels.contains(ch));
    for ch in &cfg.text_channels {
        history.entry(ch.clone()).or_default();
    }
    history
}

fn save_history(cfg: &Config, history: &BTreeMap<String, Vec<ChatMessage>>) {
    let path = history_path(cfg);
    let tmp = path.with_extension("json.tmp");
    let result = std::fs::create_dir_all(&cfg.data_dir)
        .and_then(|_| std::fs::write(&tmp, serde_json::to_vec(history).unwrap()))
        .and_then(|_| std::fs::rename(&tmp, &path));
    match result {
        Ok(()) => trace!("chat", "Chat history saved"),
        Err(e) => error!("chat", "Saving chat history failed: {e}"),
    }
}

// ---------------------------------------------------------------- messages and images

/// Add a message to history (dropping the oldest past the limit) and send it to everyone.
fn post_message(st: &mut State, message: ChatMessage) {
    let limit = st.cfg.history_limit;
    let list = st.history.entry(message.channel.clone()).or_default();
    list.push(message.clone());
    let mut removed = Vec::new();
    if list.len() > limit {
        let extra = list.len() - limit;
        for old in list.drain(..extra) {
            removed.extend(old.attachments.into_iter().map(|a| a.id));
        }
    }
    st.history_dirty = true;
    st.last_chat = Some(Instant::now());
    if !removed.is_empty() {
        let dir = attachments_dir(&st.cfg);
        for id in removed {
            let _ = std::fs::remove_file(dir.join(&id));
            debug!(
                "chat",
                "Deleted image {id} (its message aged out of history)"
            );
        }
    }
    st.broadcast(&ServerMsg::Chat { message });
}

fn attachments_dir(cfg: &Config) -> PathBuf {
    cfg.data_dir.join("attachments")
}

fn size_label(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}

fn clean_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base.chars().filter(|c| !c.is_control()).take(80).collect();
    if cleaned.trim().is_empty() {
        "image".into()
    } else {
        cleaned.trim().to_string()
    }
}

fn handle_upload(shared: &Shared, id: u32, frame: &[u8]) {
    let mut st = shared.lock().unwrap();
    if !st.clients.get(&id).is_some_and(|c| c.authed) {
        return;
    }
    let name = st.who(id);
    let fail = |st: &State, code: &str, message: String| {
        st.send(
            id,
            &ServerMsg::Error {
                code: code.into(),
                message,
            },
        );
    };
    let Some((header, bytes)) = parse_upload(frame) else {
        debug!("chat", "Ignored a malformed image upload from {name}");
        return;
    };
    let max = st.cfg.max_attachment_bytes;
    if bytes.len() as u64 > max {
        warn!(
            "chat",
            "{name} tried to send an image that's too big ({})",
            size_label(bytes.len() as u64)
        );
        fail(
            &st,
            "too_big",
            format!("That image is too big ({} MB max).", max / 1_048_576),
        );
        return;
    }
    if !st.cfg.text_channels.contains(&header.channel) {
        debug!(
            "chat",
            "{name} sent an image to a channel that doesn't exist"
        );
        return;
    }
    let Some(mime) = sniff_image(bytes) else {
        warn!("chat", "{name} sent a file that isn't a supported image");
        fail(
            &st,
            "bad_image",
            "Only PNG, JPEG, GIF and WebP images can be sent.".into(),
        );
        return;
    };
    let now = Instant::now();
    let too_many = {
        let c = st.clients.get_mut(&id).unwrap();
        while c
            .upload_times
            .front()
            .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60))
        {
            c.upload_times.pop_front();
        }
        let too_many = c.upload_times.len() >= 10;
        if !too_many {
            c.upload_times.push_back(now);
        }
        too_many
    };
    if too_many {
        warn!("chat", "{name} is sending images too fast; one was dropped");
        fail(
            &st,
            "slow_down",
            "You're sending images too fast. Wait a minute.".into(),
        );
        return;
    }

    let attachment = Attachment {
        id: random_id(),
        name: clean_file_name(&header.name),
        mime: mime.to_string(),
        size: bytes.len() as u64,
        width: header.width.min(20_000),
        height: header.height.min(20_000),
    };
    let text: String = header
        .text
        .replace("\r\n", "\n")
        .trim()
        .chars()
        .take(2000)
        .collect();
    let channel = header.channel.clone();
    let dir = attachments_dir(&st.cfg);
    let path = dir.join(&attachment.id);
    let bytes = bytes.to_vec();
    let shared = shared.clone();
    drop(st);

    // Write the file off the main thread, then post the message.
    tokio::spawn(async move {
        let written = tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, &bytes))
        })
        .await;
        let mut st = shared.lock().unwrap();
        match written {
            Ok(Ok(())) => {
                info!(
                    "chat",
                    "{name} sent an image in #{channel} ({})",
                    size_label(attachment.size)
                );
                let message = ChatMessage {
                    id: random_id(),
                    channel,
                    author: name,
                    author_id: id.to_string(),
                    text,
                    ts: now_ms(),
                    attachments: vec![attachment],
                };
                post_message(&mut st, message);
            }
            Ok(Err(e)) => {
                error!("chat", "Saving an image from {name} failed: {e}");
                st.send(
                    id,
                    &ServerMsg::Error {
                        code: "save_failed".into(),
                        message: "The server couldn't save that image.".into(),
                    },
                );
            }
            Err(e) => error!("chat", "Saving an image from {name} failed: {e}"),
        }
    });
}

fn request_attachment(st: &State, client_id: u32, att_id: &str) {
    let Some(c) = st.clients.get(&client_id) else {
        return;
    };
    let known = att_id.len() <= 64
        && att_id.chars().all(|ch| ch.is_ascii_hexdigit())
        && st
            .history
            .values()
            .flatten()
            .any(|m| m.attachments.iter().any(|a| a.id == att_id));
    if !known {
        st.send(
            client_id,
            &ServerMsg::AttachmentGone {
                id: att_id.to_string(),
            },
        );
        return;
    }
    let path = attachments_dir(&st.cfg).join(att_id);
    let tx = c.tx.clone();
    let who = c.name.clone();
    let att_id = att_id.to_string();
    tokio::spawn(async move {
        match tokio::task::spawn_blocking(move || std::fs::read(&path)).await {
            Ok(Ok(bytes)) => {
                trace!(
                    "chat",
                    "Sending image {att_id} to {who} ({})",
                    size_label(bytes.len() as u64)
                );
                let _ = tx
                    .send(Message::Binary(Bytes::from(encode_attachment_data(
                        &att_id, &bytes,
                    ))))
                    .await;
            }
            _ => {
                warn!("chat", "Image {att_id} is missing from data/attachments");
                let gone =
                    serde_json::to_string(&ServerMsg::AttachmentGone { id: att_id }).unwrap();
                let _ = tx.send(Message::Text(gone.into())).await;
            }
        }
    });
}

/// Delete stored images no message refers to any more.
fn cleanup_attachments(cfg: &Config, history: &BTreeMap<String, Vec<ChatMessage>>) {
    let Ok(entries) = std::fs::read_dir(attachments_dir(cfg)) else {
        return;
    };
    let keep: HashSet<&str> = history
        .values()
        .flatten()
        .flat_map(|m| m.attachments.iter().map(|a| a.id.as_str()))
        .collect();
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !keep.contains(name.to_string_lossy().as_ref())
            && std::fs::remove_file(entry.path()).is_ok()
        {
            removed += 1;
        }
    }
    if removed > 0 {
        info!(
            "chat",
            "Removed {removed} stored images that are no longer in chat history"
        );
    }
}

// ---------------------------------------------------------------- console commands

fn console_commands(shared: Shared, updates: std::sync::mpsc::Sender<selfupdate::Request>) {
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let mut parts = line.split_whitespace();
        let Some(cmd) = parts.next() else { continue };
        let arg = parts.collect::<Vec<_>>().join(" ");
        match cmd.to_lowercase().as_str() {
            "help" => println!(
                "Commands:\n  level              show the current log level\n  level <name>       change it now ({})\n  status             who is online and in voice, uptime, memory use\n  update             install the newest version now and restart\n  help               show this list",
                log::LEVEL_NAMES.join(", ")
            ),
            "level" if arg.is_empty() => println!("Log level is \"{}\". Options, most to least detail: {}", log::level().name(), log::LEVEL_NAMES.join(", ")),
            "level" => match Level::parse(&arg) {
                Some(l) => {
                    let before = log::level();
                    log::set_level(l);
                    log::write(Level::Info, "server", &format!("Log level changed from {} to {}", before.name(), l.name()), true);
                }
                None => println!("\"{arg}\" isn't a log level. Options: {}", log::LEVEL_NAMES.join(", ")),
            },
            "status" => println!("{}", shared.lock().unwrap().status_text()),
            "update" => {
                let _ = updates.send(selfupdate::Request::Now);
            }
            _ => println!("Unknown command \"{cmd}\". Type \"help\" for the list."),
        }
    }
}

// ---------------------------------------------------------------- updates

/// Restart into a newly installed version once nobody would notice: no one in
/// voice and no chat for a minute (or right away if asked). Called every second.
fn should_restart(st: &State) -> Option<String> {
    let version = st.pending_update.clone()?;
    let in_voice = st.clients.values().any(|c| c.authed && c.voice.is_some());
    let quiet = st
        .last_chat
        .is_none_or(|t| t.elapsed() >= Duration::from_secs(60));
    (st.restart_now || (!in_voice && quiet)).then_some(version)
}

async fn restart_for_update(shared: &Shared, version: String) -> ! {
    {
        let st = shared.lock().unwrap();
        let online = st.online_count();
        info!(
            "update",
            "Restarting to finish updating to {version}{}",
            if online > 0 {
                format!(" ({online} online will reconnect by themselves)")
            } else {
                String::new()
            }
        );
        st.broadcast(&ServerMsg::Restarting {
            version: version.clone(),
        });
        save_history(&st.cfg, &st.history);
    }
    // Let the goodbye messages go out.
    tokio::time::sleep(Duration::from_millis(400)).await;
    if !selfupdate::supervised() {
        warn!(
            "update",
            "Backroom server {version} is installed. Start the server again to use it."
        );
    }
    std::process::exit(selfupdate::RESTART_CODE);
}

// ---------------------------------------------------------------- main

fn main() {
    selfupdate::print_version_if_asked();
    // Double-clicked: become the watcher that runs (and restarts) the server.
    selfupdate::watch_unless_child();
    let restarted = selfupdate::restart_reason();
    let cfg = config::load();
    log::init(cfg.log_level, cfg.log_file.clone());
    std::panic::set_hook(Box::new(|info| {
        log::write(Level::Critical, "server", &format!("Crashed: {info}"), true);
    }));
    for (level, note) in &cfg.notes {
        log::write(*level, "server", note, false);
    }
    if let Some(r) = restarted {
        log::write(
            if r.starts_with("Restarted") {
                Level::Info
            } else {
                Level::Warn
            },
            "server",
            &r,
            true,
        );
    }
    proto::update::cleanup_leftovers();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(run(Arc::new(cfg)));
}

async fn run(cfg: Arc<Config>) {
    let history = load_history(&cfg);
    cleanup_attachments(&cfg, &history);
    let shared: Shared = Arc::new(Mutex::new(State {
        cfg: cfg.clone(),
        clients: HashMap::new(),
        history,
        history_dirty: false,
        next_id: 0,
        next_voice_order: 0,
        started: Instant::now(),
        pending_update: None,
        restart_now: false,
        last_chat: None,
    }));

    let addr = format!("{}:{}", cfg.host, cfg.port);
    let mut bind = TcpListener::bind(&addr).await;
    // Just restarted: the previous copy may still be letting go of the port.
    for _ in 0..20 {
        match &bind {
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && selfupdate::was_restarted() => {
                tokio::time::sleep(Duration::from_millis(250)).await;
                bind = TcpListener::bind(&addr).await;
            }
            _ => break,
        }
    }
    let listener = match bind {
        Ok(l) => l,
        Err(e) => {
            if e.kind() == std::io::ErrorKind::AddrInUse {
                critical!(
                    "server",
                    "Port {} is already in use. Is Backroom already running in another window?",
                    cfg.port
                );
            } else {
                critical!("server", "Could not start on {addr}: {e}");
            }
            std::process::exit(1);
        }
    };

    info!(
        "server",
        "{} server {} running on port {} (connect with ws://localhost:{} on this computer)",
        cfg.app_name,
        proto::update::CURRENT,
        cfg.port,
        cfg.port
    );
    info!(
        "server",
        "Log level: {}{} | type \"help\" for commands",
        log::level().name(),
        log::file_path()
            .map(|p| format!(" | log file: {}", p.display()))
            .unwrap_or_else(|| " | no log file".into())
    );
    info!("server", "Settings file: {}", cfg.config_path.display());
    debug!(
        "server",
        "Text channels: {} | voice channels: {}",
        cfg.text_channels.join(", "),
        cfg.voice_channels.join(", ")
    );
    if cfg.password.is_empty() {
        warn!(
            "server",
            "No password set: anyone with the address can join."
        );
    }

    let (update_tx, update_rx) = std::sync::mpsc::channel();
    {
        let shared = shared.clone();
        std::thread::spawn(move || console_commands(shared, update_tx));
    }

    {
        let shared = shared.clone();
        let checker = selfupdate::Checker {
            periodic: cfg.check_updates,
            auto_install: cfg.auto_update,
            requests: update_rx,
            installed: Box::new(move |version, now| {
                let mut st = shared.lock().unwrap();
                st.pending_update = Some(version.clone());
                st.restart_now |= now;
                if now {
                    info!("update", "Backroom server {version} is installed");
                } else {
                    info!(
                        "update",
                        "Backroom server {version} is installed. The server will restart to use it once nobody is in voice (type \"update\" to restart now)."
                    );
                }
            }),
        };
        std::thread::Builder::new()
            .name("updates".into())
            .spawn(move || checker.run())
            .expect("spawn update thread");
    }

    // Once a second: save chat history if it changed, and restart into an installed update.
    {
        let shared = shared.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(Duration::from_secs(1));
            loop {
                every.tick().await;
                let (snapshot, restart) = {
                    let mut st = shared.lock().unwrap();
                    let restart = should_restart(&st);
                    let snapshot = (st.history_dirty && restart.is_none()).then(|| {
                        st.history_dirty = false;
                        st.history.clone()
                    });
                    (snapshot, restart)
                };
                if let Some(version) = restart {
                    restart_for_update(&shared, version).await;
                }
                if let Some(snapshot) = snapshot {
                    save_history(&cfg, &snapshot);
                }
            }
        });
    }

    let accept_loop = async {
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    tokio::spawn(handle_connection(stream, addr, shared.clone()));
                }
                Err(e) => {
                    error!("server", "Accepting a connection failed: {e}");
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
    };

    tokio::select! {
        _ = accept_loop => {}
        _ = tokio::signal::ctrl_c() => {}
    }
    info!("server", "Shutting down");
    let st = shared.lock().unwrap();
    if st.history_dirty {
        save_history(&st.cfg, &st.history);
    }
    std::process::exit(0);
}
