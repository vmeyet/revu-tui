//! Keys the user adds on top of revu's own: a layout preset and per-action aliases from `[keys]`.
//! A user key never replaces a default one; it stands for it, so it does what that key does in the
//! pane you are in, and the help shows it in front of the default.
use crate::config::{Keys, Layout};
use anyhow::{Result, bail};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::BTreeMap;

/// Every action `[keys.bind]` can name, with the default keys it stands for.
pub const ACTIONS: &[(&str, &str)] = &[
    ("next_thread", "]n"),
    ("prev_thread", "[n"),
    ("next_hunk", "]c"),
    ("prev_hunk", "[c"),
    ("next_file_unresolved", "]f"),
    ("prev_file_unresolved", "[f"),
    ("next_file", "tab"),
    ("prev_file", "backtab"),
    ("fold_toggle", "za"),
    ("fold_open", "zo"),
    ("fold_close", "zc"),
    ("fold_all", "zM"),
    ("unfold_all", "zR"),
    ("fold_header", "zh"),
    ("viewed", "zv"),
    ("zen", "zz"),
    ("prev_mr", "left"),
    ("next_mr", "right"),
    ("split", "D"),
    ("tree", "t"),
    ("pipeline", "p"),
    ("wrap", "w"),
    ("whitespace", "W"),
    ("more_context", "+"),
    ("view_file", "v"),
    ("description", "i"),
    ("comment", "c"),
    ("comment_old", "C"),
    ("select", "V"),
    ("suggest", "s"),
    ("editor", "E"),
    ("resolve", "R"),
    ("publish", "P"),
    ("approve", "A"),
    ("share", "Y"),
    ("open_browser", "o"),
    ("copy_link", "y"),
    ("scope", "*"),
    ("sort_queue", "s"),
    ("group_by_author", "S"),
    ("filter", "/"),
    ("palette", ":"),
    ("views", "'"),
    ("jump", "ctrl-k"),
    ("help", "?"),
    ("quit", "q"),
];

/// Every key revu reads outside a text box, in some pane: a user key may not take one of them.
const TAKEN: &str = "qhl:z[]ai/*royjkgGDtpWw+xvcCVsEPAmedJKRSuY?'123456789";

/// The second keys each built-in prefix already reads.
const PREFIXED: &[(char, &str)] = &[('z', "aochzvMR"), ('[', "cnf"), (']', "cnf"), ('a', "ersta")];

/// One key as the terminal reports it; letters keep their case, so shift is not a separate flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    code: Code,
    ctrl: bool,
    alt: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Code {
    Char(char),
    Tab,
    BackTab,
    Enter,
    Esc,
    Up,
    Down,
    Left,
    Right,
}

impl Key {
    fn char(c: char) -> Self {
        Self { code: Code::Char(c), ctrl: false, alt: false }
    }

    /// `None` for keys bindings cannot name, which pass through as they came: ⌘ and friends
    /// among them, or ⌘K would reach the app as a plain `k`.
    fn of(event: KeyEvent) -> Option<Self> {
        if event.modifiers.intersects(KeyModifiers::SUPER | KeyModifiers::HYPER | KeyModifiers::META) {
            return None;
        }
        let code = match event.code {
            KeyCode::Char(c) => Code::Char(c),
            KeyCode::Tab => Code::Tab,
            KeyCode::BackTab => Code::BackTab,
            KeyCode::Enter => Code::Enter,
            KeyCode::Esc => Code::Esc,
            KeyCode::Up => Code::Up,
            KeyCode::Down => Code::Down,
            KeyCode::Left => Code::Left,
            KeyCode::Right => Code::Right,
            _ => return None,
        };
        let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
        let alt = event.modifiers.contains(KeyModifiers::ALT);
        Some(Self { code, ctrl, alt })
    }

    fn event(self) -> KeyEvent {
        let code = match self.code {
            Code::Char(c) => KeyCode::Char(c),
            Code::Tab => KeyCode::Tab,
            Code::BackTab => KeyCode::BackTab,
            Code::Enter => KeyCode::Enter,
            Code::Esc => KeyCode::Esc,
            Code::Up => KeyCode::Up,
            Code::Down => KeyCode::Down,
            Code::Left => KeyCode::Left,
            Code::Right => KeyCode::Right,
        };
        let mut modifiers = KeyModifiers::NONE;
        modifiers.set(KeyModifiers::CONTROL, self.ctrl);
        modifiers.set(KeyModifiers::ALT, self.alt);
        KeyEvent::new(code, modifiers)
    }

    /// As the help writes it: `^k`, `⌥x`, `S-tab`, `)`.
    fn label(self) -> String {
        let base = match self.code {
            Code::Char(c) => c.to_string(),
            Code::Tab => "tab".into(),
            Code::BackTab => "S-tab".into(),
            Code::Enter => "enter".into(),
            Code::Esc => "esc".into(),
            Code::Up => "↑".into(),
            Code::Down => "↓".into(),
            Code::Left => "←".into(),
            Code::Right => "→".into(),
        };
        match (self.ctrl, self.alt) {
            (true, _) => format!("^{base}"),
            (false, true) => format!("⌥{base}"),
            (false, false) => base,
        }
    }

    fn plain_char(self) -> Option<char> {
        match self.code {
            Code::Char(c) if !self.ctrl && !self.alt => Some(c),
            _ => None,
        }
    }
}

/// `"n"`, `")n"`, `"zz"`: one or two plain keys; `"ctrl-k"`, `"alt-n"`, `"tab"`, `"backtab"`, `"enter"`: one named key.
pub fn parse(text: &str) -> Result<Vec<Key>> {
    if let Some(key) = named(text)? {
        return Ok(vec![key]);
    }
    let keys: Vec<Key> = text.chars().map(Key::char).collect();
    match keys.len() {
        1 | 2 if !text.contains(char::is_whitespace) => Ok(keys),
        _ => bail!("`{text}` is not a key: write one or two keys (`n`, `)n`) or a named one (`ctrl-k`, `tab`)"),
    }
}

fn named(text: &str) -> Result<Option<Key>> {
    let (ctrl, alt, rest) = match text.split_once('-') {
        Some(("ctrl", rest)) if !rest.is_empty() => (true, false, rest),
        Some(("alt", rest)) if !rest.is_empty() => (false, true, rest),
        _ => (false, false, text),
    };
    let code = match rest {
        "tab" => Code::Tab,
        "backtab" | "shift-tab" => Code::BackTab,
        "enter" => Code::Enter,
        "esc" => Code::Esc,
        "up" => Code::Up,
        "down" => Code::Down,
        "left" => Code::Left,
        "right" => Code::Right,
        _ if ctrl || alt => {
            let mut chars = rest.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Code::Char(c),
                _ => bail!("`{text}` is not a key: after `ctrl-` or `alt-` comes one character"),
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(Key { code, ctrl, alt }))
}

/// What the loop does with a key before the app sees it.
#[derive(Debug, PartialEq, Eq)]
pub enum Feed {
    /// The first key of a two-key binding of the user's: wait for the second.
    Hold(KeyEvent),
    /// Hand these to the app, in order: the default keys a binding stands for, or the key itself.
    Keys(Vec<KeyEvent>),
}

/// The user's keys, each standing for revu's default keys.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keymap {
    aliases: BTreeMap<Vec<Key>, Vec<Key>>,
    /// The help labels: per default binding, the user's keys to show in front of it.
    shown: BTreeMap<Vec<Key>, Vec<Vec<Key>>>,
    azerty: bool,
}

impl Keymap {
    /// Fails on an unknown action, a key that does not parse, or a key already taken.
    pub fn new(keys: &Keys) -> Result<Self> {
        let azerty = keys.layout == Layout::Azerty;
        let mut owners: BTreeMap<Vec<Key>, &str> = BTreeMap::new();
        let mut map = Self { azerty, ..Self::default() };
        if azerty {
            map.aliases.insert(vec![Key::char('(')], vec![Key::char('[')]);
            map.aliases.insert(vec![Key::char(')')], vec![Key::char(']')]);
        }
        for (action, texts) in &keys.bind {
            let Some((_, default)) = ACTIONS.iter().find(|(name, _)| name == action) else {
                bail!("`[keys.bind] {action}` is not an action; the actions are {}", names());
            };
            let target = parse(default)?;
            for text in texts.all() {
                let binding = parse(text).map_err(|e| anyhow::anyhow!("`[keys.bind] {action}`: {e}"))?;
                map.check_free(&binding, action, &owners)?;
                owners.insert(binding.clone(), action);
                map.aliases.insert(binding.clone(), target.clone());
                map.shown.entry(target.clone()).or_default().push(binding);
            }
        }
        Ok(map)
    }

    fn check_free(&self, binding: &[Key], action: &str, owners: &BTreeMap<Vec<Key>, &str>) -> Result<()> {
        let text = label(binding);
        for (other, owner) in owners {
            if other.as_slice() == binding || other.starts_with(binding) || binding.starts_with(other) {
                bail!("`[keys.bind]`: `{text}` for {action} collides with `{}` for {owner}", label(other));
            }
        }
        if let Some(existing) = self.taken_by(binding) {
            bail!("`[keys.bind]`: `{text}` for {action} is already revu's {existing}");
        }
        Ok(())
    }

    /// Which built-in key the binding would shadow, if any.
    fn taken_by(&self, binding: &[Key]) -> Option<String> {
        let first = binding[0];
        let default_of = |keys: &[Key]| ACTIONS.iter().find(|(_, d)| parse(d).is_ok_and(|p| p == keys)).map(|(name, _)| *name);
        let prefix = first.plain_char().and_then(|c| self.prefix_of(c));
        match (binding, prefix) {
            ([_, second], Some(p)) => {
                let reads = PREFIXED.iter().find(|(c, _)| *c == p).is_some_and(|(_, s)| second.plain_char().is_some_and(|c| s.contains(c)));
                let canon = [Key::char(p), *second];
                reads.then(|| default_of(&canon).map_or_else(|| format!("`{}`", label(&canon)), |n| format!("`{}` ({n})", label(&canon))))
            }
            _ if self.aliases.contains_key(&binding[..1]) => Some(format!("`{}`", first.label())),
            _ => {
                let taken = match first.plain_char() {
                    Some(c) => TAKEN.contains(c),
                    None => {
                        matches!(first.code, Code::Enter | Code::Esc | Code::Tab | Code::BackTab)
                            || (first.ctrl && matches!(first.code, Code::Char('c' | 'd' | 'u' | 'k')))
                    }
                };
                taken.then(|| {
                    default_of(&binding[..1]).map_or_else(|| format!("`{}`", first.label()), |n| format!("`{}` ({n})", first.label()))
                })
            }
        }
    }

    /// The built-in prefix a key opens: its own, or the one the layout gives it.
    fn prefix_of(&self, c: char) -> Option<char> {
        match c {
            '(' if self.azerty => Some('['),
            ')' if self.azerty => Some(']'),
            'z' | '[' | ']' | 'a' => Some(c),
            _ => None,
        }
    }

    /// One key from the terminal, with the key held since the last call, if any. A held key
    /// whose second key matches nothing is handed over as itself, so `z` then `a` stays `za`.
    pub fn feed(&self, held: Option<KeyEvent>, event: KeyEvent) -> Feed {
        let Some(key) = Key::of(event) else { return Feed::Keys(held.into_iter().chain([event]).collect()) };
        if let Some(first) = held.and_then(Key::of) {
            return match self.aliases.get(&vec![first, key]) {
                Some(target) => Feed::Keys(events(target)),
                None => Feed::Keys([self.one(first), self.one(key)].concat()),
            };
        }
        if self.aliases.keys().any(|b| b.len() == 2 && b[0] == key) {
            return Feed::Hold(event);
        }
        Feed::Keys(self.one(key))
    }

    /// A single key through the one-key bindings.
    fn one(&self, key: Key) -> Vec<KeyEvent> {
        self.aliases.get(&vec![key]).map_or_else(|| vec![key.event()], |target| events(target))
    }

    /// A help entry's keys (`]n [n`) as they are in effect: the user's in front, `(`/`)` for `[`/`]` on AZERTY.
    pub fn label(&self, keys: &str) -> String {
        keys.split(' ').map(|word| self.label_word(word)).collect::<Vec<_>>().join(" ")
    }

    fn label_word(&self, word: &str) -> String {
        let canonical = help_word(word);
        let mine: Vec<String> = canonical.and_then(|c| self.shown.get(&c)).into_iter().flatten().map(|b| label(b)).collect();
        let own = match word.chars().next() {
            Some('[') if self.azerty => word.replacen('[', "(", 1),
            Some(']') if self.azerty => word.replacen(']', ")", 1),
            _ => word.to_owned(),
        };
        mine.into_iter().chain([own]).collect::<Vec<_>>().join(" ")
    }
}

/// A help word back to keys: `^k` is ctrl-k, `S-tab` is backtab; anything else parses as written.
fn help_word(word: &str) -> Option<Vec<Key>> {
    match word {
        "S-tab" => parse("backtab").ok(),
        "←" => parse("left").ok(),
        "→" => parse("right").ok(),
        _ => match word.strip_prefix('^') {
            Some(rest) if rest.chars().count() == 1 => parse(&format!("ctrl-{rest}")).ok(),
            _ => parse(word).ok(),
        },
    }
}

fn events(keys: &[Key]) -> Vec<KeyEvent> {
    keys.iter().map(|k| k.event()).collect()
}

fn label(keys: &[Key]) -> String {
    keys.iter().map(|k| k.label()).collect()
}

fn names() -> String {
    ACTIONS.iter().map(|(name, _)| *name).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::config::Bind;

    fn keys(layout: Layout, bind: &[(&str, &[&str])]) -> Keys {
        let bind = bind.iter().map(|(a, k)| ((*a).to_owned(), Bind::Many(k.iter().map(|s| (*s).to_owned()).collect()))).collect();
        Keys { layout, bind }
    }

    fn press(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn chars(feed: &Feed) -> String {
        match feed {
            Feed::Keys(events) => events.iter().map(|e| Key::of(*e).unwrap().label()).collect(),
            Feed::Hold(_) => "…".into(),
        }
    }

    #[test]
    fn a_command_key_passes_through_with_its_modifier() {
        let keymap = Keymap::new(&keys(Layout::Azerty, &[])).unwrap();
        let cmd_k = KeyEvent::new(KeyCode::Char('k'), KeyModifiers::SUPER);
        assert_eq!(keymap.feed(None, cmd_k), Feed::Keys(vec![cmd_k]));
    }

    #[test]
    fn keys_parse_as_one_or_two_plain_keys_or_one_named_key() {
        assert_eq!(parse("n").unwrap(), vec![Key::char('n')]);
        assert_eq!(parse(")n").unwrap(), vec![Key::char(')'), Key::char('n')]);
        assert_eq!(label(&parse("ctrl-k").unwrap()), "^k");
        assert_eq!(label(&parse("alt-n").unwrap()), "⌥n");
        assert_eq!(label(&parse("backtab").unwrap()), "S-tab");
        for bad in ["", "abc", "ctrl-", "ctrl-ab", "a b"] {
            assert!(parse(bad).is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn without_config_every_key_passes_through() {
        let map = Keymap::new(&Keys::default()).unwrap();
        assert_eq!(chars(&map.feed(None, press(')'))), ")");
        assert_eq!(map.label("]n [n"), "]n [n");
    }

    #[test]
    fn the_azerty_preset_reads_parentheses_as_brackets_and_says_so_in_the_help() {
        let map = Keymap::new(&keys(Layout::Azerty, &[])).unwrap();
        assert_eq!(chars(&map.feed(None, press(')'))), "]");
        assert_eq!(chars(&map.feed(None, press('('))), "[");
        assert_eq!(chars(&map.feed(None, press(']'))), "]", "brackets keep working");
        assert_eq!(map.label("]n [n"), ")n (n");
        assert_eq!(map.label("zc zo"), "zc zo");
    }

    #[test]
    fn a_binding_stands_for_the_default_keys_of_its_action() {
        let map = Keymap::new(&keys(Layout::Qwerty, &[("next_thread", &["n"]), ("prev_thread", &["N"]), ("jump", &["ctrl-p"])])).unwrap();
        assert_eq!(chars(&map.feed(None, press('n'))), "]n");
        assert_eq!(chars(&map.feed(None, press('N'))), "[n");
        assert_eq!(chars(&map.feed(None, KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL))), "^k");
        assert_eq!(map.label("]n [n"), "n ]n N [n");
        assert_eq!(map.label("^k"), "^p ^k");
    }

    #[test]
    fn a_two_key_binding_waits_for_its_second_key_and_lets_anything_else_through() {
        let map = Keymap::new(&keys(Layout::Qwerty, &[("next_hunk", &["gh"])])).unwrap_err();
        assert!(map.to_string().contains("`g`"), "{map}");
        let map = Keymap::new(&keys(Layout::Qwerty, &[("next_hunk", &["nh"])])).unwrap();
        let held = map.feed(None, press('n'));
        assert_eq!(held, Feed::Hold(press('n')));
        assert_eq!(chars(&map.feed(Some(press('n')), press('h'))), "]c");
        assert_eq!(chars(&map.feed(Some(press('n')), press('j'))), "nj", "a wrong second key hands both over as they are");
    }

    #[test]
    fn a_taken_key_fails_and_names_what_owns_it() {
        let err = Keymap::new(&keys(Layout::Qwerty, &[("next_thread", &["c"])])).unwrap_err().to_string();
        assert!(err.contains("`c`") && err.contains("comment") && err.contains("next_thread"), "{err}");
        let err = Keymap::new(&keys(Layout::Qwerty, &[("next_thread", &["za"])])).unwrap_err().to_string();
        assert!(err.contains("fold_toggle"), "{err}");
        let err = Keymap::new(&keys(Layout::Azerty, &[("next_thread", &[")c"])])).unwrap_err().to_string();
        assert!(err.contains("next_hunk"), "with the preset `)c` is `]c`: {err}");
        let map = Keymap::new(&keys(Layout::Azerty, &[("next_thread", &[")x"])])).unwrap();
        assert_eq!(map.feed(None, press(')')), Feed::Hold(press(')')), "`)` waits: it may start `)x`");
        assert_eq!(chars(&map.feed(Some(press(')')), press('x'))), "]n");
        assert_eq!(chars(&map.feed(Some(press(')')), press('c'))), "]c", "anything else is still the `]` prefix");
        let zx = Keymap::new(&keys(Layout::Qwerty, &[("zen", &["zx"])])).unwrap();
        assert_eq!(chars(&zx.feed(Some(press('z')), press('a'))), "za", "holding `z` never breaks `za`");
    }

    #[test]
    fn two_bindings_on_one_key_fail_and_name_both_actions() {
        let err = Keymap::new(&keys(Layout::Qwerty, &[("next_thread", &["n"]), ("next_hunk", &["n"])])).unwrap_err().to_string();
        assert!(err.contains("next_thread") && err.contains("next_hunk"), "{err}");
        let err = Keymap::new(&keys(Layout::Qwerty, &[("next_thread", &["n"]), ("next_hunk", &["nh"])])).unwrap_err().to_string();
        assert!(err.contains("next_thread") && err.contains("next_hunk"), "a key that starts another binding collides: {err}");
    }

    #[test]
    fn an_unknown_action_fails_and_lists_the_real_ones() {
        let err = Keymap::new(&keys(Layout::Qwerty, &[("next_thred", &["n"])])).unwrap_err().to_string();
        assert!(err.contains("next_thred") && err.contains("next_thread"), "{err}");
    }

    #[test]
    fn every_action_default_parses_and_every_one_key_default_is_taken() {
        for (name, default) in ACTIONS {
            let keys = parse(default).unwrap();
            if let [key] = keys.as_slice()
                && let Some(c) = key.plain_char()
            {
                assert!(TAKEN.contains(c), "{name}: `{c}` should be in TAKEN");
            }
        }
    }
}
