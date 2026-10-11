//! Crate-private boundary matching for path spellings embedded in text.

use serde::de::{Deserialize as _, IgnoredAny};

use crate::session_path_map::{root_family, RootFamily};

/// Conservative delimiters before an absolute path spelling embedded in
/// provider text. The decisive descendant proof is on the right side: a root
/// match may end the value or be followed by a separator for its path grammar.
fn is_left_boundary_byte(byte: u8) -> bool {
    byte.is_ascii()
        && matches!(
            byte,
            b'/' | b'\\'
                | b' '
                | b'\t'
                | b'\r'
                | b'\n'
                | b'"'
                | b'\''
                | b'('
                | b')'
                | b'['
                | b']'
                | b'{'
                | b'}'
                | b','
                | b';'
                | b':'
                | b'='
                | b'|'
                | b'&'
                | b'<'
                | b'>'
                | b'!'
                | b'?'
        )
}

pub(crate) fn left_boundary_valid(bytes: &[u8], start: usize) -> bool {
    start <= bytes.len() && (start == 0 || is_left_boundary_byte(bytes[start - 1]))
}

/// `separators` are the bytes that may continue a matched root: `/` for
/// POSIX roots, `\` or `/` for Windows roots.
pub(crate) fn boundary_valid(bytes: &[u8], start: usize, end: usize, separators: &[u8]) -> bool {
    if start > end || end > bytes.len() {
        return false;
    }
    left_boundary_valid(bytes, start) && (end == bytes.len() || separators.contains(&bytes[end]))
}

pub(crate) fn valid_json_text(input: &str) -> bool {
    let mut deserializer = serde_json::Deserializer::from_str(input);
    IgnoredAny::deserialize(&mut deserializer).is_ok() && deserializer.end().is_ok()
}

/// Replace every boundary-valid occurrence of one path spelling. A rejected
/// occurrence advances by one Unicode scalar so a later or overlapping valid
/// occurrence can still be found. Replacement text is never scanned again.
pub(crate) fn replace_boundary_valid_occurrences(
    input: &str,
    needle: &str,
    replacement: &str,
) -> String {
    if needle.is_empty() {
        return input.to_string();
    }

    let separators: &[u8] = match root_family(needle) {
        Some(RootFamily::Windows) => b"\\/",
        _ => b"/",
    };
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut last_emit = 0_usize;
    let mut search_from = 0_usize;
    let mut matched = false;

    while search_from <= input.len() {
        let Some(relative_start) = input[search_from..].find(needle) else {
            break;
        };
        let start = search_from + relative_start;
        let end = start + needle.len();
        if boundary_valid(bytes, start, end, separators) {
            output.push_str(&input[last_emit..start]);
            output.push_str(replacement);
            last_emit = end;
            search_from = end;
            matched = true;
        } else {
            let advance = input[start..]
                .chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or(1);
            search_from = start + advance;
        }
    }

    if !matched {
        return input.to_string();
    }
    output.push_str(&input[last_emit..]);
    output
}

/// Redact a sensitive path with the conservative path boundary, plus the one
/// prose case that must favor privacy: an exact path followed by a sentence
/// period. Including the period in the match distinguishes it from a sibling
/// such as `project.bak`, and avoids rendering `..` when the replacement is
/// already `.`.
pub(crate) fn replace_sensitive_path_occurrences(
    input: &str,
    needle: &str,
    replacement: &str,
) -> String {
    if needle.is_empty() {
        return input.to_string();
    }
    if valid_json_text(input) {
        // Match the serialized spelling, including escaped Windows separators.
        let encoded_needle = serde_json::to_string(needle).expect("path string serializes");
        let encoded_replacement =
            serde_json::to_string(replacement).expect("replacement string serializes");
        let needle = &encoded_needle[1..encoded_needle.len() - 1];
        let replacement = &encoded_replacement[1..encoded_replacement.len() - 1];
        let redacted = replace_json_string_closing_quote_occurrences(input, needle, replacement);
        return replace_boundary_valid_occurrences(&redacted, needle, replacement);
    }
    let redacted = replace_sentence_period_occurrences(input, needle, replacement);
    replace_boundary_valid_occurrences(&redacted, needle, replacement)
}

/// In a syntactically valid JSON value, an unescaped `"` immediately after a
/// root occurrence closes the current JSON string. This safely handles exact
/// roots in serialized object values and keys without treating legal POSIX
/// suffix bytes as path boundaries.
fn replace_json_string_closing_quote_occurrences(
    input: &str,
    needle: &str,
    replacement: &str,
) -> String {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut last_emit = 0_usize;
    let mut search_from = 0_usize;
    let mut matched = false;

    while search_from <= input.len() {
        let Some(relative_start) = input[search_from..].find(needle) else {
            break;
        };
        let start = search_from + relative_start;
        let end = start + needle.len();
        if left_boundary_valid(bytes, start) && bytes.get(end) == Some(&b'"') {
            output.push_str(&input[last_emit..start]);
            output.push_str(replacement);
            last_emit = end;
            search_from = end;
            matched = true;
        } else {
            let advance = input[start..]
                .chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or(1);
            search_from = start + advance;
        }
    }

    if !matched {
        return input.to_string();
    }
    output.push_str(&input[last_emit..]);
    output
}

/// Session Details favors redaction for the unambiguous prose shape
/// `<exact-root>.` followed by the end of the value, whitespace, or closing
/// punctuation. This policy is intentionally separate from portable path
/// remapping: suffixes such as `:copy`, `=copy`, or `!copy` remain outside
/// paths and are never structural root matches.
fn replace_sentence_period_occurrences(input: &str, needle: &str, replacement: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut last_emit = 0_usize;
    let mut search_from = 0_usize;
    let mut matched = false;

    while search_from <= input.len() {
        let Some(relative_start) = input[search_from..].find(needle) else {
            break;
        };
        let start = search_from + relative_start;
        let end = start + needle.len();
        let period_end = end.saturating_add(1);
        let left_valid = start == 0 || is_left_boundary_byte(bytes[start - 1]);
        let period_valid = bytes.get(end) == Some(&b'.');
        let tail_valid = period_end == bytes.len()
            || bytes.get(period_end).is_some_and(|byte| {
                byte.is_ascii_whitespace() || matches!(byte, b'"' | b'\'' | b')' | b']' | b'}')
            });
        if left_valid && period_valid && tail_valid {
            output.push_str(&input[last_emit..start]);
            output.push_str(replacement);
            if !replacement.ends_with('.') {
                output.push('.');
            }
            last_emit = period_end;
            search_from = period_end;
            matched = true;
        } else {
            let advance = input[start..]
                .chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or(1);
            search_from = start + advance;
        }
    }

    if !matched {
        return input.to_string();
    }
    output.push_str(&input[last_emit..]);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_obeys_the_shared_path_text_boundary() {
        let input = concat!(
            "/tmp/run/project1/file ",
            "/tmp/run/project1_external/outside ",
            "file:///tmp/run/project1/uri ",
            "{\"/tmp/run/project1/key\":true}"
        );
        assert_eq!(
            replace_boundary_valid_occurrences(input, "/tmp/run/project1", "."),
            concat!(
                "./file ",
                "/tmp/run/project1_external/outside ",
                "file://./uri ",
                "{\"./key\":true}"
            )
        );
    }

    #[test]
    fn component_suffixes_are_not_path_boundaries() {
        for sibling in [
            "/tmp/run/project1_external/outside",
            "/tmp/run/project1+copy/outside",
            "/tmp/run/project1:copy/outside",
            "/tmp/run/project1=copy/outside",
            "/tmp/run/project1?copy/outside",
            "/tmp/run/project1!copy/outside",
            "/tmp/run/project1\\copy/outside",
            "/tmp/run/project1é/outside",
            "/tmp/run/project1🦆/outside",
        ] {
            assert_eq!(
                replace_boundary_valid_occurrences(sibling, "/tmp/run/project1", "."),
                sibling
            );
        }
    }

    #[test]
    fn sensitive_replacement_redacts_sentence_period_without_touching_a_sibling() {
        assert_eq!(
            replace_sensitive_path_occurrences(
                "wrote /tmp/run/project1. kept /tmp/run/project1.bak/file",
                "/tmp/run/project1",
                ".",
            ),
            "wrote . kept /tmp/run/project1.bak/file"
        );
    }

    #[test]
    fn sensitive_replacement_handles_windows_descendants_without_touching_siblings() {
        let root = r"C:\fixture\project1";
        for (input, expected) in [
            (r"C:\fixture\project1", "."),
            (
                r"A C:\fixture\project1\generated\inside.txt",
                r"A .\generated\inside.txt",
            ),
            (
                r"A C:\fixture\project1/generated/inside.txt",
                "A ./generated/inside.txt",
            ),
            (
                r"C:\fixture\project1_external\outside.txt",
                r"C:\fixture\project1_external\outside.txt",
            ),
            (
                r"C:\fixture\project1.bak\outside.txt",
                r"C:\fixture\project1.bak\outside.txt",
            ),
        ] {
            assert_eq!(
                replace_sensitive_path_occurrences(input, root, "."),
                expected
            );
        }
    }

    #[test]
    fn sensitive_replacement_redacts_windows_paths_in_json_strings_and_keys() {
        let root = r"C:\fixture\project1";
        let input = serde_json::json!({
            "file_path": r"C:\fixture\project1\read-marker.txt",
            "cwd": root,
            "changes": {r"C:\fixture\project1\src\lib.rs": true},
            "outside": r"C:\fixture\project1_external\outside.txt",
            "sibling": r"C:\fixture\project1.",
        });
        let sanitized = replace_sensitive_path_occurrences(&input.to_string(), root, ".");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&sanitized).unwrap(),
            serde_json::json!({
                "file_path": r".\read-marker.txt",
                "cwd": ".",
                "changes": {r".\src\lib.rs": true},
                "outside": r"C:\fixture\project1_external\outside.txt",
                "sibling": r"C:\fixture\project1.",
            }),
        );
    }

    #[test]
    fn sensitive_replacement_treats_valid_json_as_structure_not_prose() {
        assert_eq!(
            replace_sensitive_path_occurrences(
                r#"{"cwd":"/tmp/run/project1","key":{"/tmp/run/project1":true},"outside":"/tmp/run/project1."}"#,
                "/tmp/run/project1",
                ".",
            ),
            r#"{"cwd":".","key":{".":true},"outside":"/tmp/run/project1."}"#
        );
    }

    #[test]
    fn rejected_occurrence_does_not_hide_a_later_valid_one() {
        assert_eq!(
            replace_boundary_valid_occurrences(
                "x/tmp/run/project1 /tmp/run/project1/file",
                "/tmp/run/project1",
                ".",
            ),
            "x/tmp/run/project1 ./file"
        );
        assert_eq!(
            replace_boundary_valid_occurrences("value", "", "."),
            "value"
        );
    }
}
