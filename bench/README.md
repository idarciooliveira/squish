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
