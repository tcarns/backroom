# TUFFcord

Voice and text chat for one friend group (all on Windows), in Rust. Called
Backroom before 0.8; code, crate names and env vars still say `backroom`.
Repo: github.com/tcarns/TUFFcord (was tcarns/backroom; GitHub redirects the old
name, and `proto/src/update.rs` keeps it as a fallback). Owner: Tyler Carns.

**Priorities:** the app stays lightweight (memory, CPU); usage-efficient
development. **Direction:** the server will later move to a small rented Linux
host (systemd, Caddy for HTTPS, no Docker), one server at a time; hosting at
home must keep working.

## Start here
- Run `tools/setup.sh` only before the first build or test in a session (not
  for questions, docs or analysis): it installs what's missing, builds, runs the
  tests and the health check (~5 min in a bare workspace, less when the cloud
  environment has already installed the tools).
- `tools/doctor.sh` runs at session start (hook in `.claude/settings.json`); its
  `doctor:` lines show version, release, GitHub runs, disk, problems. If they're
  not in context, run it.
- `docs/status.md`: open items and next steps. **Rewrite it at the end of each
  working session** (current state, not a log).
- `docs/decisions.md`: settled choices and why; check before re-deciding.
- `docs/troubleshooting.md`: symptom → cause → fix; check before debugging,
  add an entry when something took more than one try.

## Layout
- `proto/` shared by both: `lib.rs` wire protocol (serde, camelCase JSON),
  `files.rs` file-transfer types and sniffing, `update.rs` GitHub release
  check, verified download, exe swap.
- `server/src/` tokio current_thread. `main.rs` state, message handling, chat,
  voice relay, console commands, `delete_message`; `auth.rs` sign-in, admin
  actions; `accounts.rs` storage (argon2); `files.rs` HTTP endpoints on the same
  port (POST/PUT/GET /files, Range, chunked bodies); `selfupdate.rs` watcher +
  updater; `config.rs` settings file and its upgrades; `log.rs`.
- `client/src/` egui 0.32 (glow). `main.rs` (3.3k lines: app state, server
  messages, chat view, settings window, dialogs); `accounts.rs` sign-in screen
  and account UI; `attach.rs` + `attach_ui.rs` attachments and players;
  `media.rs` Media Foundation player (Windows only); `crash.rs` crash records;
  `net.rs`, `voice.rs`, `audio.rs` (cpal, opus); `files.rs` HTTP transfers;
  `settings.rs`, `applog.rs`, `updater.rs`, `members.rs`, `chat_text.rs`,
  `emoji.rs`, `twemoji.rs`, `theme.rs`, `keys.rs`; `bin/bot.rs` headless test
  client (tones, `--send-file`).
- `tests/` end-to-end suites (Python), `tools/` scripts, `.github/workflows/`
  `tests.yml` (every push) and `release.yml` (publishes when the version rises).

## Never change (older copies depend on them)
- Env vars `BACKROOM_SERVER_CHILD`, `BACKROOM_RESUME`; watcher exit code 75.
- `--version` prints exactly `backroom-server <version>` (0.7.1 and earlier
  refuse an update otherwise). Readers accept `TUFFcord-server ` too.
- Releases carry `backroom.exe` and `backroom-server.exe` copies alongside
  `TUFFcord*.exe`, all listed in `SHA256SUMS.txt`.
- Protocol: only add optional fields/messages; old clients skip unknown types.
- `backroom-server.toml` / `TUFFcord-server.toml` hold the group password:
  never commit (both in .gitignore).

## Commands
- Unit tests: `cargo test --workspace`
- End to end: `tests/run.py` (one line per suite; logs in `target/tests/`),
  `tests/run.py files delete`, `--compat` adds old/new version pairs (previous
  release built once into `target/compat/`), `-v` shows every check.
- Windows: `tools/windows.sh check` (fast) / `build`.
- Wine UI: `tools/wine-ui.sh start <exe>`, `shot out.png 400x80+340+440`, `stop`.
- Release: bump `version` in root `Cargo.toml`, add a `## x.y.z` section at the
  top of `CHANGELOG.md` (it becomes the notes the app shows), commit, push to
  main. Then `tools/release-check.sh` (run passed, files, checksums, old-name
  copies, zip, `--version`). `gh release` uses GraphQL, which the proxy blocks;
  use `gh api` (REST).
- Commits: author Tyler Carns (repo-local config is set); end messages with
  the Co-Authored-By / Claude-Session lines from the session's instructions.

## Workspace quirks
- Disk allowance is small. `tools/doctor.sh` prunes build leftovers when space
  runs low. "Bus error" from the linker means the disk is full.
- `pkill -f <pattern>` matches its own shell and kills it (exit 144). Use
  `pkill -x <name>` or filter `ps -eo pid,args`.
- The proxy blocks repo settings changes and GraphQL; plain `gh api` REST works.
  Other websites only via WebFetch.
- Wine: real GPU behaviour can't be tested; software video decoding gives no
  picture. Media Foundation frames work on the GPU path for videos without sound.

## Working habits (usage)
- Read the parts of files you need (grep for the function first); `main.rs` is big.
- Filter command output (`tail`, `grep -E "^(error|warning)"`); never paste
  whole build logs.
- Prefer the test runner over ad-hoc scripts; add a suite or a check to
  `tests/` instead of writing throwaway tests.
- Screenshots: crop to the part being checked; full window only for layout.
- Try anything that may be blocked (permissions, network) before building on it.
- Keep wrap-ups short: what changed, how it was checked, what the user does.
- Keep this file under ~120 lines; detail goes in `docs/`.
