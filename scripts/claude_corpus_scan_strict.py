#!/usr/bin/env python3
"""Corpus gate (historical baseline): classify ~/.claude/projects session files
under the SHIPPED strict claude.lineage.v1 rules (as of commit 86a03c1).

Mirrors crates/mallard-infrastructure/src/providers/claude/codec.rs
analyze_claude_transcript before the branch-tolerant revision
(docs/PLAN_CLAUDE_SESSION_CURSOR_PARITY.md Part A). Types and structure only;
no transcript content is read into the report.

Shipped strict rules mirrored here:
  - chain record: type in {user,assistant,system,attachment}, not isSidechain,
    has a "uuid" key
  - strict linearity: first chain record parent null; each next chain record's
    parentUuid == previous chain record's uuid
  - re-emission of an already-chained uuid with the same parent -> skipped
    (fingerprint approximated as "same parent")
  - any two distinct sessionId values -> ConflictingSessionMetadata
  - malformed JSON / non-object / non-string uuid / bad parentUuid type ->
    MalformedJson-class Unknown
  - zero chain records / >100_000 records -> Unknown
"""

import json
import os
from collections import Counter

ROOT = os.path.expanduser("~/.claude/projects")
MAX_RECORDS = 100_000
CHAIN_TYPES = ("user", "assistant", "system", "attachment")


def classify(data: bytes):
    lines = [l for l in data.split(b"\n") if l.strip()]
    session_id = None
    chain = {}
    prev_uuid = None
    record_count = 0
    for raw in lines:
        record_count += 1
        if record_count > MAX_RECORDS:
            return "TooManyRecords"
        try:
            rec = json.loads(raw)
        except Exception:
            return "MalformedJson"
        if not isinstance(rec, dict):
            return "MalformedJson"
        sid = rec.get("sessionId")
        if isinstance(sid, str):
            if session_id is None:
                session_id = sid
            elif sid != session_id:
                return "ConflictingSessionMetadata"
        t = rec.get("type")
        side = bool(rec.get("isSidechain"))
        if t not in CHAIN_TYPES or side or "uuid" not in rec:
            continue
        u = rec.get("uuid")
        if not isinstance(u, str) or not u:
            return "MissingMessageId"
        p = rec.get("parentUuid")
        if p is not None and not isinstance(p, str):
            return "MalformedJson"
        if u in chain:
            if chain[u] == p:
                continue
            return "NonLinearParentage"
        if p != prev_uuid:
            return "NonLinearParentage"
        chain[u] = p
        prev_uuid = u
    if session_id is None:
        return "MissingSessionMetadata"
    if not chain:
        return "NoChainRecords"
    return "Complete"


def main():
    outcomes = Counter()
    complete_bytes = 0
    other_bytes = 0
    files = 0
    largest = []
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
            files += 1
            outcome = classify(data)
            outcomes[outcome] += 1
            if outcome == "Complete":
                complete_bytes += len(data)
            else:
                other_bytes += len(data)
            largest.append((len(data), outcome, name[:8]))
    largest.sort(reverse=True)
    print(f"files: {files}")
    for outcome, count in outcomes.most_common():
        print(f"  {outcome:28s} {count}")
    print(f"bytes in Complete files: {complete_bytes / 1e6:.1f} MB")
    print(f"bytes in non-Complete files: {other_bytes / 1e6:.1f} MB")
    print("largest 10 files -> outcome:")
    for size, outcome, name in largest[:10]:
        print(f"  {size / 1e6:8.1f} MB  {outcome:24s} {name}")


if __name__ == "__main__":
    main()
