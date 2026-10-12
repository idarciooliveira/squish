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
