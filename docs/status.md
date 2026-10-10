# Status

Where things stand. Rewrite (don't append) at the end of each working session.

**Last updated:** 2026-10-10. **Released:** 0.8.5 (app and server).

## Waiting on the owner
- **Video freeze (owner, 0.8.2)**: the app hung seconds into a video, with
  the Windows volume mixer, with and without the graphics card. Cause: a
  lock-order deadlock between the player thread and Media Foundation's
  callback (see troubleshooting). Fixed in 0.8.3; 0.8.5 undoes the fallbacks the freeze had
  switched on (system player for that file, no GPU). Not reproducible here (no
  Windows audio), so the owner confirms on his machine and re-enables
  "Use the graphics card for videos" (0.8.2 turned it off after the freeze).
  The friend's earlier 0.7.1 "crash" may have been the same bug (unchecked).

## Workspace (since 2026-10-10)
- Repo is in the project settings and the project uses the "TUFFcord" cloud
  environment, whose setup script installs the toolchain before a thread starts
  (source: `analysis/cloud-env-setup.sh` in the project files). A check thread
  confirmed: auto clone, tools present, CLAUDE.md loaded, doctor ran. Windows
  `.exe` builds work there since `tools/setup.sh` builds Rust's missing Windows
  startup objects (2026-10-10); the app runs under Wine, audio plays.
- `tools/setup.sh` only before building or testing (~5 min bare, all passing).

## In 0.8.4
- Rename finished: crates `tuffcord`/`tuffcord-server`, programs `TUFFcord`,
  `TUFFcord-server`, `tuffcord-bot`. Copies started as `backroom*.exe` rename
  themselves (`proto::update::adopt_new_name`; checked under Wine and in the
  selfupdate suite); the server's `data/backroom.log` becomes `TUFFcord.log`.
  Kept for old copies: env var names, `--version` wording, `backroom*.exe`
  release copies, old repo fallback. Users may need to remake shortcuts and
  re-allow the server in the Windows firewall.

## In 0.8.3
- Video freeze fix above (`client/src/media.rs`).

## In 0.8.2
- Chat messages get a subtle highlight under the pointer (Discord style).
- Video and audio players: the volume bar is now a vertical popup above the
  speaker button, shown while it's hovered (`client/src/volume_ui.rs`); click
  still mutes, scroll still changes volume. Checked under Wine on the audio
  player; the video bar uses the same code but Wine shows no video picture.

## Check early next session
- `tests/run.py --compat` in the Release workflow: first runs on the next release.
- Small fix pending: when `gh api` is refused, `tools/doctor.sh` prints GitHub's
  JSON error into its status line; send gh's errors to /dev/null and print `?`.

## Planned, in order
1. Hosting on a rented Linux server (owner's choice: small x86 VPS near the
   group; systemd, Caddy for HTTPS with a domain, no Docker; one server at a time):
   - Linux server build in each release; Linux self-update (download, swap,
     exit; systemd restarts it; no watcher).
   - Management without the server window: first admin from the settings file
     (`admins = [...]`), status / log level / "update now" in the app's admin
     settings.
   - Setup guide (VPS, service file, Caddy, domain, copying `data/`).
   - Later, optional: HTTPS built into the server (drops Caddy, ~20–40 MB).

## Undecided options (from the efficiency review)
Full review with token estimates: `analysis/token-efficiency-review.md` in the
project files (2026-10-10). Client `main.rs` split done 2026-10-10 (D, client
half): 8 modules, settings window one function per section, rules in CLAUDE.md
"Where new code goes", doctor lists files over 1,000 lines. No behavior change;
build, Windows check and all e2e suites passed; no Wine screenshot taken.
Owner wants to revisit H next.

- C: support only current + previous version; drop pre-0.7 compatibility code.
- D (server half): split server `main.rs` (1.5k lines) when it grows; cuts:
  voice, history, attachments.
- E: automated UI tests (egui renders without a window) instead of screenshots.
- H: keep the built-in video player, or open videos in the system player.
