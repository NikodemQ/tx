//! Vim-style key handling. A registry names every command once; the default bindings, the
//! config file and the help overlay are all derived from it.

use std::collections::{BTreeMap, HashMap, HashSet};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub code: KeyCode,
    pub ctrl: bool,
    pub alt: bool,
}

impl Key {
    pub fn plain(code: KeyCode) -> Key {
        Key {
            code,
            ctrl: false,
            alt: false,
        }
    }

    /// Shift is folded into the character, so `G` is just `Char('G')`.
    pub fn from_event(event: KeyEvent) -> Key {
        Key {
            code: event.code,
            ctrl: event.modifiers.contains(KeyModifiers::CONTROL),
            alt: event.modifiers.contains(KeyModifiers::ALT),
        }
    }

    /// The character this key types, if it is an unmodified printable one.
    pub fn typed_char(self) -> Option<char> {
        match self.code {
            KeyCode::Char(c) if !self.ctrl && !self.alt => Some(c),
            _ => None,
        }
    }

    /// Parses vim notation such as `gg`, `<c-d>`, `<space>` or `<cr>`.
    pub fn parse_seq(text: &str) -> Result<Vec<Key>, String> {
        let mut keys = Vec::new();
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c != '<' {
                keys.push(Key::plain(KeyCode::Char(c)));
                continue;
            }
            let mut name = String::new();
            loop {
                match chars.next() {
                    Some('>') => break,
                    Some(ch) => name.push(ch),
                    None => return Err(format!("missing > in {text:?}")),
                }
            }
            keys.push(parse_named(&name)?);
        }
        if keys.is_empty() {
            return Err("empty key sequence".into());
        }
        Ok(keys)
    }

    pub fn label(self) -> String {
        let base = match self.code {
            KeyCode::Char(' ') => "space".to_string(),
            KeyCode::Char('<') => "lt".to_string(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Enter => "cr".into(),
            KeyCode::Esc => "esc".into(),
            KeyCode::Tab => "tab".into(),
            KeyCode::Backspace => "bs".into(),
            KeyCode::Delete => "del".into(),
            KeyCode::Up => "up".into(),
            KeyCode::Down => "down".into(),
            KeyCode::Left => "left".into(),
            KeyCode::Right => "right".into(),
            KeyCode::Home => "home".into(),
            KeyCode::End => "end".into(),
            KeyCode::PageUp => "pageup".into(),
            KeyCode::PageDown => "pagedown".into(),
            other => format!("{other:?}").to_lowercase(),
        };
        let plain_char = matches!(self.code, KeyCode::Char(c) if c != ' ' && c != '<');
        if plain_char && !self.ctrl && !self.alt {
            return base;
        }
        let prefix = format!(
            "{}{}",
            if self.ctrl { "c-" } else { "" },
            if self.alt { "a-" } else { "" }
        );
        format!("<{prefix}{base}>")
    }
}

fn parse_named(name: &str) -> Result<Key, String> {
    let mut rest = name;
    let (mut ctrl, mut alt) = (false, false);
    loop {
        let lower = rest.to_ascii_lowercase();
        if let Some(tail) = lower
            .strip_prefix("c-")
            .or_else(|| lower.strip_prefix("ctrl-"))
        {
            ctrl = true;
            rest = &rest[rest.len() - tail.len()..];
        } else if let Some(tail) = lower
            .strip_prefix("a-")
            .or_else(|| lower.strip_prefix("alt-"))
        {
            alt = true;
            rest = &rest[rest.len() - tail.len()..];
        } else {
            break;
        }
    }
    let code = match rest.to_ascii_lowercase().as_str() {
        "cr" | "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "space" => KeyCode::Char(' '),
        "tab" => KeyCode::Tab,
        "bs" | "backspace" => KeyCode::Backspace,
        "del" | "delete" => KeyCode::Delete,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "lt" => KeyCode::Char('<'),
        _ => {
            let mut chars = rest.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if ctrl => KeyCode::Char(c.to_ascii_lowercase()),
                (Some(c), None) => KeyCode::Char(c),
                _ => return Err(format!("unknown key <{name}>")),
            }
        }
    };
    Ok(Key { code, ctrl, alt })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Down,
    Up,
    First,
    Last,
    HalfPageDown,
    HalfPageUp,
    PageDown,
    PageUp,
    PreviewDown,
    PreviewUp,
    Enter,
    Leave,
    Search,
    SearchNext,
    SearchPrev,
    Find,
    FindBack,
    FindRepeat,
    FindRepeatBack,
    ExPrompt,
    Help,
    OpenExternal,
    Trash,
    Yank,
    Cut,
    Visual,
    ToggleSelect,
    Paste,
    PasteInto,
    Rename,
    NewEntry,
    Undo,
    Redo,
    SetMark,
    JumpMark,
    JumpBack,
    JumpForward,
    ToggleHidden,
    Quit,
    Abort,
}

/// Commands that act on the entries a motion covers, like vim's `d` and `y`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    Trash,
    Yank,
    Cut,
}

impl Command {
    pub fn operator(self) -> Option<Operator> {
        match self {
            Command::Trash => Some(Operator::Trash),
            Command::Yank => Some(Operator::Yank),
            Command::Cut => Some(Operator::Cut),
            _ => None,
        }
    }

    /// Commands that only move the cursor, so an operator can take them as its range.
    pub fn is_motion(self) -> bool {
        matches!(
            self,
            Command::Down
                | Command::Up
                | Command::First
                | Command::Last
                | Command::HalfPageDown
                | Command::HalfPageUp
                | Command::PageDown
                | Command::PageUp
                | Command::SearchNext
                | Command::SearchPrev
                | Command::Find
                | Command::FindBack
                | Command::FindRepeat
                | Command::FindRepeatBack
        )
    }

    /// Commands that read one more key as their argument, like vim's `f{char}`.
    pub fn wants_char(self) -> bool {
        matches!(
            self,
            Command::Find | Command::FindBack | Command::SetMark | Command::JumpMark
        )
    }
}

pub struct CommandInfo {
    pub name: &'static str,
    pub command: Command,
    pub help: &'static str,
}

const fn info(name: &'static str, command: Command, help: &'static str) -> CommandInfo {
    CommandInfo {
        name,
        command,
        help,
    }
}

pub const COMMANDS: &[CommandInfo] = &[
    info("down", Command::Down, "move down"),
    info("up", Command::Up, "move up"),
    info(
        "first",
        Command::First,
        "first entry, or entry N with a count",
    ),
    info("last", Command::Last, "last entry, or entry N with a count"),
    info(
        "half_page_down",
        Command::HalfPageDown,
        "scroll half a screen down",
    ),
    info(
        "half_page_up",
        Command::HalfPageUp,
        "scroll half a screen up",
    ),
    info("page_down", Command::PageDown, "scroll a screen down"),
    info("page_up", Command::PageUp, "scroll a screen up"),
    info(
        "preview_down",
        Command::PreviewDown,
        "scroll the file preview down",
    ),
    info(
        "preview_up",
        Command::PreviewUp,
        "scroll the file preview up",
    ),
    info(
        "enter",
        Command::Enter,
        "open the folder, edit the file right here, or develop the photo",
    ),
    info("leave", Command::Leave, "go to the parent directory"),
    info("search", Command::Search, "search names in this column"),
    info("search_next", Command::SearchNext, "next search match"),
    info("search_prev", Command::SearchPrev, "previous search match"),
    info(
        "find",
        Command::Find,
        "jump to the next name starting with a letter",
    ),
    info(
        "find_back",
        Command::FindBack,
        "jump to the previous name starting with a letter",
    ),
    info(
        "find_repeat",
        Command::FindRepeat,
        "repeat the last letter jump",
    ),
    info(
        "find_repeat_back",
        Command::FindRepeatBack,
        "repeat the last letter jump backwards",
    ),
    info("ex", Command::ExPrompt, "open the command line"),
    info("help", Command::Help, "show this help"),
    info(
        "open_external",
        Command::OpenExternal,
        "open the file in your own editor, or the desktop app for other files",
    ),
    info(
        "trash",
        Command::Trash,
        "move entries to the trash, like d{motion}; dd is this entry",
    ),
    info(
        "yank",
        Command::Yank,
        "copy entries to the register, like y{motion}; yy is this entry",
    ),
    info(
        "cut",
        Command::Cut,
        "cut entries to move them with paste, like x{motion}",
    ),
    info(
        "visual",
        Command::Visual,
        "select a range of entries with the motions",
    ),
    info(
        "select",
        Command::ToggleSelect,
        "select or unselect this entry and move down",
    ),
    info("paste", Command::Paste, "paste into this directory"),
    info(
        "paste_into",
        Command::PasteInto,
        "paste into the directory under the cursor",
    ),
    info("rename", Command::Rename, "rename this entry"),
    info(
        "new",
        Command::NewEntry,
        "create a file, or a folder when the name ends in /",
    ),
    info("undo", Command::Undo, "undo the last file operation"),
    info("redo", Command::Redo, "redo an undone file operation"),
    info(
        "mark",
        Command::SetMark,
        "set a mark, saved across sessions when uppercase",
    ),
    info(
        "jump_mark",
        Command::JumpMark,
        "jump to a mark, or '' to the previous place",
    ),
    info(
        "jump_back",
        Command::JumpBack,
        "go back in the jump history",
    ),
    info(
        "jump_forward",
        Command::JumpForward,
        "go forward in the jump history",
    ),
    info(
        "toggle_hidden",
        Command::ToggleHidden,
        "show or hide dotfiles",
    ),
    info(
        "quit",
        Command::Quit,
        "quit and leave the shell in this directory",
    ),
    info(
        "abort",
        Command::Abort,
        "quit without changing the shell's directory",
    ),
];

const DEFAULT_KEYS: &[(&str, &str)] = &[
    ("j", "down"),
    ("<down>", "down"),
    ("k", "up"),
    ("<up>", "up"),
    ("gg", "first"),
    ("<home>", "first"),
    ("G", "last"),
    ("<end>", "last"),
    ("<c-d>", "half_page_down"),
    ("<c-u>", "half_page_up"),
    ("<c-f>", "page_down"),
    ("<pagedown>", "page_down"),
    ("<c-b>", "page_up"),
    ("<pageup>", "page_up"),
    ("J", "preview_down"),
    ("<c-e>", "preview_down"),
    ("K", "preview_up"),
    ("<c-y>", "preview_up"),
    ("l", "enter"),
    ("<right>", "enter"),
    ("<cr>", "enter"),
    ("h", "leave"),
    ("<left>", "leave"),
    ("<bs>", "leave"),
    ("/", "search"),
    ("n", "search_next"),
    ("N", "search_prev"),
    ("f", "find"),
    ("F", "find_back"),
    (";", "find_repeat"),
    (",", "find_repeat_back"),
    (":", "ex"),
    ("?", "help"),
    ("i", "open_external"),
    ("d", "trash"),
    ("y", "yank"),
    ("x", "cut"),
    ("v", "visual"),
    ("<space>", "select"),
    ("p", "paste"),
    ("P", "paste_into"),
    ("r", "rename"),
    ("cw", "rename"),
    ("cc", "rename"),
    ("o", "new"),
    ("u", "undo"),
    ("<c-r>", "redo"),
    ("m", "mark"),
    ("'", "jump_mark"),
    ("`", "jump_mark"),
    ("<c-o>", "jump_back"),
    ("<tab>", "jump_forward"),
    ("zh", "toggle_hidden"),
    ("q", "quit"),
    ("<esc>", "quit"),
    ("<c-c>", "abort"),
];

pub fn command_named(name: &str) -> Option<Command> {
    COMMANDS.iter().find(|c| c.name == name).map(|c| c.command)
}

/// A set of commands the input engine can drive: explorer commands, or the editor's.
pub trait Cmd: Copy + Eq + std::fmt::Debug {
    type Op: Copy + Eq + std::fmt::Debug;
    fn operator(self) -> Option<Self::Op>;
    fn is_motion(self) -> bool;
    fn wants_char(self) -> bool;
}

impl Cmd for Command {
    type Op = Operator;

    fn operator(self) -> Option<Operator> {
        Command::operator(self)
    }

    fn is_motion(self) -> bool {
        Command::is_motion(self)
    }

    fn wants_char(self) -> bool {
        Command::wants_char(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lookup<C = Command> {
    Exact(C),
    Prefix,
    Unbound,
}

#[derive(Debug, Clone)]
pub struct Keymap<C = Command> {
    bindings: HashMap<Vec<Key>, C>,
    prefixes: HashSet<Vec<Key>>,
}

impl<C: Cmd> Keymap<C> {
    /// Builds a keymap, refusing bindings where one sequence is the start of another.
    pub fn from_bindings(bindings: HashMap<Vec<Key>, C>) -> Result<Keymap<C>, String> {
        let mut prefixes = HashSet::new();
        for seq in bindings.keys() {
            for len in 1..seq.len() {
                let prefix = seq[..len].to_vec();
                if bindings.contains_key(&prefix) {
                    return Err(format!(
                        "{} is bound, so {} can never be typed",
                        seq_label(&prefix),
                        seq_label(seq)
                    ));
                }
                prefixes.insert(prefix);
            }
        }
        Ok(Keymap { bindings, prefixes })
    }

    pub fn lookup(&self, keys: &[Key]) -> Lookup<C> {
        match self.bindings.get(keys) {
            Some(&command) => Lookup::Exact(command),
            None if self.prefixes.contains(keys) => Lookup::Prefix,
            None => Lookup::Unbound,
        }
    }
}

impl Default for Keymap {
    fn default() -> Keymap {
        Keymap::with_overrides(&BTreeMap::new()).expect("default key bindings are valid")
    }
}

impl Keymap {
    /// Defaults plus the user's `key = "command"` entries. An empty or `"none"` command unbinds.
    pub fn with_overrides(overrides: &BTreeMap<String, String>) -> Result<Keymap, String> {
        let mut bindings = HashMap::new();
        for (keys, name) in DEFAULT_KEYS {
            bindings.insert(
                Key::parse_seq(keys)?,
                command_named(name).ok_or(format!("unknown command {name}"))?,
            );
        }
        for (keys, name) in overrides {
            let seq = Key::parse_seq(keys).map_err(|e| format!("keys.{keys}: {e}"))?;
            if name.is_empty() || name == "none" {
                bindings.remove(&seq);
            } else {
                let command = command_named(name)
                    .ok_or_else(|| format!("keys.{keys}: unknown command {name:?}"))?;
                bindings.insert(seq, command);
            }
        }
        Keymap::from_bindings(bindings)
    }

    /// Every command with the keys bound to it, in registry order. Unbound commands are left out.
    pub fn describe(&self) -> Vec<(String, &'static str)> {
        COMMANDS
            .iter()
            .filter_map(|info| {
                let mut keys: Vec<String> = self
                    .bindings
                    .iter()
                    .filter(|(_, c)| **c == info.command)
                    .map(|(seq, _)| seq_label(seq))
                    .collect();
                keys.sort();
                (!keys.is_empty()).then(|| (keys.join("  "), info.help))
            })
            .collect()
    }
}

pub fn seq_label(seq: &[Key]) -> String {
    seq.iter().map(|k| k.label()).collect()
}

/// What one key press did to the half-typed command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fed<C: Cmd = Command> {
    Pending,
    Run {
        command: C,
        count: Option<usize>,
        arg: Option<char>,
    },
    /// An operator with the motion that says what it covers. Without a motion the
    /// operator was doubled (`dd`) and covers `count` lines from the cursor.
    Operate {
        operator: C::Op,
        motion: Option<(C, Option<char>)>,
        count: Option<usize>,
    },
    Unbound,
}

const MAX_COUNT: usize = 99_999_999;

#[derive(Debug, Clone)]
struct PendingOp<O> {
    operator: O,
    count: Option<usize>,
    label: String,
}

/// Count prefix, multi-key sequences, character arguments and operators waiting for a motion.
#[derive(Debug)]
pub struct InputState<C: Cmd = Command> {
    count: Option<usize>,
    keys: Vec<Key>,
    awaiting: Option<C>,
    operator: Option<PendingOp<C::Op>>,
}

impl<C: Cmd> Default for InputState<C> {
    fn default() -> Self {
        InputState {
            count: None,
            keys: Vec::new(),
            awaiting: None,
            operator: None,
        }
    }
}

fn combine(before: Option<usize>, after: Option<usize>) -> Option<usize> {
    match (before, after) {
        (None, None) => None,
        (a, b) => Some((a.unwrap_or(1).saturating_mul(b.unwrap_or(1))).min(MAX_COUNT)),
    }
}

impl<C: Cmd> InputState<C> {
    /// `visual` makes operators act at once on the selected range instead of waiting for a motion.
    pub fn feed(&mut self, key: Key, keymap: &Keymap<C>, visual: bool) -> Fed<C> {
        let cancels = key.code == KeyCode::Esc && !key.ctrl && !key.alt;
        if self.is_pending() && cancels {
            self.reset();
            return Fed::Unbound;
        }
        if let Some(command) = self.awaiting {
            let count = self.count;
            let pending = self.operator.take();
            self.reset();
            return match key.typed_char() {
                Some(c) => match pending {
                    Some(op) => Fed::Operate {
                        operator: op.operator,
                        motion: Some((command, Some(c))),
                        count: combine(op.count, count),
                    },
                    None => Fed::Run {
                        command,
                        count,
                        arg: Some(c),
                    },
                },
                None => Fed::Unbound,
            };
        }
        if self.keys.is_empty()
            && let Some(digit) = key.typed_char().and_then(|c| c.to_digit(10))
            && (digit != 0 || self.count.is_some())
        {
            let count = self.count.unwrap_or(0) * 10 + digit as usize;
            self.count = Some(count.min(MAX_COUNT));
            return Fed::Pending;
        }
        self.keys.push(key);
        match keymap.lookup(&self.keys) {
            Lookup::Prefix => Fed::Pending,
            Lookup::Unbound => {
                self.reset();
                Fed::Unbound
            }
            Lookup::Exact(command) => self.exact(command, visual),
        }
    }

    fn exact(&mut self, command: C, visual: bool) -> Fed<C> {
        if let Some(pending) = self.operator.clone() {
            if command.operator() == Some(pending.operator) {
                let count = combine(pending.count, self.count);
                self.reset();
                return Fed::Operate {
                    operator: pending.operator,
                    motion: None,
                    count,
                };
            }
            if !command.is_motion() {
                self.reset();
                return Fed::Unbound;
            }
            if command.wants_char() {
                self.awaiting = Some(command);
                return Fed::Pending;
            }
            let count = combine(pending.count, self.count);
            self.reset();
            return Fed::Operate {
                operator: pending.operator,
                motion: Some((command, None)),
                count,
            };
        }
        if !visual && let Some(operator) = command.operator() {
            self.operator = Some(PendingOp {
                operator,
                count: self.count.take(),
                label: seq_label(&self.keys),
            });
            self.keys.clear();
            return Fed::Pending;
        }
        if command.wants_char() {
            self.awaiting = Some(command);
            return Fed::Pending;
        }
        let count = self.count;
        self.reset();
        Fed::Run {
            command,
            count,
            arg: None,
        }
    }

    /// Whether an operator such as `d` is waiting for its motion.
    pub fn has_operator(&self) -> bool {
        self.operator.is_some()
    }

    pub fn is_pending(&self) -> bool {
        self.count.is_some()
            || !self.keys.is_empty()
            || self.awaiting.is_some()
            || self.operator.is_some()
    }

    /// The half-typed command as vim's showcmd would print it, such as `12g` or `2d3`.
    pub fn display(&self) -> String {
        let count = self.count.map(|c| c.to_string()).unwrap_or_default();
        let operator = self.operator.as_ref().map_or(String::new(), |op| {
            format!(
                "{}{}",
                op.count.map(|c| c.to_string()).unwrap_or_default(),
                op.label
            )
        });
        format!("{operator}{count}{}", seq_label(&self.keys))
    }

    pub fn reset(&mut self) {
        *self = InputState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(keys: &str) -> (Vec<Fed>, InputState) {
        let keymap = Keymap::default();
        let mut state = InputState::default();
        let fed = Key::parse_seq(keys)
            .unwrap()
            .into_iter()
            .map(|k| state.feed(k, &keymap, false))
            .collect();
        (fed, state)
    }

    fn last(keys: &str) -> Fed {
        *feed_all(keys).0.last().unwrap()
    }

    fn run(command: Command, count: Option<usize>, arg: Option<char>) -> Fed {
        Fed::Run {
            command,
            count,
            arg,
        }
    }

    #[test]
    fn notation_parses_plain_named_and_modified_keys() {
        let keys = Key::parse_seq("g<c-D><space><CR><a-x><lt>").unwrap();
        assert_eq!(keys[0], Key::plain(KeyCode::Char('g')));
        assert_eq!(
            keys[1],
            Key {
                code: KeyCode::Char('d'),
                ctrl: true,
                alt: false
            }
        );
        assert_eq!(keys[2], Key::plain(KeyCode::Char(' ')));
        assert_eq!(keys[3], Key::plain(KeyCode::Enter));
        assert_eq!(
            keys[4],
            Key {
                code: KeyCode::Char('x'),
                ctrl: false,
                alt: true
            }
        );
        assert_eq!(keys[5], Key::plain(KeyCode::Char('<')));
        assert!(Key::parse_seq("<bogus>").is_err());
        assert!(Key::parse_seq("<c-d").is_err());
        assert!(Key::parse_seq("").is_err());
    }

    #[test]
    fn labels_round_trip_through_the_parser() {
        for text in [
            "j",
            "G",
            "<c-d>",
            "<space>",
            "<cr>",
            "<esc>",
            "<lt>",
            "<a-x>",
            "<pagedown>",
        ] {
            let key = Key::parse_seq(text).unwrap()[0];
            assert_eq!(key.label(), text);
        }
    }

    #[test]
    fn single_keys_run_immediately() {
        assert_eq!(last("j"), run(Command::Down, None, None));
        assert_eq!(last("<c-d>"), run(Command::HalfPageDown, None, None));
        assert_eq!(last("G"), run(Command::Last, None, None));
    }

    #[test]
    fn multi_key_sequences_wait_for_the_rest() {
        let (fed, state) = feed_all("g");
        assert_eq!(fed, [Fed::Pending]);
        assert_eq!(state.display(), "g");
        assert_eq!(last("gg"), run(Command::First, None, None));
    }

    #[test]
    fn a_count_prefixes_the_command_and_zero_only_extends_it() {
        assert_eq!(last("5j"), run(Command::Down, Some(5), None));
        assert_eq!(last("12G"), run(Command::Last, Some(12), None));
        assert_eq!(last("10j"), run(Command::Down, Some(10), None));
        assert_eq!(last("3gg"), run(Command::First, Some(3), None));
        assert_eq!(last("0"), Fed::Unbound, "a lone zero is not a count");
    }

    #[test]
    fn huge_counts_saturate_instead_of_overflowing() {
        let keys = "9".repeat(30) + "j";
        assert_eq!(last(&keys), run(Command::Down, Some(MAX_COUNT), None));
    }

    #[test]
    fn find_reads_one_more_character_as_its_argument() {
        assert_eq!(last("fx"), run(Command::Find, None, Some('x')));
        assert_eq!(last("2Fa"), run(Command::FindBack, Some(2), Some('a')));
        let (fed, state) = feed_all("f");
        assert_eq!(fed, [Fed::Pending]);
        assert_eq!(state.display(), "f");
    }

    #[test]
    fn marks_and_jumps_read_a_character_argument() {
        assert_eq!(last("ma"), run(Command::SetMark, None, Some('a')));
        assert_eq!(last("'A"), run(Command::JumpMark, None, Some('A')));
        assert_eq!(last("''"), run(Command::JumpMark, None, Some('\'')));
        assert_eq!(last("<c-o>"), run(Command::JumpBack, None, None));
        assert_eq!(last("<tab>"), run(Command::JumpForward, None, None));
        assert_eq!(last("zh"), run(Command::ToggleHidden, None, None));
    }

    #[test]
    fn find_takes_command_letters_literally() {
        assert_eq!(last("fq"), run(Command::Find, None, Some('q')));
        assert_eq!(last("fj"), run(Command::Find, None, Some('j')));
    }

    #[test]
    fn escape_cancels_a_half_typed_command_instead_of_quitting() {
        for keys in ["3<esc>", "g<esc>", "f<esc>"] {
            let (fed, state) = feed_all(keys);
            assert_eq!(*fed.last().unwrap(), Fed::Unbound, "{keys}");
            assert!(!state.is_pending(), "{keys}");
        }
        assert_eq!(last("<esc>"), run(Command::Quit, None, None));
    }

    #[test]
    fn unbound_keys_clear_the_pending_state() {
        let (fed, state) = feed_all("gz");
        assert_eq!(fed, [Fed::Pending, Fed::Unbound]);
        assert!(!state.is_pending());
        assert_eq!(last("gzj"), run(Command::Down, None, None));
    }

    #[test]
    fn user_bindings_override_add_and_remove() {
        let overrides = BTreeMap::from([
            ("j".to_string(), "up".to_string()),
            ("<space>".to_string(), "enter".to_string()),
            ("q".to_string(), "none".to_string()),
        ]);
        let keymap = Keymap::with_overrides(&overrides).unwrap();
        let one = |s: &str| keymap.lookup(&Key::parse_seq(s).unwrap());
        assert_eq!(one("j"), Lookup::Exact(Command::Up));
        assert_eq!(one("<space>"), Lookup::Exact(Command::Enter));
        assert_eq!(one("q"), Lookup::Unbound);
        assert_eq!(one("k"), Lookup::Exact(Command::Up));
    }

    #[test]
    fn bad_config_is_reported_with_the_offending_key() {
        let bad = |k: &str, v: &str| {
            Keymap::with_overrides(&BTreeMap::from([(k.to_string(), v.to_string())])).unwrap_err()
        };
        assert!(bad("x", "explode").contains("unknown command"));
        assert!(bad("<nope>", "up").contains("unknown key"));
        let conflict = bad("ggx", "up");
        assert!(conflict.contains("gg is bound"), "{conflict}");
        let shadow = bad("g", "up");
        assert!(shadow.contains("g is bound"), "{shadow}");
    }

    fn operate(
        operator: Operator,
        motion: Option<(Command, Option<char>)>,
        count: Option<usize>,
    ) -> Fed {
        Fed::Operate {
            operator,
            motion,
            count,
        }
    }

    #[test]
    fn a_doubled_operator_covers_the_current_entry_or_a_count_of_them() {
        assert_eq!(last("dd"), operate(Operator::Trash, None, None));
        assert_eq!(last("yy"), operate(Operator::Yank, None, None));
        assert_eq!(last("xx"), operate(Operator::Cut, None, None));
        assert_eq!(last("3dd"), operate(Operator::Trash, None, Some(3)));
        assert_eq!(last("d3d"), operate(Operator::Trash, None, Some(3)));
        assert_eq!(last("2d3d"), operate(Operator::Trash, None, Some(6)));
    }

    #[test]
    fn an_operator_waits_for_a_motion_and_combines_the_counts() {
        let (fed, state) = feed_all("d");
        assert_eq!(fed, [Fed::Pending]);
        assert_eq!(state.display(), "d");
        assert_eq!(
            last("dj"),
            operate(Operator::Trash, Some((Command::Down, None)), None)
        );
        assert_eq!(
            last("d3j"),
            operate(Operator::Trash, Some((Command::Down, None)), Some(3))
        );
        assert_eq!(
            last("2d3j"),
            operate(Operator::Trash, Some((Command::Down, None)), Some(6))
        );
        assert_eq!(
            last("yG"),
            operate(Operator::Yank, Some((Command::Last, None)), None)
        );
        assert_eq!(
            last("dgg"),
            operate(Operator::Trash, Some((Command::First, None)), None)
        );
        assert_eq!(
            last("y5G"),
            operate(Operator::Yank, Some((Command::Last, None)), Some(5))
        );
    }

    #[test]
    fn an_operator_can_take_a_letter_jump_as_its_motion() {
        assert_eq!(
            last("dfx"),
            operate(Operator::Trash, Some((Command::Find, Some('x'))), None)
        );
        assert_eq!(
            last("2yFa"),
            operate(
                Operator::Yank,
                Some((Command::FindBack, Some('a'))),
                Some(2)
            )
        );
        let (_, state) = feed_all("df");
        assert_eq!(state.display(), "df");
    }

    #[test]
    fn a_command_that_is_not_a_motion_cancels_the_operator() {
        for keys in ["dl", "dq", "dp", "dv", "d:", "d?"] {
            let (fed, state) = feed_all(keys);
            assert_eq!(*fed.last().unwrap(), Fed::Unbound, "{keys}");
            assert!(!state.is_pending(), "{keys}");
        }
        assert_eq!(last("dlj"), run(Command::Down, None, None));
    }

    #[test]
    fn escape_cancels_a_pending_operator_instead_of_quitting() {
        for keys in ["d<esc>", "3d<esc>", "d2<esc>", "df<esc>"] {
            let (fed, state) = feed_all(keys);
            assert_eq!(*fed.last().unwrap(), Fed::Unbound, "{keys}");
            assert!(!state.is_pending(), "{keys}");
        }
    }

    #[test]
    fn the_pending_display_shows_counts_operator_and_motion_keys() {
        assert_eq!(feed_all("2d3").1.display(), "2d3");
        assert_eq!(feed_all("3d").1.display(), "3d");
        assert_eq!(feed_all("dg").1.display(), "dg");
    }

    #[test]
    fn in_visual_mode_operators_run_at_once() {
        let keymap = Keymap::default();
        let mut state = InputState::default();
        let d = Key::parse_seq("d").unwrap()[0];
        assert_eq!(
            state.feed(d, &keymap, true),
            run(Command::Trash, None, None)
        );
        assert!(!state.is_pending());
        let y = Key::parse_seq("3y").unwrap();
        state.feed(y[0], &keymap, true);
        assert_eq!(
            state.feed(y[1], &keymap, true),
            run(Command::Yank, Some(3), None)
        );
    }

    #[test]
    fn editing_keys_are_plain_commands() {
        assert_eq!(last("p"), run(Command::Paste, None, None));
        assert_eq!(last("P"), run(Command::PasteInto, None, None));
        assert_eq!(last("r"), run(Command::Rename, None, None));
        assert_eq!(last("cw"), run(Command::Rename, None, None));
        assert_eq!(last("cc"), run(Command::Rename, None, None));
        assert_eq!(last("o"), run(Command::NewEntry, None, None));
        assert_eq!(last("u"), run(Command::Undo, None, None));
        assert_eq!(last("<c-r>"), run(Command::Redo, None, None));
        assert_eq!(last("v"), run(Command::Visual, None, None));
        assert_eq!(last("<space>"), run(Command::ToggleSelect, None, None));
    }

    #[test]
    fn every_command_has_a_default_key_and_help_text() {
        let described = Keymap::default().describe();
        assert_eq!(described.len(), COMMANDS.len());
        assert!(
            described
                .iter()
                .all(|(keys, help)| !keys.is_empty() && !help.is_empty())
        );
        assert!(described.iter().any(|(keys, _)| keys.contains("gg")));
    }
}
