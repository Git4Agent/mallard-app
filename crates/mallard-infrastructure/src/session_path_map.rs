//! In-cwd path encoder/decoder for the canonical conversation wire forms.
//!
//! Cloud conversation artifacts store a machine-neutral encoding: every path
//! occurrence under the **project working directory** — string values and
//! object keys alike — is replaced by the one absolute-path-shaped token
//! `/__mallard_root__` (URI context: `file:///__mallard_root__`). Paths
//! outside the project root are never touched, by construction: the matcher
//! is built only from the binding's project root (plus its canonical alias
//! spelling), so an out-of-root occurrence is unrepresentable in the wire
//! grammar. One mapper drives Push encode, Pull decode, first-contact
//! adoption, and same-machine repair; the round-trip gates below are the
//! only proof machinery — there is no escaping, and reserved-namespace
//! collisions fail closed with a typed error naming the record.
//!
//! The record transforms are provider-neutral `serde_json::Value` walks.
//! Codex composes them inside its standalone builder (record-level, with
//! the in-band `__mallard_path_map__` declaration); Claude composes them
//! through the line-level facades below (its wire is pure provider JSONL,
//! declared out-of-band by the thread row's `wire_path_map` stamp).

use crate::path_text_boundary::{boundary_valid, left_boundary_valid, valid_json_text};
use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use serde_json::{Map, Value};

/// The single wire codec version declared by `__mallard_path_map__`.
pub const PATH_MAP_CODEC_V1: &str = "mallard.pathmap.v1";
/// The wire-only marker field on the authoritative `session_meta` payload.
pub const PATH_MAP_FIELD: &str = "__mallard_path_map__";
/// The one path-context token. URI-ness is contextual: the same token
/// preceded by `file://` is the URI wire form.
pub const ROOT_TOKEN: &str = "/__mallard_root__";
const URI_ROOT_TOKEN: &str = "file:///__mallard_root__";
const FILE_URI_PREFIX: &str = "file://";
/// The reserved namespace: any literal occurrence in source content blocks
/// the build or pull (there is no escaping machinery).
const RESERVED_NAMESPACE: &str = "/__mallard_root__";

/// The constant wire marker payload: a bare codec declaration. It carries no
/// paths — every machine's patterns derive from its own binding root.
pub fn path_map_marker() -> Value {
    let mut marker = Map::new();
    marker.insert(
        "codec".to_string(),
        Value::String(PATH_MAP_CODEC_V1.to_string()),
    );
    Value::Object(marker)
}

/// Validate a declared wire marker value against the supported codec.
pub fn validate_path_map_marker(marker: &Value) -> Result<(), String> {
    let codec = marker
        .as_object()
        .and_then(|object| object.get("codec"))
        .and_then(Value::as_str)
        .ok_or_else(|| "conversation wire object declares no path-map codec".to_string())?;
    if codec != PATH_MAP_CODEC_V1 {
        return Err(format!(
            "conversation wire object declares unsupported path-map codec '{}'; update Mallard",
            codec.escape_debug()
        ));
    }
    Ok(())
}

/// Root grammar shared by encode and decode. This is also the operation-level
/// Windows gate: a non-POSIX binding root (drive letter, UNC, `\` separators)
/// fails here with a typed error before any rollout bytes are parsed.
pub fn validate_pathmap_root(label: &str, root: &str) -> Result<(), String> {
    if root.len() < 6 || root.len() > 4096 {
        return Err(format!(
            "{label} '{}' is outside the supported 6..=4096 byte range for conversation path \
             remapping",
            root.escape_debug()
        ));
    }
    if !root.starts_with('/') || root.contains('\\') {
        return Err(format!(
            "{label} '{}' is not an absolute POSIX path; conversation Push and Pull support \
             POSIX roots only (Windows paths need their own design)",
            root.escape_debug()
        ));
    }
    if root
        .bytes()
        .any(|byte| byte == b'"' || byte < 0x20 || byte == 0x7f)
    {
        return Err(format!(
            "{label} contains quote or control bytes and cannot be remapped safely"
        ));
    }
    if root.ends_with('/') {
        return Err(format!("{label} '{root}' is not a clean path"));
    }
    let components: Vec<&str> = root[1..].split('/').collect();
    if components.len() < 2 {
        return Err(format!(
            "{label} '{root}' must have at least two path components"
        ));
    }
    if components
        .iter()
        .any(|component| component.is_empty() || *component == "." || *component == "..")
    {
        return Err(format!("{label} '{root}' is not a clean path"));
    }
    if root.contains("__mallard_root__") {
        return Err(format!(
            "{label} '{root}' collides with the reserved wire token namespace"
        ));
    }
    Ok(())
}

/// Percent-encode an absolute path with exactly the escape set the paginated
/// projection's `path_to_file_uri` uses, so projection-emitted URIs and
/// decoded URIs share one spelling.
fn percent_encode_path(path: &str) -> String {
    let mut output = String::with_capacity(path.len());
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                output.push(byte as char);
            }
            other => {
                output.push('%');
                output.push_str(&format!("{other:02X}"));
            }
        }
    }
    output
}

/// One matched-source-spelling pattern with its two replacement spellings.
struct EncodePattern {
    /// Wire form emitted at this span.
    wire: &'static str,
    /// The deterministic canonical *local* respelling of this span: what the
    /// span reads as after an identity-map decode. This is the recorded-span
    /// half of the push span gate.
    canonical: String,
}

/// The shared matcher for one machine's binding root. Patterns are a pure
/// function of `(root, canonical alias, $HOME)` — nothing accumulates across
/// pushes, so re-encoding old records always reproduces the accepted wire
/// prefix byte-for-byte.
pub struct SessionPathMapper {
    root: String,
    encode_automaton: AhoCorasick,
    encode_patterns: Vec<EncodePattern>,
    decode_automaton: AhoCorasick,
    decode_replacements: Vec<String>,
}

impl SessionPathMapper {
    fn match_boundary_valid(
        input: &str,
        start: usize,
        end: usize,
        embedded_json_valid: &mut Option<bool>,
    ) -> bool {
        let bytes = input.as_bytes();
        if boundary_valid(bytes, start, end) {
            return true;
        }
        if !left_boundary_valid(bytes, start) || bytes.get(end) != Some(&b'"') {
            return false;
        }

        *embedded_json_valid.get_or_insert_with(|| valid_json_text(input))
    }

    /// Build the mapper for one binding root. `canonical_root` is the
    /// binding's canonical alias spelling when it differs (macOS `/var` ↔
    /// `/private/var`); `home` is the encoding machine's `$HOME`, used only
    /// to derive the tilde source spelling.
    pub fn new(
        root: &str,
        canonical_root: Option<&str>,
        home: Option<&str>,
    ) -> Result<Self, String> {
        validate_pathmap_root("project root", root)?;
        let mut spellings = vec![root.to_string()];
        if let Some(alias) = canonical_root {
            if alias != root {
                validate_pathmap_root("canonical project root", alias)?;
                spellings.push(alias.to_string());
            }
        }
        let home = home
            .map(|home| home.trim_end_matches('/'))
            .filter(|home| home.starts_with('/') && !home.is_empty());

        let canonical_uri = format!("{FILE_URI_PREFIX}{}", percent_encode_path(root));
        let mut texts: Vec<String> = Vec::new();
        let mut patterns: Vec<EncodePattern> = Vec::new();
        let mut push_pattern = |text: String, wire: &'static str, canonical: String| {
            if !texts.contains(&text) {
                texts.push(text);
                patterns.push(EncodePattern { wire, canonical });
            }
        };
        for spelling in &spellings {
            push_pattern(spelling.clone(), ROOT_TOKEN, root.to_string());
            if let Some(home) = home {
                if let Some(rest) = spelling.strip_prefix(home) {
                    if rest.starts_with('/') {
                        push_pattern(format!("~{rest}"), ROOT_TOKEN, root.to_string());
                    }
                }
            }
            push_pattern(
                format!("{FILE_URI_PREFIX}{spelling}"),
                URI_ROOT_TOKEN,
                canonical_uri.clone(),
            );
            let escaped = percent_encode_path(spelling);
            if escaped != *spelling {
                push_pattern(
                    format!("{FILE_URI_PREFIX}{escaped}"),
                    URI_ROOT_TOKEN,
                    canonical_uri.clone(),
                );
            }
        }
        let encode_automaton = AhoCorasickBuilder::new()
            .match_kind(MatchKind::LeftmostLongest)
            .build(&texts)
            .map_err(|error| format!("build conversation path matcher: {error}"))?;
        let decode_automaton = AhoCorasickBuilder::new()
            .match_kind(MatchKind::LeftmostLongest)
            .build([URI_ROOT_TOKEN, ROOT_TOKEN])
            .map_err(|error| format!("build conversation token matcher: {error}"))?;
        Ok(Self {
            root: root.to_string(),
            encode_automaton,
            encode_patterns: patterns,
            decode_automaton,
            decode_replacements: vec![canonical_uri, root.to_string()],
        })
    }

    /// The primary root spelling this mapper encodes from and decodes to.
    pub fn root(&self) -> &str {
        &self.root
    }

    /// Scan-and-replace over one string with the shared boundary rule. A
    /// match whose boundary fails restarts one byte later, so an embedded
    /// spelling (the bare root inside a boundary-failed URI) is still found.
    /// Returns `None` when nothing matched.
    fn transform_str<R: Fn(usize) -> (String, String)>(
        automaton: &AhoCorasick,
        input: &str,
        replacement: R,
    ) -> Option<(String, String)> {
        let bytes = input.as_bytes();
        let mut primary = String::new();
        let mut secondary = String::new();
        let mut last_emit = 0_usize;
        let mut search_from = 0_usize;
        let mut matched = false;
        let mut embedded_json_valid = None;
        while search_from < bytes.len() {
            let Some(found) = automaton.find(&input[search_from..]) else {
                break;
            };
            let start = search_from + found.start();
            let end = search_from + found.end();
            if Self::match_boundary_valid(input, start, end, &mut embedded_json_valid) {
                primary.push_str(&input[last_emit..start]);
                secondary.push_str(&input[last_emit..start]);
                let (primary_replacement, secondary_replacement) =
                    replacement(found.pattern().as_usize());
                primary.push_str(&primary_replacement);
                secondary.push_str(&secondary_replacement);
                matched = true;
                last_emit = end;
                search_from = end;
            } else {
                search_from = start + 1;
            }
        }
        if !matched {
            return None;
        }
        primary.push_str(&input[last_emit..]);
        secondary.push_str(&input[last_emit..]);
        Some((primary, secondary))
    }

    /// Encode one string: `(wire form, canonical local respelling)`, or
    /// `None` when it contains no in-root occurrence. A literal reserved
    /// token in source content is a typed block: there is no escaping.
    fn encode_str(&self, input: &str) -> Result<Option<(String, String)>, String> {
        if input.contains(RESERVED_NAMESPACE) {
            return Err(format!(
                "source content contains the reserved wire token '{RESERVED_NAMESPACE}' and \
                 cannot be encoded"
            ));
        }
        Ok(Self::transform_str(
            &self.encode_automaton,
            input,
            |pattern| {
                let pattern = &self.encode_patterns[pattern];
                (pattern.wire.to_string(), pattern.canonical.clone())
            },
        ))
    }

    /// Decode one wire string to this mapper's root, or `None` when it
    /// contains no token.
    fn decode_str(&self, input: &str) -> Option<String> {
        Self::transform_str(&self.decode_automaton, input, |pattern| {
            let replacement = self.decode_replacements[pattern].clone();
            (replacement.clone(), replacement)
        })
        .map(|(decoded, _)| decoded)
    }

    /// Encode every string value and object key of one record.
    pub fn encode_record(&self, record: &Value) -> Result<EncodedRecord, String> {
        let mut changed = false;
        let wire = self.encode_value(record, true, &mut changed)?;
        let canonical_source = if changed {
            let mut ignored = false;
            self.encode_value(record, false, &mut ignored)?
        } else {
            record.clone()
        };
        Ok(EncodedRecord {
            wire,
            canonical_source,
            changed,
        })
    }

    fn encode_value(
        &self,
        value: &Value,
        wire_form: bool,
        changed: &mut bool,
    ) -> Result<Value, String> {
        Ok(match value {
            Value::String(text) => match self.encode_str(text)? {
                Some((wire, canonical)) => {
                    *changed = true;
                    Value::String(if wire_form { wire } else { canonical })
                }
                None => value.clone(),
            },
            Value::Array(values) => Value::Array(
                values
                    .iter()
                    .map(|value| self.encode_value(value, wire_form, changed))
                    .collect::<Result<_, String>>()?,
            ),
            Value::Object(object) => {
                let mut output = Map::new();
                for (key, entry) in object {
                    let encoded_key = match self.encode_str(key)? {
                        Some((wire, canonical)) => {
                            *changed = true;
                            if wire_form {
                                wire
                            } else {
                                canonical
                            }
                        }
                        None => key.clone(),
                    };
                    let encoded_entry = self.encode_value(entry, wire_form, changed)?;
                    if output.insert(encoded_key, encoded_entry).is_some() {
                        return Err(
                            "path canonicalization collides two object keys in one record"
                                .to_string(),
                        );
                    }
                }
                Value::Object(output)
            }
            other => other.clone(),
        })
    }

    /// Decode every string value and object key of one wire record to this
    /// mapper's root.
    pub fn decode_record(&self, record: &Value) -> Result<Value, String> {
        Ok(match record {
            Value::String(text) => match self.decode_str(text) {
                Some(decoded) => Value::String(decoded),
                None => record.clone(),
            },
            Value::Array(values) => Value::Array(
                values
                    .iter()
                    .map(|value| self.decode_record(value))
                    .collect::<Result<_, String>>()?,
            ),
            Value::Object(object) => {
                let mut output = Map::new();
                for (key, entry) in object {
                    let decoded_key = self.decode_str(key).unwrap_or_else(|| key.clone());
                    let decoded_entry = self.decode_record(entry)?;
                    if output.insert(decoded_key, decoded_entry).is_some() {
                        return Err(
                            "path decoding collides two object keys in one record".to_string()
                        );
                    }
                }
                Value::Object(output)
            }
            other => other.clone(),
        })
    }

    /// Push gate: encode one source record and prove the span-bounded
    /// structural diff — an identity-map decode of the wire form must equal
    /// the source with exactly the matched spans respelled canonically and
    /// every other byte untouched. Structural (`Value`) comparison absorbs
    /// the legal BTreeMap key re-sort a canonicalized key can cause.
    pub fn encode_record_gated(&self, record: &Value) -> Result<Value, String> {
        let encoded = self.encode_record(record)?;
        self.gate_encoded(&encoded)?;
        Ok(encoded.wire)
    }

    fn gate_encoded(&self, encoded: &EncodedRecord) -> Result<(), String> {
        if encoded.changed {
            let decoded = self.decode_record(&encoded.wire)?;
            if decoded != encoded.canonical_source {
                return Err(
                    "path encoding failed its span-bounded round-trip gate: the identity-map \
                     decode does not equal the canonically respelled source"
                        .to_string(),
                );
            }
        }
        Ok(())
    }

    /// Line-level push facade for pure-JSONL wire forms (Claude): parse one
    /// transcript line, encode it under the span gate, and keep the original
    /// bytes when nothing matched, so untouched lines ship byte-identical.
    /// Unparseable input is a typed error — unreachable behind a Complete
    /// analysis, defensive otherwise. Callers name the line ordinal.
    pub fn encode_jsonl_line_gated(&self, line: &str) -> Result<String, String> {
        let record: Value = serde_json::from_str(line).map_err(|_| {
            "transcript line is not valid JSON and cannot be path-encoded".to_string()
        })?;
        let encoded = self.encode_record(&record)?;
        if !encoded.changed {
            return Ok(line.to_string());
        }
        self.gate_encoded(&encoded)?;
        serde_json::to_string(&encoded.wire)
            .map_err(|error| format!("serialize path-encoded record: {error}"))
    }

    /// Line-level pull facade: parse one wire line, prove the wire
    /// self-check (`encode(decode(w)) == w` under this machine's
    /// future-push pattern set — also the target-root collision detector),
    /// then decode to this mapper's root. Returns the decoded `Value` so
    /// the caller can restamp identity fields before serializing once.
    pub fn decode_jsonl_line_verified(&self, line: &str) -> Result<Value, String> {
        let record: Value = serde_json::from_str(line).map_err(|_| {
            "wire transcript line is not valid JSON and cannot be path-decoded".to_string()
        })?;
        self.verify_wire_record(&record)?;
        self.decode_record(&record)
    }

    /// Wire self-check (push over the complete emitted body, pull over the
    /// staged body before reidentification): `encode(decode(w)) == w` per
    /// record. This is also the target-root collision detector — decoded
    /// content indistinguishable from a decoded token re-encodes into extra
    /// tokens and fails equality here.
    pub fn verify_wire_record(&self, wire: &Value) -> Result<(), String> {
        let decoded = self.decode_record(wire)?;
        let re_encoded = self.encode_record(&decoded).map_err(|error| {
            format!("wire record does not re-encode under the target root: {error}")
        })?;
        if re_encoded.wire != *wire {
            return Err(
                "wire record failed its round-trip gate: decode followed by re-encode does not \
                 reproduce the wire form (target-root collision or undeclared spelling)"
                    .to_string(),
            );
        }
        Ok(())
    }

    /// First boundary-valid source-spelling occurrence anywhere in one
    /// record: the repair pre-scan for unpinned threads (occurrences of the
    /// *new* root in content that predates the move would be silently
    /// captured into remap scope).
    pub fn find_source_match(&self, record: &Value) -> Option<String> {
        match record {
            Value::String(text) => self.first_match_in(text),
            Value::Array(values) => values
                .iter()
                .find_map(|value| self.find_source_match(value)),
            Value::Object(object) => object.iter().find_map(|(key, entry)| {
                self.first_match_in(key)
                    .or_else(|| self.find_source_match(entry))
            }),
            _ => None,
        }
    }

    fn first_match_in(&self, input: &str) -> Option<String> {
        let bytes = input.as_bytes();
        let mut search_from = 0_usize;
        let mut embedded_json_valid = None;
        while search_from < bytes.len() {
            let found = self.encode_automaton.find(&input[search_from..])?;
            let start = search_from + found.start();
            let end = search_from + found.end();
            if Self::match_boundary_valid(input, start, end, &mut embedded_json_valid) {
                return Some(input[start..end].to_string());
            }
            search_from = start + 1;
        }
        None
    }
}

/// One encoded record: the wire form plus the canonical respelling of the
/// source (the source with each matched span replaced by its deterministic
/// canonical local spelling — the expected value of an identity-map decode).
#[derive(Debug)]
pub struct EncodedRecord {
    pub wire: Value,
    pub canonical_source: Value,
    /// Whether any span matched; unmatched records are byte-identical on the
    /// wire and callers may keep their original serialization.
    pub changed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mapper() -> SessionPathMapper {
        SessionPathMapper::new("/homeA/projact_alpha", None, Some("/homeA")).unwrap()
    }

    fn target(root: &str) -> SessionPathMapper {
        SessionPathMapper::new(root, None, Some("/homeB")).unwrap()
    }

    #[test]
    fn four_source_spellings_collapse_to_two_wire_contexts() {
        let mapper = mapper();
        let record = json!({
            "raw": "created /homeA/projact_alpha/readme",
            "tilde": "see ~/projact_alpha/readme",
            "uri": "open file:///homeA/projact_alpha/read%20me",
            "legacy_uri": "open file:///homeA/projact_alpha/readme",
            "subdir": "/homeA/projact_alpha/frontend/app.ts",
            "root_only": "/homeA/projact_alpha",
        });
        let wire = mapper.encode_record_gated(&record).unwrap();
        assert_eq!(
            wire,
            json!({
                "raw": "created /__mallard_root__/readme",
                "tilde": "see /__mallard_root__/readme",
                "uri": "open file:///__mallard_root__/read%20me",
                "legacy_uri": "open file:///__mallard_root__/readme",
                "subdir": "/__mallard_root__/frontend/app.ts",
                "root_only": "/__mallard_root__",
            })
        );
        for value in wire.as_object().unwrap().values() {
            mapper.verify_wire_record(value).unwrap();
        }

        // Decode onto a different machine's root, then re-encode: the wire
        // reproduces byte-for-byte (the pull gate).
        let puller = target("/homeB/projact_beta");
        let decoded = puller.decode_record(&wire).unwrap();
        assert_eq!(
            decoded,
            json!({
                "raw": "created /homeB/projact_beta/readme",
                "tilde": "see /homeB/projact_beta/readme",
                "uri": "open file:///homeB/projact_beta/read%20me",
                "legacy_uri": "open file:///homeB/projact_beta/readme",
                "subdir": "/homeB/projact_beta/frontend/app.ts",
                "root_only": "/homeB/projact_beta",
            })
        );
        puller.verify_wire_record(&wire).unwrap();
    }

    #[test]
    fn boundary_rule_rejects_component_extensions_and_foreign_homes() {
        let mapper = mapper();
        let record = json!({
            "extended": "/homeA/projact_alphabet/x",
            "suffixed": "/homeA/projact_alpha-2/x",
            "dotted": "/homeA/projact_alpha.bak",
            "hosted": "/host/homeA/projact_alpha/x",
            "quoted": "\"/homeA/projact_alpha\"",
            "colon": "/homeA/projact_alpha:42",
        });
        let wire = mapper.encode_record_gated(&record).unwrap();
        assert_eq!(
            wire,
            json!({
                "extended": "/homeA/projact_alphabet/x",
                "suffixed": "/homeA/projact_alpha-2/x",
                "dotted": "/homeA/projact_alpha.bak",
                "hosted": "/host/homeA/projact_alpha/x",
                "quoted": "\"/__mallard_root__\"",
                "colon": "/homeA/projact_alpha:42",
            })
        );
    }

    #[test]
    fn boundary_rule_does_not_rewrite_a_project_name_prefix_sibling() {
        let mapper = SessionPathMapper::new("/tmp/run/project1", None, None).unwrap();
        let record = json!({
            "inside": "/tmp/run/project1/file",
            "outside_underscore": "/tmp/run/project1_external/outside.txt",
            "outside_plus": "/tmp/run/project1+copy/outside.txt",
            "outside_colon": "/tmp/run/project1:copy/outside.txt",
            "outside_equals": "/tmp/run/project1=copy/outside.txt",
            "outside_question": "/tmp/run/project1?copy/outside.txt",
            "outside_bang": "/tmp/run/project1!copy/outside.txt",
            "outside_backslash": "/tmp/run/project1\\copy/outside.txt",
            "outside_unicode": "/tmp/run/project1é/outside.txt",
            "outside_emoji": "/tmp/run/project1🦆/outside.txt",
        });
        assert_eq!(
            mapper.encode_record_gated(&record).unwrap(),
            json!({
                "inside": "/__mallard_root__/file",
                "outside_underscore": "/tmp/run/project1_external/outside.txt",
                "outside_plus": "/tmp/run/project1+copy/outside.txt",
                "outside_colon": "/tmp/run/project1:copy/outside.txt",
                "outside_equals": "/tmp/run/project1=copy/outside.txt",
                "outside_question": "/tmp/run/project1?copy/outside.txt",
                "outside_bang": "/tmp/run/project1!copy/outside.txt",
                "outside_backslash": "/tmp/run/project1\\copy/outside.txt",
                "outside_unicode": "/tmp/run/project1é/outside.txt",
                "outside_emoji": "/tmp/run/project1🦆/outside.txt",
            })
        );
    }

    #[test]
    fn object_keys_ride_the_rewrite_and_resort_stably() {
        let mapper = mapper();
        // patch_apply_end-style path keys, including a tilde key whose
        // canonicalization changes its BTreeMap sort position.
        let record = json!({
            "type": "event_msg",
            "payload": {
                "type": "patch_apply_end",
                "changes": {
                    "/homeA/projact_alpha/a.rs": {"kind": "edit"},
                    "~/projact_alpha/b.rs": {"kind": "add"},
                    "/other/place/c.rs": {"kind": "edit"},
                }
            }
        });
        let wire = mapper.encode_record_gated(&record).unwrap();
        let changes = &wire["payload"]["changes"];
        assert_eq!(
            changes.as_object().unwrap().keys().collect::<Vec<_>>(),
            vec![
                "/__mallard_root__/a.rs",
                "/__mallard_root__/b.rs",
                "/other/place/c.rs"
            ]
        );
        mapper.verify_wire_record(&wire).unwrap();
        let decoded = target("/homeB/projact_beta").decode_record(&wire).unwrap();
        assert_eq!(
            decoded["payload"]["changes"]
                .as_object()
                .unwrap()
                .keys()
                .collect::<Vec<_>>(),
            vec![
                "/homeB/projact_beta/a.rs",
                "/homeB/projact_beta/b.rs",
                "/other/place/c.rs"
            ]
        );
    }

    #[test]
    fn nested_json_strings_and_out_of_root_paths() {
        let mapper = mapper();
        let record = json!({
            "payload": {
                "type": "function_call",
                "arguments": "{\"cmd\":\"cat /homeA/projact_alpha/x\",\"cwd\":\"/homeA/projact_alpha\"}",
                "sandbox": ["/tmp", "/homeA/.codex/log", "/homeA/other-project/y"],
            }
        });
        let wire = mapper.encode_record_gated(&record).unwrap();
        assert_eq!(
            wire["payload"]["arguments"],
            json!("{\"cmd\":\"cat /__mallard_root__/x\",\"cwd\":\"/__mallard_root__\"}")
        );
        // Out-of-root occurrences are byte-identical end to end.
        assert_eq!(
            wire["payload"]["sandbox"],
            json!(["/tmp", "/homeA/.codex/log", "/homeA/other-project/y"])
        );
        let decoded = target("/homeB/projact_beta").decode_record(&wire).unwrap();
        assert_eq!(
            decoded["payload"]["sandbox"],
            json!(["/tmp", "/homeA/.codex/log", "/homeA/other-project/y"])
        );
    }

    #[test]
    fn injectivity_pins_hold_on_hostile_targets() {
        // Space-containing source root onto an escape-free target.
        let spaced = SessionPathMapper::new("/homeA/my project", None, Some("/homeA")).unwrap();
        let record = json!({
            "uri": "file:///homeA/my%20project/doc.md",
            "raw": "/homeA/my project/doc.md",
        });
        let wire = spaced.encode_record_gated(&record).unwrap();
        assert_eq!(
            wire,
            json!({
                "uri": "file:///__mallard_root__/doc.md",
                "raw": "/__mallard_root__/doc.md",
            })
        );
        let plain = target("/homeB/plain");
        let decoded = plain.decode_record(&wire).unwrap();
        assert_eq!(
            decoded,
            json!({
                "uri": "file:///homeB/plain/doc.md",
                "raw": "/homeB/plain/doc.md",
            })
        );
        plain.verify_wire_record(&wire).unwrap();

        // Escape-free source onto a space-containing target: the URI root
        // portion stays validly escaped.
        let mapper = mapper();
        let wire = mapper
            .encode_record_gated(&json!("file:///homeA/projact_alpha/x"))
            .unwrap();
        let spacey = SessionPathMapper::new("/homeB/my stuff", None, Some("/homeB")).unwrap();
        assert_eq!(
            spacey.decode_record(&wire).unwrap(),
            json!("file:///homeB/my%20stuff/x")
        );
        spacey.verify_wire_record(&wire).unwrap();

        // Tilde source onto a target outside the puller's home: canonical
        // expansion is raw absolute, never tilde.
        let wire = mapper
            .encode_record_gated(&json!("~/projact_alpha/x"))
            .unwrap();
        let outside = SessionPathMapper::new("/srv/checkout", None, Some("/homeB")).unwrap();
        assert_eq!(
            outside.decode_record(&wire).unwrap(),
            json!("/srv/checkout/x")
        );
        outside.verify_wire_record(&wire).unwrap();
    }

    #[test]
    fn canonical_alias_spellings_collapse_to_one_token() {
        let mapper =
            SessionPathMapper::new("/var/folders/proj", Some("/private/var/folders/proj"), None)
                .unwrap();
        let record = json!(["/var/folders/proj/a", "/private/var/folders/proj/b"]);
        let wire = mapper.encode_record_gated(&record).unwrap();
        assert_eq!(wire, json!(["/__mallard_root__/a", "/__mallard_root__/b"]));
        // Same semantic path form ⇒ same canonical target spelling.
        let decoded = mapper.decode_record(&wire).unwrap();
        assert_eq!(
            decoded,
            json!(["/var/folders/proj/a", "/var/folders/proj/b"])
        );
    }

    #[test]
    fn reserved_namespace_in_source_blocks_with_a_typed_error() {
        let mapper = mapper();
        let error = mapper
            .encode_record(&json!({"text": "token is /__mallard_root__/x"}))
            .unwrap_err();
        assert!(error.contains("reserved wire token"), "{error}");
    }

    #[test]
    fn target_root_collision_blocks_the_wire_self_check() {
        // Alice's transcript mentions /b/project/x verbatim (out of root on
        // her machine); Bob's project *is* /b/project. Decode makes the
        // mention indistinguishable from a decoded token; re-encode
        // tokenizes it and equality fails — safe, never silent.
        let alice = mapper();
        let wire = alice
            .encode_record_gated(&json!({
                "mine": "/homeA/projact_alpha/x",
                "other": "/b/project/x",
            }))
            .unwrap();
        let bob = target("/b/project");
        let error = bob.verify_wire_record(&wire).unwrap_err();
        assert!(error.contains("round-trip gate"), "{error}");
        // The same content pulls cleanly onto a non-colliding root.
        target("/homeB/clean").verify_wire_record(&wire).unwrap();
    }

    #[test]
    fn root_followed_by_root_shaped_suffix_tokenizes_once() {
        let mapper = SessionPathMapper::new("/a/proj", None, None).unwrap();
        // The suffix repeats the root's own spelling; the boundary rule
        // (preceding byte 'j' is a word byte) keeps it a suffix.
        let wire = mapper
            .encode_record_gated(&json!("/a/proj/a/proj"))
            .unwrap();
        assert_eq!(wire, json!("/__mallard_root__/a/proj"));
        let decoded = target("/homeB/projact_beta").decode_record(&wire).unwrap();
        assert_eq!(decoded, json!("/homeB/projact_beta/a/proj"));
    }

    #[test]
    fn embedded_bare_root_is_found_when_the_uri_boundary_fails() {
        let mapper = mapper();
        // 'x' before file:// fails the URI pattern's boundary; the embedded
        // bare root still matches with '/' before it.
        let wire = mapper
            .encode_record_gated(&json!("xfile:///homeA/projact_alpha/y"))
            .unwrap();
        assert_eq!(wire, json!("xfile:///__mallard_root__/y"));
        mapper.verify_wire_record(&wire).unwrap();
    }

    #[test]
    fn windows_and_unclean_roots_are_typed_precondition_errors() {
        for root in [
            "C:\\Users\\dev\\proj",
            "\\\\server\\share\\proj",
            "relative/path",
            "/",
            "/single",
            "/a//b",
            "/a/./b",
            "/a/../b",
            "/a/b/",
            "/x/__mallard_root__/y",
        ] {
            assert!(
                validate_pathmap_root("project root", root).is_err(),
                "{root} should be rejected"
            );
        }
        validate_pathmap_root("project root", "/homeA/projact_alpha").unwrap();
    }

    #[test]
    fn marker_round_trip_and_unknown_codec_block() {
        validate_path_map_marker(&path_map_marker()).unwrap();
        let error = validate_path_map_marker(&json!({"codec": "mallard.pathmap.v9"})).unwrap_err();
        assert!(error.contains("update Mallard"), "{error}");
        assert!(validate_path_map_marker(&json!({})).is_err());
    }

    #[test]
    fn jsonl_line_facades_round_trip_and_keep_unmatched_lines_byte_identical() {
        let encoder = mapper();
        // Claude-shaped record: structural cwd in a subdir, tool input path,
        // tilde and URI spellings in content — all one line.
        let line = concat!(
            "{\"type\":\"user\",\"uuid\":\"u-1\",\"parentUuid\":null,",
            "\"sessionId\":\"s-1\",\"cwd\":\"/homeA/projact_alpha/frontend\",",
            "\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_use\",",
            "\"input\":{\"file_path\":\"/homeA/projact_alpha/readme\"},",
            "\"text\":\"see ~/projact_alpha/x and file:///homeA/projact_alpha/y\"}]}}"
        );
        let wire = encoder.encode_jsonl_line_gated(line).unwrap();
        assert!(!wire.contains("/homeA/projact_alpha"));
        assert!(!wire.contains("~/projact_alpha"));
        assert!(wire.contains("\"cwd\":\"/__mallard_root__/frontend\""));
        assert!(wire.contains("\"file_path\":\"/__mallard_root__/readme\""));
        assert!(wire.contains("file:///__mallard_root__/y"));

        let puller = target("/homeB/projact_beta");
        let decoded = puller.decode_jsonl_line_verified(&wire).unwrap();
        // The subdir suffix survives the hop — the structural cwd is no
        // longer flattened to the bare destination root.
        assert_eq!(decoded["cwd"], json!("/homeB/projact_beta/frontend"));
        assert_eq!(
            decoded["message"]["content"][0]["input"]["file_path"],
            json!("/homeB/projact_beta/readme")
        );

        // A line with no in-root occurrence (foreign cwd included) passes
        // through byte-identical: no parse-and-reserialize churn.
        let foreign = "{\"type\":\"user\",\"uuid\":\"u-2\",\"cwd\":\"/other/place\",\"z\":1}";
        assert_eq!(encoder.encode_jsonl_line_gated(foreign).unwrap(), foreign);
        assert_eq!(
            puller.decode_jsonl_line_verified(foreign).unwrap(),
            serde_json::from_str::<Value>(foreign).unwrap()
        );
    }

    #[test]
    fn jsonl_line_facades_fail_typed_on_malformed_and_reserved_input() {
        let encoder = mapper();
        assert!(encoder
            .encode_jsonl_line_gated("{not json}")
            .unwrap_err()
            .contains("not valid JSON"));
        assert!(target("/homeB/projact_beta")
            .decode_jsonl_line_verified("{not json}")
            .unwrap_err()
            .contains("not valid JSON"));
        let reserved = "{\"text\":\"token is /__mallard_root__/x\"}";
        assert!(encoder
            .encode_jsonl_line_gated(reserved)
            .unwrap_err()
            .contains("reserved wire token"));
        // The decode facade runs the collision detector: wire content
        // holding the destination root verbatim fails closed.
        let colliding = "{\"other\":\"/b/project/x\",\"mine\":\"/__mallard_root__/y\"}";
        assert!(target("/b/project")
            .decode_jsonl_line_verified(colliding)
            .unwrap_err()
            .contains("round-trip gate"));
    }

    #[test]
    fn repair_pre_scan_finds_boundary_valid_new_root_occurrences() {
        let new_root = target("/homeB/projact_beta");
        assert_eq!(
            new_root.find_source_match(&json!({"t": "see /homeB/projact_beta/x"})),
            Some("/homeB/projact_beta".to_string())
        );
        assert_eq!(
            new_root.find_source_match(&json!({"t": "see /homeB/projact_betamax/x"})),
            None
        );
    }
}
