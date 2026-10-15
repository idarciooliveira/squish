#![no_main]

use libfuzzer_sys::fuzz_target;
use squish_core::{Effort, MetaPolicy, Options, Quality, DEFAULT_MAX_PIXELS};

/// Fixed qualities sampled by the fuzzer: the two named modes plus one
/// setting per JPEG/WebP quality band. `Auto` runs the search, so it is the
/// slowest entry here.
const QUALITIES: [Quality; 6] =
    [Quality::Auto, Quality::Lossless, Quality::Fixed(0), Quality::Fixed(60), Quality::Fixed(85), Quality::Fixed(100)];

const EFFORTS: [Effort; 3] = [Effort::Fast, Effort::Default, Effort::Max];

const POLICIES: [MetaPolicy; 3] = [MetaPolicy::StripKeepIcc, MetaPolicy::StripAll, MetaPolicy::KeepAll];

fuzz_target!(|input: &[u8]| {
    // The first three bytes select the options and the whole input is the
    // image, so committed seeds keep their headers and reach the decoders.
    // Cap the size so one input can't eat the whole fuzz budget (NFR-6 cares
    // about malformed input, not throughput).
    if input.len() < 4 || input.len() > 1_000_000 {
        return;
    }
    let selector = &input[..3];
    let data = input;
    let options = Options {
        quality: QUALITIES[usize::from(selector[0]) % QUALITIES.len()],
        effort: EFFORTS[usize::from(selector[1]) % EFFORTS.len()],
        metadata: POLICIES[usize::from(selector[2]) % POLICIES.len()],
        max_pixels: DEFAULT_MAX_PIXELS,
        target: None,
    };
    // Both entry points must return `Err` on bad input, never panic or crash.
    let _ = squish_core::detect(data);
    let _ = squish_core::compress(data, &options);
});
