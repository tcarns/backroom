//! Backroom client library: voice engine, networking, audio devices, settings.
//! The desktop app (main.rs) and the headless test bot (bin/bot.rs) both use it.

pub mod audio;
pub mod emoji;
pub mod images;
pub mod keys;
pub mod net;
pub mod settings;
pub mod updater;
pub mod voice;
