# ADR-0010: Strip metadata except the color profile and orientation

- Status: Accepted. For PNG, ADR-0027 counts `sRGB`, `gAMA`, `cHRM`, `cICP`, `mDCV` and `cLLI` as part of the color profile. For WebP, ADR-0028 sets the chunk rules.
- Date: 2026-10-11

## Context

Photos carry EXIF (camera data, GPS location), XMP and embedded thumbnails, often tens of kilobytes. Removing them saves space and protects privacy. Two pieces change how the image looks:

- The ICC color profile. Without it, wide-gamut photos (Display P3 from phones, for example) show shifted colors.
- The EXIF orientation tag. Without it, phone photos can come out rotated.

## Decision

Default policy `strip-keep-icc`:

- Keep the ICC profile.
- Keep the orientation. In practice we copy the orientation tag into a minimal EXIF block. Rotating the pixels instead would cost a decode and re-encode for formats we'd otherwise only optimize.
- Drop everything else, GPS included.

CLI flag `--metadata strip-keep-icc|strip-all|keep-all`. The web UI uses the default only.

## Consequences

- No GPS leaks by default.
- Colors and rotation stay correct.
- Files are a few hundred bytes bigger than with `strip-all`.

## Alternatives considered

- **Strip everything.** Smallest files, but some photos shift color or come out rotated.
- **Keep everything.** Safe for photographers, but bigger files and GPS leaks.
