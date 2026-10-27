//! Rebuild a derived cloud preview from bounded wire bytes on stdin.
//! Uses only a temporary provider home; never reads the user's provider state.
use mallard_domain::{ProviderKey, MAX_CONVERSATION_FILE_BYTES};
use mallard_infrastructure::{managed_cloud::views::build_view, providers::AgentProviderRegistry};
use std::io::{Read, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if !(3..=4).contains(&args.len()) {
        return Err("usage: build_session_view <provider> <resource-id> <session-id> [title] < transcript.jsonl".into());
    }
    let mut transcript = Vec::new();
    std::io::stdin()
        .lock()
        .take(MAX_CONVERSATION_FILE_BYTES + 1)
        .read_to_end(&mut transcript)?;
    if transcript.len() as u64 > MAX_CONVERSATION_FILE_BYTES {
        return Err("transcript exceeds the bounded transcript limit".into());
    }
    let view = build_view(
        AgentProviderRegistry::production(),
        &ProviderKey::parse(&args[0])?,
        &args[1],
        &args[2],
        args.get(3).map(String::as_str),
        &transcript,
    )?;
    std::io::stdout().lock().write_all(&view.bytes)?;
    Ok(())
}
