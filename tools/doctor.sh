#!/usr/bin/env bash
# Workspace health check, run at the start of each session (a SessionStart hook
# does it automatically). Fixes what it safely can, reports the rest in a few
# lines:
#   - frees disk when space is low (old build leftovers, test output)
#   - ends test servers, bots, Wine and virtual displays left running
#   - checks the tools builds and tests need
#   - shows the version, the latest release, and the last GitHub runs
# --quick skips the network checks. --keep leaves running processes alone
# (used after a conversation summary, when a test may be running).
cd "$(dirname "$0")/.." || exit 0
REPO=tcarns/backroom
say() { printf '%s\n' "$*"; }
problems=()

# --- leftovers from earlier runs (nothing should be running at session start)
ended=0
keep=0; case " $* " in *" --keep "*) keep=1;; esac
[ $keep = 1 ] || for name in backroom-server backroom-bot wineserver Xvfb; do
  n=$(pgrep -x "$name" | wc -l)
  if [ "$n" -gt 0 ]; then pkill -x "$name"; ended=$((ended + n)); fi
done
[ $keep = 1 ] || for pid in $(ps -eo pid=,args= | awk '$2 ~ /python3/ && ($3 ~ /tunnel\.py$/ || ($3 == "-m" && $4 == "http.server")) {print $1}'); do
  kill "$pid" 2>/dev/null && ended=$((ended + 1))
done

# --- disk
avail_gb() { df -BG --output=avail / | tail -1 | tr -dc 0-9; }
freed=""
if [ "$(avail_gb)" -lt 10 ] || [ "$(du -s -BG target 2>/dev/null | tr -dc 0-9 | head -c 4)" -gt 12 ] 2>/dev/null; then
  before=$(avail_gb)
  rm -rf target/debug/incremental target/tests target/compat/src-* target/x86_64-pc-windows-gnu/release/incremental
  # Keep only the newest old-version build.
  ls -d target/compat/v* 2>/dev/null | sort -V | head -n -1 | xargs -r rm -rf
  freed=" (freed $(( $(avail_gb) - before )) GB)"
fi
[ "$(avail_gb)" -lt 5 ] && problems+=("disk nearly full: $(avail_gb) GB free even after pruning; delete target/ and rebuild")

# --- tools
need() { command -v "$1" >/dev/null 2>&1 || problems+=("missing $1 ($2)"); }
need cargo "builds"
need python3 "tests"
need openssl "tunnel test"
need x86_64-w64-mingw32-gcc "Windows builds"
need wine "Windows UI checks"
need Xvfb "Windows UI checks"
need gh "releases"
[ -x /usr/lib/rust-1.91/bin/cargo ] || problems+=("no /usr/lib/rust-1.91 (tools/windows.sh needs it)")
[ -d "${STD_SRC:-/home/claude/rust-std-src/library}" ] || problems+=("no Rust std source for Windows builds (STD_SRC)")
python3 -c "import websockets" 2>/dev/null || problems+=("python websockets missing: pip install --break-system-packages websockets")

# --- project
version=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
changed=$(git status --porcelain 2>/dev/null | wc -l)
line="TUFFcord $version, $changed uncommitted file(s)"
if [[ " $* " != *" --quick "* ]] && command -v gh >/dev/null; then
  latest=$(timeout 15 gh api "repos/$REPO/releases/latest" --jq .tag_name 2>/dev/null || echo "?")
  runs=$(timeout 15 gh api "repos/$REPO/actions/runs?per_page=4" \
    --jq '[.workflow_runs[] | "\(.name) \(.conclusion // .status)"] | unique | join(", ")' 2>/dev/null || echo "?")
  line="$line; latest release $latest; GitHub: $runs"
  case "$runs" in *failure*) problems+=("a GitHub run failed: gh api repos/$REPO/actions/runs?per_page=4");; esac
fi

say "doctor: $line"
say "doctor: disk $(avail_gb) GB free, target $(du -sh target 2>/dev/null | cut -f1)$freed; ended $ended leftover process(es)"
for p in "${problems[@]}"; do say "doctor: PROBLEM $p"; done
say "doctor: open items are in docs/status.md"
exit 0
