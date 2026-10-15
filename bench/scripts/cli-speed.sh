#!/usr/bin/env bash
# Time the squish CLI on a folder of images, with an optional TinyPNG comparison.
#
# Measures:
#   1. `squish <dir> -r` at default effort (per-file quality search, prd.md 4.4).
#   2. Same at `--effort fast` (fixed settings, no search).
#   3. Optionally, the TinyPNG API roundtrip (shrink POST + result download) on
#      the N smallest files, sequentially, like tinypng-baseline.sh.
#      Each file costs one API credit.
#
# Usage:
#   bench/scripts/cli-speed.sh [--tinypng-sample N] [--jobs N] [--no-build] [DIR]
#
# DIR defaults to bench/images. The TinyPNG leg needs TINIFY_API_KEY.
# Prints a Markdown table plus the machine facts behind it.
set -euo pipefail

SRC="bench/images"
SAMPLE=0
JOBS=""
BUILD=1
while [ $# -gt 0 ]; do
  case "$1" in
    --tinypng-sample) SAMPLE="${2:?--tinypng-sample needs a number}"; shift 2 ;;
    --tinypng-sample=*) SAMPLE="${1#*=}"; shift ;;
    --jobs) JOBS="$2"; shift 2 ;;
    --jobs=*) JOBS="${1#*=}"; shift ;;
    --no-build) BUILD=0; shift ;;
    -h|--help) sed -n '1,13p' "$0"; exit 0 ;;
    *) SRC="$1"; shift ;;
  esac
done

if [ "$BUILD" -eq 1 ]; then
  cargo build --release -p squish --quiet
fi
BIN="target/release/squish"
VERSION="$("$BIN" --version | awk '{print $2}')"
REV="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"

OUT="$(mktemp -d)"
trap 'rm -rf "$OUT"' EXIT

run_squish() { # <effort> <name> — prints: files original_bytes output_bytes elapsed_ms jobs
  rm -rf "$OUT/$2" && mkdir -p "$OUT/$2"
  if [ -n "$JOBS" ]; then
    "$BIN" "$SRC" -r -o "$OUT/$2" -e "$1" --jobs "$JOBS" --json > "$OUT/$2.json"
  else
    "$BIN" "$SRC" -r -o "$OUT/$2" -e "$1" --json > "$OUT/$2.json"
  fi
  python3 -c 'import json,sys; s=json.load(open(sys.argv[1]))["summary"]; print(s["files"], s["original_bytes"], s["output_bytes"], s["elapsed_ms"], s["jobs"])' "$OUT/$2.json"
}

# shellcheck disable=SC2162
read d_files d_orig d_out d_ms d_jobs < <(run_squish default squish-default)
# shellcheck disable=SC2162
read f_files f_orig f_out f_ms f_jobs < <(run_squish fast squish-fast)

TINY_LINE="| TinyPNG API (upload+download, sequential) | — | — | — |"
TINY_NOTE="TinyPNG leg skipped (pass --tinypng-sample N, needs TINIFY_API_KEY)."
if [ "$SAMPLE" -gt 0 ]; then
  : "${TINIFY_API_KEY:?Set TINIFY_API_KEY first. Get one free at https://tinypng.com/developers}"
  # N smallest regular files: cheap on quota and wall time, deterministic.
  mapfile -t PICK < <(find "$SRC" -type f -printf '%s %p\n' | sort -n | head -n "$SAMPLE" | cut -d' ' -f2-)
  TDIR="$(mktemp -d)"
  shrink_total=0; dl_total=0; tiny_bytes=0; tiny_orig=0
  for f in "${PICK[@]}"; do
    tiny_orig=$((tiny_orig + $(stat -c%s "$f")))
    resp="$(mktemp)"
    s=$(date +%s.%N)
    curl -sS -o "$resp" --user "api:$TINIFY_API_KEY" \
      --data-binary @"$f" --header 'Content-Type: application/octet-stream' \
      https://api.tinify.com/shrink
    e=$(date +%s.%N)
    shrink_total=$(echo "$shrink_total + ($e - $s)" | bc)
    url="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["output"]["url"])' "$resp")"
    rm -f "$resp"
    s=$(date +%s.%N)
    curl -sS -o "$TDIR/out" "$url"
    e=$(date +%s.%N)
    dl_total=$(echo "$dl_total + ($e - $s)" | bc)
    tiny_bytes=$((tiny_bytes + $(stat -c%s "$TDIR/out")))
    sleep 1 # same politeness pause as tinypng-baseline.sh
  done
  rm -rf "$TDIR"
  TINY_MS="$(echo "($shrink_total + $dl_total + $SAMPLE) * 1000" | bc | cut -d. -f1)"
  TINY_IN_MB="$(python3 -c "print(f'{$tiny_orig/1e6:.1f}')")"
  TINY_OUT_MB="$(python3 -c "print(f'{$tiny_bytes/1e6:.1f}')")"
  TINY_LINE="| TinyPNG API (upload+download, sequential) | $SAMPLE smallest files, $TINY_IN_MB MB | $TINY_OUT_MB MB | ~$TINY_MS ms (incl. ${SAMPLE}s pauses) |"
  TINY_NOTE="TinyPNG: shrink POST + result download per file, sequentially, +1 s pause between files (same as tinypng-baseline.sh). Each file costs one API credit."
fi

CPU="$(grep -m1 'model name' /proc/cpuinfo 2>/dev/null | cut -d: -f2 | xargs || echo unknown)"
CORES="$(nproc 2>/dev/null || echo unknown)"
MEM="$(free -h 2>/dev/null | awk '/Mem:/ {print $2}' || echo unknown)"

md_row() { # <label> <files> <in-bytes> <out-bytes> <ms>
  python3 -c '
import sys
label, files, ib, ob, ms = sys.argv[1], sys.argv[2], float(sys.argv[3]), float(sys.argv[4]), float(sys.argv[5])
pct = (ob - ib) / ib * 100 if ib else 0
rate = ib / 1e6 / (ms / 1000) if ms else 0
print(f"| {label} | {files} files, {ib/1e6:.1f} MB | {ob/1e6:.1f} MB ({pct:+.1f}%) | {ms:.0f} ms ({rate:.1f} MB/s) |")' \
    "$1" "$2" "$3" "$4" "$5"
}

echo "| Tool | Input | Output | Wall time |"
echo "|---|---|---|---|"
md_row "squish (default effort, $d_jobs jobs)" "$d_files" "$d_orig" "$d_out" "$d_ms"
md_row "squish (--effort fast)" "$f_files" "$f_orig" "$f_out" "$f_ms"
echo "$TINY_LINE"
cat <<EOF

Machine: $CPU, $CORES threads, ${MEM} RAM. squish $VERSION at $REV, $(rustc --version 2>/dev/null || echo "rustc unknown").
Source: $SRC ($d_files files).
$TINY_NOTE
Reproduce: bench/scripts/cli-speed.sh [--tinypng-sample N]. TinyPNG needs TINIFY_API_KEY; every sampled file costs one API credit, so keep N small.
EOF
