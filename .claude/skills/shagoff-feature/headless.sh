#!/usr/bin/env bash
# Run Shagoff Commander in an invisible sway session and drive it, without touching the user's
# desktop, focus, config or state. Needs: sway, wtype, grim.
#
#   headless.sh start [WxH]        start headless sway (default 1036x530) + the app (debug build)
#   headless.sh panes LEFT RIGHT   restart the app with these dirs in the two panes
#   headless.sh key ARGS...        wtype ARGS (e.g. -k F7, -M shift -k F2 -m shift)
#   headless.sh click X Y [right|middle|double|none|up|down]  mouse at (X, Y), click or wheel (built from ./vpointer once)
#   headless.sh drag X1 Y1 X2 Y2   press at (X1, Y1), move to (X2, Y2), release
#   headless.sh shot NAME          screenshot to $DIR/NAME.png
#   headless.sh stop
#
# Limits: wtype uploads its own keymap, so letter shortcuts bound to physical keys (Ctrl+M,
# Ctrl+Shift+S…) do not arrive; named keys (F-keys, Enter, Tab, arrows, Alt+F7…) do. Keys need
# the 300 ms pauses below or the first one is lost to the keymap switch.
set -euo pipefail
DIR=${SHAGOFF_HEADLESS:-/tmp/shagoff-headless}
APP=io.github.shagovAlexei.cosmic-shagoff-commander
ROOT=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
mkdir -p "$DIR"/{cfg,state,cache}
sock() { cat "$DIR/wayland"; }
# pid files, not `pkill -f`: a pattern also matches the shell that runs this script.
kill_pid() { [ -f "$DIR/$1.pid" ] && { kill "$(cat "$DIR/$1.pid")" 2>/dev/null || true; }; rm -f "$DIR/$1.pid"; }
app() {
    kill_pid app
    cd "$ROOT"
    WAYLAND_DISPLAY="$(sock)" XDG_CONFIG_HOME="$DIR/cfg" XDG_STATE_HOME="$DIR/state" \
        XDG_CACHE_HOME="$DIR/cache" setsid "$ROOT/target/debug/shagoff-commander" >"$DIR/app.log" 2>&1 &
    echo $! >"$DIR/app.pid"
    sleep 3
}
case ${1:-} in
start)
    before=$(ls "$XDG_RUNTIME_DIR" | grep -E '^wayland-[0-9]+$' || true)
    printf 'output HEADLESS-1 resolution %s\ndefault_border none\n' "${2:-1036x530}" >"$DIR/sway.cfg"
    echo "${2:-1036x530}" >"$DIR/size"
    env -u WAYLAND_DISPLAY -u DISPLAY WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman \
        setsid sway -c "$DIR/sway.cfg" >"$DIR/sway.log" 2>&1 &
    echo $! >"$DIR/sway.pid"
    sleep 2
    comm -13 <(echo "$before" | sort) <(ls "$XDG_RUNTIME_DIR" | grep -E '^wayland-[0-9]+$' | sort) | head -1 >"$DIR/wayland"
    [ -s "$DIR/wayland" ] || { echo "sway did not start, see $DIR/sway.log"; exit 1; }
    (cd "$ROOT" && cargo build -q -p shagoff-commander)
    app ;;
panes)
    mkdir -p "$DIR/state/cosmic/$APP/v1"
    printf '((tabs: ["%s"], active: 0), (tabs: ["%s"], active: 0))\n' "$2" "$3" >"$DIR/state/cosmic/$APP/v1/panes"
    app ;;
key) shift; WAYLAND_DISPLAY="$(sock)" wtype -s 300 "$@" -s 300 ;;
click)
    vp="$ROOT/.claude/skills/shagoff-feature/vpointer"
    (cd "$vp" && cargo build -q --release --offline)
    size=$(cat "$DIR/size" 2>/dev/null || echo 1036x530)  # the output's, for absolute motion
    WAYLAND_DISPLAY="$(sock)" "$vp/target/release/vpointer" "$2" "$3" "${4:-left}" "${size%x*}" "${size#*x}"; sleep 0.5 ;;
drag)
    vp="$ROOT/.claude/skills/shagoff-feature/vpointer"
    (cd "$vp" && cargo build -q --release --offline)
    size=$(cat "$DIR/size" 2>/dev/null || echo 1036x530)
    WAYLAND_DISPLAY="$(sock)" "$vp/target/release/vpointer" drag "$2" "$3" "$4" "$5" "${size%x*}" "${size#*x}"; sleep 0.5 ;;
shot) WAYLAND_DISPLAY="$(sock)" grim "$DIR/$2.png"; echo "$DIR/$2.png" ;;
stop) kill_pid app; kill_pid sway ;;
*) sed -n '2,15p' "$0"; exit 1 ;;
esac
