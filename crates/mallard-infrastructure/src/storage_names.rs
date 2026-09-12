//! Readable storage labels. Providers own creation, journals, and identity.
use std::sync::LazyLock;

static WORDS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    let lists = petname::Petnames::small();
    let mut words: Vec<_> = lists
        .nouns
        .iter()
        .chain(lists.adjectives.iter())
        .chain(lists.adverbs.iter())
        .copied()
        .filter(|word| valid_word(word))
        .collect();
    words.sort_unstable();
    words.dedup();
    words
});

pub(crate) fn valid_word(word: &str) -> bool {
    (3..=10).contains(&word.len()) && word.bytes().all(|byte| byte.is_ascii_lowercase())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NameError {
    Exhausted,
    Random,
}

pub(crate) fn select_word(rejected: &[String]) -> Result<String, NameError> {
    let available: Vec<_> = WORDS
        .iter()
        .copied()
        .filter(|word| !rejected.iter().any(|used| used == word))
        .collect();
    if available.is_empty() {
        return Err(NameError::Exhausted);
    }
    let count = available.len() as u64;
    // Reject the incomplete range, so every candidate has equal probability.
    let threshold = count.wrapping_neg() % count;
    loop {
        let mut bytes = [0; 8];
        getrandom::fill(&mut bytes).map_err(|_| NameError::Random)?;
        let value = u64::from_le_bytes(bytes);
        if value >= threshold {
            return Ok(available[(value % count) as usize].to_string());
        }
    }
}

pub(crate) fn r2_bucket(word: &str) -> String {
    format!("mallard-{word}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_list_has_distinct_short_words_and_valid_provider_names() {
        assert_eq!(WORDS.len(), 1155);
        for word in ["fox", "brave", "gently"] {
            assert!(WORDS.contains(&word));
        }
        assert!(WORDS.windows(2).all(|pair| pair[0] < pair[1]));
        for word in WORDS.iter() {
            assert!(valid_word(word));
            assert!(r2_bucket(word).len() <= 63);
        }
    }

    #[test]
    fn selection_excludes_rejected_candidates_and_detects_exhaustion() {
        let mut rejected: Vec<_> = WORDS.iter().map(|word| word.to_string()).collect();
        let last = rejected.pop().unwrap();
        assert_eq!(select_word(&rejected), Ok(last.clone()));
        rejected.push(last);
        assert_eq!(select_word(&rejected), Err(NameError::Exhausted));
        for invalid in [
            "",
            "ab",
            "Upper",
            "two-words",
            "two words",
            "東京",
            "abcdefghijkl",
        ] {
            assert!(!valid_word(invalid));
        }
    }

}
