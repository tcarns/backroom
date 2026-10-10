# Status

Where things stand. Rewrite (don't append) at the end of each working session.

**Last updated:** 2026-10-10. **Released:** 0.8.1 (app and server).

## Waiting on the owner
- **A friend's app crashes playing a video** (worked for the owner; friend was
  on 0.7.1). Asked for Windows Reliability Monitor details (faulting module +
  exception code) and the video. 0.8.1 now records crashes (app log + server
  log on next start) and falls back to no-GPU decoding, then to the system
  player. Next: read the report when it arrives; if it names a GPU driver,
  suggest a driver update; if `backroom.exe`, it's ours.

## Workspace (since 2026-10-10)
- Repo is in the project settings and the project uses the "TUFFcord" cloud
  environment, whose setup script installs the toolchain before a thread starts
  (source: `analysis/cloud-env-setup.sh` in the project files). A check thread
  confirmed: auto clone, tools present, CLAUDE.md loaded, doctor ran. Not yet
  seen: a Windows build there (`tools/windows.sh check`).
- `tools/setup.sh` only before building or testing (~5 min bare, all passing).

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
project files (2026-10-10). Recommended there: split client `main.rs` now (scripted
moves, no behavior change, before the admin-settings UI), server later if it
grows. Owner wants to revisit H next, now that the setup changes are done.

- C: support only current + previous version; drop pre-0.7 compatibility code.
- D: split client `main.rs` (3.3k lines) and server `main.rs` into modules.
- E: automated UI tests (egui renders without a window) instead of screenshots.
- G: finish the internal rename (crates, `BACKROOM_*` names); careful with update
  compatibility (see CLAUDE.md "Never change").
- H: keep the built-in video player, or open videos in the system player.
