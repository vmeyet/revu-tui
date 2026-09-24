//! The reference pages under `docs/reference/`, written from the code so they never drift:
//! keys from the help groups and the keymap, commands from clap, config keys from [`SETTINGS`].
use crate::cli::Cli;
use crate::config::{Keys, Layout};
use crate::keymap::{ACTIONS, Keymap};
use crate::tui::help::GROUPS;
use anyhow::Result;
use clap::{Arg, Command, CommandFactory};

/// One config key as the reference page shows it.
pub struct Setting {
    /// The TOML table, `""` for the top level.
    pub table: &'static str,
    pub key: &'static str,
    pub kind: &'static str,
    pub default: &'static str,
    pub meaning: &'static str,
    /// A TOML value the config accepts for this key, as written after `key = `.
    pub example: &'static str,
}

/// Every key `config.toml` accepts; a test fails when `src/config.rs` gains a field missing here.
pub const SETTINGS: &[Setting] = &[
    Setting {
        table: "",
        key: "host",
        kind: "text",
        default: "gitlab.com",
        meaning: "The host revu uses outside a checkout.",
        example: "\"gitlab.com\"",
    },
    Setting {
        table: "",
        key: "username",
        kind: "text",
        default: "set by `revu login`",
        meaning: "Your name on that host.",
        example: "\"nina\"",
    },
    Setting {
        table: "queue",
        key: "watch_labels",
        kind: "list of text",
        default: "`[]`",
        meaning: "MRs with one of these labels go to WATCHING.",
        example: "[\"infra\"]",
    },
    Setting { table: "queue", key: "groups", kind: "list of text", default: "`[]`", meaning: "Not used yet.", example: "[]" },
    Setting { table: "queue", key: "projects", kind: "list of text", default: "`[]`", meaning: "Not used yet.", example: "[]" },
    Setting {
        table: "queue.ready",
        key: "command",
        kind: "command line",
        default: "none",
        meaning: "A program that prints MR links; the open ones that need you fill READY.",
        example: "\"slack messages '#review' --since 14d --json\"",
    },
    Setting {
        table: "queue.rules",
        key: "enabled",
        kind: "true or false",
        default: "`true`",
        meaning: "Keep To review, Watching and Open to what needs you; the rest moves to OTHER, folded.",
        example: "true",
    },
    Setting {
        table: "queue.rules",
        key: "stale_days",
        kind: "number",
        default: "`14`",
        meaning: "An MR with no activity for longer moves to OTHER.",
        example: "14",
    },
    Setting {
        table: "queue.rules",
        key: "reviewed_comments",
        kind: "number",
        default: "`3`",
        meaning: "An MR with this many comments from others and none from you sorts last.",
        example: "3",
    },
    Setting {
        table: "queue.rules",
        key: "not_ready",
        kind: "list of text",
        default: "`[\"wip\", \"do not review\", \"don't review\", \"not ready\"]`",
        meaning: "Words in the title or the description's first paragraph that mark an MR as not ready.",
        example: "[\"wip\", \"not ready\"]",
    },
    Setting {
        table: "share",
        key: "command",
        kind: "command line",
        default: "none",
        meaning: "What `Y` posts through: the message arrives on its stdin.",
        example: "\"slack send '#review'\"",
    },
    Setting {
        table: "share",
        key: "template",
        kind: "text with placeholders",
        default: "`[{ref} {title}]({url})` then `_{note}_`",
        meaning: "The message; a line with `{note}` goes when the note is empty.",
        example: "\"[{ref} {title}]({url})\\n_{note}_\"",
    },
    Setting {
        table: "share.targets",
        key: "<name>",
        kind: "table",
        default: "none",
        meaning: "One more place to post to; `Y` asks which when there are several.",
        example: "{ command = \"slack send '#team'\" }",
    },
    Setting {
        table: "share.targets.<name>",
        key: "command",
        kind: "command line",
        default: "none",
        meaning: "What this target posts through: the message arrives on its stdin.",
        example: "\"slack send '#team'\"",
    },
    Setting {
        table: "share.targets.<name>",
        key: "template",
        kind: "text with placeholders",
        default: "`[share] template`",
        meaning: "This target's message.",
        example: "\"{ref} {title} {url}\"",
    },
    Setting {
        table: "queue.views",
        key: "<name>",
        kind: "search query",
        default: "none",
        meaning: "A saved filter; `'` and its first letter applies it.",
        example: "\"@me ~frontend\"",
    },
    Setting {
        table: "review",
        key: "fold",
        kind: "list of globs",
        default: "`[]`",
        meaning: "Files that open folded.",
        example: "[\"*.lock\", \"**/generated/**\"]",
    },
    Setting {
        table: "review",
        key: "inline_max_words",
        kind: "number",
        default: "2",
        meaning: "At most this many changed words per side show on one row.",
        example: "2",
    },
    Setting {
        table: "review",
        key: "inline_min_same",
        kind: "percent",
        default: "60",
        meaning: "Both lines keep at least this share of text to show on one row.",
        example: "60",
    },
    Setting { table: "tui", key: "theme", kind: "text", default: "`default`", meaning: "The colour palette.", example: "\"catppuccin\"" },
    Setting {
        table: "tui",
        key: "queue",
        kind: "`comfortable` or `compact`",
        default: "`comfortable`",
        meaning: "Two lines per MR in the queue, or one.",
        example: "\"compact\"",
    },
    Setting {
        table: "tui",
        key: "images",
        kind: "true or false",
        default: "true",
        meaning: "Draw pictures from comments where the terminal can.",
        example: "false",
    },
    Setting { table: "tui", key: "ascii", kind: "true or false", default: "false", meaning: "Not used yet.", example: "false" },
    Setting {
        table: "open",
        key: "default",
        kind: "command",
        default: "`$VISUAL`, `$EDITOR`, then `less`",
        meaning: "The program `v` opens a file with.",
        example: "\"hx\"",
    },
    Setting {
        table: "open.files",
        key: "<glob>",
        kind: "command",
        default: "none",
        meaning: "The program for files that match; the longest glob wins.",
        example: "\"glow -p\"",
    },
    Setting {
        table: "notify",
        key: "enabled",
        kind: "true or false",
        default: "true",
        meaning: "A macOS notification when an MR lands in TO REVIEW.",
        example: "false",
    },
    Setting {
        table: "keys",
        key: "layout",
        kind: "`qwerty` or `azerty`",
        default: "`qwerty`",
        meaning: "With `azerty`, `(` and `)` work like `[` and `]`.",
        example: "\"azerty\"",
    },
    Setting {
        table: "keys.bind",
        key: "<action>",
        kind: "key or list of keys",
        default: "none",
        meaning: "Extra keys for an action; see the actions table.",
        example: "\"N\"",
    },
    Setting {
        table: "ai.typesafe",
        key: "enabled",
        kind: "true or false",
        default: "false",
        meaning: "Jev marks urgent MRs and risky files.",
        example: "true",
    },
    Setting {
        table: "ai.anthropic",
        key: "enabled",
        kind: "true or false",
        default: "false",
        meaning: "`a` asks Claude about the diff.",
        example: "true",
    },
    Setting {
        table: "ai.anthropic",
        key: "model",
        kind: "text",
        default: "`claude-opus-5`",
        meaning: "The Claude model to ask.",
        example: "\"claude-opus-5\"",
    },
    Setting {
        table: "hosts.\"<host>\"",
        key: "forge",
        kind: "`gitlab` or `github`",
        default: "from the host name",
        meaning: "The forge of a host whose name does not say it.",
        example: "\"github\"",
    },
    Setting {
        table: "hosts.\"<host>\"",
        key: "username",
        kind: "text",
        default: "set by `revu login`",
        meaning: "Your name on that host.",
        example: "\"nina\"",
    },
];

/// Each page: its path under `docs/reference/`, then its text.
pub fn pages() -> Result<Vec<(&'static str, String)>> {
    Ok(vec![("keys.md", keys_page()?), ("config.md", config_page()), ("commands.md", commands_page())])
}

const GENERATED: &str = "<!-- Written by `revu docs` from the code. Edit the code, then run `revu docs`. -->";

fn keys_page() -> Result<String> {
    let azerty = Keymap::new(&Keys { layout: Layout::Azerty, ..Keys::default() })?;
    let mut page = vec![
        GENERATED.to_owned(),
        "# Keys".to_owned(),
        String::new(),
        "Every key revu reads, grouped by task.".to_owned(),
        "Press `?` in revu for the same list, with your own keys.".to_owned(),
        "A key like `]n` is two keys in a row: `]`, then `n`.".to_owned(),
    ];
    for group in &GROUPS {
        page.extend([
            String::new(),
            format!("## {}", capitalized(group.title)),
            String::new(),
            "| Keys | Does |".into(),
            "|---|---|".into(),
        ]);
        page.extend(group.keys.iter().map(|(keys, what)| format!("| {} | {what} |", code_words(keys))));
    }
    page.extend([
        String::new(),
        "## AZERTY".to_owned(),
        String::new(),
        "With `[keys] layout = \"azerty\"`, these keys change.".to_owned(),
        "The old ones keep working.".to_owned(),
        String::new(),
        "| Default | AZERTY |".to_owned(),
        "|---|---|".to_owned(),
    ]);
    let changed = GROUPS.iter().flat_map(|g| g.keys.iter()).filter_map(|(keys, _)| {
        let label = azerty.label(keys);
        (label != *keys).then(|| format!("| {} | {} |", code_words(keys), code_words(&label)))
    });
    page.extend(changed);
    page.extend([
        String::new(),
        "## Actions you can bind".to_owned(),
        String::new(),
        "Give an action more keys under `[keys.bind]`, as in `next_thread = \"N\"`.".to_owned(),
        "A bound key adds to the default key, it does not replace it.".to_owned(),
        String::new(),
        "| Action | Default key |".to_owned(),
        "|---|---|".to_owned(),
    ]);
    page.extend(ACTIONS.iter().map(|(action, key)| format!("| `{action}` | `{key}` |")));
    Ok(finish(&page))
}

fn config_page() -> String {
    let mut page = vec![
        GENERATED.to_owned(),
        "# Config".to_owned(),
        String::new(),
        "revu reads `~/.config/revu/config.toml`, or `$XDG_CONFIG_HOME/revu/config.toml`.".to_owned(),
        "Every key is optional.".to_owned(),
        "A key revu does not know stops it at start, with the file and the key named.".to_owned(),
        "Tokens never go here: they live in the macOS keychain.".to_owned(),
    ];
    let mut tables: Vec<&str> = vec![];
    for setting in SETTINGS {
        if !tables.contains(&setting.table) {
            tables.push(setting.table);
        }
    }
    for table in tables {
        let title = if table.is_empty() { "Top level".to_owned() } else { format!("`[{table}]`") };
        page.extend([
            String::new(),
            format!("## {title}"),
            String::new(),
            "| Key | Type | Default | Does |".into(),
            "|---|---|---|---|".into(),
        ]);
        page.extend(
            SETTINGS.iter().filter(|s| s.table == table).map(|s| format!("| `{}` | {} | {} | {} |", s.key, s.kind, s.default, s.meaning)),
        );
    }
    page.extend([String::new(), "## Example".to_owned(), String::new(), "```toml".to_owned()]);
    page.extend(example_toml().lines().map(str::to_owned));
    page.push("```".to_owned());
    finish(&page)
}

/// Every setting with its example value, as one TOML document; a test parses it.
pub fn example_toml() -> String {
    let mut lines = vec![];
    let mut current = None;
    for setting in SETTINGS.iter().filter(|s| !s.meaning.starts_with("Not used")) {
        let table = setting.table.replace("<host>", "git.acme.dev").replace("<name>", "team");
        if current.as_deref() != Some(table.as_str()) {
            if !table.is_empty() {
                lines.push(String::new());
                lines.push(format!("[{table}]"));
            }
            current = Some(table);
        }
        let key = match setting.key {
            "<name>" => "mine",
            "<glob>" => "\"*.md\"",
            "<action>" => "next_thread",
            key => key,
        };
        lines.push(format!("{key} = {}", setting.example));
    }
    lines.join("\n")
}

fn commands_page() -> String {
    let cli = Cli::command();
    let mut page = vec![
        GENERATED.to_owned(),
        "# Commands".to_owned(),
        String::new(),
        "`revu` with no command opens the review screen.".to_owned(),
        "Every command also takes the flags below.".to_owned(),
        String::new(),
    ];
    page.extend(flag_table(cli.get_arguments().filter(|a| !a.is_positional() && !a.is_hide_set())));
    for command in cli.get_subcommands().filter(|c| !c.is_hide_set() && c.get_name() != "help") {
        page.extend(command_section(command, "revu"));
    }
    finish(&page)
}

fn command_section(command: &Command, parent: &str) -> Vec<String> {
    let name = format!("{parent} {}", command.get_name());
    let about = command.get_about().map(ToString::to_string).map(sentence).unwrap_or_default();
    let mut lines = vec![String::new(), format!("## `{name}`"), String::new(), about];
    let positionals: Vec<&Arg> = command.get_arguments().filter(|a| a.is_positional()).collect();
    if !positionals.is_empty() {
        lines.extend([String::new(), "| Argument | Means |".into(), "|---|---|".into()]);
        lines.extend(positionals.iter().map(|a| format!("| `{}` | {} |", a.get_id().as_str().to_uppercase(), help_of(a))));
    }
    let flags: Vec<&Arg> = command
        .get_arguments()
        .filter(|a| !a.is_positional() && !a.is_global_set() && a.get_id() != "help" && a.get_id() != "version")
        .collect();
    if !flags.is_empty() {
        lines.push(String::new());
        lines.extend(flag_table(flags.into_iter()));
    }
    for sub in command.get_subcommands().filter(|c| !c.is_hide_set() && c.get_name() != "help") {
        lines.extend(command_section(sub, &name));
    }
    lines
}

fn flag_table<'a>(flags: impl Iterator<Item = &'a Arg>) -> Vec<String> {
    let mut lines = vec!["| Flag | Means |".to_owned(), "|---|---|".to_owned()];
    lines.extend(flags.filter(|a| a.get_id() != "help" && a.get_id() != "version").map(|a| {
        let long = a.get_long().map(|l| format!("--{l}")).unwrap_or_default();
        let short = a.get_short().map(|s| format!("-{s}, ")).unwrap_or_default();
        format!("| `{short}{long}` | {} |", help_of(a))
    }));
    lines
}

fn help_of(arg: &Arg) -> String {
    sentence(arg.get_help().map(ToString::to_string).unwrap_or_default()).replace('|', "\\|")
}

/// clap drops the final period of a doc comment; the page puts it back.
fn sentence(text: String) -> String {
    if text.is_empty() || text.ends_with(['.', '?', '!', ')']) { text } else { format!("{text}.") }
}

fn code_words(keys: &str) -> String {
    keys.split(' ').map(|k| format!("`{k}`")).collect::<Vec<_>>().join(" ")
}

fn capitalized(title: &str) -> String {
    let mut chars = title.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

fn finish(lines: &[String]) -> String {
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn the_example_config_is_a_config_revu_accepts() {
        let text = example_toml();
        let path = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(path.path(), &text).unwrap();
        crate::config::Config::load_from(path.path()).unwrap_or_else(|e| panic!("{e:#}\n{text}"));
    }

    /// Every `pub` field of the config structs has its row, so a new key cannot ship undocumented.
    #[test]
    fn every_config_field_is_documented() {
        let source = include_str!("config.rs");
        let tables = [
            ("Config", ""),
            ("Keys", "keys"),
            ("Host", "hosts.\"<host>\""),
            ("Queue", "queue"),
            ("Review", "review"),
            ("Notify", "notify"),
            ("Tui", "tui"),
            ("Typesafe", "ai.typesafe"),
            ("Anthropic", "ai.anthropic"),
            ("Open", "open"),
            ("Share", "share"),
            ("ShareTarget", "share.targets.<name>"),
        ];
        let nested = ["queue", "review", "tui", "ai", "open", "notify", "keys", "hosts", "share"];
        for (name, table) in tables {
            let start = source.find(&format!("pub struct {name} {{")).unwrap_or_else(|| panic!("struct {name}"));
            let body = &source[start..start + source[start..].find("\n}").unwrap()];
            for field in body.lines().filter_map(|l| l.trim().strip_prefix("pub ")).filter_map(|l| l.split(':').next()) {
                let field = field.trim();
                if field.starts_with("struct") || (table.is_empty() && nested.contains(&field)) {
                    continue;
                }
                let nested_table = format!("{table}.{field}");
                let documented = SETTINGS.iter().any(|s| (s.table == table && s.key == field) || s.table == nested_table);
                assert!(documented, "`{name}.{field}` has no row in docs::SETTINGS");
            }
        }
    }

    #[test]
    fn every_queue_rule_is_documented() {
        let source = include_str!("forge/rules.rs");
        let start = source.find("pub struct Rules {").expect("struct Rules");
        let body = &source[start..start + source[start..].find("\n}").unwrap()];
        let fields = body.lines().filter_map(|l| l.trim().strip_prefix("pub ")).filter(|l| !l.starts_with("struct"));
        for field in fields.filter_map(|l| l.split(':').next()) {
            assert!(
                SETTINGS.iter().any(|s| s.table == "queue.rules" && s.key == field.trim()),
                "`queue.rules.{}` has no row in docs::SETTINGS",
                field.trim()
            );
        }
    }

    #[test]
    fn every_page_says_it_is_generated() {
        for (name, text) in pages().unwrap() {
            assert!(text.starts_with(GENERATED), "{name}");
        }
    }
}
