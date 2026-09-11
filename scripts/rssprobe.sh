#!/usr/bin/env bash
# rssprobe.sh — sample one process's memory to CSV for RSS-hunting.
#
# The sampler writes one CSV row per interval: epoch, ISO-8601 timestamp,
# VmRSS, and VmHWM (peak) in kB. RSS below HWM means memory was freed but
# not returned to the OS (allocator retention); RSS climbing toward or
# past the old HWM means real growth. Scenario markers are rows whose
# note column carries the label, so a plot or a diff shows exactly which
# scenario moved the needle.
#
# Usage:
#   scripts/rssprobe.sh                    # sample `pidof hyprlayd` every 2s
#   scripts/rssprobe.sh -p PID -i 2 -o f.csv
#   scripts/rssprobe.sh mark "VC join"     # append a marker row (other shell)
#   scripts/rssprobe.sh mark               # marker with no label ("scenario")
#
# Ctrl-C stops the sampler. Both commands append to the same CSV (header
# written once), so run the sampler in one terminal and mark scenarios
# from another while you work.

set -euo pipefail

usage() {
    grep '^# ' "$0" | sed 's/^# //'
    exit "${1:-0}"
}

csv=rssprobe.csv
pid=""
interval=2

cmd=${1:-sample}
case $cmd in
    mark)
        shift
        label=""
        while [ $# -gt 0 ]; do
            case $1 in
                -o) csv=$2; shift 2 ;;
                -i) shift 2 ;; # tolerated for copy-paste symmetry, unused here
                *) label="$label${label:+ }$1"; shift ;;
            esac
        done
        [ -n "$label" ] || label=scenario
        [ -f "$csv" ] || echo "epoch,timestamp,rss_kb,hwm_kb,note" > "$csv"
        printf '%s,%s,,,%s\n' "$(date +%s)" "$(date +%FT%T)" "$label" >> "$csv"
        exit 0
        ;;
    sample)
        shift || true
        ;;
    -p | -i | -o | --help)
        ;;
    *)
        usage 2
        ;;
esac

while [ $# -gt 0 ]; do
    case $1 in
        -p) pid=$2; shift 2 ;;
        -i) interval=$2; shift 2 ;;
        -o) csv=$2; shift 2 ;;
        --help) usage ;;
        *) usage 2 ;;
    esac
done

if [ -z "$pid" ]; then
    pid=$(pidof hyprlayd 2>/dev/null | tr ' ' '\n' | head -1) || {
        echo "rssprobe: no hyprlayd running; pass -p PID" >&2
        exit 1
    }
fi
if [ "$(pidof hyprlayd 2>/dev/null | wc -w)" -gt 1 ] && [ "$pid" = "$(pidof hyprlayd | tr ' ' '\n' | head -1)" ]; then
    echo "rssprobe: multiple hyprlayd processes; sampling $pid (first). Pass -p to pick." >&2
fi

status=/proc/$pid/status
[ -r "$status" ] || { echo "rssprobe: cannot read $status" >&2; exit 1; }

[ -f "$csv" ] || echo "epoch,timestamp,rss_kb,hwm_kb,note" > "$csv"
echo "rssprobe: sampling pid $pid every ${interval}s -> $csv (Ctrl-C to stop)" >&2

while :; do
    rss=$(awk '/^VmRSS:/{print $2}' "$status") || exit 0  # process gone
    hwm=$(awk '/^VmHWM:/{print $2}' "$status")
    printf '%s,%s,%s,%s,\n' "$(date +%s)" "$(date +%FT%T)" "$rss" "$hwm" >> "$csv"
    sleep "$interval"
done
