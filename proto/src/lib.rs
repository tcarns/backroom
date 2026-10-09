//! Wire protocol shared by the Backroom server and client.
//!
//! Everything travels over one WebSocket connection:
//! - Control messages are JSON text frames (`ClientMsg` / `ServerMsg`).
//! - Voice is binary frames carrying one 20 ms Opus packet each.
//! - Image uploads and downloads are binary frames too (kinds 3 and 4).

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
    /// Ask for an attachment's bytes; the server answers with an ATTACH_DATA frame.
    GetAttachment {
        id: String,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub width: u32,
    pub height: u32,
}

/// Sent ahead of the image bytes in an upload frame.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UploadHeader {
    pub channel: String,
    #[serde(default)]
    pub text: String,
    pub name: String,
    pub mime: String,
    pub width: u32,
    pub height: u32,
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
        /// What this server supports beyond the basics, e.g. "attachments".
        /// Older servers leave it out.
        #[serde(default)]
        features: Vec<String>,
        #[serde(default)]
        max_attachment_bytes: u64,
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
    /// The attachment no longer exists (its message aged out of history).
    AttachmentGone {
        id: String,
    },
}

pub const FEATURE_ATTACHMENTS: &str = "attachments";

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

// ---------------------------------------------------------------- attachment frames

/// Client to server: `[3][header length u32 LE][UploadHeader JSON][image bytes]`
pub const ATTACH_UPLOAD: u8 = 3;
/// Server to client: `[4][id length u8][id][image bytes]`
pub const ATTACH_DATA: u8 = 4;

/// Image types the server accepts.
pub const IMAGE_MIMES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

pub fn encode_upload(header: &UploadHeader, bytes: &[u8]) -> Vec<u8> {
    let h = serde_json::to_vec(header).expect("serialize header");
    let mut v = Vec::with_capacity(5 + h.len() + bytes.len());
    v.push(ATTACH_UPLOAD);
    v.extend_from_slice(&(h.len() as u32).to_le_bytes());
    v.extend_from_slice(&h);
    v.extend_from_slice(bytes);
    v
}

pub fn parse_upload(frame: &[u8]) -> Option<(UploadHeader, &[u8])> {
    if frame.len() < 5 || frame[0] != ATTACH_UPLOAD {
        return None;
    }
    let len = u32::from_le_bytes(frame[1..5].try_into().ok()?) as usize;
    if len > 4096 || frame.len() < 5 + len {
        return None;
    }
    let header = serde_json::from_slice(&frame[5..5 + len]).ok()?;
    Some((header, &frame[5 + len..]))
}

pub fn encode_attachment_data(id: &str, bytes: &[u8]) -> Vec<u8> {
    let id = &id.as_bytes()[..id.len().min(255)];
    let mut v = Vec::with_capacity(2 + id.len() + bytes.len());
    v.push(ATTACH_DATA);
    v.push(id.len() as u8);
    v.extend_from_slice(id);
    v.extend_from_slice(bytes);
    v
}

pub fn parse_attachment_data(frame: &[u8]) -> Option<(&str, &[u8])> {
    if frame.len() < 2 || frame[0] != ATTACH_DATA {
        return None;
    }
    let n = frame[1] as usize;
    let id = std::str::from_utf8(frame.get(2..2 + n)?).ok()?;
    Some((id, &frame[2 + n..]))
}

/// What the bytes actually are, judged by their first bytes (not the file name).
pub fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
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
    fn attachment_frames() {
        let h = UploadHeader {
            channel: "general".into(),
            text: "look".into(),
            name: "cat.png".into(),
            mime: "image/png".into(),
            width: 4,
            height: 3,
        };
        let f = encode_upload(&h, &[1, 2, 3]);
        let (h2, b) = parse_upload(&f).unwrap();
        assert_eq!((h2, b), (h, &[1u8, 2, 3][..]));
        let d = encode_attachment_data("abc123", &[9, 8]);
        assert_eq!(
            parse_attachment_data(&d).unwrap(),
            ("abc123", &[9u8, 8][..])
        );
        assert!(parse_upload(&[ATTACH_UPLOAD, 255, 255, 0, 0]).is_none());
        assert_eq!(
            sniff_image(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0]),
            Some("image/png")
        );
        assert_eq!(sniff_image(b"GIF89a..."), Some("image/gif"));
        assert_eq!(sniff_image(b"MZ\x90\x00 not an image"), None);
    }

    #[test]
    fn old_messages_still_parse() {
        // Chat history saved by 0.2.0 (and the browser version) has no attachments field.
        let old =
            r#"{"id":"1","channel":"general","author":"A","authorId":"x","text":"hi","ts":5}"#;
        let m: ChatMessage = serde_json::from_str(old).unwrap();
        assert!(m.attachments.is_empty());
        assert!(!serde_json::to_string(&m).unwrap().contains("attachments"));
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
