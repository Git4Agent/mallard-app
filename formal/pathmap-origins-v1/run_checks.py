#!/usr/bin/env python3
"""Run the path-origin models. Keep TLC output in a temporary directory."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time


CHECKS = [
    ("PathMapOriginsV1-source", "PathMapOriginsV1", None),
    ("PathMapOriginsV1-continuation", "PathMapOriginsV1", None),
    ("PathMapOriginsV1-wire", "PathMapOriginsV1", None),
    ("PathMapOriginsV1-guards", "PathMapOriginsV1", None),
    ("PathMapOriginsV1-pair", "PathMapOriginsV1", None),
    ("PathMapSpellingV1", "PathMapSpellingV1", None),
    ("PathMapOriginCommitV1", "PathMapOriginCommitV1", None),
    ("PathMapOriginsV1-negative-blind", "PathMapOriginsV1", "BlindReplayIsLossless"),
    ("PathMapOriginsV1-negative-empty", "PathMapOriginsV1", "EmptyTraceFallbackIsSafe"),
    ("PathMapOriginsV1-negative-keys", "PathMapOriginsV1", "UnsafeKeyInstallIsLossless"),
    ("PathMapSpellingV1-negative-collapse", "PathMapSpellingV1", "CanonicalSpellingIsLossless"),
    ("PathMapSpellingV1-negative-repush", "PathMapSpellingV1", "DroppingSpellingOnRepushIsLossless"),
    ("PathMapOriginCommitV1-negative", "PathMapOriginCommitV1", "AcceptedOnlyAfterCompleteInstall"),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jar", type=Path, required=True)
    parser.add_argument("--only", help="Run configurations whose names contain this text")
    args = parser.parse_args()
    jar = args.jar.resolve(strict=True)
    source = Path(__file__).resolve().parent
    output = Path(tempfile.mkdtemp(prefix="mallard-path-origin-tlc-"))
    for pattern in ("*.tla", "*.cfg"):
        for item in source.glob(pattern):
            shutil.copy2(item, output / item.name)
    print(f"TLC output: {output}", flush=True)
    results = []
    for config, module, expected_invariant in CHECKS:
        if args.only and args.only not in config:
            continue
        started = time.monotonic()
        log = output / f"{config}.log"
        with log.open("w") as stream:
            process = subprocess.run(
                [
                    "java", "-XX:+UseParallelGC", "-Xmx1g", "-jar", str(jar),
                    "-workers", "1", "-metadir", str(output / f"states-{config}"),
                    "-config", f"{config}.cfg", f"{module}.tla",
                ],
                cwd=output,
                stdout=stream,
                stderr=subprocess.STDOUT,
            )
        text = log.read_text()
        if expected_invariant:
            ok = process.returncode == 12 and (
                f"Invariant {expected_invariant} is violated" in text
            )
        else:
            ok = process.returncode == 0 and (
                "Model checking completed. No error has been found." in text
            )
        counts = re.search(r"([\d,]+) states generated, ([\d,]+) distinct states found", text)
        result = {
            "configuration": config,
            "expected_invariant_failure": expected_invariant,
            "exit_code": process.returncode,
            "ok": ok,
            "seconds": round(time.monotonic() - started, 3),
            "generated_states": int(counts[1].replace(",", "")) if counts else None,
            "distinct_states": int(counts[2].replace(",", "")) if counts else None,
        }
        results.append(result)
        print(json.dumps(result), flush=True)
        if not ok:
            print(text[-7000:], flush=True)
            break
    report = {
        "jar_sha256": hashlib.sha256(jar.read_bytes()).hexdigest(),
        "results": results,
    }
    (output / "results.json").write_text(json.dumps(report, indent=2) + "\n")
    if not results or not all(result["ok"] for result in results):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
