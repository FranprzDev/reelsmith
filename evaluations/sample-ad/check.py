#!/usr/bin/env python3
"""Compare a fresh sample-ad QA report against the committed reference.

Run after `promo build examples/sample/brief.toml --preview`
(see docs/EVALUATION.md). Stdlib only; exits 1 and prints every
mismatch when the fresh report drifts from the reference.
"""
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent


def main() -> int:
    expected = json.loads((HERE / "expected-qa.json").read_text())
    qa_path = ROOT / "examples" / "sample" / "build" / "sample_ad" / "qa" / "qa.json"
    if not qa_path.exists():
        print(f"FAIL: no QA report at {qa_path} - run the build first (docs/EVALUATION.md)")
        return 1
    qa = json.loads(qa_path.read_text())
    failures = []

    checks = {c["name"]: c for c in qa["checks"]}
    for name in expected["checks_pass"]:
        if name not in checks:
            failures.append(f"check {name!r} missing from the fresh report")
        elif not checks[name]["ok"]:
            failures.append(f"check {name!r} failed: {checks[name]['detail']}")

    dur = qa["container"]["duration_s"]
    if abs(dur - expected["duration_s"]) > expected["duration_tol_s"]:
        failures.append(f"duration {dur:.3f}s outside {expected['duration_s']} +/- {expected['duration_tol_s']}")

    size = [qa["container"]["video"]["width"], qa["container"]["video"]["height"]]
    if size != expected["size"]:
        failures.append(f"size {size} != {expected['size']}")

    lufs = qa["loudness_mp4"]["integrated_lufs"]
    if abs(lufs - expected["integrated_lufs"]) > expected["lufs_tol"]:
        failures.append(f"loudness {lufs:.2f} LUFS outside {expected['integrated_lufs']} +/- {expected['lufs_tol']}")

    tp = qa["loudness_mp4"]["true_peak_dbtp"]
    if tp > expected["true_peak_dbtp_max"]:
        failures.append(f"true peak {tp:.2f} dBTP over {expected['true_peak_dbtp_max']}")

    motion = qa.get("motion")
    ref = expected.get("shot_source_median_ydif")
    if motion is not None and ref is not None:
        fresh = motion.get("shot_source_median_ydif", [])
        if len(fresh) != len(ref):
            failures.append(f"shot count changed: {len(fresh)} medians, reference has {len(ref)}")
        else:
            tol = expected["shot_ydif_rel_tol"]
            for i, (f, r) in enumerate(zip(fresh, ref)):
                if f is None or abs(f - r) > r * tol:
                    failures.append(f"shot {i} source motion {f} outside {r} +/- {tol:.0%}")

    if failures:
        print("FAIL: sample-ad drifted from its reference:")
        for f in failures:
            print(f"  - {f}")
        return 1
    print(f"PASS: sample-ad matches its reference ({len(expected['checks_pass'])} checks, duration, size, loudness, true peak, shot motion)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
