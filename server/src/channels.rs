//! Channels added by admins from the app: checked, saved to the settings file
//! and sent to everyone signed in.

use crate::{clean, config, State};
use proto::ServerMsg;
use std::sync::Arc;

/// Most channels of each kind, so the sidebar stays usable.
const MAX_CHANNELS: usize = 50;
const MAX_NAME: usize = 40;

/// An admin asked for a new text channel (or voice channel when `voice`).
pub fn create(st: &mut State, conn: u32, name: &str, voice: bool) {
    let by = st.who(conn);
    // Check the account, not the connection: admin may have just been removed.
    let admin = st
        .clients
        .get(&conn)
        .and_then(|c| c.account)
        .and_then(|a| st.accounts.get(a))
        .is_some_and(|a| a.admin);
    if !admin {
        warn!("admin", "{by} tried to add a channel but isn't an admin");
        return refuse(st, conn, "Only admins can add channels.");
    }
    let kind = if voice { "voice" } else { "text" };
    let name = if voice {
        clean(name, MAX_NAME)
    } else {
        let c = config::clean_channel(name);
        let c: String = c.trim_matches('-').chars().take(MAX_NAME).collect();
        c.trim_end_matches('-').to_string()
    };
    let list = if voice {
        &st.cfg.voice_channels
    } else {
        &st.cfg.text_channels
    };
    if name.is_empty() {
        return refuse(st, conn, "Give the channel a name.");
    }
    if list.iter().any(|c| c.to_lowercase() == name.to_lowercase()) {
        return refuse(
            st,
            conn,
            &format!("There's already a {kind} channel called {name}."),
        );
    }
    if list.len() >= MAX_CHANNELS {
        return refuse(
            st,
            conn,
            &format!("There are already {MAX_CHANNELS} {kind} channels."),
        );
    }

    let cfg = Arc::make_mut(&mut st.cfg);
    if voice {
        cfg.voice_channels.push(name.clone());
    } else {
        cfg.text_channels.push(name.clone());
        st.history.entry(name.clone()).or_default();
    }
    let cfg = &st.cfg;
    match config::save_channels(&cfg.config_path, &cfg.text_channels, &cfg.voice_channels) {
        Ok(()) => {}
        Err(e) => error!(
            "admin",
            "Could not save the new channel to {} ({e}); it's gone after a restart",
            cfg.config_path.display()
        ),
    }
    let env = if voice {
        "VOICE_CHANNELS"
    } else {
        "TEXT_CHANNELS"
    };
    if std::env::var_os(env).is_some_and(|v| !v.is_empty()) {
        warn!(
            "admin",
            "{env} is set, so the channel list in the settings file is ignored; {name} is gone after a restart"
        );
    }
    let name = if voice { name } else { format!("#{name}") };
    warn!("admin", "{by} added the {kind} channel {name}");
    st.broadcast(&ServerMsg::Channels {
        text_channels: st.cfg.text_channels.clone(),
        voice_channels: st.cfg.voice_channels.clone(),
    });
}

fn refuse(st: &State, conn: u32, message: &str) {
    st.send(
        conn,
        &ServerMsg::Error {
            code: "channel_failed".into(),
            message: message.into(),
        },
    );
}
