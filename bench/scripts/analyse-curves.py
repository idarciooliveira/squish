#!/usr/bin/env python3
"""Analyse bench/results/curves.csv (written by `spike/src/bin/curves.rs`).

Prints, per format:

1. The oracle: per file, the cheapest grid point whose dssim is at or below
   TinyPNG's (the Phase 0 sweep, on this grid).
2. Fixed dssim targets: one target for every file, as a runtime search would use.
3. Each candidate metric at each scale: the spread of TinyPNG's own scores, then
   the fixed target that gives the fewest bytes under each quality rule.
4. Estimated search cost per file, from the measured timings.

Usage: analyse-curves.py [curves.csv] [--sweep=metric:scale ...]
"""

import csv
import math
import statistics as st
import sys
from collections import defaultdict

FLOOR = 0.0002  # dssim below this counts as equal (NFR-1)
HIGHER_IS_BETTER = {"fs2", "ic_ssim", "ic_hybrid", "s2rs"}
METRICS = ["fs2", "ba", "ba3", "ic_ssim", "ic_hybrid"]


def load(path):
    files = defaultdict(lambda: {"fmt": None, "points": defaultdict(dict), "times": defaultdict(lambda: defaultdict(float))})
    for r in csv.DictReader(open(path, newline="")):
        f = files[r["file"]]
        f["fmt"] = r["format"]
        key = (r["metric"], r["scale"])
        if r["setting"] == "original":
            f["pixels"] = float(r["value"])
            f["orig_bytes"] = int(r["bytes"])
            continue
        if r["setting"] != "ref":
            p = f["points"][r["setting"]]
            p["bytes"] = int(r["bytes"])
            if r["value"]:
                p[key] = float(r["value"])
        f["times"][r["setting"]][key] += float(r["cpu_ms"] or 0)
    return files


def grid(f):
    return [(s, p) for s, p in f["points"].items() if s != "tinypng"]


def fallback(f):
    """The best-quality grid point, used when nothing meets the target."""
    return min(grid(f), key=lambda sp: sp[1][("dssim", "1")])


def pick(f, ok):
    good = [sp for sp in grid(f) if ok(sp[1])]
    return min(good, key=lambda sp: sp[1]["bytes"]) if good else fallback(f)


def target(f):
    return max(f["points"]["tinypng"][("dssim", "1")], FLOOR)


def summarise(files, sel):
    """sel maps file -> chosen point. Returns the NFR-1 view of that selection."""
    ours = sum(p["bytes"] for p in sel.values())
    tiny = sum(files[n]["points"]["tinypng"]["bytes"] for n in sel)
    ratios = [sel[n][("dssim", "1")] / target(files[n]) for n in sel]
    return {
        "bytes": ours / tiny - 1,
        "strict": sum(r <= 1 for r in ratios),
        "within25": sum(r <= 1.25 for r in ratios),
        "within50": sum(r <= 1.5 for r in ratios),
        "worst": max(ratios),
        "median_ratio": st.median(ratios),
        "median_dssim": st.median(p[("dssim", "1")] for p in sel.values()),
        "n": len(sel),
    }


def fmt_row(label, s):
    return (f"  {label:<26} bytes {s['bytes']:+6.1%}  strict {s['strict']:>2}/{s['n']}  "
            f"<=1.25x {s['within25']:>2}  <=1.5x {s['within50']:>2}  worst {s['worst']:4.2f}x  "
            f"median dssim {s['median_dssim']:.5f} ({s['median_ratio']:.2f}x)")


# Quality rules a fixed target can be held to. Each takes a summary and says pass/fail.
RULES = {
    "every file <= 1x": lambda s: s["strict"] == s["n"],
    "median <= 1x, worst <= 1.5x": lambda s: s["median_ratio"] <= 1 and s["worst"] <= 1.5,
    "median <= 1x, worst <= 2x": lambda s: s["median_ratio"] <= 1 and s["worst"] <= 2,
}


def best_fixed(files, names, key, thresholds):
    """For each rule, the threshold on metric `key` with the fewest bytes that passes."""
    hib = key[0] in HIGHER_IS_BETTER
    out = {}
    for t in thresholds:
        ok = (lambda p: p.get(key, -math.inf) >= t) if hib else (lambda p: p.get(key, math.inf) <= t)
        s = summarise(files, {n: pick(files[n], ok)[1] for n in names})
        for rule, passes in RULES.items():
            if passes(s) and (rule not in out or s["bytes"] < out[rule][1]["bytes"]):
                out[rule] = (t, s)
    return out


def search_cost(files, names, metric, scale, steps):
    """Per-file CPU ms for a binary search: reference setup plus `steps` probes plus one final encode."""
    costs = []
    for n in names:
        f = files[n]
        ref = f["times"]["ref"]
        setup = ref[("dssim", "1")] * 0 + ref[("downscale", scale)] + ref[(metric, scale)]
        probes = []
        for s, _ in grid(f):
            t = f["times"][s]
            enc = t[("encode", "1")] + t[("decode", "1")] + t[("quantize", "1")]
            probes.append(enc + t[("downscale", scale)] + t[(metric, scale)])
        final = st.median(f["times"][s][("oxipng", "1")] for s, _ in grid(f)) if f["fmt"] == "png" else 0
        costs.append(setup + steps * st.median(probes) + final)
    return costs


def pct(xs, q):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(q * len(xs)))]


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--sweep=")]
    # --sweep=fs2:1mp prints a threshold table for that metric and scale.
    sweeps = {tuple(a[8:].split(":")) for a in sys.argv[1:] if a.startswith("--sweep=")}
    files = load(args[0] if args else "bench/results/curves.csv")
    for fmt in ("jpeg", "png"):
        names = sorted(n for n, f in files.items() if f["fmt"] == fmt and "tinypng" in f["points"])
        if not names:
            continue
        print(f"\n===== {fmt.upper()}: {len(names)} files =====")
        tiny_d = [files[n]["points"]["tinypng"][("dssim", "1")] for n in names]
        print(f"TinyPNG dssim: min {min(tiny_d):.5f}  median {st.median(tiny_d):.5f}  max {max(tiny_d):.5f}")

        print("\n1. Oracle, per-file TinyPNG target (Phase 0 method, this grid)")
        sel = {n: pick(files[n], lambda p, t=target(files[n]): p[("dssim", "1")] <= t)[1] for n in names}
        print(fmt_row("oracle", summarise(files, sel)))

        print("\n2. One fixed dssim target for every file")
        for t in [0.0003, 0.0005, 0.0008, 0.001, 0.0012, 0.0015, 0.002, 0.0025, 0.003]:
            sel = {n: pick(files[n], lambda p, t=t: p[("dssim", "1")] <= t)[1] for n in names}
            print(fmt_row(f"dssim <= {t}", summarise(files, sel)))

        print("\n3. Candidate metrics with one fixed target")
        scales = sorted({k[1] for n in names for p in files[n]["points"].values() for k in p if isinstance(k, tuple) and k[0] == "fs2"})
        for m in METRICS:
            for sc in scales:
                key = (m, sc)
                # Files above 30 MP only have `1mp` scores, so each key uses the files that have it.
                kn = [n for n in names if key in files[n]["points"]["tinypng"]]
                if not kn:
                    continue
                tv = [files[n]["points"]["tinypng"][key] for n in kn]
                spread = st.pstdev(tv) / abs(st.mean(tv)) if st.mean(tv) else float("nan")
                print(f"\n  {m} @ {sc} ({len(kn)} files): TinyPNG scores min {min(tv):.3f} median {st.median(tv):.3f} max {max(tv):.3f} (cv {spread:.2f})")
                med = st.median(tv)
                ok = (lambda p, t=med: p.get(key, -math.inf) >= t) if m in HIGHER_IS_BETTER else (lambda p, t=med: p.get(key, math.inf) <= t)
                print(fmt_row(f"target = TinyPNG median", summarise(files, {n: pick(files[n], ok)[1] for n in kn})))
                values = sorted({p[key] for n in kn for _, p in grid(files[n]) if key in p})
                for rule, (t, s) in best_fixed(files, kn, key, values).items():
                    print(fmt_row(f"{rule} t={t:.2f}", s))
                if key in sweeps:
                    lo, hi = pct(tv, 0.0), pct(values, 0.98) if m in HIGHER_IS_BETTER else pct(values, 0.02)
                    for i in range(9):
                        t = lo + (hi - lo) * i / 8
                        ok = (lambda p, t=t: p.get(key, -math.inf) >= t) if m in HIGHER_IS_BETTER else (lambda p, t=t: p.get(key, math.inf) <= t)
                        print(fmt_row(f"  sweep t={t:.2f}", summarise(files, {n: pick(files[n], ok)[1] for n in kn})))

        print("\n4. Search cost per file, CPU ms (binary search, measured per-probe costs)")
        steps = {"jpeg": [7, 4, 3], "png": [9, 5, 3]}[fmt]
        for m in ["dssim", "fs2", "ba"]:
            for sc in (["1"] if m == "dssim" else scales):
                for k in steps:
                    c = search_cost(files, [n for n in names if (m, sc) in files[n]["times"]["ref"]], m, sc, k)
                    print(f"  {m:>5} @ {sc:<3} {k} probes: median {st.median(c):7.0f}  p95 {pct(c, 0.95):7.0f}  max {max(c):7.0f}")
        single = [st.median(files[n]["times"][s][("encode", "1")] + files[n]["times"][s][("quantize", "1")] + files[n]["times"][s][("oxipng", "1")] for s, _ in grid(files[n])) for n in names]
        print(f"  no search, one encode: median {st.median(single):7.0f}  p95 {pct(single, 0.95):7.0f}  max {max(single):7.0f}")
        mp = [files[n]["pixels"] / 1e6 for n in names]
        print(f"  image size: median {st.median(mp):.1f} MP, max {max(mp):.1f} MP")


if __name__ == "__main__":
    main()
