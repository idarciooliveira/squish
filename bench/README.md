# bench

Phase 0 corpus and results. Layout follows ADR-0020.

## Folders

- `images/` — drop your ~50 originals here. Gitignored.
- `tinypng/` — TinyPNG outputs for the same files, same names. Gitignored.
- `results/` — committed CSVs per run.
- `scripts/` — benchmark scripts.
- `spike/` — throwaway Cargo project, outside the workspace.

## Manual upload

1. Copy files into `bench/images/`. Aim for photos, screenshots, logos with alpha, illustrations, already-optimized files.
2. Keep original filenames. Do not resize first.
3. Run the same set through TinyPNG once, save outputs into `bench/tinypng/` with identical names.
4. Expected count: ~50.

## Calibration curves (Phase 1)

`spike/src/bin/curves.rs` encodes every file over a grid of settings and scores each result with dssim and the candidate metrics at several scales, timing every step. `scripts/analyse-curves.py` reads the CSV and prints the fixed-target tests, metric calibration and search-cost estimates behind ADR-0025 and ADR-0026.

```sh
cd bench/spike && cargo build --release --bin curves && cd ..
spike/target/release/curves images tinypng results/curves.csv   # about an hour, needs ~16 GB
python3 scripts/analyse-curves.py results/curves.csv --sweep=fs2:1mp > results/curves-analysis.txt
```

Files over 50 MP are outside squish's default pixel limit. Leave them out, because scoring them at full size runs out of memory.

## WebP (Phase 2)

Put WebP files in `images/` and run the baseline script as before. It skips files that already have an output, and it uploads every file it finds, so move byte-identical copies out first (the TinyPNG call costs quota).

```sh
set -a; . bench/.env; set +a
bench/scripts/tinypng-baseline.sh
```

`curves` takes `CURVES_ONLY=webp` and sweeps libwebp quality 25 to 95 at method 4 for lossy files (`CURVES_WEBP_GRID=66:76:1` sets another range and step), or runs one lossless encode for lossless ones, scoring each with dssim and fs2 at ~1 MP. Animated files are skipped. `analyse-curves.py --guard` models the size guard, which matters for WebP because inputs are often tight already.

```sh
cd bench/spike && cargo build --release --bin curves
CURVES_ONLY=webp target/release/curves ../images ../tinypng ../results/curves-webp.csv
CURVES_ONLY=webp CURVES_WEBP_GRID=66:76:1 target/release/curves ../images ../tinypng ../results/curves-webp-fine.csv
cd .. && python3 scripts/analyse-curves.py results/curves-webp.csv --guard --sweep=fs2:1mp > results/curves-webp-analysis.txt
python3 scripts/analyse-curves.py results/curves-webp-fine.csv --guard > results/curves-webp-fine-analysis.txt
```

Decisions from this run are in ADR-0028. The harness takes `--only webp` and `--webp-target N`. It lists WebP files as skipped until `squish-core` has a WebP codec.
