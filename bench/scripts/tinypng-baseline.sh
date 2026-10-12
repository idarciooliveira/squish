#!/usr/bin/env bash
# Compress bench/images/ via the Tinify (TinyPNG) API into bench/tinypng/.
# Usage: TINIFY_API_KEY=xxx ./bench/scripts/tinypng-baseline.sh [src_dir] [dst_dir]
# Skips files that already exist in dst unless --force is passed.
set -euo pipefail

SRC="${1:-bench/images}"
DST="${2:-bench/tinypng}"
FORCE=0
if [[ "${*:-}" == *"--force"* ]]; then FORCE=1; fi

if [[ -z "${TINIFY_API_KEY:-}" ]]; then
  echo "Set TINIFY_API_KEY first. Get one free at https://tinypng.com/developers" >&2
  exit 2
fi
mkdir -p "$SRC" "$DST"

count=0; skipped=0; failed=0
shopt -s nullglob
for src in "$SRC"/*; do
  [[ -f "$src" ]] || continue
  base="$(basename "$src")"
  # --force appears in $* but never as a file, skip it
  [[ "$base" == "--force" ]] && continue
  dst="$DST/$base"
  if [[ -f "$dst" && $FORCE -eq 0 ]]; then
    echo "skip (exists): $base"
    skipped=$((skipped+1))
    continue
  fi
  echo "shrink: $base ($(du -h "$src" | cut -f1))"
  resp="$(mktemp)"
  http="$(curl -sS -w '%{http_code}' -o "$resp" \
    --user "api:$TINIFY_API_KEY" \
    --data-binary @"$src" \
    --header 'Content-Type: application/octet-stream' \
    https://api.tinify.com/shrink)"
  if [[ "$http" != "201" ]]; then
    echo "FAIL $base: HTTP $http"
    cat "$resp" >&2; echo >&2
    rm -f "$resp"
    failed=$((failed+1))
    continue
  fi
  url="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["output"]["url"])' "$resp")"
  rm -f "$resp"
  curl -sS -o "$dst" "$url"
  echo "  -> $base ($(du -h "$dst" | cut -f1))"
  count=$((count+1))
  sleep 1
done

echo "done: $count compressed, $skipped skipped, $failed failed -> $DST"
