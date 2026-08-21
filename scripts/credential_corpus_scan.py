#!/usr/bin/env python3
"""Corpus scan (pre-freeze input) for docs/PLAN_CREDENTIAL_REDACTION.md §12:
approximate the mallard.redact.v1 detector layers over the local Codex and
Claude corpora and report what WOULD be redacted, as types and counts only.

Reports, per provider:
  layer x field-name match counts     which JSON fields trip which layer
  per-label counts                    what the wire would read as
  skip-list hit counts                how often each skip rule shields a leaf
  entropy candidates by field         structural fields whose values often
                                      exceed the 4.5-bit threshold — the
                                      input that freezes §4.2's mallard
                                      skip-list additions (V3) and validates
                                      D6's threshold

House rule: NEVER print content. Field names, rule ids, labels, counts, and
aggregate byte statistics only. No leaf value, path value, or match text is
ever written to the report.
"""

import json
import math
import os
import re
import sys
from collections import Counter

CODEX_ROOT = os.path.expanduser("~/.codex/sessions")
CLAUDE_ROOT = os.path.expanduser("~/.claude/projects")
MAX_FILE_BYTES = 512 * 1024 * 1024

ENTROPY_CANDIDATE = re.compile(r"[A-Za-z0-9+_=-]{10,}")
ENTROPY_THRESHOLD = 4.5

VENDOR_RULES = [
    ("aws-access-key-id", "<aws-access-key>", re.compile(r"\b(?:AKIA|ASIA|ABIA|ACCA)[0-9A-Z]{16}\b")),
    ("github-token", "<github-token>", re.compile(r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36,255}\b")),
    ("gitlab-pat", "<gitlab-token>", re.compile(r"\bglpat-[A-Za-z0-9_\-]{20,}")),
    ("slack-token", "<slack-token>", re.compile(r"\bxox[baprs]-[A-Za-z0-9\-]{10,}")),
    ("stripe-secret-key", "<stripe-key>", re.compile(r"\b[rs]k_(?:live|test)_[A-Za-z0-9]{20,}")),
    ("google-api-key", "<google-api-key>", re.compile(r"\bAIza[0-9A-Za-z_\-]{35}\b")),
    ("private-key-block", "<private-key>", re.compile(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY(?: BLOCK)?-----", re.S)),
    ("jwt", "<jwt>", re.compile(r"\beyJ[A-Za-z0-9_\-]{12,}\.eyJ[A-Za-z0-9_\-]{12,}\.[A-Za-z0-9_/+=\-]{10,}")),
    ("npm-token", "<npm-token>", re.compile(r"\bnpm_[A-Za-z0-9]{36}\b")),
]

PREFIX_RULES = [
    ("anthropic-api-key", "<anthropic-api-key>", re.compile(r"sk-ant-[A-Za-z0-9_\-]{20,}")),
    ("openai-project-key", "<openai-api-key>", re.compile(r"sk-proj-[A-Za-z0-9_\-]{20,}")),
    ("openai-legacy-key", "<openai-api-key>", re.compile(r"sk-[A-Za-z0-9]{48}")),
    ("supabase-secret-key", "<supabase-secret-key>", re.compile(r"sb_secret_[A-Za-z0-9_\-]{20,}")),
    ("supabase-access-token", "<supabase-access-token>", re.compile(r"sbp_[a-z0-9_\-]{20,}")),
    ("replicate-token", "<replicate-token>", re.compile(r"r8_[A-Za-z0-9]{20,}")),
    ("huggingface-token", "<huggingface-token>", re.compile(r"hf_[A-Za-z0-9]{20,}")),
    ("groq-api-key", "<api-key>", re.compile(r"gsk_[A-Za-z0-9]{20,}")),
]

CREDENTIALED_URI = re.compile(
    r"(?i)\b[a-z][a-z0-9+.\-]{1,31}://[^\s/?#@\"'`<>:]*:([^\s/?#@\"'`<>]+)@[^\s\"'`<>]+"
)
DB_KEY_SHAPE = (
    r"(?:db|database|pg|postgres|postgresql|mysql|mariadb|redis|mongo|mongodb|"
    r"sqlserver|mssql|jdbc)(?:[_\-]+[a-z0-9]+)*[_\-]*(?:password|passwd|pwd)"
)
CREDENTIAL_VALUE = re.compile(
    r"(?i)(?:^|[^A-Za-z0-9])(" + DB_KEY_SHAPE + r")\s*=\s*(\"[^\"]*\"|'[^']*'|[^\s,;&]+)"
)
PASSWORD_ASSIGNMENT = re.compile(r"(?i)(?:^|[?&;\s])(?:password|pwd)=(\"[^\"]*\"|'[^']*'|[^&;\s\"']+)")

SKIP_KEY_SUFFIXES = ("signature", "id", "ids")
SKIP_PATH_KEYS = {"filepath", "file_path", "cwd", "root", "directory", "dir", "path"}
SKIP_EXACT_KEYS = {"encrypted_content", "__mallard_path_map__"}

PLACEHOLDERS = {
    "redacted", "[redacted]", "<redacted>", "changeme", "example",
    "placeholder", "your_password", "your_db_password", "your_secret",
    "secret_here",
}
BRACKET_INTERIOR = re.compile(r"^[a-z][a-z_-]*$")


def shannon(s):
    if not s:
        return 0.0
    counts = Counter(s.encode("utf-8", "surrogatepass"))
    n = float(sum(counts.values()))
    return -sum((c / n) * math.log2(c / n) for c in counts.values())


def is_placeholder(value):
    t = value.strip().strip("\"'")
    if not t:
        return True
    if len(t) >= 5 and t.startswith("<") and t.endswith(">") and BRACKET_INTERIOR.match(t[1:-1]):
        return True
    n = t.lower()
    if n.startswith("${") and n.endswith("}"):
        return True
    if n in PLACEHOLDERS:
        return True
    return len(n) >= 3 and n[0] in "*x.-" and all(c == n[0] for c in n)


def should_skip(key):
    lower = key.lower()
    if any(lower.endswith(suffix) for suffix in SKIP_KEY_SUFFIXES):
        return "suffix:" + next(s for s in SKIP_KEY_SUFFIXES if lower.endswith(s))
    if lower in SKIP_PATH_KEYS:
        return "path-field"
    if key in SKIP_EXACT_KEYS:
        return "mallard:" + key
    return None


class Tally:
    def __init__(self):
        self.files = 0
        self.records = 0
        self.leaves = 0
        self.bytes_scanned = 0
        self.layer_by_field = Counter()   # (layer, field) -> hits
        self.labels = Counter()           # label -> hits
        self.skips = Counter()            # skip rule -> shielded leaves
        self.entropy_by_field = Counter() # field -> candidate hits > threshold
        self.would_change_records = 0


def value_scan_mode(text):
    """Mirror of the Rust value-shape guard (corpus-frozen 2026-08-21)."""
    t = text.strip()
    if len(t) >= 5 and t[:5].lower() == "data:" and ";base64," in t[:160]:
        return "skip"
    if t and not any(c.isspace() for c in t):
        head = t[:8].lower()
        if head.startswith(("http://", "https://", "file://")) or t.startswith("/"):
            return "entropy-exempt"
    return "full"


def scan_leaf(key, text, tally):
    tally.leaves += 1
    tally.bytes_scanned += len(text)
    hit = False
    mode = value_scan_mode(text)
    if mode == "skip":
        tally.skips["value:data-uri"] += 1
        return False
    entropy_candidates = ENTROPY_CANDIDATE.finditer(text) if mode == "full" else ()
    if mode == "entropy-exempt":
        tally.skips["value:bare-url-or-path"] += 1
    for m in entropy_candidates:
        if shannon(m.group(0)) > ENTROPY_THRESHOLD:
            tally.layer_by_field[("entropy", key or "<array-item>")] += 1
            tally.entropy_by_field[key or "<array-item>"] += 1
            tally.labels["<secret>"] += 1
            hit = True
    for rules, layer in ((VENDOR_RULES, "vendor"), (PREFIX_RULES, "prefix")):
        for rule_id, label, pattern in rules:
            n = len(pattern.findall(text))
            if n:
                tally.layer_by_field[(layer + ":" + rule_id, key or "<array-item>")] += n
                tally.labels[label] += n
                hit = True
    for m in CREDENTIALED_URI.finditer(text):
        if not is_placeholder(m.group(1)):
            tally.layer_by_field[("credentialed-uri", key or "<array-item>")] += 1
            tally.labels["<password>"] += 1
            hit = True
    for m in CREDENTIAL_VALUE.finditer(text):
        if not is_placeholder(m.group(2)):
            tally.layer_by_field[("credential-key-value", key or "<array-item>")] += 1
            tally.labels["<password>"] += 1
            hit = True
    if "=" in text:
        for m in PASSWORD_ASSIGNMENT.finditer(text):
            if not is_placeholder(m.group(1)):
                tally.layer_by_field[("password-assignment", key or "<array-item>")] += 1
                tally.labels["<password>"] += 1
                hit = True
    return hit


def walk(key, value, tally):
    changed = False
    if isinstance(value, dict):
        kind = value.get("type")
        if isinstance(kind, str) and (kind.startswith("image") or kind == "base64"):
            tally.skips["object:image/base64"] += 1
            return False
        for child_key, child in value.items():
            skip = should_skip(child_key)
            if skip:
                if isinstance(child, str):
                    tally.skips[skip] += 1
                continue
            changed |= walk(child_key, child, tally)
    elif isinstance(value, list):
        for child in value:
            changed |= walk(key, child, tally)
    elif isinstance(value, str):
        changed |= scan_leaf(key, value, tally)
    return changed


def scan_file(path, tally):
    try:
        size = os.path.getsize(path)
        if size > MAX_FILE_BYTES:
            return
        with open(path, "rb") as handle:
            data = handle.read()
    except OSError:
        return
    tally.files += 1
    for raw in data.split(b"\n"):
        if not raw.strip():
            continue
        try:
            record = json.loads(raw)
        except Exception:
            continue
        tally.records += 1
        if walk("", record, tally):
            tally.would_change_records += 1


def scan_root(root):
    tally = Tally()
    for dirpath, _dirnames, filenames in os.walk(root):
        for name in filenames:
            if name.endswith(".jsonl"):
                scan_file(os.path.join(dirpath, name), tally)
    return tally


def report(name, tally):
    print(f"=== {name} ===")
    print(
        f"files={tally.files} records={tally.records} leaves={tally.leaves} "
        f"scanned_mb={tally.bytes_scanned / 1e6:.1f} "
        f"records_with_findings={tally.would_change_records}"
    )
    print("-- labels --")
    for label, count in tally.labels.most_common():
        print(f"  {label}: {count}")
    print("-- layer x field (top 40) --")
    for (layer, field), count in tally.layer_by_field.most_common(40):
        print(f"  {layer} @ {field}: {count}")
    print("-- skip shields --")
    for rule, count in tally.skips.most_common():
        print(f"  {rule}: {count}")
    print("-- entropy candidates by field (top 25; skip-list candidates) --")
    for field, count in tally.entropy_by_field.most_common(25):
        print(f"  {field}: {count}")
    print()


def main():
    any_root = False
    for name, root in (("codex", CODEX_ROOT), ("claude", CLAUDE_ROOT)):
        if not os.path.isdir(root):
            print(f"=== {name} === (no corpus at {root})")
            continue
        any_root = True
        report(name, scan_root(root))
    return 0 if any_root else 1


if __name__ == "__main__":
    sys.exit(main())
