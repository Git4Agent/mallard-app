//! Claude auto-memory stays on the machine that wrote it.
//!
//! Claude Code keeps per-project memory under
//! `<CLAUDE_CONFIG_DIR>/projects/<bucket>/memory/` and copies it into every
//! session: the directory's path into the system prompt, and the contents of
//! `MEMORY.md` into the instructions reminder. Mallard does not sync memory,
//! so a wire record carries neither. This module removes exactly what Claude
//! itself injected, in the records Claude generates for it:
//!
//! - `attachment` records: `AutoMem` entries of `attachment.files`, their
//!   block in `rendered[].content`, and the path in
//!   `attachment.systemPrompt[]`;
//! - `api-request-shape` records: the path in `shape.system[].text`; and
//! - `api-request-blob` records of the user role: the copy of the
//!   instructions reminder.
//!
//! What a session itself read or wrote under the memory directory is
//! conversation content and is left as it is, like any other file outside
//! the project.
//!
//! The rewrite is a pure function of one wire record, changes content only,
//! and is idempotent. It runs after path encoding, so saved path origins
//! keep describing the whole source record and a later Push of the same
//! record produces the same wire bytes.

use serde_json::Value;

const REMINDER_OPEN: &str = "<system-reminder>\n";
const REMINDER_CLOSE: &str = "\n</system-reminder>";
/// Claude renders each instruction file as `Contents of <path> (<label>):`,
/// a blank line, and the file. This label is the auto-memory index.
const FILE_HEADER: &str = "Contents of ";
const AUTO_MEMORY_LABEL: &str = " (user's auto-memory, persists across conversations):";
const AUTO_MEMORY_FILE_TYPE: &str = "AutoMem";
/// How the system prompt's sentence about the memory directory opens. The
/// desktop build continues "file-based memory at `<path>`" and the CLI
/// ", file-based memory system at `<path>`"; the shape the two share is
/// read, not either wording.
const DIRECTORY_SENTENCE_OPEN: &str = "You have a persistent";
/// What the directory's path becomes on the wire. Bracketed lower-case text
/// is inert to credential redaction and holds no path separator.
const DIRECTORY_PLACEHOLDER: &str = "<auto-memory-dir>";

/// Byte offsets of the lines of `body` that are instruction-file headers,
/// each with whether it heads the auto-memory index. A header stands alone:
/// a blank line precedes it unless it opens the text, and one follows it
/// unless it ends the text.
fn file_headers(body: &str) -> Vec<(usize, bool)> {
    let lines = body.split('\n').collect::<Vec<_>>();
    let mut headers = Vec::new();
    let mut offset = 0;
    for (index, line) in lines.iter().enumerate() {
        let alone = (index == 0 || lines[index - 1].is_empty())
            && lines.get(index + 1).is_none_or(|next| next.is_empty());
        if alone && line.starts_with(FILE_HEADER) && line.ends_with("):") {
            headers.push((offset, line.ends_with(AUTO_MEMORY_LABEL)));
        }
        offset += line.len() + 1;
    }
    headers
}

/// One reminder body without its auto-memory sections, or `None` when it
/// has none. A section runs from its header to the next header, or to the
/// end of the reminder.
fn without_memory_sections(body: &str) -> Option<String> {
    let headers = file_headers(body);
    if !headers.iter().any(|(_, auto_memory)| *auto_memory) {
        return None;
    }
    let mut output = String::with_capacity(body.len());
    let mut cursor = 0;
    for (index, (start, auto_memory)) in headers.iter().enumerate() {
        if !auto_memory {
            continue;
        }
        output.push_str(&body[cursor..*start]);
        cursor = headers.get(index + 1).map_or(body.len(), |next| next.0);
    }
    if cursor == body.len() {
        // The last section went: so does the blank line that set it apart.
        if let Some(kept) = output.strip_suffix("\n\n").map(str::len) {
            output.truncate(kept);
        }
    } else {
        output.push_str(&body[cursor..]);
    }
    Some(output)
}

/// `text` without the auto-memory sections of its complete reminder blocks,
/// or `None` when nothing changes. Text outside a reminder is never read.
fn without_memory_reminders(text: &str) -> Option<String> {
    if !text.contains(AUTO_MEMORY_LABEL) {
        return None;
    }
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    let mut changed = false;
    while let Some((before, after)) = rest.split_once(REMINDER_OPEN) {
        output.push_str(before);
        output.push_str(REMINDER_OPEN);
        let Some((body, after)) = after.split_once(REMINDER_CLOSE) else {
            rest = after;
            break;
        };
        match without_memory_sections(body) {
            Some(kept) => {
                output.push_str(&kept);
                changed = true;
            }
            None => output.push_str(body),
        }
        output.push_str(REMINDER_CLOSE);
        rest = after;
    }
    output.push_str(rest);
    changed.then_some(output)
}

/// The memory directory the system prompt names: the first backticked text
/// on the line of its sentence, when the words before it speak of memory and
/// end in " at ", and the text is a path.
fn named_memory_directory(text: &str) -> Option<&str> {
    let mut rest = text;
    while let Some((_, after)) = rest.split_once(DIRECTORY_SENTENCE_OPEN) {
        rest = after;
        let (lead, named) = after.split_once('`')?;
        if lead.contains('\n') || !lead.contains("memory") || !lead.ends_with(" at ") {
            continue;
        }
        let (path, _) = named.split_once('`')?;
        if path.len() >= 6 && !path.contains('\n') && path.contains(['/', '\\']) {
            return Some(path);
        }
    }
    None
}

/// `text` with every mention of the memory directory the system prompt names
/// replaced, or `None` when it names none. The placeholder is no path, so a
/// second pass finds nothing.
fn without_memory_directory(text: &str) -> Option<String> {
    let directory = named_memory_directory(text)?;
    Some(text.replace(directory, DIRECTORY_PLACEHOLDER))
}

fn rewrite_string(value: &mut Value, rewrite: fn(&str) -> Option<String>) -> bool {
    let Some(rewritten) = value.as_str().and_then(rewrite) else {
        return false;
    };
    *value = Value::String(rewritten);
    true
}

fn rewrite_strings<'a>(
    values: impl IntoIterator<Item = &'a mut Value>,
    rewrite: fn(&str) -> Option<String>,
) -> bool {
    let mut changed = false;
    for value in values {
        changed |= rewrite_string(value, rewrite);
    }
    changed
}

/// The copy of the instructions reminder in one API request message: the
/// message text, a text block, or the text a tool result carries.
fn omit_from_request_content(content: &mut Value) -> bool {
    if content.is_string() {
        return rewrite_string(content, without_memory_reminders);
    }
    let Some(blocks) = content.as_array_mut() else {
        return false;
    };
    let mut changed = false;
    for block in blocks {
        let field = match block["type"].as_str() {
            Some("text") => "text",
            Some("tool_result") => "content",
            _ => continue,
        };
        if let Some(inner) = block.get_mut(field) {
            changed |= omit_from_request_content(inner);
        }
    }
    changed
}

fn omit_from_attachment(record: &mut Value) -> bool {
    let mut changed = false;
    if let Some(files) = record
        .pointer_mut("/attachment/files")
        .and_then(Value::as_array_mut)
    {
        let before = files.len();
        files.retain(|file| file["type"] != AUTO_MEMORY_FILE_TYPE);
        changed |= files.len() != before;
    }
    if let Some(sections) = record
        .pointer_mut("/attachment/systemPrompt")
        .and_then(Value::as_array_mut)
    {
        changed |= rewrite_strings(sections, without_memory_directory);
    }
    if let Some(rendered) = record.get_mut("rendered").and_then(Value::as_array_mut) {
        changed |= rewrite_strings(
            rendered
                .iter_mut()
                .filter_map(|block| block.get_mut("content")),
            without_memory_reminders,
        );
    }
    changed
}

/// Remove what Claude injected about auto-memory from one wire record.
/// Returns whether the record changed.
pub(super) fn omit(wire: &mut Value) -> bool {
    match wire["type"].as_str() {
        Some("attachment") => omit_from_attachment(wire),
        Some("api-request-shape") => wire
            .pointer_mut("/shape/system")
            .and_then(Value::as_array_mut)
            .is_some_and(|system| {
                rewrite_strings(
                    system.iter_mut().filter_map(|block| block.get_mut("text")),
                    without_memory_directory,
                )
            }),
        Some("api-request-blob") if wire["message"]["role"] == "user" => wire
            .pointer_mut("/message/content")
            .is_some_and(omit_from_request_content),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const MEMORY_DIRECTORY: &str = "/fixture/claude/projects/-fixture-source/memory/";
    const MEMORY_INDEX: &str = "- [Ducks](ducks.md) — remember the ducks\n- [Geese](geese.md)";

    fn memory_header() -> String {
        format!("Contents of {MEMORY_DIRECTORY}MEMORY.md{AUTO_MEMORY_LABEL}")
    }

    /// The reminder as Claude Code 2.1.288 to 2.1.328 renders it: project
    /// instructions first, the auto-memory index last.
    fn instructions_reminder() -> String {
        format!(
            "<system-reminder>\nCodebase and user instructions are shown below.\n\n\
             Contents of /fixture/source/AGENTS.md (project instructions, checked into the \
             codebase):\n\n# Guide\n\nUse ducks.\n\n{}\n\n{MEMORY_INDEX}\n</system-reminder>",
            memory_header()
        )
    }

    fn stripped_reminder() -> &'static str {
        "<system-reminder>\nCodebase and user instructions are shown below.\n\n\
         Contents of /fixture/source/AGENTS.md (project instructions, checked into the \
         codebase):\n\n# Guide\n\nUse ducks.\n</system-reminder>"
    }

    /// The sentence as the desktop build and as the CLI word it.
    const DESKTOP_SENTENCE: &str = "You have a persistent file-based memory at `";
    const CLI_SENTENCE: &str = "You have a persistent, file-based memory system at `";

    fn memory_sentence(path: &str) -> String {
        format!(
            "# Memory\n\n{DESKTOP_SENTENCE}{path}`. This directory already exists.\n\n\
             Save one fact per file."
        )
    }

    fn assert_no_memory(record: &Value) {
        let text = record.to_string();
        for needle in [MEMORY_DIRECTORY, "remember the ducks", AUTO_MEMORY_LABEL] {
            assert!(!text.contains(needle), "{needle}: {text}");
        }
    }

    #[test]
    fn the_instructions_attachment_loses_its_memory_entry_and_block() {
        let mut record = json!({"type":"attachment","uuid":"a1","parentUuid":"u1",
            "attachment":{"type":"instructions","files":[
                {"type":"Project","path":"/fixture/source/AGENTS.md","content":"# Guide\n\nUse ducks."},
                {"type":"AutoMem","path":format!("{MEMORY_DIRECTORY}MEMORY.md"),"content":MEMORY_INDEX}]},
            "rendered":[{"content":instructions_reminder()}],"renderedRole":"user"});
        assert!(omit(&mut record));
        assert_no_memory(&record);
        assert_eq!(
            record["attachment"]["files"],
            json!([{"type":"Project","path":"/fixture/source/AGENTS.md","content":"# Guide\n\nUse ducks."}])
        );
        assert_eq!(record["rendered"][0]["content"], stripped_reminder());
        // The record, its place in the conversation, and its other fields stay.
        assert_eq!(record["uuid"], "a1");
        assert_eq!(record["parentUuid"], "u1");
        let once = record.clone();
        assert!(!omit(&mut record));
        assert_eq!(record, once);
    }

    #[test]
    fn a_memory_section_ends_at_the_next_file_and_only_it_is_removed() {
        let reminder = format!(
            "before\n<system-reminder>\nIntro.\n\n{}\n\n{MEMORY_INDEX}\n\n\
             Contents of /fixture/source/CLAUDE.local.md (user's private project instructions, \
             not checked in):\n\nLocal.\n</system-reminder>\nafter",
            memory_header()
        );
        assert_eq!(
            without_memory_reminders(&reminder).unwrap(),
            "before\n<system-reminder>\nIntro.\n\n\
             Contents of /fixture/source/CLAUDE.local.md (user's private project instructions, \
             not checked in):\n\nLocal.\n</system-reminder>\nafter"
        );
        // Memory alone leaves the reminder's own first line.
        let alone = format!(
            "<system-reminder>\nIntro.\n\n{}\n\n{MEMORY_INDEX}\n</system-reminder>",
            memory_header()
        );
        assert_eq!(
            without_memory_reminders(&alone).unwrap(),
            "<system-reminder>\nIntro.\n</system-reminder>"
        );
    }

    #[test]
    fn request_copies_of_the_reminder_lose_the_same_block() {
        let reminder = instructions_reminder();
        for content in [
            json!(reminder),
            json!([{"type":"text","text":reminder}, {"type":"text","text":"user prompt"}]),
            json!([{"type":"tool_result","tool_use_id":"call","content":reminder}]),
            json!([{"type":"tool_result","tool_use_id":"call",
                "content":[{"type":"text","text":reminder}]}]),
        ] {
            let mut record = json!({"type":"api-request-blob","sessionId":"session",
                "hash":"request-message-id","message":{"role":"user","content":content}});
            assert!(omit(&mut record), "{record}");
            assert_no_memory(&record);
            assert!(record.to_string().contains("Use ducks."));
            assert_eq!(record["hash"], "request-message-id");
            assert!(!omit(&mut record));
        }
    }

    #[test]
    fn the_system_prompt_names_a_placeholder_instead_of_the_directory() {
        let expected = memory_sentence(DIRECTORY_PLACEHOLDER);
        let mut snapshot = json!({"type":"attachment","attachment":{"type":"prompt_snapshot",
            "systemPrompt":["# Intro", memory_sentence(MEMORY_DIRECTORY), "# Environment"]}});
        assert!(omit(&mut snapshot));
        assert_eq!(
            snapshot["attachment"]["systemPrompt"],
            json!(["# Intro", expected, "# Environment"])
        );
        assert!(!omit(&mut snapshot));

        let joined = format!(
            "# Intro\n\n{}\n\n# Environment",
            memory_sentence(MEMORY_DIRECTORY)
        );
        let mut shape = json!({"type":"api-request-shape","shapeHash":"h","shape":{
            "system":[{"type":"text","text":"You are Claude."}, {"type":"text","text":joined}],
            "tools":[]}});
        assert!(omit(&mut shape));
        assert_eq!(
            shape["shape"]["system"][1]["text"],
            format!("# Intro\n\n{expected}\n\n# Environment")
        );
        assert_eq!(shape["shape"]["system"][0]["text"], "You are Claude.");
        assert_no_memory(&shape);
        assert!(!omit(&mut shape));

        // The CLI words the sentence differently and names the directory
        // again further on. Every mention goes.
        let cli = |path: &str| {
            format!(
                "# auto memory\n\n{CLI_SENTENCE}{path}`. Its contents persist.\n\n\
                 The index is `{path}MEMORY.md`."
            )
        };
        let mut snapshot = json!({"type":"attachment","attachment":{"type":"prompt_snapshot",
            "systemPrompt":[cli(MEMORY_DIRECTORY)]}});
        assert!(omit(&mut snapshot));
        assert_eq!(
            snapshot["attachment"]["systemPrompt"],
            json!([cli(DIRECTORY_PLACEHOLDER)])
        );
        assert_no_memory(&snapshot);
        assert!(!omit(&mut snapshot));
    }

    #[test]
    fn conversation_content_and_incomplete_shapes_are_left_alone() {
        let reminder = instructions_reminder();
        let sentence = memory_sentence(MEMORY_DIRECTORY);
        for record in [
            // What a person typed or a tool printed is conversation content.
            json!({"type":"user","message":{"role":"user","content":reminder}}),
            json!({"type":"user","message":{"role":"user","content":sentence}}),
            json!({"type":"user","message":{"role":"user","content":[
                {"type":"tool_result","tool_use_id":"call","content":reminder}]}}),
            json!({"type":"assistant","message":{"role":"assistant","content":[
                {"type":"tool_use","id":"call","name":"Write","input":{
                    "file_path":format!("{MEMORY_DIRECTORY}ducks.md"),"content":"remember the ducks"}}]}}),
            // Only the user role of a request copy carries the reminder.
            json!({"type":"api-request-blob","message":{"role":"assistant","content":reminder}}),
            // The system prompt sentence is read only where Claude stores the prompt.
            json!({"type":"api-request-blob","message":{"role":"user","content":sentence}}),
            // A reminder that never closes, and a label outside any reminder.
            json!({"type":"api-request-blob","message":{"role":"user","content":
                format!("<system-reminder>\nIntro.\n\n{}\n\n{MEMORY_INDEX}", memory_header())}}),
            json!({"type":"api-request-blob","message":{"role":"user","content":
                format!("Intro.\n\n{}\n\n{MEMORY_INDEX}", memory_header())}}),
            // A header is a line of its own between blank lines.
            json!({"type":"api-request-blob","message":{"role":"user","content":
                format!("<system-reminder>\nSee {}\n{MEMORY_INDEX}\n</system-reminder>", memory_header())}}),
            // An unterminated or empty path names no directory, nor does
            // backticked text that is no path, a sentence about something
            // else, or one whose backticks are on a later line.
            json!({"type":"api-request-shape","shape":{"system":[
                {"type":"text","text":format!("{DESKTOP_SENTENCE}/fixture/unterminated")},
                {"type":"text","text":format!("{DESKTOP_SENTENCE}`.")},
                {"type":"text","text":format!("{CLI_SENTENCE}MEMORY.md`.")},
                {"type":"text","text":"You have a persistent shell at `/fixture/shell/`."},
                {"type":"text","text":
                    "You have a persistent memory.\nIt lives at `/fixture/elsewhere/`."}]}}),
        ] {
            let mut rewritten = record.clone();
            assert!(!omit(&mut rewritten), "{record}");
            assert_eq!(rewritten, record);
        }
    }
}
