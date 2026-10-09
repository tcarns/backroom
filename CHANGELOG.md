# Changelog

Each version's section becomes the notes on its GitHub release, which the app shows when an update is available.

## 0.3.0

- Send images: click +, drag one onto the window, or paste with Ctrl+V. Big photos are shrunk before sending; click an image to see it full size.
- Emoji: `:)` `<3` `:D` and codes like `:fire:` turn into emoji when sent, and there's an emoji picker in the message box.
- Themes: Plum, Midnight, Light, Cream, Pastel pink and Mint, under Settings.
- Fixed dark text on a dark background on PCs that use Windows' light mode.

## 0.2.0

First native release, replacing the browser version.

- Windows app and server written in Rust.
- Voice goes through the server, so a friend's router can't block a direct connection.
- Noise suppression, voice activation sensitivity, and push to talk that works while a game is in front (keys or mouse buttons 4/5).
- Per-person volume up to 200%, and mute for me.
- Server logging with live level changes (`level`, `status`, `help`).
- The app and server tell you when a new version is available.
