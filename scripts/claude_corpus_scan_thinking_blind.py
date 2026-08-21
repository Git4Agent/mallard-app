#!/usr/bin/env python3
"""Corpus gate (equivalence): prove that PROJECTION semantics for
claude.lineage.v1 — proposed in docs/PLAN_CLAUDE_DESKTOP_IMPORT_TOLERANCE.md —
make Claude Desktop's import sanitizer invisible to sync.

Projection semantics: the canonical conversation is derived from the
Desktop-sanitized projection of the transcript, not from the raw rows:

  P1  project(rows): for assistant rows, remove thinking/redacted_thinking
      content blocks; rows left with empty content are dropped entirely —
      uuid, links, sessionId/cwd contributions and all (exactly Desktop
      1.32885.1's observed import rewrite, which is idempotent). All other
      rows pass through. Bytes, captures, probes, and pins stay RAW — the
      projection exists only inside the semantic derivation.
  P2  the chain walk runs on the projection with the shipped positional
      rules; a parentUuid absent from the entire projection STITCHES: the
      effective parent is the nearest preceding chain record in file order.
      (Consistency, not truth-recovery, is the correctness criterion: both
      the original and the sanitized file project to the same record
      sequence, so the stitch picks identically on both. Stitching is also
      append-monotone — it only ever looks backward.)
  P3  a parent present in the projection only at later positions
      (forward-only) severs: the walk roots there. Conservative; corpus: 0.
  P4  identity (sessionId/cwd fallback) and record_count come from the
      projection, so they too are invariant across an import.
  Everything else is unchanged: leaf-wins identity, dup-on-path fails
  closed, caps, torn-tail handling (raw bytes).

The equivalence property under test, for every real file F:
  analyze(F) == analyze(desktop_sanitize(F))
which holds by construction iff the projection is idempotent and the
derivation never peeks outside it — this scan verifies exactly that on the
full corpus, comparing (chain length, leaf uuid, sessionId, digest over
(uuid, effective-parent, fingerprint) triples).

An earlier link-transparent formulation (thinking rows excluded but their
links followed) scored 103/106: three files rewind onto a thinking-only
record, whose deletion destroys the branch attachment — unrecoverable, hence
the projection formulation. Types and structure only; no transcript content
is read into the report.
"""

import hashlib
import json
import os
from collections import Counter, defaultdict

ROOT = os.path.expanduser("~/.claude/projects")
MAX_RECORDS = 100_000
CHAIN_TYPES = ("user", "assistant", "system", "attachment")
THINKING = ("thinking", "redacted_thinking")


def canonical_json(value):
    """Mirror of the Rust codec's canonical_json (sorted keys, compact)."""
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def is_thinking_block(block):
    return isinstance(block, dict) and block.get("type") in THINKING


def fingerprint(rec):
    rec = dict(rec)
    for key in ("sessionId", "cwd", "uuid", "parentUuid", "timestamp",
                "version", "requestId"):
        rec.pop(key, None)
    return hashlib.sha256(canonical_json(rec).encode()).hexdigest()


def desktop_sanitize(rows):
    """Desktop's observed import rewrite == the projection. Idempotent."""
    out = []
    for rec in rows:
        if isinstance(rec, dict) and rec.get("type") == "assistant":
            message = rec.get("message")
            content = message.get("content") if isinstance(message, dict) else None
            if isinstance(content, list) and any(is_thinking_block(b) for b in content):
                remaining = [b for b in content if not is_thinking_block(b)]
                if not remaining:
                    continue  # row deleted; children keep their parentUuid
                rec = dict(rec)
                message = dict(message)
                message["content"] = remaining
                rec["message"] = message
        out.append(rec)
    return out


def analyze(rows):
    """Projection-semantics analysis. Returns (outcome, summary-or-None)."""
    if len(rows) > MAX_RECORDS:
        return "TooManyRecords", None
    rows = desktop_sanitize(rows)  # P1: the derivation sees only the projection
    all_uuids = set()
    chain = []  # (uuid, parent, sid, fingerprint)
    last_sid = None
    for rec in rows:
        if not isinstance(rec, dict):
            return "MalformedJson", None
        u = rec.get("uuid")
        if isinstance(u, str):
            all_uuids.add(u)
        sid = rec.get("sessionId")
        if isinstance(sid, str):
            last_sid = sid
        t = rec.get("type")
        if t not in CHAIN_TYPES or rec.get("isSidechain") or "uuid" not in rec:
            continue
        if not isinstance(u, str) or not u:
            return "MissingMessageId", None
        p = rec.get("parentUuid")
        if p is not None and not isinstance(p, str):
            return "MalformedJson", None
        chain.append((u, p, sid if isinstance(sid, str) else None, fingerprint(rec)))
    if not chain:
        return "NoChainRecords", None

    occurrences = defaultdict(list)  # uuid -> ascending chain positions
    for position, (u, _p, _s, _f) in enumerate(chain):
        occurrences[u].append(position)

    path = []
    roots = Counter()
    stitches = 0
    position = len(chain) - 1
    while True:
        path.append(position)
        parent = chain[position][1]
        if parent is None:
            roots["null"] += 1
            break
        candidates = occurrences.get(parent)
        resolved = None
        if candidates:
            for candidate in reversed(candidates):
                if candidate < position:
                    resolved = candidate
                    break
        if resolved is None:
            if parent not in all_uuids:
                if position == 0:
                    roots["stitch_floor"] += 1   # P2 floor -> root
                    break
                stitches += 1                     # P2 stitch
                resolved = position - 1
            else:
                roots["forward_only"] += 1        # P3 sever
                break
        position = resolved
    path.reverse()

    uuids = [chain[p][0] for p in path]
    if len(set(uuids)) != len(uuids):
        return "DuplicateOnPath", None
    leaf = chain[path[-1]]
    session_id = leaf[2] if leaf[2] is not None else last_sid  # P4
    if session_id is None:
        return "MissingSessionMetadata", None

    digest = hashlib.sha256()
    previous = None
    for p in path:
        u, _raw_parent, _s, f = chain[p]
        digest.update(canonical_json([u, previous, f]).encode())
        previous = u
    return "Complete", {
        "chain_len": len(path),
        "record_count": len(rows),  # P4: projected count
        "leaf": uuids[-1],
        "session_id": session_id,
        "digest": digest.hexdigest(),
        "stitches": stitches,
        "root_kind": next(iter(roots)),
    }


def parse_rows(data: bytes):
    rows = []
    for raw in data.split(b"\n"):
        if not raw.strip():
            continue
        try:
            rows.append(json.loads(raw))
        except Exception:
            return None
    return rows


def main():
    files = 0
    outcomes = Counter()
    equivalent = 0
    mismatches = []
    stitch_files = 0
    stitch_total = 0
    root_kinds = Counter()
    chain_records = 0
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
            if not data.strip():
                continue
            files += 1
            rows = parse_rows(data)
            if rows is None:
                outcomes["MalformedJson"] += 1
                continue
            outcome, original = analyze(rows)
            outcomes[outcome] += 1
            if outcome != "Complete":
                continue
            root_kinds[original["root_kind"]] += 1
            chain_records += original["chain_len"]
            if original["stitches"]:
                stitch_files += 1
                stitch_total += original["stitches"]
            sanitized_outcome, sanitized = analyze(desktop_sanitize(rows))
            if sanitized_outcome != "Complete":
                mismatches.append((name[:8], f"sanitized -> {sanitized_outcome}"))
                continue
            keys = ("chain_len", "record_count", "leaf", "session_id", "digest")
            if all(original[k] == sanitized[k] for k in keys):
                equivalent += 1
            else:
                detail = ", ".join(
                    f"{k}: {original[k]} != {sanitized[k]}"
                    for k in keys if original[k] != sanitized[k]
                )
                mismatches.append((name[:8], detail))
    print(f"files with content: {files}")
    for outcome, count in outcomes.most_common():
        print(f"  {outcome:28s} {count}")
    print(f"root kinds: {dict(root_kinds)}")
    print(f"chain records across Complete files: {chain_records}")
    print(f"files whose walk stitches: {stitch_files} (total stitches on paths: {stitch_total})")
    print(f"EQUIVALENT analyze(F) == analyze(sanitize(F)): {equivalent} / {outcomes['Complete']}")
    if mismatches:
        print("MISMATCHES:")
        for name, detail in mismatches[:20]:
            print(f"  {name}: {detail}")
    else:
        print("MISMATCHES: none")


if __name__ == "__main__":
    main()
