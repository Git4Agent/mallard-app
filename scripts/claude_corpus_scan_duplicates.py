#!/usr/bin/env python3
"""Corpus gate (diagnostics): characterize duplicate-uuid-with-different-parent
records in ~/.claude/projects session files.

These are the re-emitted-uuid files that strict claude.lineage.v1 rejected as
DuplicateMessageId and that the revised rules (R2/R4 positional shadowing in
docs/PLAN_CLAUDE_SESSION_CURSOR_PARITY.md) recover. Metadata only: types,
flags, positions, byte-equality of raw lines with envelope fields removed --
never content.
"""

import json
import os
from collections import defaultdict

ROOT = os.path.expanduser("~/.claude/projects")
CHAIN_TYPES = ("user", "assistant", "system", "attachment")


def normalized_without(raw, drop):
    try:
        rec = json.loads(raw)
    except Exception:
        return None
    if not isinstance(rec, dict):
        return None
    for key in drop:
        rec.pop(key, None)
    return json.dumps(rec, sort_keys=True)


def main():
    for dirpath, _dirs, filenames in os.walk(ROOT):
        for name in filenames:
            if not name.endswith(".jsonl"):
                continue
            path = os.path.join(dirpath, name)
            try:
                with open(path, "rb") as handle:
                    data = handle.read()
            except OSError:
                continue
            lines = [l for l in data.split(b"\n") if l.strip()]
            occurrences = defaultdict(list)  # uuid -> [(line_idx, parent, raw)]
            for index, raw in enumerate(lines):
                try:
                    rec = json.loads(raw)
                except Exception:
                    continue
                if not isinstance(rec, dict):
                    continue
                if (
                    rec.get("type") not in CHAIN_TYPES
                    or rec.get("isSidechain")
                    or "uuid" not in rec
                ):
                    continue
                uuid = rec.get("uuid")
                parent = rec.get("parentUuid")
                if isinstance(uuid, str):
                    occurrences[uuid].append(
                        (index, parent if isinstance(parent, str) else None, raw)
                    )
            conflicted = {
                uuid: occ
                for uuid, occ in occurrences.items()
                if len(occ) > 1 and len({parent for _i, parent, _r in occ}) > 1
            }
            if not conflicted:
                continue
            total = len(lines)
            print(
                f"\n== {name[:8]}...  {len(data) / 1e6:.1f} MB, {total} lines, "
                f"dup-uuids-with-diff-parent: {len(conflicted)}"
            )
            same_body = 0
            same_fingerprint = 0
            gaps = []
            positions = []
            examples = 0
            envelope = ("sessionId", "cwd", "uuid", "parentUuid", "timestamp",
                        "version", "requestId")
            for _uuid, occ in conflicted.items():
                first, last = occ[0], occ[-1]
                gaps.append(last[0] - first[0])
                positions.append(first[0] / max(1, total))
                body_first = normalized_without(first[2], ("parentUuid",))
                body_last = normalized_without(last[2], ("parentUuid",))
                if body_first is not None and body_first == body_last:
                    same_body += 1
                fp_first = normalized_without(first[2], envelope)
                fp_last = normalized_without(last[2], envelope)
                if fp_first is not None and fp_first == fp_last:
                    same_fingerprint += 1
                if examples < 3:
                    type_first = json.loads(first[2]).get("type")
                    type_last = json.loads(last[2]).get("type")
                    print(
                        f"   e.g. lines {first[0]}->{last[0]} types "
                        f"{type_first}/{type_last} parents-differ "
                        f"identical-minus-parent={body_first == body_last} "
                        f"identical-fingerprint={fp_first == fp_last}"
                    )
                    examples += 1
            gaps.sort()
            positions.sort()
            print(
                f"   identical-body-minus-parentUuid: {same_body}/{len(conflicted)}; "
                f"identical-semantic-fingerprint: {same_fingerprint}/{len(conflicted)}"
            )
            print(
                f"   line-gap first->last occurrence: median {gaps[len(gaps) // 2]}, "
                f"max {gaps[-1]}; first-occurrence file position: "
                f"median {positions[len(positions) // 2]:.2f}"
            )


if __name__ == "__main__":
    main()
