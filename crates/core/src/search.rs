//! The per-file quality search shared by the codecs (ADR-0024, ADR-0026).
//!
//! Both formats have a "setting gives a score" function that rises with the setting:
//! JPEG quality, or an index into the palette sizes. The search finds the lowest
//! setting whose score reaches the target, spending as few probes as it can.

use crate::error::Error;

pub(crate) struct Params {
    /// Inclusive bounds of the setting.
    pub lo: i32,
    pub hi: i32,
    pub start: i32,
    pub target: f64,
    /// Probes before giving up on tightening the answer.
    pub max_probes: usize,
    /// Typical score change per setting step. Only used to aim the first moves
    /// before a bracket exists. `None` means plain bisection.
    pub slope: Option<f64>,
}

pub(crate) struct Probe<T> {
    pub setting: i32,
    pub score: f64,
    pub value: T,
}

pub(crate) struct Outcome<T> {
    /// The lowest passing probe, or the highest probe if nothing passed.
    pub best: Probe<T>,
    pub probes: usize,
}

/// Runs the search. `eval` encodes at a setting and returns its score and artifact.
pub(crate) fn find_lowest<T>(
    params: &Params,
    mut eval: impl FnMut(i32) -> Result<(f64, T), Error>,
) -> Result<Outcome<T>, Error> {
    assert!(params.lo <= params.hi && (params.lo..=params.hi).contains(&params.start));
    let mut pass: Option<Probe<T>> = None; // lowest setting known to reach the target
    let mut fail: Option<Probe<T>> = None; // highest setting known to miss it
    let mut probes = 0;

    let mut run = |setting: i32,
                   pass: &mut Option<Probe<T>>,
                   fail: &mut Option<Probe<T>>,
                   probes: &mut usize|
     -> Result<(), Error> {
        let (score, value) = eval(setting)?;
        *probes += 1;
        let probe = Probe { setting, score, value };
        if score >= params.target {
            if pass.as_ref().is_none_or(|p| setting < p.setting) {
                *pass = Some(probe);
            }
        } else if fail.as_ref().is_none_or(|f| setting > f.setting) {
            *fail = Some(probe);
        }
        Ok(())
    };

    let mut next = params.start;
    while probes < params.max_probes {
        run(next, &mut pass, &mut fail, &mut probes)?;

        // Settings strictly between the highest miss and the lowest pass are still open.
        let low = fail.as_ref().map_or(params.lo, |f| f.setting + 1);
        let high = pass.as_ref().map_or(params.hi, |p| p.setting - 1);
        if low > high {
            break;
        }
        next = aim(params, fail.as_ref(), pass.as_ref(), low, high);
    }

    if pass.is_none() {
        // Nothing passed within the probe budget. The top of the range is the best
        // chance, and it is the answer when even that misses (PNG at 256 colors).
        let top_probed = fail.as_ref().is_some_and(|f| f.setting == params.hi);
        if !top_probed {
            run(params.hi, &mut pass, &mut fail, &mut probes)?;
        }
    }

    Ok(match (pass, fail) {
        (Some(best), _) | (None, Some(best)) => Outcome { best, probes },
        (None, None) => unreachable!("at least one probe ran"),
    })
}

/// Chooses the next setting inside `low..=high`.
fn aim<T>(params: &Params, fail: Option<&Probe<T>>, pass: Option<&Probe<T>>, low: i32, high: i32) -> i32 {
    let guess = match (fail, pass) {
        // A bracket: interpolate between the two scores, rounding up to the passing side.
        (Some(f), Some(p)) if p.score > f.score => {
            let t = (params.target - f.score) / (p.score - f.score);
            f64::from(f.setting) + t * f64::from(p.setting - f.setting)
        }
        (Some(_), Some(_)) => f64::from(low + high) / 2.0,
        // Only misses so far: step up by what the slope says is missing.
        (Some(f), None) => match params.slope {
            Some(s) => f64::from(f.setting) + (params.target - f.score) / s,
            None => f64::from(low + high) / 2.0,
        },
        // Only passes so far: step down by the surplus.
        (None, Some(p)) => match params.slope {
            Some(s) => f64::from(p.setting) - (p.score - params.target) / s,
            None => f64::from(low + high) / 2.0,
        },
        (None, None) => f64::from(params.start),
    };
    (guess.ceil() as i32).clamp(low, high)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A score curve that rises linearly from `at_lo` at setting `lo` to `at_hi` at `hi`.
    fn linear(lo: i32, hi: i32, at_lo: f64, at_hi: f64) -> impl Fn(i32) -> f64 {
        move |s| at_lo + (at_hi - at_lo) * f64::from(s - lo) / f64::from(hi - lo)
    }

    fn run(params: &Params, curve: impl Fn(i32) -> f64) -> (Outcome<i32>, Vec<i32>) {
        let seen = RefCell::new(vec![]);
        let outcome = find_lowest(params, |s| {
            seen.borrow_mut().push(s);
            Ok((curve(s), s))
        })
        .unwrap();
        (outcome, seen.into_inner())
    }

    fn met<T>(out: &Outcome<T>, target: f64) -> bool {
        out.best.score >= target
    }

    fn jpeg_like(target: f64) -> Params {
        Params { lo: 30, hi: 95, start: 66, target, max_probes: 5, slope: Some(0.4) }
    }

    /// The lowest setting that meets the target, found by scanning.
    fn truth(p: &Params, curve: &impl Fn(i32) -> f64) -> Option<i32> {
        (p.lo..=p.hi).find(|&s| curve(s) >= p.target)
    }

    #[test]
    fn finds_the_exact_crossing_on_a_smooth_curve() {
        let curve = linear(30, 95, 60.0, 96.0);
        let p = jpeg_like(89.3);
        let (out, seen) = run(&p, &curve);
        assert!(met(&out, p.target));
        assert_eq!(Some(out.best.setting), truth(&p, &curve));
        assert!(seen.len() <= 5, "{seen:?}");
    }

    #[test]
    fn a_start_that_already_passes_moves_down() {
        let curve = linear(30, 95, 80.0, 99.0);
        let p = jpeg_like(89.3);
        let (out, seen) = run(&p, &curve);
        assert_eq!(seen[0], 66);
        assert!(seen[1] < 66);
        assert_eq!(Some(out.best.setting), truth(&p, &curve));
    }

    #[test]
    fn target_above_the_whole_range_returns_the_top() {
        let curve = linear(30, 95, 10.0, 80.0);
        let (out, seen) = run(&jpeg_like(89.3), curve);
        assert!(!met(&out, 89.3));
        assert_eq!(out.best.setting, 95);
        assert!(seen.contains(&95));
    }

    #[test]
    fn target_below_the_whole_range_returns_the_bottom_within_budget() {
        let curve = linear(30, 95, 91.0, 99.0);
        let (out, seen) = run(&jpeg_like(89.3), curve);
        assert!(met(&out, 89.3));
        assert_eq!(out.best.setting, 30);
        assert!(seen.len() <= 5, "{seen:?}");
    }

    #[test]
    fn top_first_search_stops_after_one_probe_when_the_top_misses() {
        // PNG: index 0..=60, start at the top. A miss there is final.
        let p = Params { lo: 0, hi: 60, start: 60, target: 90.0, max_probes: 12, slope: None };
        let (out, seen) = run(&p, |s| 50.0 + f64::from(s) / 3.0);
        assert_eq!(seen, [60]);
        assert!(!met(&out, p.target));
        assert_eq!(out.best.setting, 60);
    }

    #[test]
    fn bisection_finds_the_crossing_in_log_steps() {
        let p = Params { lo: 0, hi: 63, start: 63, target: 90.0, max_probes: 12, slope: None };
        let curve = |s: i32| 70.0 + f64::from(s) * 0.4; // crosses 90 at s = 50
        let (out, seen) = run(&p, curve);
        assert!(met(&out, p.target));
        assert_eq!(out.best.setting, 50);
        assert!(seen.len() <= 8, "{seen:?}");
    }

    #[test]
    fn a_noisy_curve_still_returns_a_passing_setting() {
        // Zig-zags by up to 1.5 points, like the real grid data.
        let curve = |s: i32| 60.0 + f64::from(s) * 0.45 + if s % 2 == 0 { 1.5 } else { -1.5 };
        for target in [80.0, 85.0, 89.3] {
            let p = Params { target, ..jpeg_like(target) };
            let (out, _) = run(&p, curve);
            // The curve reaches past every target here, so a pass must always be found.
            assert!(met(&out, target), "target {target}: best {} at {}", out.best.score, out.best.setting);
        }
    }

    #[test]
    fn exhausting_the_budget_without_a_pass_probes_the_top_once() {
        // Needs q95 but only two probes allowed: the last resort is the top setting.
        let p = Params { max_probes: 2, ..jpeg_like(95.0) };
        let curve = linear(30, 95, 50.0, 95.0);
        let (out, seen) = run(&p, curve);
        assert_eq!(*seen.last().unwrap(), 95);
        assert_eq!(out.best.setting, 95);
        assert_eq!(out.probes, seen.len());
    }

    #[test]
    fn every_probe_is_within_bounds_and_never_repeated() {
        for target in [70.0, 82.0, 89.3, 93.0, 99.0] {
            let p = jpeg_like(target);
            let (_, seen) = run(&p, linear(30, 95, 55.0, 97.0));
            assert!(seen.iter().all(|s| (30..=95).contains(s)));
            let mut sorted = seen.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), seen.len(), "repeated probe in {seen:?}");
        }
    }

    #[test]
    fn errors_from_the_evaluator_propagate() {
        let p = jpeg_like(89.3);
        let r = find_lowest::<()>(&p, |_| Err(Error::Encode("boom".into())));
        assert!(matches!(r, Err(Error::Encode(m)) if m == "boom"));
    }
}
