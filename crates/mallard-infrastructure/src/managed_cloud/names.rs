//! The repository name rule, as the service's `validate/names.ts` states it:
//! the one rule the desktop previews a new address by, parses a pasted
//! address with, and the service stub enforces. A name is Unicode (RFC
//! 3987: any script in the path, percent-encoded UTF-8 on the wire), stored
//! and compared in NFC (UAX #15), and compared without case by `name_key`.
//! The character set is UAX #31's identifier set — letters, marks and
//! numbers — plus GitHub's `.`, `_` and `-`: no space, separator, control or
//! format character, symbol or emoji.
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use regex::Regex;
use std::collections::BTreeSet;
use std::sync::OnceLock;
use unicode_normalization::UnicodeNormalization;

pub const REPOSITORY_NAME_MAX: usize = 100;
pub const REPOSITORY_NAME_RULE: &str =
    "letters or digits of any script, '.', '_' and '-'; 1 to 100 characters; no spaces";

/// The path segment escape: `A-Za-z0-9._-` travel as they are, everything
/// else as percent-encoded UTF-8, which is how the website writes an href.
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC.remove(b'.').remove(b'_').remove(b'-');

fn name_scalars() -> &'static Regex {
    static RULE: OnceLock<Regex> = OnceLock::new();
    RULE.get_or_init(|| Regex::new(r"^[\p{L}\p{M}\p{N}._-]{1,100}$").expect("the name rule"))
}

fn other_runs() -> &'static Regex {
    static RULE: OnceLock<Regex> = OnceLock::new();
    RULE.get_or_init(|| Regex::new(r"[^\p{L}\p{M}\p{N}._-]+").expect("the other-character rule"))
}

fn leading_marks() -> &'static Regex {
    static RULE: OnceLock<Regex> = OnceLock::new();
    RULE.get_or_init(|| Regex::new(r"^\p{M}+").expect("the leading-mark rule"))
}

/// NFC-normalized, as a name is stored and compared.
pub fn nfc(value: &str) -> String {
    value.nfc().collect()
}

/// Trimmed and NFC-normalized: the form a requested name is read in.
pub fn canonical_name(value: &str) -> String {
    nfc(value.trim())
}

/// Whether `value` is a name as the service stores one: 1 to 100 scalars of
/// letters, marks (not first), numbers and `._-`, and not `.` or `..`.
pub fn is_repository_name(value: &str) -> bool {
    name_scalars().is_match(value)
        && !leading_marks().is_match(value)
        && value != "."
        && value != ".."
}

/// The key two spellings of one name share: NFC, then Unicode default case
/// mapping. Owner-uniqueness, the by-name lookup and the preview's "taken"
/// check all compare keys.
pub fn name_key(value: &str) -> String {
    canonical_name(value).to_lowercase()
}

/// The first `max` scalars of `value`.
fn cut(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

/// The name a title gets, as the service derives it. A valid title is kept
/// exactly (`--Repo--`, `.github`); otherwise every run of other characters
/// becomes one hyphen, hyphens at the ends go, and the result is cut to the
/// limit. A title with nothing usable (`🦆🦆`, `..`, `///`) derives no name:
/// the create is refused, never renamed to a placeholder.
pub fn derive_repository_name(requested: &str) -> Option<String> {
    let title = canonical_name(requested);
    if is_repository_name(&title) {
        return Some(title);
    }
    let mut cleaned = other_runs().replace_all(&title, "-").into_owned();
    while cleaned.contains("--") {
        cleaned = cleaned.replace("--", "-");
    }
    let cleaned = leading_marks().replace(cleaned.trim_matches('-'), "");
    let name = cut(&cleaned, REPOSITORY_NAME_MAX)
        .trim_end_matches('-')
        .to_owned();
    is_repository_name(&name).then_some(name)
}

/// `base-n`, shortening `base` so the whole stays within the limit.
pub fn with_suffix(base: &str, n: u64) -> String {
    let suffix = format!("-{n}");
    let head = cut(base, REPOSITORY_NAME_MAX - suffix.len());
    format!("{}{suffix}", head.trim_end_matches('-'))
}

/// The first of `base`, `base-2`, `base-3`, … whose key is not in `taken`
/// (the keys of the Owner's active repositories).
pub fn pick_available_name(base: &str, taken: &BTreeSet<String>) -> String {
    if !taken.contains(&name_key(base)) {
        return base.to_owned();
    }
    (2u64..)
        .map(|n| with_suffix(base, n))
        .find(|candidate| !taken.contains(&name_key(candidate)))
        .expect("an unbounded sequence has a free name")
}

/// One path segment as it travels in a request path or an href:
/// percent-encoded UTF-8, `A-Za-z0-9._-` kept.
pub fn encode_segment(value: &str) -> String {
    utf8_percent_encode(value, SEGMENT).to_string()
}

/// One path segment as it was pasted or handed over: percent-decoded when
/// it was encoded, then NFC. `None` when the bytes are not UTF-8.
pub fn decode_segment(value: &str) -> Option<String> {
    percent_encoding::percent_decode_str(value)
        .decode_utf8()
        .ok()
        .map(|decoded| nfc(&decoded))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_a_name_from_a_title_by_the_case_table() {
        for (requested, expected) in [
            ("HorrorGame", "HorrorGame"),
            ("Horror Game", "Horror-Game"),
            ("大黄", "大黄"),
            ("  日本語  ", "日本語"),
            ("Cafe\u{301}", "Caf\u{e9}"),
            ("大 黄 / 二", "大-黄-二"),
            ("a/b\\c?", "a-b-c"),
            ("--weird -- name--", "weird-name"),
            ("horror_game", "horror_game"),
            ("  My--Repo  ", "My--Repo"),
            ("--Repo--", "--Repo--"),
            (".github", ".github"),
            ("😀abc", "abc"),
        ] {
            assert_eq!(
                derive_repository_name(requested).as_deref(),
                Some(expected),
                "{requested:?}"
            );
        }
        assert_eq!(
            derive_repository_name(&"x".repeat(150)).unwrap(),
            "x".repeat(100)
        );
        // The cut counts scalars and never leaves a trailing hyphen.
        assert_eq!(
            derive_repository_name(&format!("{}-yyy", "x".repeat(99))).unwrap(),
            "x".repeat(99)
        );
        assert_eq!(
            derive_repository_name(&"黄".repeat(150)).unwrap(),
            "黄".repeat(100)
        );
        for nothing in ["🦆🦆", ".", "..", "///", "", "   ", "\u{301}"] {
            assert_eq!(derive_repository_name(nothing), None, "{nothing:?}");
        }
    }

    #[test]
    fn the_rule_takes_any_script_and_nothing_else() {
        assert!(is_repository_name("大黄"));
        assert!(is_repository_name(&"𝒳".repeat(100)));
        assert!(!is_repository_name(&"𝒳".repeat(101)));
        for bad in [
            "大 黄",
            "大/黄",
            "大%黄",
            "\u{301}a",
            "a\u{200d}b",
            "🦆",
            ".",
            "..",
            "",
        ] {
            assert!(!is_repository_name(bad), "{bad:?}");
        }
        assert_eq!(name_key("Cafe\u{301}"), name_key("CAFÉ"));
        assert_eq!(name_key("Ärger"), "ärger");
        assert_eq!(name_key("  大黄 "), "大黄");
    }

    #[test]
    fn suffixes_by_key_within_the_limit() {
        assert_eq!(
            with_suffix(&"x".repeat(100), 12),
            format!("{}-12", "x".repeat(97))
        );
        assert_eq!(
            with_suffix(&"黄".repeat(100), 2),
            format!("{}-2", "黄".repeat(98))
        );
        let taken = BTreeSet::from(["project".to_owned(), "project-2".to_owned()]);
        assert_eq!(pick_available_name("Project", &taken), "Project-3");
        assert_eq!(
            pick_available_name("大黄", &BTreeSet::from(["大黄".to_owned()])),
            "大黄-2"
        );
        assert_eq!(
            pick_available_name("Ärger", &BTreeSet::from([name_key("A\u{308}RGER")])),
            "Ärger-2"
        );
        let long = "a".repeat(120);
        let base = derive_repository_name(&long).unwrap();
        assert_eq!(
            pick_available_name(&base, &BTreeSet::from(["a".repeat(100)])),
            format!("{}-2", "a".repeat(98))
        );
    }

    #[test]
    fn a_segment_travels_encoded_and_reads_back_in_nfc() {
        assert_eq!(encode_segment("HorrorGame"), "HorrorGame");
        assert_eq!(encode_segment("Birding.Log_v2-x"), "Birding.Log_v2-x");
        assert_eq!(encode_segment("大黄"), "%E5%A4%A7%E9%BB%84");
        assert_eq!(encode_segment("a b/c"), "a%20b%2Fc");
        assert_eq!(
            decode_segment("%E5%A4%A7%E9%BB%84").as_deref(),
            Some("大黄")
        );
        assert_eq!(decode_segment("大黄").as_deref(), Some("大黄"));
        assert_eq!(decode_segment("Cafe%CC%81").as_deref(), Some("Caf\u{e9}"));
        assert_eq!(decode_segment("%FF"), None);
    }
}
