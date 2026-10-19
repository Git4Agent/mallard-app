//! The root help page, laid out the way `gh` lays out its own: a line about
//! the tool, USAGE, the commands in named groups, FLAGS, EXAMPLES, what the
//! output looks like, and where to learn more. Headings are bold on a
//! terminal and plain when the output is piped, as an agent reads it.
use clap::Command;

/// Each group's heading and the nouns under it, in display order. Every
/// noun of the root command appears exactly once; a test holds it to that.
pub(crate) const GROUPS: &[(&str, &[&str])] = &[
    (
        "CORE COMMANDS",
        &["status", "project", "link", "push", "pull", "session"],
    ),
    ("STORAGE COMMANDS", &["storage", "cloud"]),
    ("LIBRARY COMMANDS", &["catalog"]),
    ("ADDITIONAL COMMANDS", &["provider", "settings", "log"]),
];

const EXAMPLES: &[&str] = &[
    "$ mallard-cli status",
    "$ mallard-cli storage add local --path /Volumes/shared/mallard --name Shared",
    "$ mallard-cli project add \"$PWD\" --provider codex=\"$HOME/.codex\" --storage Shared",
    "$ mallard-cli push review",
    "$ mallard-cli push apply --files",
    "$ mallard-cli --json pull review > review.json",
    "$ mallard-cli --json pull apply --review review.json --approve all",
    "$ mallard-cli session list",
    "$ mallard-cli catalog push <scope-id> --storage Shared --approve",
];

/// Aligned `name  description` rows, as the command groups are.
const OUTPUT: &[(&str, &str)] = &[
    (
        "--json",
        "one JSON document on stdout, NDJSON events on stderr; the payload is the shape the desktop app receives",
    ),
    ("(default)", "plain text on both"),
];

const EXIT_CODES: &[(&str, &str)] = &[
    ("0", "success"),
    ("2", "invalid input, or nothing matched"),
    (
        "3",
        "a revision or review no longer holds; read or review again",
    ),
    ("4", "the storage, provider, or network failed"),
    ("5", "Mallard's local state could not be used"),
];

const CONVENTIONS: &[(&str, &str)] = &[
    (
        "project",
        "the current folder's, unless --project <id|path> says otherwise",
    ),
    (
        "storage",
        "the project's only link, unless --storage <id|name> says otherwise",
    ),
    ("reviews", "print first; apply takes only what you approve"),
    (
        "writes",
        "take --expected-revision; destructive ones also --confirm",
    ),
    ("secrets", "never printed"),
];

const LEARN_MORE: &[&str] = &[
    "Use `mallard-cli <command> --help` for more information about a command.",
    "Read the guide in docs/CLI.md.",
];

/// An Examples section for one command's help, after clap's own sections.
/// The heading is bold the way clap's are; clap strips it when the output is
/// piped.
pub(crate) fn examples(lines: &[&str]) -> String {
    let mut text = String::from("\x1b[1mExamples:\x1b[0m\n");
    for line in lines {
        text.push_str("  $ mallard-cli ");
        text.push_str(line);
        text.push('\n');
    }
    text
}

fn heading(text: &str, styled: bool) -> String {
    if styled {
        format!("\x1b[1m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// The page for `mallard-cli`, `mallard-cli --help`, and `mallard-cli help`.
pub(crate) fn root_page(command: &Command, styled: bool) -> String {
    let mut page = String::new();
    if let Some(about) = command.get_about() {
        page.push_str(&about.to_string());
        page.push_str("\n\n");
    }
    page.push_str(&heading("USAGE", styled));
    page.push_str("\n  mallard-cli <command> <subcommand> [flags]\n\n");

    let width = command
        .get_subcommands()
        .map(|subcommand| subcommand.get_name().len())
        .max()
        .unwrap_or(0)
        + 1;
    for (title, names) in GROUPS {
        page.push_str(&heading(title, styled));
        page.push('\n');
        for name in *names {
            let about = command
                .find_subcommand(name)
                .and_then(|subcommand| subcommand.get_about())
                .map(|about| about.to_string())
                .unwrap_or_default();
            page.push_str(&format!("  {:<width$} {}\n", format!("{name}:"), about));
        }
        page.push('\n');
    }

    page.push_str(&heading("FLAGS", styled));
    page.push('\n');
    for argument in command.get_arguments() {
        let Some(long) = argument.get_long() else {
            continue;
        };
        let short = argument
            .get_short()
            .map(|short| format!("-{short}, "))
            .unwrap_or_else(|| "    ".to_string());
        page.push_str(&format!(
            "  {short}--{:<10} {}\n",
            long,
            argument
                .get_help()
                .map(|help| help.to_string())
                .unwrap_or_default()
        ));
    }
    // clap adds these two when it builds the command, so they are not among
    // `get_arguments` here.
    page.push_str("  -h, --help       Show help for a command\n");
    page.push_str("  -V, --version    Show the mallard-cli version\n");
    page.push('\n');

    page.push_str(&heading("EXAMPLES", styled));
    page.push('\n');
    for example in EXAMPLES {
        page.push_str(&format!("  {example}\n"));
    }
    page.push('\n');

    for (title, rows) in [
        ("OUTPUT", OUTPUT),
        ("EXIT CODES", EXIT_CODES),
        ("CONVENTIONS", CONVENTIONS),
    ] {
        page.push_str(&heading(title, styled));
        page.push('\n');
        let width = rows.iter().map(|(name, _)| name.len()).max().unwrap_or(0);
        for (name, description) in rows {
            page.push_str(&format!("  {name:<width$}  {description}\n"));
        }
        page.push('\n');
    }

    page.push_str(&heading("LEARN MORE", styled));
    page.push('\n');
    for line in LEARN_MORE {
        page.push_str(&format!("  {line}\n"));
    }
    page
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::Cli;
    use clap::CommandFactory as _;
    use std::collections::BTreeSet;

    #[test]
    fn every_noun_is_in_exactly_one_group() {
        let command = Cli::command();
        let nouns: BTreeSet<&str> = command
            .get_subcommands()
            .map(|subcommand| subcommand.get_name())
            .collect();
        let mut grouped = Vec::new();
        for (_, names) in GROUPS {
            grouped.extend(names.iter().copied());
        }
        let grouped_set: BTreeSet<&str> = grouped.iter().copied().collect();
        assert_eq!(grouped.len(), grouped_set.len(), "a noun is listed twice");
        assert_eq!(grouped_set, nouns, "the groups and the clap tree disagree");
    }

    #[test]
    fn every_noun_and_every_flagged_verb_has_examples() {
        let mut command = Cli::command();
        command.build();
        for noun in command
            .get_subcommands()
            .filter(|noun| noun.get_name() != "help")
        {
            let help = noun.clone().render_long_help().to_string();
            assert!(
                help.contains("Examples:\n  $ mallard-cli "),
                "{}",
                noun.get_name()
            );
            assert!(!help.contains('\x1b'), "piped help carries no styling");
        }
        for path in [
            ["project", "add"],
            ["push", "apply"],
            ["pull", "apply"],
            ["catalog", "push"],
            ["catalog", "pull"],
            ["session", "timeline"],
            ["settings", "set"],
            ["log", "query"],
            ["cloud", "clone"],
        ] {
            let verb = command
                .find_subcommand(path[0])
                .and_then(|noun| noun.find_subcommand(path[1]))
                .unwrap_or_else(|| panic!("{path:?}"));
            let help = verb.clone().render_long_help().to_string();
            assert!(help.contains("Examples:\n"), "{path:?}");
        }
    }

    #[test]
    fn the_page_is_plain_when_piped_and_bold_on_a_terminal() {
        let command = Cli::command();
        let plain = root_page(&command, false);
        assert!(!plain.contains('\x1b'));
        for section in [
            "USAGE",
            "CORE COMMANDS",
            "STORAGE COMMANDS",
            "LIBRARY COMMANDS",
            "ADDITIONAL COMMANDS",
            "FLAGS",
            "EXAMPLES",
            "OUTPUT",
            "EXIT CODES",
            "CONVENTIONS",
            "LEARN MORE",
        ] {
            assert!(plain.contains(&format!("{section}\n")), "{section}");
        }
        assert!(plain.contains("  push:"));
        assert!(plain.contains("--json"));
        assert!(plain.contains("$ mallard-cli push review"));
        let styled = root_page(&command, true);
        assert!(styled.contains("\x1b[1mCORE COMMANDS\x1b[0m"));
    }
}
