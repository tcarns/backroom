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

## Check early next session (new, not yet seen working)
- The SessionStart hook (`.claude/settings.json`) running `tools/doctor.sh`:
  `doctor:` lines should appear at the start; if not, run it by hand and fix.
  It only fires when the session starts inside the repo, which needs the repo
  in the project's settings (added 2026-10-10; first new thread will show it).
- `tests/run.py --compat` in the Release workflow: first runs on the next release.

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
project files (2026-10-10). Recommended there: split client `main.rs` now, server
later if it grows; revisit H after the setup changes.

- C: support only current + previous version; drop pre-0.7 compatibility code.
- D: split client `main.rs` (3.3k lines) and server `main.rs` into modules.
- E: automated UI tests (egui renders without a window) instead of screenshots.
- G: finish the internal rename (crates, `BACKROOM_*` names); careful with update
  compatibility (see CLAUDE.md "Never change").
- H: keep the built-in video player, or open videos in the system player.
