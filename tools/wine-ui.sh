#!/usr/bin/env bash
# Looking at the Windows app under Wine, for checks that need eyes.
#   tools/wine-ui.sh start <exe> [args]   start a virtual display (:97) if needed and the app;
#                                        waits for its window (no guessing with sleep)
#   tools/wine-ui.sh shot <out.png> [WxH+X+Y]   screenshot, cropped if a region is given
#                                        (crops cost far fewer tokens to look at)
#   tools/wine-ui.sh stop                 end the app and Wine
# Env for the app passes through (BACKROOM_SETTINGS, BACKROOM_UPDATE_URL, ...).
# Under Wine: software video decoding shows no picture, and the graphics card
# it reports is made up; don't read anything into those.
set -euo pipefail
export DISPLAY=:97 WINEDEBUG=-all
case "${1:-}" in
  start)
    shift
    if ! xdotool getdisplaygeometry >/dev/null 2>&1; then
      (nohup Xvfb :97 -screen 0 1280x800x24 >/dev/null 2>&1 &)
      for _ in $(seq 50); do xdotool getdisplaygeometry >/dev/null 2>&1 && break; sleep 0.1; done
    fi
    exe="$1"; shift
    (cd "$(dirname "$exe")" && nohup wine "$(basename "$exe")" "$@" >"${TMPDIR:-/tmp}/wine-ui.log" 2>&1 &)
    if ! timeout 60 xdotool search --sync --onlyvisible --name 'TUFFcord|Backroom' >/dev/null; then
      echo "no window after 60 s; see ${TMPDIR:-/tmp}/wine-ui.log"; exit 1
    fi
    sleep 2  # first frames
    echo "window up" ;;
  shot)
    out="$2"
    if [ -n "${3:-}" ]; then import -window root -crop "$3" +repage "$out"; else import -window root "$out"; fi
    echo "$out" ;;
  stop)
    wineserver -k 2>/dev/null || true ;;
  *) echo "usage: tools/wine-ui.sh start <exe> [args] | shot <out.png> [WxH+X+Y] | stop"; exit 2 ;;
esac
