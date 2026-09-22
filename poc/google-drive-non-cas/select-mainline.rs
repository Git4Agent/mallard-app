// Compile the production selector directly. No Cargo dependency or product edit.
// Only the domain crate's SHA-256 string validator is supplied by this harness.
use std::io::{self, Read};

fn validate_sha256(_label: &str, value: &str) -> Result<(), String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        Ok(())
    } else {
        Err("invalid SHA-256".into())
    }
}

#[allow(dead_code)]
#[path = "../../crates/mallard-domain/src/publication.rs"]
mod publication;

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    for group in input.trim_end_matches('\n').split("\n\n") {
        let selected =
            publication::Mainline::select(group.lines().filter_map(publication::RecordName::parse));
        println!(
            "{}",
            selected
                .selected()
                .map_or(String::new(), |name| name.file_name())
        );
    }
}
