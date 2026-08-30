#!/usr/bin/env bash

set -euo pipefail

DEMO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DEMO_RAW_DIR="${HYPRLAY_DEMO_RAW_DIR:-$HOME/Videos/hyprlay-demo/raw}"
DEMO_OUT_DIR="${HYPRLAY_DEMO_OUT_DIR:-$HOME/Videos/hyprlay-demo/out}"
DEMO_WORKSPACE=99
DEMO_PREVIOUS_WORKSPACE=""
DEMO_DAEMON_PID=""
DEMO_CONFIG_BACKUP=""
DEMO_TARGET_DIR=""

demo_build() {
    DEMO_TARGET_DIR="$(cargo metadata --no-deps --format-version 1 | jq -r '.target_directory')"
    if [[ "${HYPRLAY_DEMO_REBUILD:-0}" == "1" ]]; then
        inf "Building demo binaries with demo-roster feature..."
        cargo build --release --features demo-roster
    else
        inf "Using existing demo binaries (set HYPRLAY_DEMO_REBUILD=1 to rebuild)..."
    fi
    for binary in hyprlay hyprlayd hyprlay-gui hyprlay-tray; do
        [[ -x "$DEMO_TARGET_DIR/release/$binary" ]] || {
            err "Missing demo binary: $binary"
            return 1
        }
    done
    scs "Demo binaries built"
}

demo_stage() {
    DEMO_PREVIOUS_WORKSPACE="$(hyprctl activeworkspace -j | jq -r '.id')"
    hyprctl dispatch workspace "$DEMO_WORKSPACE" >/dev/null
    pkill -x hyprlay-gui 2>/dev/null || true
    hyprctl clients -j | jq -r '.[] | select(.class == "demo-fullscreen") | .address' |
        while read -r address; do
            hyprctl dispatch closewindow "address:$address" >/dev/null
        done
    local config="$HOME/.config/hyprlay/config.toml"
    if [[ -f "$config" ]]; then
        DEMO_CONFIG_BACKUP="$config.demo-backup"
        cp "$config" "$DEMO_CONFIG_BACKUP"
    fi
    mkdir -p "$DEMO_RAW_DIR" "$DEMO_OUT_DIR"
}

demo_start_daemon() {
    pkill -x hyprlayd 2>/dev/null || true
    sleep 0.3
    HYPRLAY_DEMO_ROSTER="${HYPRLAY_DEMO_ROSTER:-4}" \
        "$DEMO_TARGET_DIR/release/hyprlayd" >"$DEMO_OUT_DIR/daemon.log" 2>&1 &
    DEMO_DAEMON_PID=$!
    sleep 1
}

demo_launch_gui() {
    "$DEMO_TARGET_DIR/release/hyprlay" gui >/dev/null 2>&1 &
    for _ in {1..20}; do
        if hyprctl clients -j | jq -e '.[] | select(.title == "Hyprlay - Iced")' >/dev/null; then
            return
        fi
        sleep 0.2
    done
    err "Settings GUI did not open"
    return 1
}

demo_key() {
    case "$1" in
        CTRL+1) ydotool key "29:1 2:1 2:0 29:0" ;;
        CTRL+2) ydotool key "29:1 3:1 3:0 29:0" ;;
        CTRL+3) ydotool key "29:1 4:1 4:0 29:0" ;;
        CTRL+4) ydotool key "29:1 5:1 5:0 29:0" ;;
        CTRL+5) ydotool key "29:1 6:1 6:0 29:0" ;;
        CTRL+F) ydotool key "29:1 33:1 33:0 29:0" ;;
        CTRL+S) ydotool key "29:1 31:1 31:0 29:0" ;;
        ALT+F4) ydotool key "56:1 62:1 62:0 56:0" ;;
        ESC) ydotool key "1:1 1:0" ;;
        *) err "Unsupported demo key: $1"; return 1 ;;
    esac
    sleep 0.4
}

demo_focus_gui() {
    local address
    address="$(hyprctl clients -j | jq -r '.[] | select(.title == "Hyprlay - Iced") | .address' | head -1)"
    [[ -n "$address" ]] || { err "Settings GUI is not available"; return 1; }
    hyprctl dispatch focuswindow "address:$address" >/dev/null
    sleep 0.4
}

demo_type() {
    wtype -- "$1"
    sleep 0.6
}

demo_cursor() {
    hyprctl dispatch movecursor "$1" "$2" >/dev/null
    sleep 1
}

demo_fullscreen() {
    hyprctl dispatch exec "[workspace $DEMO_WORKSPACE silent] kitty --class demo-fullscreen --title 'Hyprlay fullscreen demo'" >/dev/null
    sleep 2
    local address
    address="$(hyprctl clients -j | jq -r '.[] | select(.class == "demo-fullscreen") | .address' | head -1)"
    if [[ -n "$address" ]]; then
        hyprctl dispatch focuswindow "address:$address" >/dev/null
        hyprctl dispatch fullscreen 1 >/dev/null
    fi
}

demo_run_scenes() {
    local scene
    for scene in "$DEMO_ROOT"/scripts/demo/scenes/s*.sh; do
        [[ -f "$scene" ]] || continue
        source "$scene"
    done
}

demo_record() {
    local name="$1"
    local seconds="$2"
    local region="${3:-}"
    local output="$DEMO_RAW_DIR/$name.mkv"
    local -a args=(-c libx264 -p crf=20 -f "$output")
    [[ -n "$region" ]] && args+=(-g "$region")
    timeout --signal=INT "$((seconds + 2))" wf-recorder "${args[@]}" >/dev/null 2>&1 &
    local recorder=$!
    wait "$recorder" || [[ "$?" -eq 124 ]]
}

demo_cleanup() {
    set +e
    [[ -n "$DEMO_DAEMON_PID" ]] && kill "$DEMO_DAEMON_PID" 2>/dev/null
    pkill -x hyprlayd 2>/dev/null
    pkill -x hyprlay-gui 2>/dev/null
    hyprctl clients -j | jq -r '.[] | select(.class == "demo-fullscreen") | .address' |
        while read -r address; do
            hyprctl dispatch closewindow "address:$address" >/dev/null
        done
    [[ -n "$DEMO_PREVIOUS_WORKSPACE" ]] && hyprctl dispatch workspace "$DEMO_PREVIOUS_WORKSPACE" >/dev/null
    [[ -n "$DEMO_CONFIG_BACKUP" && -f "$DEMO_CONFIG_BACKUP" ]] && mv "$DEMO_CONFIG_BACKUP" "$HOME/.config/hyprlay/config.toml"
}

demo_build_deliverables() {
    mkdir -p "$DEMO_OUT_DIR"
    local clip
    local -a inputs=()
    local filter=""
    local labels=""
    local count=0
    while IFS= read -r clip; do
        [[ -f "$clip" ]] || continue
        inputs+=(-i "$clip")
        filter+="[$count:v]setpts=PTS-STARTPTS,fps=60[v$count];"
        labels+="[v$count]"
        count=$((count + 1))
    done < <(find "$DEMO_RAW_DIR" -maxdepth 1 -type f -name '*.mkv' -print | sort)
    (( count > 0 )) || { err "No raw demo clips found"; return 1; }
    filter+="$labels concat=n=$count:v=1:a=0[outv]"
    ffmpeg -y "${inputs[@]}" -filter_complex "$filter" -map '[outv]' -c:v libx264 -preset medium -crf 18 -pix_fmt yuv420p "$DEMO_OUT_DIR/hyprlay-demo.mp4" >/dev/null 2>&1
    ffmpeg -y -i "$DEMO_OUT_DIR/hyprlay-demo.mp4" -vf 'fps=12,scale=800:-2:flags=lanczos,split[s0][s1];[s0]palettegen=max_colors=128[p];[s1][p]paletteuse=dither=bayer' "$DEMO_OUT_DIR/hyprlay-hero.gif" >/dev/null 2>&1
    scs "Wrote $DEMO_OUT_DIR/hyprlay-demo.mp4 and hyprlay-hero.gif"
}
