# Changelog

Each version's section becomes the notes on its GitHub release, which the app shows when an update is available.

## 0.7.0

- Member list on the right, like Discord: who's online (and in which voice channel), then everyone else. The people button at the top of the chat hides it.
- Deafened people now show the muted mark too.
- Send any file, up to 100 MB: videos and audio play right in the chat, GIFs animate, and other files have Open and Save buttons. Files travel separately from voice, so big ones never make voices stutter.
- Color emoji (Twemoji, the set Discord uses), shown big when a message is only emoji. The picker has every emoji, with categories, search and recently used, and `:codes:` work for all of them.
- The server keeps up to 5 GB of files and clears the oldest when it's full (`max_storage_mb`).

## 0.6.0

- Accounts: everyone signs in with their own name and password, and nobody else can use their name. The group password is now only needed once, to create an account. If you used Backroom before, the app fills in your name and the group password; just pick a password.
- "Keep me signed in" saves a sign-in key instead of a password.
- Rename yourself or change your password in Settings.
- Admins can kick, ban and reset passwords: click someone's name, or use Settings → People. To become the first admin, type `admin <your name>` in the server window.
- Older apps can't sign in to this server. They're asked to update.
- Bans and the wrong-password limit go by each person's real internet address: only a proxy on the server's own PC (like the Cloudflare tunnel) can pass one along.
- Fixed the Settings window stretching across the screen when the update check failed (for example, when offline).

## 0.5.0

- The server updates itself: it downloads and checks the new version, then restarts once nobody is in voice. The app shows "The server is updating…" and reconnects on its own. Type `update` in the server window to do it right away. (This one server update, from 0.4.0 or earlier, still has to be done by hand.)
- If the server ever crashes, it starts itself again.

## 0.4.0

- Update from inside the app: click **Update now** and Backroom downloads the new version, checks it, installs it and restarts, putting you back in your voice channel. (This one update, from 0.3.0, still has to be done by hand.)

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
