# Fixtures

Small synthetic images, about 140 KB in total. Nothing here comes from `bench/images/`.

Regenerate with `cargo run --release --manifest-path bench/fixture-gen/Cargo.toml -- crates/core/tests/fixtures`.

| File | What it covers |
|---|---|
| `photo-rgb.png`, `photo-gray.png` | noisy photo-like content, RGB and 8-bit gray |
| `photo-icc.png` | the same photo with an `iCCP` chunk |
| `logo-alpha.png` | soft-edged disc on a transparent background |
| `palette.png` | already an indexed image, with one transparent entry |
| `animated.png` | `palette.png` with an `acTL` chunk, so it reads as APNG |
| `photo.jpg` | baseline, unoptimized q92, so there is room to shrink |
| `photo-meta.jpg` | the same with an ICC profile, EXIF orientation 6, GPS text and a comment |
| `photo-gray.jpg` | single-component JPEG |
