#!/usr/bin/env python3
"""Corpus gate (acceptance): classify ~/.claude/projects session files under
the REVISED claude.lineage.v1 rules R1-R6 from
docs/PLAN_CLAUDE_SESSION_CURSOR_PARITY.md Part A.

This is the exact rule set analyze_claude_transcript implements after the
revision; the scan must be re-run and its numbers recorded in the plan doc
before the rules freeze (R8). Types and structure only; no transcript content
is read into the report.

Rules mirrored here:
  R1  leaf = last chain record in file order
  R2  chain = walk-back from the leaf via parentUuid with POSITIONAL
      resolution: a parent reference from the record at file position i
      resolves to the latest occurrence of that uuid at a position < i
  R3  parentUuid null terminates the walk wherever it sits (mid-file roots)
  R4  re-emitted uuids shadow positionally; a duplicate uuid appearing twice
      ON the derived chain fails closed (DuplicateMessageId)
  R5  sessionId / cwd are leaf-wins: the leaf chain record's value, falling
      back to the last record in file order carrying one; fail-closed only
      when no record carries a sessionId
  R6  unchanged fail-closed set: malformed JSON / non-object line, non-string
      uuid, bad parentUuid type, dangling parent on the active path,
      >100_000 records, zero chain records

The scan additionally classifies every trimmed (off-path) chain record so the
rule choice stays corpus-justified:
  pre_root            before the active root (compaction history)
  shadowed_reemission its uuid also occurs ON the path (re-emitted)
  superseded_branch   a later chain record shares its parent (rewind re-ask)
  abandoned_subtree   it has descendants, but the path left its branch
  dead_end_leaf       childless, never superseded -- the only class that
                      could be live content the walk-back would drop
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

    # R1/R2/R3: walk back from the leaf with positional resolution.
    path = []
    position = len(chain_records) - 1
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
            return "DanglingParent", None
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

    # Diagnostics over trimmed records.
    on_path = set(path)
    path_uuid_set = set(path_uuids)
    active_root = path[0]
    children = defaultdict(int)  # uuid -> referencing records at later positions
    latest_child_of_parent = {}  # parent uuid -> latest referencing position
    for position, (_u, p, _s, _c, _t) in enumerate(chain_records):
        if p is not None:
            children[p] += 1
            latest_child_of_parent[p] = position
    trims = Counter()
    dead_end_types = Counter()
    path_sids = {chain_records[p][2] for p in path if chain_records[p][2]}
    for position, (u, p, _s, _c, t) in enumerate(chain_records):
        if position in on_path:
            continue
        if position < active_root:
            trims["pre_root"] += 1
        elif u in path_uuid_set:
            trims["shadowed_reemission"] += 1
        elif p is not None and latest_child_of_parent.get(p, -1) > position:
            trims["superseded_branch"] += 1
        elif children.get(u):
            trims["abandoned_subtree"] += 1
        else:
            trims["dead_end_leaf"] += 1
            dead_end_types[t] += 1
    diagnostics = {
        "chain_len": len(path),
        "chain_type_records": len(chain_records),
        "record_count": record_count,
        "trims": trims,
        "dead_end_types": dead_end_types,
        "mid_file_root": active_root > 0,
        "leaf_sid_fallback": leaf[2] is None,
        "mixed_sids_on_path": len(path_sids) > 1,
    }
    return "Complete", diagnostics


def main():
    outcomes = Counter()
    complete_bytes = 0
    other_bytes = 0
    files = 0
    largest = []
    trims_total = Counter()
    dead_end_types_total = Counter()
    dead_end_files = []
    mid_file_roots = 0
    leaf_sid_fallbacks = 0
    mixed_sid_paths = 0
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
            trims_total.update(diagnostics["trims"])
            dead_end_types_total.update(diagnostics["dead_end_types"])
            if diagnostics["trims"]["dead_end_leaf"]:
                dead_end_files.append(
                    (diagnostics["trims"]["dead_end_leaf"], len(data), name[:8])
                )
            mid_file_roots += diagnostics["mid_file_root"]
            leaf_sid_fallbacks += diagnostics["leaf_sid_fallback"]
            mixed_sid_paths += diagnostics["mixed_sids_on_path"]
    largest.sort(reverse=True)
    dead_end_files.sort(reverse=True)
    print(f"files: {files}")
    for outcome, count in outcomes.most_common():
        print(f"  {outcome:28s} {count}")
    print(f"bytes in Complete files: {complete_bytes / 1e6:.1f} MB")
    print(f"bytes in non-Complete files: {other_bytes / 1e6:.1f} MB")
    print(f"Complete with a mid-file active root: {mid_file_roots}")
    print(f"Complete using the sessionId fallback (leaf carries none): {leaf_sid_fallbacks}")
    print(f"Complete with mixed sessionIds on the active path: {mixed_sid_paths}")
    print(f"trimmed record classes across Complete files: {dict(trims_total)}")
    print(f"dead_end_leaf record types: {dict(dead_end_types_total)}")
    print("top 10 files by dead_end_leaf count (count, size, file):")
    for count, size, name in dead_end_files[:10]:
        print(f"  {count:6d}  {size / 1e6:8.1f} MB  {name}")
    print("largest 10 files -> outcome:")
    for size, outcome, name in largest[:10]:
        print(f"  {size / 1e6:8.1f} MB  {outcome:24s} {name}")


if __name__ == "__main__":
    main()
