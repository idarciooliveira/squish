# Fixtures

Small synthetic images, about 210 KB in total. Nothing here comes from `bench/images/`.

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
| `photo.webp` | lossy q95, so there is room to shrink |
| `photo-lossless.webp` | lossless at method 0, so there is room to shrink |
| `logo-alpha.webp` | lossy q95 with alpha |
| `photo-meta.webp` | the lossy photo rewrapped with an ICC profile, EXIF orientation 6, XMP and one unknown chunk |
| `animated.webp` | the lossy photo rewrapped with the animation flag, so it reads as animated WebP |
