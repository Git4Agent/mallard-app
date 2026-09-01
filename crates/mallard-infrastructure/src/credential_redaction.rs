//! One-way credential redaction for the conversation wire form
//! (`mallard.redact.v1`).
//!
//! Every byte stream that leaves the machine — Codex rollout records, Claude
//! transcript lines, sidecars, provider-state files, content-derived
//! descriptor metadata — passes this deterministic encoder at the push
//! boundary. Detected secrets are replaced by typed placeholders from a
//! frozen vocabulary (`<password>`, `<anthropic-api-key>`, `<secret>`, …);
//! there is no decode, so placeholder spans materialize verbatim on every
//! machine, the origin included after a cloud-ahead pull.
//!
//! Detection algorithm adopted from `entireio/cli` (MIT), which vendors the
//! `betterleaks` rule corpus (a gitleaks fork, MIT): layered leaf detection
//! (entropy, vendor rules, provider prefixes, credentialed URIs, connection
//! strings, credential key/value pairs), the JSONL field skip-list, the
//! placeholder allowlist that makes `redact(redact(x)) == redact(x)` a real
//! fixpoint, and the raw-line application mode that keeps untouched lines
//! byte-identical. The replacement scheme deliberately diverges: instead of
//! one `REDACTED` token, each region takes a typed label chosen by the
//! winning rule — a shape the reference's own `isBracketedPlaceholder`
//! allowlist already treats as inert, which is what keeps the typed
//! vocabulary fixpoint-safe under every layer.
//!
//! The load-bearing property is purity: `redact` is a function of (record
//! bytes, ruleset) alone, with per-record independence. A re-push of a
//! locally-extended thread therefore re-encodes its wire prefix
//! byte-identically, which is what the accepted wire pin and the append
//! fast-forward depend on. The one drift axis left is the ruleset itself —
//! patterns *and* label spellings — so both feed the content fingerprint
//! stamped onto every cloud thread row (`ConversationWireRedaction`), and a
//! mismatch is the typed `redaction_ruleset_mismatch` blocker.
//!
//! Gate errors never echo matched text: payloads carry record/line index,
//! JSON pointer, and rule id only.

use mallard_domain::hex_lower;
use regex::{Regex, RegexSet};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

use crate::payload::PayloadBytes;

/// The single compiled-in ruleset codec this binary carries.
pub const REDACTION_CODEC_V1: &str = "mallard.redact.v1";

/// Shannon entropy floor for the L1 candidate layer (bits per char). Hex
/// strings top out at 4.0 and UUIDs near log2(17), so digests and ids are
/// structurally safe without special-casing.
const ENTROPY_THRESHOLD_MILLIBITS: u32 = 4_500;

const LABEL_SECRET: &str = "<secret>";
const LABEL_PASSWORD: &str = "<password>";

/// The frozen label vocabulary. Every interior matches `^[a-z][a-z_-]*$`
/// with total length ≥ 5 — exactly the reference's bracketed-placeholder
/// shape, so every label is natively allowlist-inert. Renaming a label
/// changes wire bytes and is a ruleset bump (the spellings feed the
/// fingerprint).
pub const LABEL_VOCABULARY: &[&str] = &[
    "<secret>",
    "<password>",
    "<api-key>",
    "<private-key>",
    "<jwt>",
    "<aws-access-key>",
    "<aws-secret-key>",
    "<github-token>",
    "<gitlab-token>",
    "<slack-token>",
    "<stripe-key>",
    "<google-api-key>",
    "<npm-token>",
    "<pypi-token>",
    "<sendgrid-key>",
    "<twilio-key>",
    "<anthropic-api-key>",
    "<openai-api-key>",
    "<supabase-secret-key>",
    "<supabase-access-token>",
    "<replicate-token>",
    "<huggingface-token>",
];

/// One anchored vendor or provider-prefix rule. `capture` selects the
/// replaced region: the whole match, or capture group 1 for context rules
/// whose match includes the key name.
struct VendorRule {
    id: &'static str,
    label: &'static str,
    pattern: &'static str,
    capture: bool,
}

macro_rules! rule {
    ($id:literal, $label:literal, $pattern:literal) => {
        VendorRule {
            id: $id,
            label: $label,
            pattern: $pattern,
            capture: false,
        }
    };
    ($id:literal, $label:literal, $pattern:literal, capture) => {
        VendorRule {
            id: $id,
            label: $label,
            pattern: $pattern,
            capture: true,
        }
    };
}

/// L2: curated vendor rules ported from the betterleaks corpus (MIT,
/// gitleaks lineage). Word-anchored where the format allows; findings whose
/// text is a known placeholder are skipped on exact match only.
const VENDOR_RULES: &[VendorRule] = &[
    rule!(
        "aws-access-key-id",
        "<aws-access-key>",
        r"\b(?:AKIA|ASIA|ABIA|ACCA)[0-9A-Z]{16}\b"
    ),
    rule!(
        "aws-secret-access-key",
        "<aws-secret-key>",
        r#"(?i)\baws[_\-a-z0-9]*(?:secret|private)[_\-a-z0-9]*\s*[:=]\s*["']?([A-Za-z0-9/+=]{40})\b"#,
        capture
    ),
    rule!(
        "github-token",
        "<github-token>",
        r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36,255}\b"
    ),
    rule!(
        "github-fine-grained-pat",
        "<github-token>",
        r"\bgithub_pat_[A-Za-z0-9]{22}_[A-Za-z0-9]{59}\b"
    ),
    rule!(
        "gitlab-pat",
        "<gitlab-token>",
        r"\bglpat-[A-Za-z0-9_\-]{20,}"
    ),
    rule!(
        "gitlab-runner-token",
        "<gitlab-token>",
        r"\bglrt-[A-Za-z0-9_\-]{20,}"
    ),
    rule!(
        "slack-token",
        "<slack-token>",
        r"\bxox[baprs]-[A-Za-z0-9\-]{10,}"
    ),
    rule!(
        "slack-app-token",
        "<slack-token>",
        r"\bxapp-[0-9]-[A-Z0-9]+-[0-9]+-[a-f0-9]{40,}"
    ),
    rule!(
        "slack-webhook-url",
        "<slack-token>",
        r"\bhooks\.slack\.com/services/T[A-Za-z0-9_]+/B[A-Za-z0-9_]+/[A-Za-z0-9_]+"
    ),
    rule!(
        "stripe-secret-key",
        "<stripe-key>",
        r"\b[rs]k_(?:live|test)_[A-Za-z0-9]{20,}"
    ),
    rule!(
        "stripe-webhook-secret",
        "<stripe-key>",
        r"\bwhsec_[A-Za-z0-9]{32,}\b"
    ),
    rule!(
        "google-api-key",
        "<google-api-key>",
        r"\bAIza[0-9A-Za-z_\-]{35}\b"
    ),
    rule!(
        "sendgrid-api-key",
        "<sendgrid-key>",
        r"\bSG\.[A-Za-z0-9_\-]{22}\.[A-Za-z0-9_\-]{43}\b"
    ),
    rule!("twilio-api-key", "<twilio-key>", r"\bSK[0-9a-fA-F]{32}\b"),
    rule!("npm-token", "<npm-token>", r"\bnpm_[A-Za-z0-9]{36}\b"),
    rule!(
        "pypi-token",
        "<pypi-token>",
        r"\bpypi-AgEIcHlwaS5vcmc[A-Za-z0-9_\-]{50,}"
    ),
    rule!(
        "private-key-block",
        "<private-key>",
        r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY(?: BLOCK)?-----.*?-----END [A-Z0-9 ]*PRIVATE KEY(?: BLOCK)?-----"
    ),
    rule!(
        "age-secret-key",
        "<private-key>",
        r"\bAGE-SECRET-KEY-1[A-Z0-9]{58}\b"
    ),
    rule!(
        "jwt",
        "<jwt>",
        r"\beyJ[A-Za-z0-9_\-]{12,}\.eyJ[A-Za-z0-9_\-]{12,}\.[A-Za-z0-9_/+=\-]{10,}"
    ),
    rule!(
        "digitalocean-token",
        "<api-key>",
        r"\bdo[por]_v1_[a-f0-9]{64}\b"
    ),
    rule!(
        "shopify-token",
        "<api-key>",
        r"\bshp(?:at|ss|ca|pa)_[a-fA-F0-9]{32}\b"
    ),
    rule!(
        "telegram-bot-token",
        "<api-key>",
        r"\b[0-9]{8,10}:AA[A-Za-z0-9_\-]{33}\b"
    ),
    rule!("mailgun-api-key", "<api-key>", r"\bkey-[a-f0-9]{32}\b"),
    rule!(
        "mailchimp-api-key",
        "<api-key>",
        r"\b[a-f0-9]{32}-us[0-9]{1,2}\b"
    ),
    rule!("databricks-token", "<api-key>", r"\bdapi[a-h0-9]{32}\b"),
    rule!(
        "postman-api-key",
        "<api-key>",
        r"\bPMAK-[a-f0-9]{24}-[a-f0-9]{34}\b"
    ),
    rule!(
        "doppler-token",
        "<api-key>",
        r"\bdp\.pt\.[A-Za-z0-9]{40,}\b"
    ),
    rule!(
        "linear-api-key",
        "<api-key>",
        r"\blin_api_[A-Za-z0-9]{40,}\b"
    ),
    rule!("notion-token", "<api-key>", r"\bntn_[A-Za-z0-9]{40,}\b"),
    rule!("figma-token", "<api-key>", r"\bfigd_[A-Za-z0-9_\-]{40,}\b"),
    rule!(
        "sentry-token",
        "<api-key>",
        r"\bsntrys_eyJ[A-Za-z0-9+/=_\-]{50,}"
    ),
    rule!("vault-token", "<api-key>", r"\bhv[sb]\.[A-Za-z0-9_\-]{24,}"),
    rule!(
        "grafana-cloud-token",
        "<api-key>",
        r"\bglc_[A-Za-z0-9+/=]{32,}\b"
    ),
    rule!(
        "openrouter-api-key",
        "<api-key>",
        r"\bsk-or-v1-[a-f0-9]{64}\b"
    ),
];

/// L3: deterministic provider prefixes for low-entropy formats the other
/// layers miss in isolation. Deliberately **not** left-anchored with `\b`,
/// per the reference rationale: a glued prefix (`FOO_sb_secret_…`, or an
/// escape-abutting `…\nsb_secret_…` in raw fallback paths) would otherwise
/// slip through. Over-redaction of look-alike identifiers is the accepted
/// cost.
const PROVIDER_PREFIX_RULES: &[VendorRule] = &[
    rule!(
        "supabase-secret-key",
        "<supabase-secret-key>",
        r"sb_secret_[A-Za-z0-9_\-]{20,}"
    ),
    rule!(
        "supabase-access-token",
        "<supabase-access-token>",
        r"sbp_[a-z0-9_\-]{20,}"
    ),
    rule!(
        "anthropic-api-key",
        "<anthropic-api-key>",
        r"sk-ant-[A-Za-z0-9_\-]{20,}"
    ),
    rule!(
        "openai-project-key",
        "<openai-api-key>",
        r"sk-proj-[A-Za-z0-9_\-]{20,}"
    ),
    rule!(
        "openai-api-key",
        "<openai-api-key>",
        r"sk-[A-Za-z0-9]{20}T3BlbkFJ[A-Za-z0-9]{20,}"
    ),
    rule!(
        "openai-legacy-key",
        "<openai-api-key>",
        r"sk-[A-Za-z0-9]{48}"
    ),
    rule!(
        "replicate-token",
        "<replicate-token>",
        r"r8_[A-Za-z0-9]{20,}"
    ),
    rule!(
        "huggingface-token",
        "<huggingface-token>",
        r"hf_[A-Za-z0-9]{20,}"
    ),
    rule!("groq-api-key", "<api-key>", r"gsk_[A-Za-z0-9]{20,}"),
    rule!("tavily-api-key", "<api-key>", r"tvly-[A-Za-z0-9_\-]{20,}"),
    rule!("perplexity-api-key", "<api-key>", r"pplx-[A-Za-z0-9]{20,}"),
    rule!("xai-api-key", "<api-key>", r"xai-[A-Za-z0-9_\-]{20,}"),
];

/// Keys whose values are never scanned or rewritten (case-insensitive
/// suffix match): cryptographic attestations and identifiers. Redacting a
/// Claude thinking `signature` breaks transcript replay; `id`/`ids` covers
/// `uuid`, `parentUuid`, `sessionId`, `call_id`, `requestId`.
const SKIP_KEY_SUFFIXES: &[&str] = &["signature", "id", "ids"];

/// Structural path/directory keys (exact, case-insensitive): protected as
/// fields so the path-remap invariant owns them. Free-text path mentions
/// remain scannable by design (a secret inside prose is scrubbed wherever
/// it appears).
const SKIP_PATH_KEYS: &[&str] = &[
    "filepath",
    "file_path",
    "cwd",
    "root",
    "directory",
    "dir",
    "path",
];

/// Mallard structural additions: Codex reasoning payloads that must survive
/// byte-exact for resume (the Codex analog of thinking signatures), and the
/// wire path-map marker object, which `Decode` validates and strips.
const SKIP_EXACT_KEYS: &[&str] = &["encrypted_content", "__mallard_path_map__"];

/// Lowercase values treated as non-secrets when they appear in a credential
/// value position: prior redactions, documentation placeholders, and
/// obviously-non-real defaults. Shape-matched families (`<name>` brackets,
/// `${VAR}` expansions, mask runs) are handled separately.
const PLACEHOLDER_SECRET_VALUES: &[&str] = &[
    "redacted",
    "[redacted]",
    "<redacted>",
    "changeme",
    "example",
    "placeholder",
    "your_password",
    "your_db_password",
    "your_secret",
    "secret_here",
];

/// Replacement priority: typed vendor/prefix labels beat the password
/// context layers, which beat the generic entropy label. Feeds the merge
/// sort so overlap resolution is fully deterministic.
const PRIORITY_TYPED: u8 = 0;
const PRIORITY_PASSWORD: u8 = 1;
const PRIORITY_SECRET: u8 = 2;

/// A connection-string rule: candidate pattern, rule id, and the gate that
/// decides whether a candidate is treated as credential-bearing.
type ConnectionStringRule<'a> = (&'a Regex, &'static str, fn(&RedactionRuleset, &str) -> bool);

/// One detected region with its replacement label. `start..end` are byte
/// offsets into the scanned string (always char-boundary-valid: they come
/// from regex matches).
struct LabeledRegion {
    start: usize,
    end: usize,
    priority: u8,
    label: &'static str,
    rule: &'static str,
}

/// The compiled detector. One per process (`shared()`); pattern tables are
/// compile-time constants, so construction failure is a programming error.
pub struct RedactionRuleset {
    entropy_candidate: Regex,
    vendor_set: RegexSet,
    vendor: Vec<Regex>,
    prefix_set: RegexSet,
    prefix: Vec<Regex>,
    credentialed_uri: Regex,
    jdbc: Regex,
    database_url: Regex,
    keyword_dsn: Regex,
    semicolon_conn: Regex,
    password_assignment: Regex,
    credential_value: Regex,
    keyword_host: Regex,
    keyword_user: Regex,
    semicolon_server: Regex,
    semicolon_user: Regex,
    credential_json_key: Regex,
    generic_password_key: Regex,
    bracketed_placeholder_interior: Regex,
    fingerprint: String,
}

/// The DB-prefixed credential key shape shared by the assignment regex and
/// the JSON-key regex (reference: `dbPasswordKeyShape`).
const DB_PASSWORD_KEY_SHAPE: &str = r"(?:db|database|pg|postgres|postgresql|mysql|mariadb|redis|mongo|mongodb|sqlserver|mssql|jdbc)(?:[_\-]+[a-z0-9]+)*[_\-]*(?:password|passwd|pwd)";

const ENTROPY_CANDIDATE_PATTERN: &str = r"[A-Za-z0-9+_=\-]{10,}";
/// Credentialed URI with the password as capture group 1. The character
/// classes exclude `<` and `>` — load-bearing for the fixpoint: after
/// `user:<password>@host` this pattern structurally cannot match.
const CREDENTIALED_URI_PATTERN: &str =
    r#"(?i)\b[a-z][a-z0-9+.\-]{1,31}://[^\s/?#@"'`<>:]*:([^\s/?#@"'`<>]+)@[^\s"'`<>]+"#;
const JDBC_PATTERN: &str = r#"(?i)\bjdbc:[^\s"'<>`]+"#;
const DATABASE_URL_PATTERN: &str =
    r#"(?i)\b(?:postgres(?:ql)?|mysql|mariadb|mongodb(?:\+srv)?|redis)://[^\s"'<>`]+"#;
const KEYWORD_DSN_PATTERN: &str = r#"(?i)\b[a-z_][a-z0-9_]*=(?:"[^"]*"|'[^']*'|[^\s"']+)(?:\s+[a-z_][a-z0-9_]*=(?:"[^"]*"|'[^']*'|[^\s"']+)){2,}"#;
const SEMICOLON_CONN_PATTERN: &str = r#"(?i)\b[a-z][a-z0-9 _\-]*=(?:\{[^}]*\}|"[^"]*"|'[^']*'|[^=;"'\s]+)(?:;[a-z][a-z0-9 _\-]*=(?:\{[^}]*\}|"[^"]*"|'[^']*'|[^=;"'\s]+)){2,}"#;
const PASSWORD_ASSIGNMENT_PATTERN: &str =
    r#"(?i)(?:^|[?&;\s])(?:password|pwd)=("[^"]*"|'[^']*'|[^&;\s"']+)"#;
const KEYWORD_HOST_PATTERN: &str = r"(?i)(?:^|\s)host=";
const KEYWORD_USER_PATTERN: &str = r"(?i)(?:^|\s)user=";
const SEMICOLON_SERVER_PATTERN: &str =
    r"(?i)(?:^|;)\s*(?:server|data source|datasource|addr|address|network address)\s*=";
const SEMICOLON_USER_PATTERN: &str = r"(?i)(?:^|;)\s*(?:user id|userid|user|uid)\s*=";
const GENERIC_PASSWORD_KEY_PATTERN: &str = r"(?i)^(?:password|passwd|pwd)$";
const BRACKETED_PLACEHOLDER_INTERIOR_PATTERN: &str = r"^[a-z][a-z_\-]*$";

fn compile(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|error| {
        panic!("credential redaction pattern '{pattern}' failed to compile: {error}")
    })
}

impl RedactionRuleset {
    fn new() -> Self {
        for label in LABEL_VOCABULARY {
            assert!(
                label_shape_valid(label),
                "redaction label '{label}' violates the frozen vocabulary shape"
            );
        }
        for rule in VENDOR_RULES.iter().chain(PROVIDER_PREFIX_RULES) {
            assert!(
                LABEL_VOCABULARY.contains(&rule.label),
                "redaction rule '{}' names a label outside the vocabulary",
                rule.id
            );
        }
        let credential_value_pattern = format!(
            r#"(?i)(?:^|[^A-Za-z0-9])({DB_PASSWORD_KEY_SHAPE})\s*=\s*("[^"]*"|'[^']*'|[^\s,;&]+)"#
        );
        let credential_json_key_pattern = format!("^{DB_PASSWORD_KEY_SHAPE}$");
        Self {
            entropy_candidate: compile(ENTROPY_CANDIDATE_PATTERN),
            vendor_set: RegexSet::new(VENDOR_RULES.iter().map(|rule| rule.pattern))
                .expect("vendor rule set compiles"),
            vendor: VENDOR_RULES
                .iter()
                .map(|rule| compile(rule.pattern))
                .collect(),
            prefix_set: RegexSet::new(PROVIDER_PREFIX_RULES.iter().map(|rule| rule.pattern))
                .expect("provider prefix set compiles"),
            prefix: PROVIDER_PREFIX_RULES
                .iter()
                .map(|rule| compile(rule.pattern))
                .collect(),
            credentialed_uri: compile(CREDENTIALED_URI_PATTERN),
            jdbc: compile(JDBC_PATTERN),
            database_url: compile(DATABASE_URL_PATTERN),
            keyword_dsn: compile(KEYWORD_DSN_PATTERN),
            semicolon_conn: compile(SEMICOLON_CONN_PATTERN),
            password_assignment: compile(PASSWORD_ASSIGNMENT_PATTERN),
            credential_value: compile(&credential_value_pattern),
            keyword_host: compile(KEYWORD_HOST_PATTERN),
            keyword_user: compile(KEYWORD_USER_PATTERN),
            semicolon_server: compile(SEMICOLON_SERVER_PATTERN),
            semicolon_user: compile(SEMICOLON_USER_PATTERN),
            credential_json_key: compile(&credential_json_key_pattern),
            generic_password_key: compile(GENERIC_PASSWORD_KEY_PATTERN),
            bracketed_placeholder_interior: compile(BRACKETED_PLACEHOLDER_INTERIOR_PATTERN),
            fingerprint: ruleset_fingerprint_hex(),
        }
    }

    /// The content fingerprint of this ruleset (patterns, labels, skip
    /// lists, thresholds, vocabulary). Part of the cloud thread identity.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// The `wire_redaction` descriptor stamp this binary publishes.
    pub fn stamp(&self) -> mallard_domain::ConversationWireRedaction {
        mallard_domain::ConversationWireRedaction {
            codec: REDACTION_CODEC_V1.to_string(),
            fingerprint: self.fingerprint.clone(),
        }
    }

    // ------------------------------------------------------------------
    // Detection layers (all pure functions of one string).
    // ------------------------------------------------------------------

    fn detect(&self, s: &str) -> Vec<LabeledRegion> {
        let mut regions = Vec::new();
        match value_scan_mode(s) {
            ValueScanMode::Skip => return regions,
            ValueScanMode::EntropyExempt => {}
            ValueScanMode::Full => self.detect_entropy(s, &mut regions),
        }
        self.detect_vendor(s, &mut regions);
        self.detect_prefixes(s, &mut regions);
        self.detect_credentialed_uris(s, &mut regions);
        self.detect_connection_strings(s, &mut regions);
        self.detect_credential_values(s, &mut regions);
        regions
    }

    /// L1: high-entropy candidates. The guard refuses to start a match on
    /// the trailing letter of a JSON escape so replacement can never
    /// fabricate an invalid escape; `<`/`>` are outside the candidate
    /// charset, so no label can seed or extend a candidate.
    fn detect_entropy(&self, s: &str, regions: &mut Vec<LabeledRegion>) {
        for found in self.entropy_candidate.find_iter(s) {
            let (mut start, end) = (found.start(), found.end());
            if start > 0 && s.as_bytes()[start - 1] == b'\\' {
                if let b'n' | b't' | b'r' | b'b' | b'f' | b'u' | b'"' | b'\\' | b'/' =
                    s.as_bytes()[start]
                {
                    start += 1;
                    if end - start < 10 {
                        continue;
                    }
                }
            }
            if shannon_entropy_millibits(&s[start..end]) > ENTROPY_THRESHOLD_MILLIBITS {
                regions.push(LabeledRegion {
                    start,
                    end,
                    priority: PRIORITY_SECRET,
                    label: LABEL_SECRET,
                    rule: "entropy",
                });
            }
        }
    }

    /// L2: vendor rules, prefiltered by one `RegexSet` pass. Placeholder
    /// findings are skipped only on exact match — splitting a greedy
    /// finding at a placeholder head could leak the tail.
    fn detect_vendor(&self, s: &str, regions: &mut Vec<LabeledRegion>) {
        for index in self.vendor_set.matches(s) {
            let rule = &VENDOR_RULES[index];
            for captures in self.vendor[index].captures_iter(s) {
                let whole = captures.get(0).expect("match has a whole group");
                if self.is_placeholder_secret_value(whole.as_str()) {
                    continue;
                }
                let span = if rule.capture {
                    captures.get(1).unwrap_or(whole)
                } else {
                    whole
                };
                regions.push(LabeledRegion {
                    start: span.start(),
                    end: span.end(),
                    priority: PRIORITY_TYPED,
                    label: rule.label,
                    rule: rule.id,
                });
            }
        }
    }

    /// L3: unanchored provider prefixes.
    fn detect_prefixes(&self, s: &str, regions: &mut Vec<LabeledRegion>) {
        for index in self.prefix_set.matches(s) {
            let rule = &PROVIDER_PREFIX_RULES[index];
            for found in self.prefix[index].find_iter(s) {
                regions.push(LabeledRegion {
                    start: found.start(),
                    end: found.end(),
                    priority: PRIORITY_TYPED,
                    label: rule.label,
                    rule: rule.id,
                });
            }
        }
    }

    /// L4: credentialed URIs, surgical — only the password sub-span is
    /// replaced, so `postgres://user:<password>@host/db` keeps its structure.
    /// The captured password is placeholder-gated, so documentation URIs
    /// (`:${DB_PASS}@`, `:changeme@`) stay readable.
    fn detect_credentialed_uris(&self, s: &str, regions: &mut Vec<LabeledRegion>) {
        for captures in self.credentialed_uri.captures_iter(s) {
            let Some(password) = captures.get(1) else {
                continue;
            };
            if password.start() >= password.end()
                || self.is_placeholder_secret_value(password.as_str())
            {
                continue;
            }
            regions.push(LabeledRegion {
                start: password.start(),
                end: password.end(),
                priority: PRIORITY_PASSWORD,
                label: LABEL_PASSWORD,
                rule: "credentialed-uri",
            });
        }
    }

    /// L5: connection strings (JDBC, database URLs, keyword DSNs, semicolon
    /// strings), each candidate gated exactly as the reference gates it,
    /// then applied surgically: every non-placeholder password-assignment
    /// value inside the candidate becomes `<password>`; hosts, users, and
    /// database names survive.
    fn detect_connection_strings(&self, s: &str, regions: &mut Vec<LabeledRegion>) {
        if !s.contains('=') {
            return;
        }
        let rules: [ConnectionStringRule<'_>; 4] = [
            (&self.jdbc, "conn-jdbc", Self::jdbc_gate),
            (&self.database_url, "conn-database-url", Self::always_gate),
            (
                &self.keyword_dsn,
                "conn-keyword-dsn",
                Self::keyword_dsn_gate,
            ),
            (&self.semicolon_conn, "conn-semicolon", Self::semicolon_gate),
        ];
        for (pattern, rule_id, gate) in rules {
            for found in pattern.find_iter(s) {
                let start = found.start();
                let end = trim_connection_string_end(s, start, found.end());
                if start >= end {
                    continue;
                }
                let candidate = &s[start..end];
                if !gate(self, candidate) {
                    continue;
                }
                self.push_password_assignments(candidate, start, rule_id, regions);
            }
        }
    }

    fn jdbc_gate(&self, candidate: &str) -> bool {
        starts_with_ascii_case_insensitive(candidate, b"jdbc:")
    }

    /// Database URLs need no host/user gate: the emission step itself only
    /// fires on non-placeholder password assignments (`?password=…`), which
    /// is the reference's `hasDatabaseURLSecret` intent without a URL
    /// parser.
    fn always_gate(&self, _candidate: &str) -> bool {
        true
    }

    fn keyword_dsn_gate(&self, candidate: &str) -> bool {
        self.keyword_host.is_match(candidate) && self.keyword_user.is_match(candidate)
    }

    fn semicolon_gate(&self, candidate: &str) -> bool {
        self.semicolon_server.is_match(candidate) && self.semicolon_user.is_match(candidate)
    }

    /// Emit one `<password>` region per non-placeholder password assignment
    /// value inside `candidate` (offsets shifted by `base`), quotes kept.
    fn push_password_assignments(
        &self,
        candidate: &str,
        base: usize,
        rule_id: &'static str,
        regions: &mut Vec<LabeledRegion>,
    ) {
        for captures in self.password_assignment.captures_iter(candidate) {
            let Some(value) = captures.get(1) else {
                continue;
            };
            let (start, end) = unquote_range(candidate, value.start(), value.end());
            if start >= end || self.is_placeholder_secret_value(&candidate[start..end]) {
                continue;
            }
            regions.push(LabeledRegion {
                start: base + start,
                end: base + end,
                priority: PRIORITY_PASSWORD,
                label: LABEL_PASSWORD,
                rule: rule_id,
            });
        }
    }

    /// L7: bounded credential key/value pairs (`DB_PASSWORD=…`), value span
    /// only, matching regardless of entropy.
    fn detect_credential_values(&self, s: &str, regions: &mut Vec<LabeledRegion>) {
        if !s.contains('=') {
            return;
        }
        for captures in self.credential_value.captures_iter(s) {
            let Some(value) = captures.get(2) else {
                continue;
            };
            let (start, end) = unquote_range(s, value.start(), value.end());
            if start >= end || self.is_placeholder_secret_value(&s[start..end]) {
                continue;
            }
            regions.push(LabeledRegion {
                start,
                end,
                priority: PRIORITY_PASSWORD,
                label: LABEL_PASSWORD,
                rule: "credential-key-value",
            });
        }
    }

    // ------------------------------------------------------------------
    // Placeholder allowlist (the fixpoint discipline).
    // ------------------------------------------------------------------

    fn is_placeholder_secret_value(&self, value: &str) -> bool {
        let trimmed = value.trim().trim_matches(['"', '\'']);
        if trimmed.is_empty() {
            return true;
        }
        if self.is_bracketed_placeholder(trimmed) {
            return true;
        }
        let normalized = trimmed.to_lowercase();
        if normalized.starts_with("${") && normalized.ends_with('}') {
            return true;
        }
        if PLACEHOLDER_SECRET_VALUES.contains(&normalized.as_str()) {
            return true;
        }
        is_repeated_char_placeholder(&normalized)
    }

    /// `<name>` documentation placeholders: lowercase interior joined by
    /// `-`/`_`, total length ≥ 5. Digits and mixed case are rejected so
    /// `<hunter2>` still redacts. Every vocabulary label passes this by
    /// construction.
    fn is_bracketed_placeholder(&self, s: &str) -> bool {
        if s.len() < 5 || !s.starts_with('<') || !s.ends_with('>') {
            return false;
        }
        self.bracketed_placeholder_interior
            .is_match(&s[1..s.len() - 1])
    }

    fn is_credential_json_secret_key(&self, key: &str, credential_context: bool) -> bool {
        let normalized = normalize_credential_json_key(key);
        if self.credential_json_key.is_match(&normalized) {
            return true;
        }
        credential_context && self.generic_password_key.is_match(&normalized)
    }

    // ------------------------------------------------------------------
    // Per-string application.
    // ------------------------------------------------------------------

    /// Redact one string: `Some(redacted)` when any layer fired, `None`
    /// when the string is untouched. Pure and deterministic.
    pub fn redact_str(&self, s: &str) -> Option<String> {
        apply_regions(s, self.detect(s))
    }

    /// The rule id of the first detected region, for content-free error
    /// payloads.
    fn first_finding_rule(&self, s: &str) -> Option<&'static str> {
        let mut regions = self.detect(s);
        if regions.is_empty() {
            return None;
        }
        sort_regions(&mut regions);
        Some(regions[0].rule)
    }

    // ------------------------------------------------------------------
    // Value-tree mode (Codex records; single-JSON-value documents).
    // ------------------------------------------------------------------

    /// Redact every eligible string leaf of one JSON value under the skip
    /// rules. Keys are never rewritten; skip-listed subtrees are cloned
    /// verbatim. Returns the redacted value and whether anything changed.
    pub fn redact_value(&self, value: &Value) -> (Value, bool) {
        let mut changed = false;
        let redacted = self.redact_value_inner("", false, value, &mut changed);
        (redacted, changed)
    }

    fn redact_value_inner(
        &self,
        key: &str,
        credential_context: bool,
        value: &Value,
        changed: &mut bool,
    ) -> Value {
        match value {
            Value::Object(object) => {
                if should_skip_object(object) {
                    return value.clone();
                }
                let child_context = credential_context || is_credential_json_object(object);
                let mut output = Map::new();
                for (child_key, child) in object {
                    let child_value = if should_skip_field(child_key) {
                        child.clone()
                    } else {
                        self.redact_value_inner(child_key, child_context, child, changed)
                    };
                    output.insert(child_key.clone(), child_value);
                }
                Value::Object(output)
            }
            Value::Array(values) => Value::Array(
                values
                    .iter()
                    .map(|child| self.redact_value_inner("", credential_context, child, changed))
                    .collect(),
            ),
            Value::String(text) => {
                let redacted = self.redact_leaf(key, credential_context, text);
                match redacted {
                    Some(redacted) => {
                        *changed = true;
                        Value::String(redacted)
                    }
                    None => value.clone(),
                }
            }
            other => other.clone(),
        }
    }

    /// One string leaf: the detector layers, then the JSON credential-key
    /// promotion (a credential-shaped key forces `<password>` even for
    /// low-entropy values like `hunter2`).
    fn redact_leaf(&self, key: &str, credential_context: bool, text: &str) -> Option<String> {
        if let Some(redacted) = self.redact_str(text) {
            return Some(redacted);
        }
        if self.is_credential_json_secret_key(key, credential_context)
            && !self.is_placeholder_secret_value(text)
        {
            return Some(LABEL_PASSWORD.to_string());
        }
        None
    }

    /// The rule id and JSON pointer of the first finding in one value, for
    /// content-free error payloads.
    fn first_value_finding(&self, value: &Value) -> Option<(String, &'static str)> {
        self.first_value_finding_inner("", false, value, String::new())
    }

    fn first_value_finding_inner(
        &self,
        key: &str,
        credential_context: bool,
        value: &Value,
        pointer: String,
    ) -> Option<(String, &'static str)> {
        match value {
            Value::Object(object) => {
                if should_skip_object(object) {
                    return None;
                }
                let child_context = credential_context || is_credential_json_object(object);
                for (child_key, child) in object {
                    if should_skip_field(child_key) {
                        continue;
                    }
                    let child_pointer = format!("{pointer}/{}", escape_pointer_token(child_key));
                    if let Some(found) = self.first_value_finding_inner(
                        child_key,
                        child_context,
                        child,
                        child_pointer,
                    ) {
                        return Some(found);
                    }
                }
                None
            }
            Value::Array(values) => {
                for (index, child) in values.iter().enumerate() {
                    let child_pointer = format!("{pointer}/{index}");
                    if let Some(found) =
                        self.first_value_finding_inner("", credential_context, child, child_pointer)
                    {
                        return Some(found);
                    }
                }
                None
            }
            Value::String(text) => {
                if let Some(rule) = self.first_finding_rule(text) {
                    return Some((pointer, rule));
                }
                if self.is_credential_json_secret_key(key, credential_context)
                    && !self.is_placeholder_secret_value(text)
                {
                    return Some((pointer, "credential-json-key"));
                }
                None
            }
            _ => None,
        }
    }

    /// Push span gate + redaction for one record: redact the value tree,
    /// then prove the transform changed nothing but string leaves, every
    /// changed leaf differing only by vocabulary-member substitutions, and
    /// that the output is a redaction fixpoint. Encoder bugs surface here
    /// as visible build failures, never as silent wire drift.
    pub fn redact_record_gated(&self, record: &Value) -> Result<Value, String> {
        let (redacted, changed) = self.redact_value(record);
        if changed {
            verify_redaction_span_diff(record, &redacted)?;
            self.verify_wire_value(&redacted).map_err(|error| {
                format!("redaction_gate_failed: redacted record is not a fixpoint: {error}")
            })?;
        }
        Ok(redacted)
    }

    /// Wire self-check for one record: `redact(w) == w`. The leak
    /// certificate — a wire value that fails carries something the ruleset
    /// can still detect.
    pub fn verify_wire_value(&self, wire: &Value) -> Result<(), String> {
        match self.first_value_finding(wire) {
            None => Ok(()),
            Some((pointer, rule)) => Err(format!(
                "wire_secret_detected: rule '{rule}' still matches at '{pointer}'"
            )),
        }
    }

    // ------------------------------------------------------------------
    // Raw-line mode (Claude transcripts; JSONL sidecars).
    // ------------------------------------------------------------------

    /// Collect `(key, original, redacted)` replacements from one parsed
    /// line, deduplicated by `(key, original)`.
    fn collect_replacements(&self, value: &Value) -> Vec<JsonReplacement> {
        let mut seen = std::collections::BTreeSet::new();
        let mut replacements = Vec::new();
        self.collect_replacements_inner("", false, value, &mut seen, &mut replacements);
        replacements
    }

    fn collect_replacements_inner(
        &self,
        key: &str,
        credential_context: bool,
        value: &Value,
        seen: &mut std::collections::BTreeSet<(String, String)>,
        replacements: &mut Vec<JsonReplacement>,
    ) {
        match value {
            Value::Object(object) => {
                if should_skip_object(object) {
                    return;
                }
                let child_context = credential_context || is_credential_json_object(object);
                for (child_key, child) in object {
                    if should_skip_field(child_key) {
                        continue;
                    }
                    self.collect_replacements_inner(
                        child_key,
                        child_context,
                        child,
                        seen,
                        replacements,
                    );
                }
            }
            Value::Array(values) => {
                for child in values {
                    self.collect_replacements_inner(
                        "",
                        credential_context,
                        child,
                        seen,
                        replacements,
                    );
                }
            }
            Value::String(text) => {
                if let Some(redacted) = self.redact_leaf(key, credential_context, text) {
                    if seen.insert((key.to_string(), text.clone())) {
                        replacements.push(JsonReplacement {
                            key: key.to_string(),
                            original: text.clone(),
                            redacted,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    /// Redact one JSONL document in raw-line mode: parse each line, collect
    /// replacements, splice them into the raw line text (keyed replacements
    /// restricted to value position). Untouched lines are byte-identical to
    /// the input; lines that do not parse as JSON get the raw string pass.
    /// Each changed line is gate-checked in place: it must still parse, be
    /// a redaction fixpoint, and introduce only vocabulary tokens.
    pub fn redact_jsonl_text(&self, text: &str) -> Result<(String, usize), String> {
        let mut output = String::with_capacity(text.len());
        let mut changed_lines = 0_usize;
        for (index, line) in text.split('\n').enumerate() {
            if index > 0 {
                output.push('\n');
            }
            let line_number = index + 1;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                output.push_str(line);
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
                match self.redact_str(line) {
                    Some(redacted) => {
                        changed_lines += 1;
                        output.push_str(&redacted);
                    }
                    None => output.push_str(line),
                }
                continue;
            };
            let replacements = self.collect_replacements(&value);
            if replacements.is_empty() {
                output.push_str(line);
                continue;
            }
            let spliced = apply_json_replacements(line, &replacements);
            self.verify_spliced_line(line, &spliced, line_number)?;
            changed_lines += 1;
            output.push_str(&spliced);
        }
        Ok((output, changed_lines))
    }

    /// The per-line gate behind the raw splice: the spliced line must still
    /// parse, must be a fixpoint of this ruleset, and every bracketed token
    /// it introduces must belong to the vocabulary.
    fn verify_spliced_line(
        &self,
        original: &str,
        spliced: &str,
        line: usize,
    ) -> Result<(), String> {
        let Ok(value) = serde_json::from_str::<Value>(spliced.trim()) else {
            return Err(format!(
                "redaction_gate_failed: line {line} no longer parses after redaction splice"
            ));
        };
        if let Some((pointer, rule)) = self.first_value_finding(&value) {
            return Err(format!(
                "wire_secret_detected: line {line} still matches rule '{rule}' at '{pointer}' \
                 after redaction"
            ));
        }
        verify_introduced_tokens(original, spliced)
            .map_err(|token_error| format!("redaction_gate_failed: line {line}: {token_error}"))
    }

    // ------------------------------------------------------------------
    // Document routing (sidecars, provider state, arbitrary artifacts).
    // ------------------------------------------------------------------

    /// Redact one artifact with the reference's routing: a document that is
    /// exactly one JSON value gets the field-aware whole-document walk
    /// (spliced into the raw text, formatting preserved); anything else
    /// takes the line-delimited path. Non-UTF-8 bytes pass through
    /// unchanged (documented residual: the detector is textual).
    pub fn redact_document(&self, bytes: Vec<u8>) -> Result<(Vec<u8>, bool), String> {
        let Ok(text) = std::str::from_utf8(&bytes) else {
            return Ok((bytes, false));
        };
        if text.trim().is_empty() {
            return Ok((bytes, false));
        }
        if let Some(value) = parse_single_json_value(text) {
            let replacements = self.collect_replacements(&value);
            if replacements.is_empty() {
                return Ok((bytes, false));
            }
            let spliced = apply_json_replacements(text, &replacements);
            self.verify_spliced_document(text, &spliced)?;
            return Ok((spliced.into_bytes(), true));
        }
        let (redacted, changed_lines) = self.redact_jsonl_text(text)?;
        if changed_lines == 0 {
            Ok((bytes, false))
        } else {
            Ok((redacted.into_bytes(), true))
        }
    }

    fn verify_spliced_document(&self, original: &str, spliced: &str) -> Result<(), String> {
        let Some(value) = parse_single_json_value(spliced) else {
            return Err(
                "redaction_gate_failed: document no longer parses after redaction splice"
                    .to_string(),
            );
        };
        if let Some((pointer, rule)) = self.first_value_finding(&value) {
            return Err(format!(
                "wire_secret_detected: document still matches rule '{rule}' at '{pointer}' \
                 after redaction"
            ));
        }
        verify_introduced_tokens(original, spliced)
            .map_err(|token_error| format!("redaction_gate_failed: {token_error}"))
    }

    /// The publish choke point: prove `redact(bytes) == bytes` and wrap the
    /// certificate in the `RedactedWireBytes` newtype. Conversation publish
    /// paths accept only the newtype, so unredacted bytes are
    /// unrepresentable at the boundary.
    pub fn certify_wire_document(
        &self,
        bytes: impl Into<PayloadBytes>,
    ) -> Result<RedactedWireBytes, String> {
        let bytes = bytes.into();
        let Ok(text) = std::str::from_utf8(bytes.as_bytes()) else {
            return Ok(RedactedWireBytes { bytes });
        };
        if let Some(value) = parse_single_json_value(text) {
            if let Some((pointer, rule)) = self.first_value_finding(&value) {
                return Err(format!(
                    "wire_secret_detected: rule '{rule}' matches at '{pointer}' in the wire \
                     document"
                ));
            }
            return Ok(RedactedWireBytes { bytes });
        }
        for (index, line) in text.split('\n').enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<Value>(trimmed) {
                Ok(value) => {
                    if let Some((pointer, rule)) = self.first_value_finding(&value) {
                        return Err(format!(
                            "wire_secret_detected: line {} matches rule '{rule}' at '{pointer}'",
                            index + 1
                        ));
                    }
                }
                Err(_) => {
                    if let Some(rule) = self.first_finding_rule(line) {
                        return Err(format!(
                            "wire_secret_detected: line {} matches rule '{rule}'",
                            index + 1
                        ));
                    }
                }
            }
        }
        Ok(RedactedWireBytes { bytes })
    }
}

/// Wire bytes certified to be a redaction fixpoint of the compiled-in
/// ruleset. The only producers are the ruleset's own appliers and
/// `certify_wire_document`; conversation publish paths require this type.
#[derive(Clone, Debug)]
pub struct RedactedWireBytes {
    bytes: PayloadBytes,
}

impl RedactedWireBytes {
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> PayloadBytes {
        self.bytes
    }
}

/// The process-wide compiled ruleset.
pub fn shared() -> &'static RedactionRuleset {
    static RULESET: OnceLock<RedactionRuleset> = OnceLock::new();
    RULESET.get_or_init(RedactionRuleset::new)
}

// ----------------------------------------------------------------------
// Region merge and splice.
// ----------------------------------------------------------------------

fn sort_regions(regions: &mut [LabeledRegion]) {
    regions.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then(b.end.cmp(&a.end))
            .then(a.priority.cmp(&b.priority))
            .then(a.label.cmp(b.label))
    });
}

/// Sort (start asc, end desc, priority asc, label lexical asc), merge
/// overlapping-or-adjacent regions into the first-sorted region's label
/// (larger-region-first preserved, so a PEM block containing high-entropy
/// runs stays one `<private-key>`), splice the labels in.
fn apply_regions(s: &str, mut regions: Vec<LabeledRegion>) -> Option<String> {
    regions.retain(|region| region.start < region.end);
    if regions.is_empty() {
        return None;
    }
    sort_regions(&mut regions);
    let mut merged: Vec<LabeledRegion> = Vec::with_capacity(regions.len());
    for region in regions {
        match merged.last_mut() {
            Some(last) if region.start <= last.end => {
                if region.end > last.end {
                    last.end = region.end;
                }
            }
            _ => merged.push(region),
        }
    }
    let mut output = String::with_capacity(s.len());
    let mut previous = 0_usize;
    for region in &merged {
        output.push_str(&s[previous..region.start]);
        output.push_str(region.label);
        previous = region.end;
    }
    output.push_str(&s[previous..]);
    Some(output)
}

fn trim_connection_string_end(s: &str, start: usize, mut end: usize) -> usize {
    while end > start {
        match s.as_bytes()[end - 1] {
            b'.' | b',' | b';' | b':' | b'!' | b'?' | b')' | b']' => end -= 1,
            _ => return end,
        }
    }
    end
}

fn unquote_range(s: &str, start: usize, end: usize) -> (usize, usize) {
    if end - start < 2 {
        return (start, end);
    }
    let (first, last) = (s.as_bytes()[start], s.as_bytes()[end - 1]);
    if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
        return (start + 1, end - 1);
    }
    (start, end)
}

fn is_repeated_char_placeholder(s: &str) -> bool {
    if s.len() < 3 {
        return false;
    }
    let first = s.as_bytes()[0];
    if !matches!(first, b'*' | b'x' | b'.' | b'-') {
        return false;
    }
    s.bytes().all(|byte| byte == first)
}

fn normalize_credential_json_key(key: &str) -> String {
    key.trim().to_lowercase().replace(['-', ' ', '.'], "_")
}

fn is_credential_json_object(object: &Map<String, Value>) -> bool {
    let mut has_host = false;
    let mut has_user = false;
    for key in object.keys() {
        match normalize_credential_json_key(key).as_str() {
            "host" | "hostname" | "server" | "addr" | "address" | "datasource" | "data_source" => {
                has_host = true;
            }
            "user" | "username" | "userid" | "user_id" | "uid" => has_user = true,
            _ => {}
        }
        if has_host && has_user {
            return true;
        }
    }
    false
}

/// Value-shape scan modes, frozen by the corpus scan (types-and-counts,
/// 2026-08-21; 8.2 GB Codex + 311 MB Claude):
///
/// - **Data URIs skip every layer** — the value-level analog of the
///   reference's `type:image*/base64` object skip. Codex spells inline
///   images as `image_url`/`images` data-URI *fields*, which accounted for
///   41.4 M of the corpus's 44.3 M entropy spans; scrubbing encoded image
///   bytes breaks replay for zero secret value.
/// - **Bare URL and bare absolute-path values skip the entropy layer
///   only** — signed-URL query tokens and random filename segments were
///   the next ~2.1 M spans. Every typed layer still runs, so a vendor key,
///   provider prefix, or `user:password@` credential inside a URL or file
///   name still redacts; the surviving residual (opaque high-entropy
///   bearer tokens inside links) is documented in the plan's §11.
///
/// The mode is a pure function of the string, applied identically in
/// redact and verify, so purity and the fixpoint are untouched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ValueScanMode {
    Full,
    EntropyExempt,
    Skip,
}

fn starts_with_ascii_case_insensitive(value: &str, prefix: &[u8]) -> bool {
    value
        .as_bytes()
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

fn value_scan_mode(s: &str) -> ValueScanMode {
    let trimmed = s.trim();
    if starts_with_ascii_case_insensitive(trimmed, b"data:") {
        let head = &trimmed.as_bytes()[..trimmed.len().min(160)];
        if head
            .windows(b";base64,".len())
            .any(|window| window == b";base64,")
        {
            return ValueScanMode::Skip;
        }
    }
    let bare = !trimmed.is_empty() && !trimmed.chars().any(char::is_whitespace);
    if bare
        && (starts_with_ascii_case_insensitive(trimmed, b"http://")
            || starts_with_ascii_case_insensitive(trimmed, b"https://")
            || starts_with_ascii_case_insensitive(trimmed, b"file://")
            || trimmed.starts_with('/'))
    {
        return ValueScanMode::EntropyExempt;
    }
    ValueScanMode::Full
}

fn should_skip_field(key: &str) -> bool {
    let lower = key.to_lowercase();
    if SKIP_KEY_SUFFIXES
        .iter()
        .any(|suffix| lower.ends_with(suffix))
    {
        return true;
    }
    if SKIP_PATH_KEYS.contains(&lower.as_str()) {
        return true;
    }
    SKIP_EXACT_KEYS.contains(&key)
}

fn should_skip_object(object: &Map<String, Value>) -> bool {
    object
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| {
            kind.starts_with("image") || kind == "ImageGeneration" || kind == "base64"
        })
}

/// Shannon entropy in millibits per char over the byte distribution
/// (candidates are ASCII by construction). Integer output keeps the
/// threshold comparison exact and platform-independent.
fn shannon_entropy_millibits(s: &str) -> u32 {
    if s.is_empty() {
        return 0;
    }
    let mut counts = [0_u32; 256];
    for byte in s.bytes() {
        counts[byte as usize] += 1;
    }
    let length = s.len() as f64;
    let mut entropy = 0.0_f64;
    for count in counts {
        if count == 0 {
            continue;
        }
        let p = f64::from(count) / length;
        entropy -= p * p.log2();
    }
    (entropy * 1000.0).round() as u32
}

fn label_shape_valid(label: &str) -> bool {
    if label.len() < 5 || !label.starts_with('<') || !label.ends_with('>') {
        return false;
    }
    let interior = &label[1..label.len() - 1];
    let mut chars = interior.chars();
    matches!(chars.next(), Some('a'..='z'))
        && chars.all(|c| c.is_ascii_lowercase() || c == '_' || c == '-')
}

fn escape_pointer_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

// ----------------------------------------------------------------------
// Raw JSON text splice (reference: applyJSONReplacements /
// replaceKeyedJSONValue). serde_json does not HTML-escape, matching the
// reference's SetEscapeHTML(false), so `<label>` survives encoding
// verbatim.
// ----------------------------------------------------------------------

struct JsonReplacement {
    key: String,
    original: String,
    redacted: String,
}

fn json_encode_string(s: &str) -> String {
    serde_json::to_string(s).expect("strings always JSON-encode")
}

fn apply_json_replacements(text: &str, replacements: &[JsonReplacement]) -> String {
    let mut output = text.to_string();
    for replacement in replacements {
        let original_json = json_encode_string(&replacement.original);
        let redacted_json = json_encode_string(&replacement.redacted);
        if replacement.key.is_empty() {
            output = output.replace(&original_json, &redacted_json);
            continue;
        }
        let key_json = json_encode_string(&replacement.key);
        output = replace_keyed_json_value(&output, &key_json, &original_json, &redacted_json);
    }
    output
}

/// Replace every occurrence of `original_json` that follows `key_json` +
/// optional whitespace + `:` + optional whitespace. Restricting
/// substitution to value positions keeps a key's own text intact when it
/// collides with another field's value.
fn replace_keyed_json_value(
    text: &str,
    key_json: &str,
    original_json: &str,
    redacted_json: &str,
) -> String {
    if !text.contains(key_json) {
        return text.to_string();
    }
    let mut output = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0_usize;
    while i < text.len() {
        let Some(offset) = text[i..].find(key_json) else {
            output.push_str(&text[i..]);
            break;
        };
        let key_end = i + offset + key_json.len();
        output.push_str(&text[i..i + offset]);
        output.push_str(key_json);
        let mut p = key_end;
        while p < text.len() && is_json_whitespace(bytes[p]) {
            p += 1;
        }
        if p >= text.len() || bytes[p] != b':' {
            i = key_end;
            continue;
        }
        p += 1;
        while p < text.len() && is_json_whitespace(bytes[p]) {
            p += 1;
        }
        if p + original_json.len() <= text.len()
            && &text[p..p + original_json.len()] == original_json
        {
            output.push_str(&text[key_end..p]);
            output.push_str(redacted_json);
            i = p + original_json.len();
            continue;
        }
        i = key_end;
    }
    output
}

fn is_json_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

/// `Some(value)` when the text is exactly one JSON value (the single-value
/// routing shape); `None` sends the caller down the line path. Never split
/// a single-JSON-value document and redact fragments.
fn parse_single_json_value(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut stream = serde_json::Deserializer::from_str(trimmed).into_iter::<Value>();
    let first = stream.next()?.ok()?;
    let consumed = stream.byte_offset();
    trimmed[consumed..].trim().is_empty().then_some(first)
}

/// Every maximal `<…>`-shaped token in `redacted` that does not occur in
/// `original` must be a vocabulary member: the check that turns an encoder
/// bug minting a malformed or dynamic label into a visible build failure.
fn verify_introduced_tokens(original: &str, redacted: &str) -> Result<(), String> {
    let mut rest = redacted;
    while let Some(open) = rest.find('<') {
        let candidate = &rest[open..];
        let Some(close) = candidate.find('>') else {
            break;
        };
        let token = &candidate[..=close];
        if token.len() >= 3
            && token[1..token.len() - 1]
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '_' || c == '-')
            && !LABEL_VOCABULARY.contains(&token)
            && !original.contains(token)
        {
            return Err(format!(
                "introduced token of length {} is outside the label vocabulary",
                token.len()
            ));
        }
        rest = &rest[open + 1..];
    }
    Ok(())
}

/// Lockstep structural diff for the value-tree span gate: the redacted
/// record must differ from the source only at scannable string leaves, and
/// each differing leaf only by vocabulary substitutions.
fn verify_redaction_span_diff(source: &Value, redacted: &Value) -> Result<(), String> {
    verify_span_diff_inner(source, redacted, String::new())
}

fn verify_span_diff_inner(source: &Value, redacted: &Value, pointer: String) -> Result<(), String> {
    match (source, redacted) {
        (Value::Object(source_object), Value::Object(redacted_object)) => {
            if source_object.len() != redacted_object.len() {
                return Err(format!(
                    "redaction_gate_failed: object shape changed at '{pointer}'"
                ));
            }
            if should_skip_object(source_object) {
                if source != redacted {
                    return Err(format!(
                        "redaction_gate_failed: skip-listed object changed at '{pointer}'"
                    ));
                }
                return Ok(());
            }
            for ((source_key, source_child), (redacted_key, redacted_child)) in
                source_object.iter().zip(redacted_object.iter())
            {
                if source_key != redacted_key {
                    return Err(format!(
                        "redaction_gate_failed: object key changed at '{pointer}'"
                    ));
                }
                let child_pointer = format!("{pointer}/{}", escape_pointer_token(source_key));
                if should_skip_field(source_key) {
                    if source_child != redacted_child {
                        return Err(format!(
                            "redaction_gate_failed: skip-listed field changed at '{child_pointer}'"
                        ));
                    }
                    continue;
                }
                verify_span_diff_inner(source_child, redacted_child, child_pointer)?;
            }
            Ok(())
        }
        (Value::Array(source_items), Value::Array(redacted_items)) => {
            if source_items.len() != redacted_items.len() {
                return Err(format!(
                    "redaction_gate_failed: array length changed at '{pointer}'"
                ));
            }
            for (index, (source_child, redacted_child)) in
                source_items.iter().zip(redacted_items.iter()).enumerate()
            {
                verify_span_diff_inner(source_child, redacted_child, format!("{pointer}/{index}"))?;
            }
            Ok(())
        }
        (Value::String(source_text), Value::String(redacted_text)) => {
            if source_text == redacted_text {
                return Ok(());
            }
            verify_introduced_tokens(source_text, redacted_text)
                .map_err(|error| format!("redaction_gate_failed: at '{pointer}': {error}"))
        }
        _ => {
            if source == redacted {
                Ok(())
            } else {
                Err(format!(
                    "redaction_gate_failed: non-string value changed at '{pointer}'"
                ))
            }
        }
    }
}

// ----------------------------------------------------------------------
// Ruleset fingerprint.
// ----------------------------------------------------------------------

/// SHA-256 over the canonical rule table: every pattern, id, label,
/// priority, skip list, placeholder, threshold, and vocabulary spelling.
/// Any change — a pattern tweak or a label rename — is a new fingerprint,
/// i.e. a breaking ruleset bump resolved by the standing wipe.
fn ruleset_fingerprint_hex() -> String {
    let mut input = String::new();
    input.push_str("codec=");
    input.push_str(REDACTION_CODEC_V1);
    input.push('\n');
    input.push_str(&format!(
        "entropy threshold_millibits={ENTROPY_THRESHOLD_MILLIBITS} pattern={ENTROPY_CANDIDATE_PATTERN} label={LABEL_SECRET} priority={PRIORITY_SECRET}\n"
    ));
    for rule in VENDOR_RULES {
        input.push_str(&format!(
            "vendor id={} label={} capture={} priority={PRIORITY_TYPED} pattern={}\n",
            rule.id, rule.label, rule.capture, rule.pattern
        ));
    }
    for rule in PROVIDER_PREFIX_RULES {
        input.push_str(&format!(
            "prefix id={} label={} priority={PRIORITY_TYPED} pattern={}\n",
            rule.id, rule.label, rule.pattern
        ));
    }
    input.push_str(&format!(
        "uri label={LABEL_PASSWORD} priority={PRIORITY_PASSWORD} pattern={CREDENTIALED_URI_PATTERN}\n"
    ));
    for (id, pattern) in [
        ("conn-jdbc", JDBC_PATTERN),
        ("conn-database-url", DATABASE_URL_PATTERN),
        ("conn-keyword-dsn", KEYWORD_DSN_PATTERN),
        ("conn-semicolon", SEMICOLON_CONN_PATTERN),
    ] {
        input.push_str(&format!(
            "conn id={id} label={LABEL_PASSWORD} priority={PRIORITY_PASSWORD} pattern={pattern}\n"
        ));
    }
    input.push_str(&format!(
        "conn-gates {PASSWORD_ASSIGNMENT_PATTERN} {KEYWORD_HOST_PATTERN} {KEYWORD_USER_PATTERN} {SEMICOLON_SERVER_PATTERN} {SEMICOLON_USER_PATTERN}\n"
    ));
    input.push_str(&format!(
        "kv label={LABEL_PASSWORD} priority={PRIORITY_PASSWORD} shape={DB_PASSWORD_KEY_SHAPE} generic={GENERIC_PASSWORD_KEY_PATTERN}\n"
    ));
    input.push_str(&format!(
        "skip suffixes={SKIP_KEY_SUFFIXES:?} paths={SKIP_PATH_KEYS:?} exact={SKIP_EXACT_KEYS:?} objects=image*,ImageGeneration,base64\n"
    ));
    input.push_str(&format!(
        "placeholders values={PLACEHOLDER_SECRET_VALUES:?} bracket={BRACKETED_PLACEHOLDER_INTERIOR_PATTERN} expansion=${{}} masks=*x.-\n"
    ));
    input.push_str(
        "value-shape data-uri=skip-all bare-url-or-path=entropy-exempt schemes=http,https,file\n",
    );
    input.push_str(&format!("vocabulary={LABEL_VOCABULARY:?}\n"));
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex_lower(&hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn redact(s: &str) -> String {
        shared().redact_str(s).unwrap_or_else(|| s.to_string())
    }

    // ------------------------------------------------------------------
    // Per-layer positive/negative tables.
    // ------------------------------------------------------------------

    #[test]
    fn vendor_rules_replace_whole_matches_with_typed_labels() {
        assert_eq!(
            redact("key AKIAIOSFODNN7EXAMPLE ok"),
            "key <aws-access-key> ok"
        );
        assert_eq!(
            redact("ghp_abcdefghijklmnopqrstuvwxyz0123456789"),
            "<github-token>"
        );
        assert_eq!(
            redact("token glpat-ABCdef123_45-xyz789aa"),
            "token <gitlab-token>"
        );
        assert_eq!(redact("xoxb-2444333222111-aaabbbccc"), "<slack-token>");
        assert_eq!(redact("sk_live_4eC39HqLyjWDarjtT1zdp7dc"), "<stripe-key>");
        assert_eq!(
            redact("AIzaSyA-1234567890abcdefghijklmnopqrstu"),
            "<google-api-key>"
        );
        assert_eq!(
            redact("npm_abcd1234efgh5678ijkl9012mnop3456qrst"),
            "<npm-token>"
        );
    }

    #[test]
    fn aws_secret_context_rule_is_surgical() {
        let input = "aws_secret_access_key = wJalrXUtnFEMIK7MDENGbPxRfiCYEXAMPLEKEYAA";
        assert_eq!(redact(input), "aws_secret_access_key = <aws-secret-key>");
    }

    #[test]
    fn pem_block_collapses_to_one_private_key_label() {
        let pem = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\n-----END OPENSSH PRIVATE KEY-----";
        assert_eq!(
            redact(&format!("cat:\n{pem}\ndone")),
            "cat:\n<private-key>\ndone"
        );
    }

    #[test]
    fn jwt_replaces_with_jwt_label() {
        let jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        assert_eq!(redact(&format!("auth {jwt} ok")), "auth <jwt> ok");
    }

    #[test]
    fn provider_prefixes_match_unanchored() {
        assert_eq!(
            redact("sk-ant-api03-abcdefghijklmnop-qrstuvwx"),
            "<anthropic-api-key>"
        );
        // Glued prefixes still match: no left `\b` anchor by design. A
        // low-entropy body is exactly the case only this layer covers.
        assert_eq!(
            redact("FOO_sb_secret_aaaabbbbccccddddeeee"),
            "FOO_<supabase-secret-key>"
        );
        // A high-entropy glued body is one entropy candidate that starts
        // earlier, so the merge resolves it to `<secret>` — redacted either
        // way, with reference-parity span selection.
        assert_eq!(redact("FOO_sb_secret_AbCdEfGhIjKlMnOpQrStUv"), "<secret>");
        assert_eq!(redact("hf_abcdefghijklmnopqrstuv"), "<huggingface-token>");
        assert_eq!(redact("r8_abcdefghijklmnopqrstuv"), "<replicate-token>");
        assert_eq!(redact("gsk_abcdefghijklmnopqrstuv"), "<api-key>");
    }

    #[test]
    fn credentialed_uri_is_surgical() {
        assert_eq!(
            redact("postgres://alice:hunter2secret@db.internal:5432/app"),
            "postgres://alice:<password>@db.internal:5432/app"
        );
        assert_eq!(
            redact("redis://:s3cr3tpass@cache.example.com/0"),
            "redis://:<password>@cache.example.com/0"
        );
    }

    #[test]
    fn documentation_uris_stay_readable() {
        for input in [
            "postgres://user:${DB_PASS}@host/db",
            "postgres://user:changeme@host/db",
            "postgres://user:<your-password>@host/db",
        ] {
            assert_eq!(redact(input), input);
        }
    }

    #[test]
    fn keyword_dsn_is_surgical_and_gated() {
        assert_eq!(
            redact("host=db.internal user=alice password=tops3cret dbname=app"),
            "host=db.internal user=alice password=<password> dbname=app"
        );
        // No host/user context: not a DSN, and `password=` alone is not an
        // L7 key shape.
        let ungated = "flag=1 mode=fast password=whatever2";
        assert_eq!(redact(ungated), ungated);
    }

    #[test]
    fn semicolon_connection_string_is_surgical() {
        assert_eq!(
            redact("Server=db;User Id=sa;Password=s3cretvalue;Database=app"),
            "Server=db;User Id=sa;Password=<password>;Database=app"
        );
    }

    #[test]
    fn jdbc_and_database_url_passwords_are_surgical() {
        assert_eq!(
            redact("jdbc:mysql://db.internal/app?user=alice&password=tops3cret"),
            "jdbc:mysql://db.internal/app?user=alice&password=<password>"
        );
        assert_eq!(
            redact("mysql://db.internal/app?password=tops3cret"),
            "mysql://db.internal/app?password=<password>"
        );
    }

    #[test]
    fn credential_key_value_matches_low_entropy_values() {
        assert_eq!(redact("DB_PASSWORD=hunter2"), "DB_PASSWORD=<password>");
        assert_eq!(
            redact("export POSTGRES_PASSWORD='hunter2'"),
            "export POSTGRES_PASSWORD='<password>'"
        );
        // Quotes survive: the value interior is the region.
        assert_eq!(
            redact(r#"mysql_pwd="hunter2""#),
            r#"mysql_pwd="<password>""#
        );
        // No credential prefix boundary: mydbpassword is one word.
        assert_eq!(redact("mydbpassword=hunter2"), "mydbpassword=hunter2");
    }

    #[test]
    fn placeholder_values_are_allowlisted() {
        for input in [
            "DB_PASSWORD=changeme",
            "DB_PASSWORD=${DB_PASSWORD}",
            "DB_PASSWORD=<password>",
            "DB_PASSWORD=xxxxx",
            "DB_PASSWORD=REDACTED",
        ] {
            assert_eq!(redact(input), input);
        }
    }

    #[test]
    fn entropy_layer_flags_random_tokens_and_spares_structure() {
        let token = "A7fK9q2ZpX4mW8vB1cN6dR3sT5uY0hgJL2eQ";
        assert_eq!(redact(&format!("token {token} ok")), "token <secret> ok");
        // Hex digests and UUIDs are structurally below threshold.
        for safe in [
            "3f786850e387550fdab836ed7e6dc881de23001b",
            "e58ed763-928c-4155-bee9-fdbaaadc15f3",
            "b2da06c5b2da06c5b2da06c5b2da06c5",
        ] {
            assert_eq!(redact(safe), safe, "{safe} must not be redacted");
        }
    }

    #[test]
    fn json_escape_guard_never_fabricates_escapes() {
        // Candidate abutting the trailing letter of an encoded `\n`: the
        // match must not consume the escape letter.
        let input = r"files:\nA7fK9q2ZpX4mW8vB1cN6dR3sT5uY0hgJ";
        let redacted = redact(input);
        assert_eq!(redacted, r"files:\n<secret>");
    }

    // ------------------------------------------------------------------
    // Vocabulary and fixpoint.
    // ------------------------------------------------------------------

    #[test]
    fn value_shape_guard_spares_payloads_but_not_typed_secrets() {
        // Data URIs: full skip, however entropic the payload.
        let image = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQ";
        assert_eq!(shared().redact_str(image), None);
        // Bare URLs: entropy-exempt, so a signed link survives verbatim…
        let signed =
            "https://bucket.s3.amazonaws.com/obj?X-Amz-Signature=A7fK9q2ZpX4mW8vB1cN6dR3sT5uY0hgJ";
        assert_eq!(shared().redact_str(signed), None);
        // …while typed layers still fire inside a bare URL.
        assert_eq!(
            redact("https://api.example.com/v1?key=sk-ant-api03-aaaabbbbccccddddeeee"),
            "https://api.example.com/v1?key=<anthropic-api-key>"
        );
        assert_eq!(
            redact("https://alice:hunter2secret@internal.example.com/x"),
            "https://alice:<password>@internal.example.com/x"
        );
        // Bare absolute paths: entropy-exempt; typed layers still run.
        assert_eq!(
            shared().redact_str("/var/folders/rj/A7fK9q2ZpX4mW8vB1cN6dR3sT5uY0hgJ.png"),
            None
        );
        // Prose containing a URL keeps the full entropy scan.
        assert_eq!(
            redact("see https://x.test then token A7fK9q2ZpX4mW8vB1cN6dR3sT5uY0hgJ"),
            "see https://x.test then token <secret>"
        );
    }

    #[test]
    fn value_shape_guard_accepts_multibyte_text_at_byte_cutoffs() {
        assert_eq!(value_scan_mode("你我"), ValueScanMode::Full);
        assert_eq!(value_scan_mode("abcdeé我"), ValueScanMode::Full);

        let data_uri = format!("data:{};base64,AAAA", "我".repeat(52));
        assert!(!data_uri.is_char_boundary(160));
        assert_eq!(value_scan_mode(&data_uri), ValueScanMode::Full);
    }

    #[test]
    fn every_label_matches_the_frozen_shape() {
        for label in LABEL_VOCABULARY {
            assert!(label_shape_valid(label), "{label} violates the shape");
            assert!(
                shared().is_bracketed_placeholder(label),
                "{label} must be allowlist-inert"
            );
        }
    }

    #[test]
    fn every_label_is_inert_under_every_layer() {
        let ruleset = shared();
        for label in LABEL_VOCABULARY {
            assert!(
                ruleset.redact_str(label).is_none(),
                "{label} must not re-redact bare"
            );
            for context in [
                format!("DB_PASSWORD={label}"),
                format!("postgres://user:{label}@host/db"),
                format!("host=h user=u password={label}"),
                format!("Server=s;User Id=u;Password={label};X=1"),
                format!("jdbc:mysql://h/db?password={label}"),
                format!("prose around {label} stays put"),
            ] {
                assert!(
                    ruleset.redact_str(&context).is_none(),
                    "'{context}' must be a fixpoint"
                );
            }
        }
    }

    #[test]
    fn redact_is_a_fixpoint_over_fixtures() {
        let fixtures = [
            "postgres://alice:hunter2secret@db/app",
            "DB_PASSWORD=hunter2 and sk-ant-api03-abcdefghijklmnop and AKIAIOSFODNN7EXAMPLE",
            "host=h user=u password=A7fK9q2ZpX4mW8vB1cN6dR3sT5uY0hgJ",
            "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQ\n-----END PRIVATE KEY-----",
        ];
        for fixture in fixtures {
            let once = redact(fixture);
            assert_ne!(once, fixture, "fixture must redact");
            assert_eq!(
                shared().redact_str(&once),
                None,
                "redact(redact(x)) == redact(x) for '{fixture}'"
            );
        }
    }

    #[test]
    fn merge_priority_is_deterministic() {
        // Typed rule and entropy candidate over the same span: typed wins.
        let aws_secret = "wJalrXUtnFEMIK7MDENGbPxRfiCYEXAMPLEKEYAA";
        assert_eq!(
            redact(&format!("aws_secret_key = {aws_secret}")),
            "aws_secret_key = <aws-secret-key>"
        );
        // A high-entropy password value: `=` sits inside the entropy
        // candidate charset, so the candidate spans `password=…`, starts
        // first, and wins the merge — reference-parity behavior; the
        // surgical `<password>` shape is reserved for values the entropy
        // layer does not flag on its own.
        assert_eq!(
            redact("host=h user=u password=A7fK9q2ZpX4mW8vB1cN6dR3sT5uY0hgJ"),
            "host=h user=u <secret>"
        );
        // Determinism: repeated runs byte-equal.
        let input =
            "host=h user=u password=A7fK9q2ZpX4mW8vB1cN6dR3sT5uY0hgJ sk-ant-api03-abcdefghijklmnop";
        assert_eq!(redact(input), redact(input));
    }

    // ------------------------------------------------------------------
    // Value-tree mode and skip rules.
    // ------------------------------------------------------------------

    #[test]
    fn value_tree_respects_the_skip_list() {
        let record = json!({
            "uuid": "A7fK9q2ZpX4mW8vB1cN6dR3sT5uY0hgJx",
            "parentUuid": "A7fK9q2ZpX4mW8vB1cN6dR3sT5uY0hgJx",
            "sessionId": "A7fK9q2ZpX4mW8vB1cN6dR3sT5uY0hgJx",
            "signature": "A7fK9q2ZpX4mW8vB1cN6dR3sT5uY0hgJx",
            "thinkingSignature": "A7fK9q2ZpX4mW8vB1cN6dR3sT5uY0hgJx",
            "encrypted_content": "A7fK9q2ZpX4mW8vB1cN6dR3sT5uY0hgJx",
            "file_path": "/tmp/A7fK9q2ZpX4mW8vB1cN6dR3sT5uY0hgJx",
            "cwd": "/home/A7fK9q2ZpX4mW8vB1cN6dR3sT5uY0hgJx",
            "image": {"type": "image", "data": "A7fK9q2ZpX4mW8vB1cN6dR3sT5uY0hgJx"},
            "__mallard_path_map__": {"codec": "mallard.pathmap.v1"},
        });
        let (redacted, changed) = shared().redact_value(&record);
        assert!(!changed);
        assert_eq!(redacted, record);
    }

    #[test]
    fn image_generation_base64_is_skipped_in_legacy_and_projected_shapes() {
        let result = "A7fK9q2ZpX4mW8vB1cN6dR3sT5uY0hgJx=";
        assert_eq!(shared().first_finding_rule(result), Some("entropy"));

        for value in [
            json!({"type": "image_generation_end", "result": result}),
            json!({"type": "ImageGeneration", "result": result}),
        ] {
            let (redacted, changed) = shared().redact_value(&value);
            assert!(!changed);
            assert_eq!(redacted, value);
            shared().verify_wire_value(&value).unwrap();
        }
    }

    #[test]
    fn value_tree_redacts_content_and_promotes_credential_context() {
        let record = json!({
            "type": "message",
            "content": "my key is sk-ant-api03-abcdefghijklmnop",
            "connection": {
                "host": "db.internal",
                "user": "alice",
                "password": "hunter2",
            },
        });
        let (redacted, changed) = shared().redact_value(&record);
        assert!(changed);
        assert_eq!(
            redacted,
            json!({
                "type": "message",
                "content": "my key is <anthropic-api-key>",
                "connection": {
                    "host": "db.internal",
                    "user": "alice",
                    "password": "<password>",
                },
            })
        );
        // Without host+user context a bare low-entropy password key stays.
        let lone = json!({"password": "hunter2"});
        let (unchanged, changed) = shared().redact_value(&lone);
        assert!(!changed);
        assert_eq!(unchanged, lone);
        // DB-shaped keys need no context.
        let shaped = json!({"db_password": "hunter2"});
        let (promoted, changed) = shared().redact_value(&shaped);
        assert!(changed);
        assert_eq!(promoted, json!({"db_password": "<password>"}));
    }

    #[test]
    fn record_gate_passes_real_redactions_and_wire_check_catches_planted_secrets() {
        let record = json!({"content": "key sk-ant-api03-abcdefghijklmnop"});
        let redacted = shared().redact_record_gated(&record).unwrap();
        shared().verify_wire_value(&redacted).unwrap();
        let error = shared().verify_wire_value(&record).unwrap_err();
        assert!(error.starts_with("wire_secret_detected:"), "{error}");
        assert!(error.contains("anthropic-api-key"), "{error}");
        assert!(
            !error.contains("abcdefghijklmnop"),
            "payload must be content-free: {error}"
        );
    }

    // ------------------------------------------------------------------
    // Raw-line mode.
    // ------------------------------------------------------------------

    #[test]
    fn raw_line_mode_keeps_untouched_lines_byte_identical() {
        let text = "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"content\":\"hello world\"}}\n{\"type\":\"assistant\",  \"weird_spacing\" :  \"kept\"}\n";
        let (redacted, changed) = shared().redact_jsonl_text(text).unwrap();
        assert_eq!(changed, 0);
        assert_eq!(redacted, text);
    }

    #[test]
    fn raw_line_mode_keeps_multibyte_conversation_text_unchanged() {
        let text = "{\"type\":\"user\",\"message\":{\"content\":\"你我\"}}\n";
        let (redacted, changed) = shared().redact_jsonl_text(text).unwrap();
        assert_eq!(changed, 0);
        assert_eq!(redacted, text);
        shared()
            .certify_wire_document(redacted.into_bytes())
            .unwrap();
    }

    #[test]
    fn raw_line_mode_redacts_only_matched_values_in_place() {
        let text = "{\"type\":\"user\",\"message\":{\"content\":\"key sk-ant-api03-abcdefghijklmnop\"},  \"keep\" : \"spacing\"}\n";
        let (redacted, changed) = shared().redact_jsonl_text(text).unwrap();
        assert_eq!(changed, 1);
        assert_eq!(
            redacted,
            "{\"type\":\"user\",\"message\":{\"content\":\"key <anthropic-api-key>\"},  \"keep\" : \"spacing\"}\n"
        );
    }

    #[test]
    fn keyed_replacement_is_restricted_to_value_position() {
        // The same string appears as a scanned value and under a
        // skip-listed key: only the scanned position changes.
        let secret = "sk-ant-api03-abcdefghijklmnop";
        let text = format!("{{\"content\":\"{secret}\",\"signature\":\"{secret}\"}}\n");
        let (redacted, _) = shared().redact_jsonl_text(&text).unwrap();
        assert_eq!(
            redacted,
            format!("{{\"content\":\"<anthropic-api-key>\",\"signature\":\"{secret}\"}}\n")
        );
    }

    #[test]
    fn per_record_composition_holds_for_raw_line_mode() {
        let a = "{\"content\":\"DB_PASSWORD=hunter2\"}\n";
        let b = "{\"content\":\"plain\"}\n{\"content\":\"key sk-ant-api03-abcdefghijklmnop\"}\n";
        let (whole, _) = shared().redact_jsonl_text(&format!("{a}{b}")).unwrap();
        let (first, _) = shared().redact_jsonl_text(a).unwrap();
        let (second, _) = shared().redact_jsonl_text(b).unwrap();
        assert_eq!(whole, format!("{first}{second}"));
    }

    // ------------------------------------------------------------------
    // Document routing and certification.
    // ------------------------------------------------------------------

    #[test]
    fn single_json_value_documents_are_field_aware_and_format_preserving() {
        let document = "{\n  \"todos\": [\n    {\"text\": \"rotate sk-ant-api03-abcdefghijklmnop\", \"itemId\": \"A7fK9q2ZpX4mW8vB1cN6dR3sT5uY0hgJx\"}\n  ]\n}\n";
        let (redacted, changed) = shared()
            .redact_document(document.as_bytes().to_vec())
            .unwrap();
        assert!(changed);
        assert_eq!(
            String::from_utf8(redacted).unwrap(),
            "{\n  \"todos\": [\n    {\"text\": \"rotate <anthropic-api-key>\", \"itemId\": \"A7fK9q2ZpX4mW8vB1cN6dR3sT5uY0hgJx\"}\n  ]\n}\n"
        );
    }

    #[test]
    fn non_json_documents_take_the_raw_line_pass() {
        let env_file = "APP=demo\nDB_PASSWORD=hunter2\nDEBUG=1\n";
        let (redacted, changed) = shared()
            .redact_document(env_file.as_bytes().to_vec())
            .unwrap();
        assert!(changed);
        assert_eq!(
            String::from_utf8(redacted).unwrap(),
            "APP=demo\nDB_PASSWORD=<password>\nDEBUG=1\n"
        );
    }

    #[test]
    fn non_utf8_documents_pass_through_unchanged() {
        let bytes = [0xff, 0xfe, 0x00, 0x41];
        let (redacted, changed) = shared().redact_document(bytes.to_vec()).unwrap();
        assert!(!changed);
        assert_eq!(redacted, bytes);
    }

    #[test]
    fn unchanged_documents_retain_the_input_allocation() {
        let bytes = b"{\"content\":\"plain text\"}\n".to_vec();
        let pointer = bytes.as_ptr();
        let (redacted, changed) = shared().redact_document(bytes).unwrap();
        assert!(!changed);
        assert_eq!(redacted.as_ptr(), pointer);
    }

    #[test]
    fn certification_accepts_fixpoints_and_rejects_planted_secrets() {
        let clean = "{\"content\":\"uses <password> and <anthropic-api-key>\"}\n";
        let clean_payload = PayloadBytes::from(clean.as_bytes().to_vec());
        let clean_pointer = clean_payload.as_bytes().as_ptr();
        let certified = shared().certify_wire_document(clean_payload).unwrap();
        assert_eq!(certified.as_bytes().as_ptr(), clean_pointer);
        let dirty = "{\"content\":\"sk-ant-api03-abcdefghijklmnop\"}\n";
        let error = shared()
            .certify_wire_document(dirty.as_bytes().to_vec())
            .unwrap_err();
        assert!(error.starts_with("wire_secret_detected:"), "{error}");
        assert!(!error.contains("abcdefghijklmnop"), "{error}");
    }

    #[test]
    fn out_of_vocabulary_tokens_fail_the_span_gate() {
        let source = json!({"content": "value"});
        let forged = json!({"content": "<forged-label>"});
        let error = verify_redaction_span_diff(&source, &forged).unwrap_err();
        assert!(error.starts_with("redaction_gate_failed:"), "{error}");
        // Pre-existing doc placeholders in the source are not "introduced".
        let doc_source = json!({"content": "<your-password> then sk-ant-api03-abcdefghijklmnop"});
        let doc_redacted = json!({"content": "<your-password> then <anthropic-api-key>"});
        verify_redaction_span_diff(&doc_source, &doc_redacted).unwrap();
    }

    /// §9's scale target, on demand (not CI): a synthetic 20 MB rollout
    /// through the raw-line applier. Run with
    /// `cargo test -p mallard-infrastructure --release -- --ignored bench`.
    #[test]
    #[ignore = "on-demand micro-bench; run under --release"]
    fn bench_20mb_synthetic_rollout() {
        let mut text = String::with_capacity(21 << 20);
        let mut index = 0_usize;
        while text.len() < 20 << 20 {
            index += 1;
            let body = match index % 100 {
                0 => format!("rotate sk-ant-api03-aaaabbbbccccdd{index:06} today"),
                1 => "postgres://alice:hunter2secret@db.internal/app".to_string(),
                _ => format!(
                    "ordinary tool output line {index} listing src/main.rs and \
                     3f786850e387550fdab836ed7e6dc881de23001b digests"
                ),
            };
            text.push_str(
                &serde_json::to_string(&serde_json::json!({
                    "type": "response_item",
                    "payload": {"type": "message", "id": format!("m-{index}"), "content": body},
                }))
                .unwrap(),
            );
            text.push('\n');
        }
        let started = std::time::Instant::now();
        let (redacted, changed_lines) = shared().redact_jsonl_text(&text).unwrap();
        let elapsed = started.elapsed();
        assert!(changed_lines > 0);
        assert!(!redacted.contains("sk-ant-api03-"));
        eprintln!(
            "redacted {:.1} MB in {:.3}s ({:.1} MB/s), {} lines changed",
            text.len() as f64 / 1e6,
            elapsed.as_secs_f64(),
            text.len() as f64 / 1e6 / elapsed.as_secs_f64(),
            changed_lines
        );
    }

    #[test]
    fn fingerprint_is_stable_and_stamped() {
        assert_eq!(shared().fingerprint().len(), 64);
        assert_eq!(shared().fingerprint(), shared().fingerprint());
        let stamp = shared().stamp();
        assert_eq!(stamp.codec, REDACTION_CODEC_V1);
        stamp.validate().unwrap();
    }
}
