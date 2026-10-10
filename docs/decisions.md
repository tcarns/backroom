# Decisions

Settled choices and why. Add one when a choice would otherwise be revisited.

- **Rust desktop app (egui, glow), not a browser app.** Started as Node + a
  browser page; replaced for lower memory and global push-to-talk. egui's
  immediate mode keeps the UI code small.
- **All traffic through the server, one port.** Voice is relayed over the
  WebSocket (Opus frames), so no router or NAT setup is needed and a Cloudflare
  tunnel or reverse proxy carries everything. Files go over plain HTTP on the
  same port (since 0.7) so big transfers never delay voice.
- **Accounts with a group password to sign up (0.6).** Group password only
  creates accounts; sign-in keys are stored hashed; argon2 for passwords, run off
  the main thread so voice isn't held up.
- **Self-updating app and server.** The app swaps its own exe and restarts with
  `BACKROOM_RESUME`; the Windows server runs under a watcher process that restarts
  it (exit code 75). Releases are built by GitHub Actions from Linux
  (cross-compiled), verified with SHA256SUMS.
- **Windows' own decoders for video and audio (Media Foundation),** GPU decoding
  when available. No codecs bundled; keeps the exe small. Cost: hard to test off
  Windows; crashes in drivers are possible (handled since 0.8.1).
- **Twemoji, packed as WebP in the exe,** painted over invisible spacer fonts so
  emoji sit inline in egui text.
- **Native TLS with the system's certificate store** (SChannel on Windows) for
  both the chat connection and HTTP, so whatever Windows trusts works.
- **Rename to TUFFcord (0.8) keeps old names where older copies look for them:**
  `--version` wording, `backroom*.exe` release copies, env var names.
- **Release builds panic = abort**, so crashes are recorded by a panic hook and a
  Windows exception filter rather than unwinding.
- **Future hosting:** small Linux VPS, systemd (not Docker) and Caddy, chosen for
  low memory; one server at a time (no multi-server app support).
- **Client UI split by area, not one big `main.rs` (2026-10-10).** `main.rs`
  had grown to 3.3k lines and every feature read large slices of it. Now each
  area (server messages, sidebar, chat, dialogs, settings, ...) is its own
  file with an `impl App` block, and settings has one function per section.
  New features go in the matching module; rules in CLAUDE.md. Server
  `main.rs` (1.5k) stays whole until it grows; next cuts there are voice,
  history and attachments.
