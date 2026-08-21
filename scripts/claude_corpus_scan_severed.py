#!/usr/bin/env python3
"""Corpus gate (acceptance): classify ~/.claude/projects session files under
the SEVERED-ROOT amendment to claude.lineage.v1 proposed in
docs/PLAN_CLAUDE_DESKTOP_IMPORT_TOLERANCE.md.

Identical to claude_corpus_scan_revised.py (rules R1-R6) except for one rule:
a parentUuid on the active path with no earlier occurrence no longer fails
closed (DanglingParent). The walk terminates there instead, and the record
holding the unresolvable reference becomes the chain root (a "severed root").
Claude Desktop's first-time CLI-session import (claude://resume) deletes
thinking-only assistant rows without repairing children's parentUuid, so a
dangling parent is a routine first-party state, not rare corruption.

For every severed file the scan reports the Desktop-sanitizer fingerprint:
  dangling_refs        chain records whose parentUuid resolves nowhere earlier
  distinct_missing     distinct referenced uuids absent from the entire file
  thinking_blocks      surviving thinking/redacted_thinking content blocks
A sanitized file shows dangling_refs == distinct_missing (one deleted row per
missing uuid) and thinking_blocks == 0 (the sanitizer strips every survivor).

Types and structure only; no transcript content is read into the report.
"""

import json
import os
from collections import Counter, defaultdict

ROOT = os.path.expanduser("~/.claude/projects")
MAX_RECORDS = 100_000
CHAIN_TYPES = ("user", "assistant", "system", "attachment")


def valid_message_id(value):
    return (
        isinstance(value, str)
        and 0 < len(value) <= 256
        and not any(ord(c) < 32 or ord(c) == 127 for c in value)
    )


def classify(data: bytes):
    """Returns (outcome, diagnostics-dict-or-None)."""
    if not data.strip():
        return "Empty", None
    lines = [l for l in data.split(b"\n") if l.strip()]
    record_count = 0
    chain_records = []  # (uuid, parent, sid, cwd, record_type)
    all_uuids = set()
    thinking_blocks = 0
    last_sid = None
    last_cwd = None
    for raw in lines:
        record_count += 1
        if record_count > MAX_RECORDS:
            return "TooManyRecords", None
        try:
            rec = json.loads(raw)
        except Exception:
            return "MalformedJson", None
        if not isinstance(rec, dict):
            return "MalformedJson", None
        u_any = rec.get("uuid")
        if isinstance(u_any, str):
            all_uuids.add(u_any)
        if rec.get("type") == "assistant":
            content = (rec.get("message") or {}).get("content")
            if isinstance(content, list):
                thinking_blocks += sum(
                    1
                    for c in content
                    if isinstance(c, dict)
                    and c.get("type") in ("thinking", "redacted_thinking")
                )
        sid = rec.get("sessionId")
        if isinstance(sid, str):
            last_sid = sid
        cwd = rec.get("cwd")
        if isinstance(cwd, str) and cwd:
            last_cwd = cwd
        t = rec.get("type")
        side = bool(rec.get("isSidechain"))
        if t not in CHAIN_TYPES or side or "uuid" not in rec:
            continue
        u = rec.get("uuid")
        if not isinstance(u, str):
            return "MissingMessageId", None
        if not valid_message_id(u):
            return "InvalidMessageId", None
        p = rec.get("parentUuid")
        if p is not None and not isinstance(p, str):
            return "MalformedJson", None
        if p is not None and not valid_message_id(p):
            return "InvalidMessageId", None
        chain_records.append(
            (u, p, sid if isinstance(sid, str) else None,
             cwd if isinstance(cwd, str) and cwd else None, t)
        )
    if not chain_records:
        return "NoChainRecords", None

    occurrences = defaultdict(list)  # uuid -> ascending positions
    for position, (u, _p, _s, _c, _t) in enumerate(chain_records):
        occurrences[u].append(position)

    dangling_refs = 0
    missing_parents = set()
    for position, (_u, p, _s, _c, _t) in enumerate(chain_records):
        if p is None:
            continue
        if not any(c < position for c in occurrences.get(p, ())):
            dangling_refs += 1
            if p not in all_uuids:
                missing_parents.add(p)

    # Walk back from the leaf with positional resolution; an unresolvable
    # parent SEVERS the walk instead of failing it (the one amended rule).
    path = []
    position = len(chain_records) - 1
    severed = False
    while True:
        path.append(position)
        parent = chain_records[position][1]
        if parent is None:
            break
        candidates = occurrences.get(parent)
        resolved = None
        if candidates:
            for candidate in reversed(candidates):
                if candidate < position:
                    resolved = candidate
                    break
        if resolved is None:
            severed = True
            break
        position = resolved
    path.reverse()

    # R4: a uuid appearing twice on the derived chain fails closed.
    path_uuids = [chain_records[p][0] for p in path]
    if len(set(path_uuids)) != len(path_uuids):
        return "DuplicateOnPath", None

    # R5: leaf-wins identity.
    leaf = chain_records[path[-1]]
    session_id = leaf[2] if leaf[2] is not None else last_sid
    if session_id is None:
        return "MissingSessionMetadata", None

    diagnostics = {
        "chain_len": len(path),
        "chain_type_records": len(chain_records),
        "record_count": record_count,
        "severed": severed,
        "dangling_refs": dangling_refs,
        "distinct_missing": len(missing_parents),
        "thinking_blocks": thinking_blocks,
        "mid_file_root": path[0] > 0 and not severed,
    }
    return "Complete", diagnostics


def main():
    outcomes = Counter()
    complete_bytes = 0
    other_bytes = 0
    files = 0
    largest = []
    severed_files = []
    mid_file_roots = 0
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
            outcome, diagnostics = classify(data)
            outcomes[outcome] += 1
            largest.append((len(data), outcome, name[:8]))
            if outcome != "Complete":
                other_bytes += len(data)
                continue
            complete_bytes += len(data)
            mid_file_roots += diagnostics["mid_file_root"]
            if diagnostics["severed"]:
                severed_files.append((len(data), name[:8], diagnostics))
    largest.sort(reverse=True)
    severed_files.sort(reverse=True)
    print(f"files: {files}")
    for outcome, count in outcomes.most_common():
        print(f"  {outcome:28s} {count}")
    print(f"bytes in Complete files: {complete_bytes / 1e6:.1f} MB")
    print(f"bytes in non-Complete files: {other_bytes / 1e6:.1f} MB")
    print(f"Complete with a mid-file active root (unsevered): {mid_file_roots}")
    print(f"severed-root files: {len(severed_files)}")
    for size, name, d in severed_files:
        sanitizer = (
            d["dangling_refs"] == d["distinct_missing"]
            and d["thinking_blocks"] == 0
        )
        print(
            f"  {size / 1e6:8.1f} MB  {name}  chain={d['chain_len']}"
            f"/{d['chain_type_records']}  dangling={d['dangling_refs']}"
            f"  missing={d['distinct_missing']}"
            f"  surviving_thinking={d['thinking_blocks']}"
            f"  sanitizer_fingerprint={'YES' if sanitizer else 'no'}"
        )
    print("largest 10 files -> outcome:")
    for size, outcome, name in largest[:10]:
        print(f"  {size / 1e6:8.1f} MB  {outcome:24s} {name}")


if __name__ == "__main__":
    main()
