//! Keyboard shortcuts (UI §10, CAT §10): one table built from the menu catalogue drives menus, the native menu's
//! labels and the dispatcher.
//!
//! The parser is ported from PhotoCraft `crates/ui-egui/src/shortcuts.rs` (MIT OR Apache-2.0): `Cmd+Shift+N`
//! strings are parsed strictly, `Cmd` is ⌘ on macOS and Ctrl elsewhere, `Ctrl` is the Control key, `Alt` is ⌥.
//! A malformed binding is rejected rather than reinterpreted.
//!
//! **Esc is reserved** (S18): it is STOP while a machine is active (consumed in `raw_input_hook` before any widget),
//! and otherwise clears the selection. It is never a table entry and cannot be rebound.

use egui::{Key, KeyboardShortcut, Modifiers};

use crate::menu_catalog::{self, CATALOG};

/// Parse `Cmd+Shift+N`. `None` for unknown keys, duplicate modifiers, extra components or Escape.
pub fn parse(s: &str) -> Option<KeyboardShortcut> {
    if s.is_empty() {
        return None;
    }
    let (prefix, key_name) = s.rsplit_once('+').unwrap_or(("", s));
    let mut mods = Modifiers::NONE;
    if !prefix.is_empty() {
        for part in prefix.split('+') {
            match part {
                "Cmd" if !mods.command => mods |= Modifiers::COMMAND,
                "Shift" if !mods.shift => mods |= Modifiers::SHIFT,
                "Alt" if !mods.alt => mods |= Modifiers::ALT,
                "Ctrl" if !mods.ctrl => mods |= Modifiers::CTRL,
                _ => return None,
            }
        }
    }
    let key = Key::from_name(key_name)?;
    // Esc is STOP while active and never a binding (S18).
    if key == Key::Escape {
        return None;
    }
    Some(KeyboardShortcut::new(mods, key))
}

/// Human-readable form: `Cmd+Shift+N` → `⌘⇧N` on macOS, `Ctrl+Shift+N` elsewhere.
pub fn pretty(s: &str) -> String {
    let mac = cfg!(target_os = "macos");
    s.split('+')
        .map(|p| match (p, mac) {
            ("Cmd", true) => "⌘".to_owned(),
            ("Cmd", false) => "Ctrl".to_owned(),
            ("Ctrl", true) => "⌃".to_owned(),
            ("Shift", true) => "⇧".to_owned(),
            ("Alt", true) => "⌥".to_owned(),
            ("Delete", true) => "⌫".to_owned(),
            ("Equals", _) => "=".to_owned(),
            ("Minus", _) => "-".to_owned(),
            ("Plus", _) => "+".to_owned(),
            ("Comma", _) => ",".to_owned(),
            ("Period", _) => ".".to_owned(),
            ("Slash", _) => "/".to_owned(),
            ("Backtick", _) => "`".to_owned(),
            (other, _) => other.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(if mac { "" } else { "+" })
}

/// Does a key press match a shortcut? Modifiers match exactly (with ⌘ = Ctrl off the Mac); Delete also matches
/// Backspace (the Mac "delete" key); `=` also matches `+`.
pub fn key_matches(sc: &KeyboardShortcut, key: Key, mods: Modifiers) -> bool {
    let k = sc.logical_key;
    let direct = key == k || (k == Key::Delete && key == Key::Backspace) || (k == Key::Equals && key == Key::Plus);
    direct && mods.matches_exact(sc.modifiers)
}

/// One binding of the dispatch table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    pub shortcut: KeyboardShortcut,
    pub id: &'static str,
    /// Plain keys (no ⌘/Ctrl/Alt) act only while no text field has focus, as LightBurn's canvas letters do.
    pub canvas_only: bool,
}

/// Single-letter canvas keys LightBurn has besides the menu shortcuts (CAT §3.4: `H`, `V` flip).
const CANVAS_KEYS: &[(&str, &str)] = &[("H", "arrange.flipH"), ("V", "arrange.flipV")];

/// The dispatch table: every catalogue shortcut (hardware rows never have one), then the canvas letters.
pub fn table() -> Vec<Binding> {
    let mut out = Vec::new();
    for row in CATALOG {
        if menu_catalog::is_separator(row) || menu_catalog::is_hw_row(row) {
            continue;
        }
        let Some(sc) = row.shortcut.and_then(parse) else { continue };
        if out.iter().any(|b: &Binding| b.id == row.id) {
            continue;
        }
        out.push(Binding { shortcut: sc, id: row.id, canvas_only: is_plain(&sc) });
    }
    for (key, id) in CANVAS_KEYS {
        if let Some(sc) = parse(key) {
            out.push(Binding { shortcut: sc, id, canvas_only: true });
        }
    }
    out
}

fn is_plain(sc: &KeyboardShortcut) -> bool {
    !sc.modifiers.command && !sc.modifiers.ctrl && !sc.modifiers.alt && !sc.modifiers.mac_cmd
}

/// The catalogue's default shortcut of a command, if any.
pub fn default_shortcut(id: &str) -> Option<&'static str> {
    menu_catalog::find(id).and_then(|r| r.shortcut)
}

/// The command ids whose shortcuts were pressed this frame, consuming those key events. Plain-key bindings are
/// skipped while a widget has keyboard focus (`text_focus`).
pub fn take_pressed(input: &mut egui::InputState, table: &[Binding], text_focus: bool) -> Vec<&'static str> {
    let mut fired = Vec::new();
    let mut keep = Vec::with_capacity(input.events.len());
    for ev in std::mem::take(&mut input.events) {
        let hit = if let egui::Event::Key { key, pressed: true, modifiers, .. } = &ev {
            table.iter().find(|b| (!b.canvas_only || !text_focus) && key_matches(&b.shortcut, *key, *modifiers)).map(|b| b.id)
        } else {
            None
        };
        match hit {
            Some(id) => fired.push(id),
            None => keep.push(ev),
        }
    }
    input.events = keep;
    fired
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_is_strict() {
        assert_eq!(parse("Cmd+N"), Some(KeyboardShortcut::new(Modifiers::COMMAND, Key::N)));
        assert_eq!(parse("Cmd+Shift+S"), Some(KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::S)));
        assert_eq!(parse("Alt+Shift+L"), Some(KeyboardShortcut::new(Modifiers::ALT | Modifiers::SHIFT, Key::L)));
        assert_eq!(parse("F1"), Some(KeyboardShortcut::new(Modifiers::NONE, Key::F1)));
        assert!(parse("Cmd+Cmd+N").is_none(), "duplicate modifier");
        assert!(parse("Hyper+N").is_none(), "unknown modifier");
        assert!(parse("Cmd+Nope").is_none(), "unknown key");
        assert!(parse("Escape").is_none(), "Esc is reserved for STOP");
        assert!(parse("Cmd+Escape").is_none(), "Esc is reserved for STOP");
        assert!(parse("").is_none());
    }

    #[test]
    fn every_catalogue_shortcut_parses_and_none_is_doubled() {
        let mut seen: Vec<(KeyboardShortcut, &str)> = Vec::new();
        for row in CATALOG.iter().filter(|r| !menu_catalog::is_separator(r)) {
            let Some(s) = row.shortcut else { continue };
            let sc = parse(s);
            assert!(sc.is_some(), "{} has an unparseable shortcut {s}", row.id);
            if let Some(sc) = sc {
                if let Some((_, other)) = seen.iter().find(|(o, _)| *o == sc) {
                    panic!("{s} is bound to both {other} and {}", row.id);
                }
                seen.push((sc, row.id));
            }
        }
    }

    #[test]
    fn delete_matches_backspace_and_modifiers_match_exactly() {
        let del = parse("Delete").unwrap();
        assert!(key_matches(&del, Key::Backspace, Modifiers::NONE));
        let save = parse("Cmd+S").unwrap();
        assert!(key_matches(&save, Key::S, Modifiers::COMMAND));
        assert!(!key_matches(&save, Key::S, Modifiers::COMMAND | Modifiers::SHIFT));
    }

    #[test]
    fn table_has_no_escape_and_no_hardware_row() {
        for b in table() {
            assert_ne!(b.shortcut.logical_key, Key::Escape);
            assert!(!b.id.starts_with(menu_catalog::HW_PREFIX));
        }
    }
}
