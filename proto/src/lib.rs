//! Wire protocol shared by the Backroom server and client.
//!
//! Everything travels over one WebSocket connection:
//! - Control messages are JSON text frames (`ClientMsg` / `ServerMsg`).
//! - Voice is binary frames carrying one 20 ms Opus packet each.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PROTOCOL_VERSION: u32 = 1;
pub const SAMPLE_RATE: u32 = 48_000;
/// 20 ms of mono audio at 48 kHz.
pub const FRAME_SAMPLES: usize = 960;
/// Largest packet Opus can produce.
pub const MAX_OPUS_PACKET: usize = 1275;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ClientMsg {
    Hello {
        name: String,
        password: String,
        #[serde(default)]
        version: u32,
    },
    Chat {
        channel: String,
        text: String,
    },
    JoinVoice {
        channel: String,
        #[serde(default)]
        muted: bool,
        #[serde(default)]
        deafened: bool,
    },
    LeaveVoice,
    Status {
        muted: bool,
        deafened: bool,
    },
    Ping {
        t: u64,
    },
    /// Something only the client can see (audio device trouble, an app error),
    /// sent so it shows up in the server log.
    Report {
        kind: String,
        #[serde(default)]
        message: String,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    pub channel: String,
    pub author: String,
    pub author_id: String,
    pub text: String,
    /// Milliseconds since the Unix epoch.
    pub ts: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceMember {
    pub id: u32,
    pub name: String,
    pub muted: bool,
    pub deafened: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct VoiceChannelState {
    pub name: String,
    pub members: Vec<VoiceMember>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct User {
    pub id: u32,
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ServerMsg {
    Welcome {
        id: u32,
        name: String,
        app_name: String,
        text_channels: Vec<String>,
        voice_channels: Vec<String>,
        max_per_voice_channel: usize,
        history: BTreeMap<String, Vec<ChatMessage>>,
        voice_state: Vec<VoiceChannelState>,
        users: Vec<User>,
    },
    Error {
        code: String,
        message: String,
    },
    Chat {
        message: ChatMessage,
    },
    VoiceState {
        state: Vec<VoiceChannelState>,
    },
    Presence {
        users: Vec<User>,
    },
    VoiceJoined {
        channel: String,
    },
    Pong {
        t: u64,
    },
}

// ---------------------------------------------------------------- voice frames

/// A 20 ms Opus packet.
pub const VOICE_DATA: u8 = 1;
/// "I stopped talking." Lets listeners stop their speaking indicator right away.
pub const VOICE_END: u8 = 2;

/// Client to server: `[kind u8][seq u32 LE][opus payload]`
pub fn encode_client_voice(kind: u8, seq: u32, payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(5 + payload.len());
    v.push(kind);
    v.extend_from_slice(&seq.to_le_bytes());
    v.extend_from_slice(payload);
    v
}

pub fn parse_client_voice(frame: &[u8]) -> Option<(u8, u32, &[u8])> {
    if frame.len() < 5 {
        return None;
    }
    let kind = frame[0];
    if kind != VOICE_DATA && kind != VOICE_END {
        return None;
    }
    let seq = u32::from_le_bytes(frame[1..5].try_into().ok()?);
    let payload = &frame[5..];
    if payload.len() > MAX_OPUS_PACKET || (kind == VOICE_DATA && payload.is_empty()) {
        return None;
    }
    Some((kind, seq, payload))
}

/// Server to client: `[kind u8][sender u32 LE][seq u32 LE][opus payload]`
pub fn encode_server_voice(kind: u8, sender: u32, seq: u32, payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(9 + payload.len());
    v.push(kind);
    v.extend_from_slice(&sender.to_le_bytes());
    v.extend_from_slice(&seq.to_le_bytes());
    v.extend_from_slice(payload);
    v
}

pub fn parse_server_voice(frame: &[u8]) -> Option<(u8, u32, u32, &[u8])> {
    if frame.len() < 9 {
        return None;
    }
    let kind = frame[0];
    let sender = u32::from_le_bytes(frame[1..5].try_into().ok()?);
    let seq = u32::from_le_bytes(frame[5..9].try_into().ok()?);
    Some((kind, sender, seq, &frame[9..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_roundtrip() {
        let c = encode_client_voice(VOICE_DATA, 7, &[1, 2, 3]);
        let (k, seq, p) = parse_client_voice(&c).unwrap();
        assert_eq!((k, seq, p), (VOICE_DATA, 7, &[1u8, 2, 3][..]));
        let s = encode_server_voice(k, 42, seq, p);
        assert_eq!(
            parse_server_voice(&s).unwrap(),
            (VOICE_DATA, 42, 7, &[1u8, 2, 3][..])
        );
        assert!(parse_client_voice(&[9, 0, 0, 0, 0, 1]).is_none());
        assert!(parse_client_voice(&[VOICE_DATA, 0, 0, 0, 0]).is_none());
    }

    #[test]
    fn json_shape() {
        let m: ClientMsg =
            serde_json::from_str(r#"{"type":"joinVoice","channel":"Lounge"}"#).unwrap();
        assert!(matches!(m, ClientMsg::JoinVoice { muted: false, .. }));
        let s = serde_json::to_string(&ServerMsg::VoiceJoined {
            channel: "x".into(),
        })
        .unwrap();
        assert_eq!(s, r#"{"type":"voiceJoined","channel":"x"}"#);
        let s = serde_json::to_string(&ServerMsg::Error {
            code: "a".into(),
            message: "b".into(),
        })
        .unwrap();
        assert!(s.contains(r#""type":"error""#));
    }
}

#[cfg(feature = "updates")]
pub mod update;
