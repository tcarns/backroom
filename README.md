# Backroom

**Download:** https://github.com/tcarns/backroom/releases/latest. Get the `Backroom-…-windows.zip` file under "Assets".

Voice and text chat for your group, written in Rust. Two programs:

- **backroom.exe**: the app. Everyone runs this.
- **backroom-server.exe**: the server. Only the person hosting runs this.

Voice goes through the server: each person sends their audio once and the server passes it on to everyone else in the channel. Nobody's router can block a direct connection, because there aren't any.

This version doesn't work with the old browser version. Everyone needs the app.

## Hosting the server

1. Put `backroom-server.exe` in a folder of its own. It saves its settings, chat history and logs next to itself.
2. Double-click it. The first time, it creates `backroom-server.toml` with a random group password and shows that password in the window. To change the password, edit that file and restart the server.
3. Keep the window open while people are using Backroom. If Windows asks whether to allow network access, allowing it on private networks is fine.
4. To let friends connect from outside your home, start a Cloudflare tunnel in PowerShell, the same as before:
   ```powershell
   cloudflared tunnel --url http://localhost:3000
   ```
5. Send your friends the `….trycloudflare.com` address and the password.

## Joining

1. Run `backroom.exe`. The first time, Windows may say "Windows protected your PC", because the app isn't signed. Click **More info**, then **Run anyway**.
2. **Server address:** the `….trycloudflare.com` address (with or without `https://`). If you're on the computer running the server, `localhost:3000` also works.
3. Enter your name and the group password, then click **Join**. With "Remember me" checked, it signs you in automatically next time.

Headphones are recommended. There's no echo cancellation yet, so speakers can feed other people's voices back into your mic.

## Using it

- **Join voice:** click a voice channel. Leave with the red phone button.
- **Mute and deafen:** the mic and headphone buttons at the bottom left.
- **Someone too loud or too quiet:** click their name in the voice channel to set their volume (up to 200%) or mute them just for you. This is remembered.
- **Settings** (the sliders button):
  - **Mic and speakers:** pick devices and use the mic meter to test your microphone.
  - **When to send your voice:**
    - **When I talk:** the sensitivity slider sets how loud you need to be. Your mic opens when the meter passes the amber line.
    - **Push to talk:** any key or mouse button 4/5 works, even while a game is in front.
  - **Noise suppression:** removes background noise like fans and keyboard clicks.
  - **Join, leave and message sounds:** turn the chimes on or off.
  - **Memory:** shows how much memory Backroom is using.

The app's settings are saved in `%APPDATA%\Backroom\settings.json`.

## Server settings

Edit `backroom-server.toml` next to the server, then restart it:

| Setting | Default | What it does |
| --- | --- | --- |
| `password` | random | Group password. `""` lets anyone with the address in. |
| `port` | `3000` | Port to listen on |
| `app_name` | `Backroom` | Name shown in the app |
| `text_channels` | `["general", "links", "memes"]` | Text channels |
| `voice_channels` | `["Lounge", "Gaming", "Quiet room"]` | Voice channels |
| `max_per_voice_channel` | `8` | Most people in one voice channel |
| `history_limit` | `300` | Messages kept per text channel |
| `log_level` | `info` | How much to log |
| `log_file` | `data/backroom.log` | Log file. `""` logs to the window only. |

Environment variables with the same names in capitals (`PASSWORD`, `PORT`, `LOG_LEVEL` and so on) override the file. These are the same names the old Node version used.

**Keeping your old chat history:** the history format is the same as the browser version's. Copy the old `data\messages.json` into the `data` folder next to `backroom-server.exe` before starting it.

## Logging

Everything is logged to the server window and to `data\backroom.log`. When the log file passes 5 MB, it's renamed to `backroom.log.1` and a fresh one starts.

| Level | Shows |
| --- | --- |
| `trace` | When each person starts and stops talking |
| `debug` | Chat activity (who posted where, never what they said), mute and deafen, ignored messages |
| `info` | Sign-ins, disconnects, voice joins, leaves and moves |
| `warn` | Wrong passwords, mic or speaker problems on someone's computer, audio breaking up, people sending too fast |
| `error` | App errors reported by someone's computer, saving problems |
| `critical` | The server crashing or failing to start |

Type these into the server window to change things while it runs:

| Command | Does |
| --- | --- |
| `level` | Show the current level |
| `level debug` | Switch to that level right away (any level name works) |
| `status` | Who's online and in voice, uptime and the server's memory use |
| `help` | List the commands |

The app reports problems only it can see, so they show up in your server log:
- a microphone or speakers that failed to open or got unplugged
- someone's audio breaking up repeatedly because of network delay

## Updates

When a new version is published, the app shows a bar saying "Backroom X is available", with a Download button that opens the release page. The server writes the same notice in its window. Both check when they start and every 6 hours after that. You can also check from the app's settings, or turn automatic checks off there. On the server, set `check_updates = false` in `backroom-server.toml`.

To update:
1. Download the new zip.
2. Close Backroom.
3. Replace `backroom.exe` (or `backroom-server.exe`) with the new one. Your settings, chat history and logs are kept.

## Publishing a new version

1. Raise `version` in `Cargo.toml` (for example `0.2.0` to `0.2.1`).
2. Add a section for that version at the top of `CHANGELOG.md`. It becomes the release notes people see.
3. Commit and push, then tag and push the tag:
   ```
   git tag v0.2.1
   git push origin v0.2.1
   ```

GitHub then builds both programs on its own machines and publishes them as a release (the "Release" workflow in the Actions tab). Running copies of Backroom notice it on their next check.

Every push to `main` also runs the tests (the "Tests" workflow).

## Building from source

On Windows:
1. Install Rust from rustup.rs.
2. Install the Visual Studio Build Tools (C++) and CMake. These are needed to compile the Opus audio codec.
3. Run `cargo build --release`. The programs land in `target\release\`.

To cross-compile the Windows programs from Linux, follow the steps in `.github/workflows/release.yml`.

`cargo test` runs the voice engine tests. The `backroom-bot` program is a headless test client that plays a tone and measures what it hears:

```
backroom-bot --server ws://localhost:3000/ws --password pw --name Bot --channel Lounge --tone 440 --listen 660 --seconds 10
```
