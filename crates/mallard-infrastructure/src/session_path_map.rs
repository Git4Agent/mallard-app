//! Lossless project-path references. Literal marker text is escaped once.
//! Spelling tags survive transport; local origin records protect later Push.

use crate::path_text_boundary::{boundary_valid, left_boundary_valid, valid_json_text};
use crate::session_path_origins::{
    pointer_child, replay_path_fields, PathField, PathSpan, RecordPathOrigins,
};
use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use serde_json::{Map, Value};

pub const PATH_MAP_CODEC_V1: &str = "mallard.pathmap.v1";
pub const PATH_MAP_FIELD: &str = "__mallard_path_map__";
pub const ROOT_TOKEN: &str = "/__mallard_root__/b;";
const ROOT_NAMESPACE: &str = "/__mallard_root__";
const ESCAPE_TOKEN: &str = "/__mallard_escape__";
const ESCAPED_ROOT_TOKEN: &str = "/__mallard_escape__/r";
const ESCAPED_ESCAPE_TOKEN: &str = "/__mallard_escape__/e";
const TOKENS: [&str; 8] = [
    ROOT_TOKEN,
    "/__mallard_root__/c;",
    "/__mallard_root__/bt;",
    "/__mallard_root__/ct;",
    "/__mallard_root__/bu;",
    "/__mallard_root__/cu;",
    "/__mallard_root__/be;",
    "/__mallard_root__/ce;",
];

pub(crate) fn is_wire_atom(text: &str) -> bool {
    TOKENS.contains(&text) || text == ESCAPED_ROOT_TOKEN || text == ESCAPED_ESCAPE_TOKEN
}

pub fn path_map_marker() -> Value {
    serde_json::json!({"codec": PATH_MAP_CODEC_V1})
}

pub fn validate_path_map_marker(marker: &Value) -> Result<(), String> {
    if marker.as_object().is_none_or(|object| object.len() != 1)
        || marker.get("codec").and_then(Value::as_str) != Some(PATH_MAP_CODEC_V1)
    {
        return Err("conversation wire object declares an unsupported path-map codec".into());
    }
    Ok(())
}

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
    if root.contains("__mallard_root__") || root.contains("__mallard_escape__") {
        return Err(format!(
            "{label} '{root}' collides with a path-map control namespace"
        ));
    }
    Ok(())
}

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

struct EncodePattern {
    token: &'static str,
    literal: bool,
}

pub struct SessionPathMapper {
    root: String,
    automaton: AhoCorasick,
    wire_automaton: AhoCorasick,
    patterns: Vec<EncodePattern>,
    spellings: [String; 8],
}

pub struct EncodedRecord {
    pub wire: Value,
    pub origins: RecordPathOrigins,
    pub changed: bool,
}

pub struct DecodedRecord {
    pub local: Value,
    pub origins: RecordPathOrigins,
}

impl SessionPathMapper {
    pub fn new(
        root: &str,
        canonical_root: Option<&str>,
        home: Option<&str>,
    ) -> Result<Self, String> {
        validate_pathmap_root("project root", root)?;
        let canonical = canonical_root.unwrap_or(root);
        validate_pathmap_root("canonical project root", canonical)?;
        let home = home
            .map(|home| home.trim_end_matches('/'))
            .filter(|home| home.starts_with('/') && !home.is_empty());
        let tilde = |path: &str| {
            home.and_then(|home| path.strip_prefix(home))
                .filter(|rest| rest.starts_with('/'))
                .map(|rest| format!("~{rest}"))
        };
        let tilde_binding = tilde(root);
        let tilde_canonical = tilde(canonical);
        let spellings = [
            root.to_string(),
            canonical.to_string(),
            tilde_binding.clone().unwrap_or_else(|| root.to_string()),
            tilde_canonical
                .clone()
                .unwrap_or_else(|| canonical.to_string()),
            format!("file://{root}"),
            format!("file://{canonical}"),
            format!("file://{}", percent_encode_path(root)),
            format!("file://{}", percent_encode_path(canonical)),
        ];
        let mut texts = Vec::new();
        let mut patterns = Vec::new();
        for (index, spelling) in spellings.iter().enumerate() {
            if (index == 2 && tilde_binding.is_none()) || (index == 3 && tilde_canonical.is_none())
            {
                continue;
            }
            if !texts.contains(spelling) {
                texts.push(spelling.clone());
                patterns.push(EncodePattern {
                    token: TOKENS[index],
                    literal: false,
                });
            }
        }
        for (text, token) in [
            (ROOT_NAMESPACE, ESCAPED_ROOT_TOKEN),
            (ESCAPE_TOKEN, ESCAPED_ESCAPE_TOKEN),
        ] {
            texts.push(text.to_string());
            patterns.push(EncodePattern {
                token,
                literal: true,
            });
        }
        let automaton = AhoCorasickBuilder::new()
            .match_kind(MatchKind::LeftmostLongest)
            .build(&texts)
            .map_err(|error| format!("build path matcher: {error}"))?;
        let wire_automaton = AhoCorasick::new([ROOT_NAMESPACE, ESCAPE_TOKEN])
            .map_err(|error| format!("build wire path matcher: {error}"))?;
        Ok(Self {
            root: root.to_string(),
            automaton,
            wire_automaton,
            patterns,
            spellings,
        })
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    fn match_boundary_valid(
        input: &str,
        start: usize,
        end: usize,
        embedded: &mut Option<bool>,
    ) -> bool {
        boundary_valid(input.as_bytes(), start, end)
            || (left_boundary_valid(input.as_bytes(), start)
                && input.as_bytes().get(end) == Some(&b'"')
                && *embedded.get_or_insert_with(|| valid_json_text(input)))
    }

    fn encode_text(&self, input: &str) -> (String, Vec<PathSpan>) {
        let mut output = String::with_capacity(input.len());
        let mut spans = Vec::new();
        let mut search = 0;
        let mut end = 0;
        let mut embedded = None;
        while search < input.len() {
            let Some(found) = self.automaton.find(&input[search..]) else {
                break;
            };
            let start = search + found.start();
            let next = search + found.end();
            let pattern = &self.patterns[found.pattern().as_usize()];
            if pattern.literal || Self::match_boundary_valid(input, start, next, &mut embedded) {
                output.push_str(&input[end..start]);
                output.push_str(pattern.token);
                spans.push(PathSpan {
                    start,
                    length: next - start,
                    token: pattern.token.to_string(),
                });
                end = next;
                search = next;
            } else {
                // Every registered pattern starts with an ASCII byte.
                search = start + 1;
            }
        }
        output.push_str(&input[end..]);
        (output, spans)
    }

    fn decode_text(&self, input: &str) -> Result<(String, Vec<PathSpan>), String> {
        let mut output = String::with_capacity(input.len());
        let mut spans = Vec::new();
        let mut cursor = 0;
        while cursor < input.len() {
            let suffix = &input[cursor..];
            let Some(found) = self.wire_automaton.find(suffix) else {
                break;
            };
            let start = cursor + found.start();
            output.push_str(&input[cursor..start]);
            let candidate = &input[start..];
            let (token, replacement) = if candidate.starts_with(ESCAPED_ROOT_TOKEN) {
                (ESCAPED_ROOT_TOKEN, ROOT_NAMESPACE)
            } else if candidate.starts_with(ESCAPED_ESCAPE_TOKEN) {
                (ESCAPED_ESCAPE_TOKEN, ESCAPE_TOKEN)
            } else if let Some(index) = TOKENS.iter().position(|token| candidate.starts_with(token))
            {
                (TOKENS[index], self.spellings[index].as_str())
            } else {
                return Err("wire content contains a malformed path reference or escape".into());
            };
            spans.push(PathSpan {
                start: output.len(),
                length: replacement.len(),
                token: token.to_string(),
            });
            output.push_str(replacement);
            cursor = start + token.len();
        }
        output.push_str(&input[cursor..]);
        Ok((output, spans))
    }

    fn walk(
        &self,
        value: &Value,
        pointer: &str,
        decode: bool,
        fields: &mut Vec<PathField>,
    ) -> Result<Value, String> {
        let transform = |text: &str| {
            if decode {
                self.decode_text(text)
            } else {
                Ok(self.encode_text(text))
            }
        };
        Ok(match value {
            Value::String(text) => {
                let (mapped, spans) = transform(text)?;
                if !spans.is_empty() {
                    fields.push(PathField {
                        pointer: pointer.to_string(),
                        key: false,
                        spans,
                    });
                }
                Value::String(mapped)
            }
            Value::Array(values) => Value::Array(
                values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        self.walk(
                            value,
                            &pointer_child(pointer, &index.to_string()),
                            decode,
                            fields,
                        )
                    })
                    .collect::<Result<_, _>>()?,
            ),
            Value::Object(object) => {
                let mut mapped = Map::new();
                for (key, value) in object {
                    let (new_key, spans) = transform(key)?;
                    let local_key = if decode { &new_key } else { key };
                    let child = pointer_child(pointer, local_key);
                    let value = self.walk(value, &child, decode, fields)?;
                    if !spans.is_empty() {
                        fields.push(PathField {
                            pointer: child,
                            key: true,
                            spans,
                        });
                    }
                    if mapped.insert(new_key, value).is_some() {
                        return Err("path conversion collides two object keys in one record".into());
                    }
                }
                Value::Object(mapped)
            }
            value => value.clone(),
        })
    }

    pub fn encode_record(&self, record: &Value) -> Result<EncodedRecord, String> {
        self.encode_record_with_origins(record, None)
    }

    pub fn encode_record_with_origins(
        &self,
        record: &Value,
        previous: Option<&RecordPathOrigins>,
    ) -> Result<EncodedRecord, String> {
        let (wire, origins) = if let Some(previous) = previous {
            (previous.replay(record)?, previous.clone())
        } else {
            let mut fields = Vec::new();
            let wire = self.walk(record, "", false, &mut fields)?;
            (wire, RecordPathOrigins::capture(record, fields)?)
        };
        if self.decode_record(&wire)? != *record {
            return Err("path encoding failed exact source restoration".into());
        }
        Ok(EncodedRecord {
            changed: !origins.fields.is_empty(),
            wire,
            origins,
        })
    }

    pub fn encode_record_gated(&self, record: &Value) -> Result<Value, String> {
        self.encode_record(record).map(|encoded| encoded.wire)
    }

    pub fn decode_record_with_origins(&self, wire: &Value) -> Result<DecodedRecord, String> {
        let (local, fields) = self.decode_record_fields(wire)?;
        let origins = RecordPathOrigins::capture(&local, fields)?;
        Ok(DecodedRecord { local, origins })
    }

    /// Verify inverse replay before any origins are persisted. Both the local
    /// value and its edits are fresh and unchanged within this call. Hashing
    /// them cannot strengthen this gate; only callers retaining origins need
    /// a hash for comparison with a later transcript.
    fn decode_record_fields(&self, wire: &Value) -> Result<(Value, Vec<PathField>), String> {
        let mut fields = Vec::new();
        let local = self.walk(wire, "", true, &mut fields)?;
        if replay_path_fields(&local, &fields)? != *wire {
            return Err("wire record failed its origin replay gate".into());
        }
        Ok((local, fields))
    }

    pub fn decode_record(&self, record: &Value) -> Result<Value, String> {
        self.decode_record_fields(record).map(|(local, _)| local)
    }

    pub fn verify_wire_record(&self, wire: &Value) -> Result<(), String> {
        self.decode_record_fields(wire).map(|_| ())
    }

    pub fn encode_jsonl_line_gated(&self, line: &str) -> Result<String, String> {
        let record: Value =
            serde_json::from_str(line).map_err(|_| "transcript line is not valid JSON")?;
        let encoded = self.encode_record(&record)?;
        if !encoded.changed {
            return Ok(line.to_string());
        }
        serde_json::to_string(&encoded.wire).map_err(|_| "serialize path-encoded record".into())
    }

    pub fn decode_jsonl_line_verified(&self, line: &str) -> Result<Value, String> {
        let record =
            serde_json::from_str(line).map_err(|_| "wire transcript line is not valid JSON")?;
        self.decode_record(&record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mapper() -> SessionPathMapper {
        SessionPathMapper::new(
            "/home-a/project space",
            Some("/alias-a/project space"),
            Some("/home-a"),
        )
        .unwrap()
    }

    fn with_hash_attempts<T>(expected: u64, operation: impl FnOnce() -> T) -> T {
        let before = crate::session_path_origins::record_hash_attempts();
        let result = operation();
        assert_eq!(
            crate::session_path_origins::record_hash_attempts() - before,
            expected
        );
        result
    }

    #[test]
    fn fresh_checks_hash_only_origins_that_are_retained() {
        let mapper = mapper();
        for source in [
            json!({"text": "ordinary text without path edits"}),
            json!({
                "/home-a/project space/key": {
                    "~field/🦆": ["~/project space/file", "/__mallard_root__/b;", "\u{0}e\u{301}"]
                }
            }),
        ] {
            let encoded = with_hash_attempts(1, || mapper.encode_record(&source).unwrap());
            with_hash_attempts(0, || mapper.verify_wire_record(&encoded.wire).unwrap());
            let local = with_hash_attempts(0, || mapper.decode_record(&encoded.wire).unwrap());
            assert_eq!(local, source);
            let decoded = with_hash_attempts(1, || {
                mapper.decode_record_with_origins(&encoded.wire).unwrap()
            });
            assert_eq!(decoded.local, source);
            assert_eq!(decoded.origins, encoded.origins);
            let retained = with_hash_attempts(1, || {
                mapper
                    .encode_record_with_origins(&decoded.local, Some(&decoded.origins))
                    .unwrap()
            });
            assert_eq!(retained.wire, encoded.wire);
            assert_eq!(retained.origins, encoded.origins);
        }
    }

    #[test]
    fn stored_origins_still_check_hashes_and_exact_restoration() {
        let mapper = mapper();
        let source = json!({"text": "/home-a/project space/file", "other": "original"});
        let encoded = mapper.encode_record(&source).unwrap();
        let stored: RecordPathOrigins =
            serde_json::from_slice(&serde_json::to_vec(&encoded.origins).unwrap()).unwrap();
        for changed in [
            json!({"text": "/home-a/project space/file", "other": "changed"}),
            json!({"text": "/home-a/project space/elsewhere", "other": "original"}),
        ] {
            let error = with_hash_attempts(1, || {
                mapper
                    .encode_record_with_origins(&changed, Some(&stored))
                    .err()
                    .expect("changed record must fail")
            });
            assert!(error.contains("retained transcript record changed"));
        }
        let mut damaged_hash = stored.clone();
        damaged_hash.local_sha256 = "0".repeat(64);
        assert!(with_hash_attempts(1, || {
            mapper.encode_record_with_origins(&source, Some(&damaged_hash))
        })
        .is_err());

        // A valid source hash does not authorize edits that restore other text.
        let mut damaged_edits = stored;
        damaged_edits.fields[0].spans[0].token = TOKENS[1].into();
        let error = with_hash_attempts(1, || {
            mapper
                .encode_record_with_origins(&source, Some(&damaged_edits))
                .err()
                .expect("edits that change source text must fail")
        });
        assert!(error.contains("exact source restoration"));
    }

    #[test]
    fn exact_source_spelling_and_literal_paths_survive_three_machines() {
        let a = mapper();
        let b = SessionPathMapper::new("/machine-b/project", None, None).unwrap();
        let c = SessionPathMapper::new("/machine-c/project", None, None).unwrap();
        let source = json!({"text": concat!(
            "/home-a/project space/src ~/project space/src /alias-a/project space/src ",
            "file:///home-a/project space/src file:///home-a/project%20space/src ",
            "/machine-b/project/literal /machine-c/project/literal ",
            "/__mallard_root__/bt;/literal /__mallard_escape__/bad 🦆 e\u{301}\u{0}"
        )});
        let wire = a.encode_record_gated(&source).unwrap();
        assert_eq!(a.decode_record(&wire).unwrap(), source);
        let on_b = b.decode_record_with_origins(&wire).unwrap();
        let repushed = b
            .encode_record_with_origins(&on_b.local, Some(&on_b.origins))
            .unwrap();
        assert_eq!(repushed.wire, wire);
        let on_c = c.decode_record_with_origins(&repushed.wire).unwrap();
        let returned = c
            .encode_record_with_origins(&on_c.local, Some(&on_c.origins))
            .unwrap();
        assert_eq!(a.decode_record(&returned.wire).unwrap(), source);
    }

    #[test]
    fn zero_edit_history_is_covered_and_cannot_be_rescanned() {
        let b = SessionPathMapper::new("/machine-b/project", None, None).unwrap();
        let wire =
            json!({"text":"<environment_context> /machine-b/project/.git </environment_context>"});
        let decoded = b.decode_record_with_origins(&wire).unwrap();
        assert!(decoded.origins.fields.is_empty());
        assert_eq!(
            b.encode_record_with_origins(&decoded.local, Some(&decoded.origins))
                .unwrap()
                .wire,
            wire
        );
        assert_ne!(b.encode_record_gated(&decoded.local).unwrap(), wire);
        let edited = json!({"text":"changed"});
        assert!(b
            .encode_record_with_origins(&edited, Some(&decoded.origins))
            .is_err());
    }

    #[test]
    fn all_spelling_tags_restore_the_registered_source_prefix() {
        let a = SessionPathMapper::new(
            "/home-a/project space",
            Some("/home-a/alias space"),
            Some("/home-a"),
        )
        .unwrap();
        for (index, prefix) in a.spellings.iter().enumerate() {
            let source = json!(format!("{prefix}/file"));
            let wire = a.encode_record_gated(&source).unwrap();
            assert_eq!(wire, json!(format!("{}/file", TOKENS[index])));
            assert_eq!(a.decode_record(&wire).unwrap(), source);
        }
    }

    #[test]
    fn keys_and_nested_pointers_are_lossless_without_transient_collisions() {
        let a = mapper();
        let source = json!({
            "/home-a/project space/x": {"~field/🦆": "/home-a/project space/y"},
            "/__mallard_root__/b;/x": "literal key",
            "/alias-a/project space/x": 3
        });
        let encoded = a.encode_record(&source).unwrap();
        assert_eq!(encoded.origins.replay(&source).unwrap(), encoded.wire);
        assert_eq!(a.decode_record(&encoded.wire).unwrap(), source);
        let b = SessionPathMapper::new("/machine-b/project", None, None).unwrap();
        // Binding and canonical keys have no distinct rendering on B.
        assert!(b
            .decode_record(&encoded.wire)
            .unwrap_err()
            .contains("collides"));
        assert!(b.verify_wire_record(&encoded.wire).is_err());
        assert!(b.decode_record_with_origins(&encoded.wire).is_err());
    }

    #[test]
    fn complete_literal_marker_syntax_and_unicode_are_data() {
        let mapper = mapper();
        let atoms = [
            "",
            "\u{0}",
            "\n\r\t",
            "\"\\",
            "é",
            "e\u{301}",
            "中",
            "🦆",
            "/__mallard_root__",
            "/__mallard_root__/bt;",
            "/__mallard_escape__",
            "/__mallard_escape__/r",
            "/home-a/project space/x",
            "/machine-b/project/x",
        ];
        for first in atoms {
            for second in atoms {
                for third in atoms {
                    let source = json!(format!("{first} {second} {third}"));
                    let wire = mapper.encode_record_gated(&source).unwrap();
                    assert_eq!(mapper.decode_record(&wire).unwrap(), source);
                }
            }
        }
        for scalar in (0..=255).chain([0x7ff, 0x800, 0xd7ff, 0xe000, 0xffff, 0x10000, 0x10ffff]) {
            let source = json!(char::from_u32(scalar).unwrap().to_string());
            let wire = mapper.encode_record_gated(&source).unwrap();
            assert_eq!(mapper.decode_record(&wire).unwrap(), source);
        }
    }

    #[test]
    fn malformed_wire_and_utf8_origins_fail() {
        let mapper = mapper();
        for wire in [
            "/__mallard_root__",
            "/__mallard_root__/unknown;",
            "/__mallard_escape__/q",
        ] {
            assert!(mapper.decode_record(&json!(wire)).is_err());
            assert!(mapper.verify_wire_record(&json!(wire)).is_err());
            assert!(mapper.decode_record_with_origins(&json!(wire)).is_err());
            // The same spelling is accepted as literal source content.
            let source = json!(wire);
            assert_eq!(
                mapper
                    .decode_record(&mapper.encode_record_gated(&source).unwrap())
                    .unwrap(),
                source
            );
        }
        let source = json!("🦆 /home-a/project space/x");
        let mut encoded = mapper.encode_record(&source).unwrap();
        encoded.origins.fields[0].spans[0].start = 1;
        assert!(encoded.origins.replay(&source).is_err());
    }

    #[test]
    fn source_boundaries_and_nested_json_are_preserved() {
        let mapper = mapper();
        let source = json!({
            "foreign": "/home-a/project space-more/x",
            "nested": "{\"cwd\":\"/home-a/project space\"}",
            "adjacent": "/__mallard_escape__/r/__mallard_root__/b;"
        });
        let wire = mapper.encode_record_gated(&source).unwrap();
        assert_eq!(wire["foreign"], source["foreign"]);
        assert_ne!(wire["nested"], source["nested"]);
        assert_eq!(mapper.decode_record(&wire).unwrap(), source);
    }

    #[test]
    fn root_validation_and_codec_declaration_remain_closed() {
        for root in [
            "C:\\project",
            "/short",
            "/a/../project",
            "/a/__mallard_root__",
            "/a/project/",
        ] {
            assert!(SessionPathMapper::new(root, None, None).is_err());
        }
        validate_path_map_marker(&path_map_marker()).unwrap();
        assert!(validate_path_map_marker(&json!({"codec":"mallard.pathmap.v9"})).is_err());
    }
}
