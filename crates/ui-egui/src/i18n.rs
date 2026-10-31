//! Interface language: English or Brazilian Portuguese (Luiz, 2026-10-31), chosen in Settings ▸ Language and saved.
//!
//! Every visible string is written as an English/Portuguese pair at its call site, `t("Start", "Iniciar")`, or
//! `tf!("{n} objects", "{n} objetos")` when it has values, so a string can never lose its translation. Tables cover
//! the two places whose English is data: the menu catalogue ([`menu`]) and refusal codes ([`refusal`]).
//!
//! What stays English in both languages: automation labels (`track`/`labeled` names, which agents and tests use),
//! command ids, refusal codes, file formats, and the word `ARM` typed to confirm a job.
//!
//! The language is per thread: the UI thread sets it from the saved settings, and parallel tests cannot see each
//! other's choice.

use std::cell::Cell;

use openlaser_types::{Refusal, SectionMode};
use serde::{Deserialize, Serialize};

mod menu_pt;
mod refusal_pt;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Language {
    #[default]
    English,
    /// Português (Brasil).
    Portuguese,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::English, Language::Portuguese];

    /// The language's own name, as the selector shows it.
    pub fn native_name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::Portuguese => "Português (Brasil)",
        }
    }
}

thread_local! {
    static LANGUAGE: Cell<Language> = const { Cell::new(Language::English) };
}

pub fn set_language(language: Language) {
    LANGUAGE.with(|l| l.set(language));
}

pub fn language() -> Language {
    LANGUAGE.with(Cell::get)
}

pub fn is_pt() -> bool {
    language() == Language::Portuguese
}

/// The visible text in the current language.
pub fn t(en: &'static str, pt: &'static str) -> &'static str {
    if is_pt() { pt } else { en }
}

/// `format!` in the current language: `tf!("{n} objects", "{n} objetos", n = count)`.
#[macro_export]
macro_rules! tf {
    ($en:literal, $pt:literal $(, $($arg:tt)*)?) => {
        if $crate::i18n::is_pt() { format!($pt $(, $($arg)*)?) } else { format!($en $(, $($arg)*)?) }
    };
}

/// A menu title or menu-catalogue label in the current language. Unknown labels stay English.
pub fn menu(en: &'static str) -> &'static str {
    if is_pt() { menu_pt::get(en).unwrap_or(en) } else { en }
}

/// A section's cut mode (Line, Fill, Image) as a word.
pub fn section_mode(mode: SectionMode) -> &'static str {
    match mode {
        SectionMode::Line => t("Line", "Linha"),
        SectionMode::Fill => t("Fill", "Preenchimento"),
        SectionMode::Image => t("Image", "Imagem"),
    }
}

/// A machine checklist item for display. The built-in profiles' items have Portuguese; items a user wrote are shown
/// as written. The stored item never changes: it is part of what the ARM confirmation covers.
pub fn checklist(item: &str) -> &str {
    if !is_pt() {
        return item;
    }
    match item {
        "Machine area clear and material secured" => "Área da máquina livre e material preso",
        "Key switch and E-stop within reach" => "Chave e botão de emergência ao alcance",
        "Laser safety glasses on for everyone in the room" => "Óculos de proteção laser em todos na sala",
        "Material in place and focus checked" => "Material posicionado e foco conferido",
        "Fume extraction on" => "Exaustão de fumaça ligada",
        other => other,
    }
}

/// A refusal as one line, `code: message`. In Portuguese the sentence for its code comes first and the engine's
/// English detail (which names the values) follows in brackets.
pub fn refusal(r: &Refusal) -> String {
    match refusal_pt::get(r.code).filter(|_| is_pt()) {
        Some(pt) => format!("{}: {pt} [{}]", r.code.as_str(), r.message),
        None => format!("{}: {}", r.code.as_str(), r.message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_by_default_and_per_thread() {
        assert_eq!(t("Start", "Iniciar"), "Start");
        set_language(Language::Portuguese);
        assert_eq!(t("Start", "Iniciar"), "Iniciar");
        assert_eq!(tf!("{} objects", "{} objetos", 3), "3 objetos");
        let other = std::thread::spawn(|| t("Start", "Iniciar")).join().unwrap();
        assert_eq!(other, "Start");
        set_language(Language::English);
        assert_eq!(tf!("{n} objects", "{n} objetos", n = 2), "2 objects");
    }

    #[test]
    fn every_menu_label_and_refusal_code_has_portuguese() {
        set_language(Language::Portuguese);
        let mut missing: Vec<&str> = crate::menu_catalog::MENUS
            .iter()
            .copied()
            .chain(crate::menu_catalog::CATALOG.iter().filter(|r| !crate::menu_catalog::is_separator(r)).map(|r| r.label))
            .chain(crate::menu_catalog::CATALOG.iter().flat_map(|r| r.path.iter().copied()))
            .filter(|l| menu_pt::get(l).is_none())
            .collect();
        missing.sort_unstable();
        missing.dedup();
        assert!(missing.is_empty(), "menu labels without Portuguese: {missing:?}");
        for code in refusal_pt::ALL_CODES {
            assert!(refusal_pt::get(*code).is_some(), "refusal without Portuguese: {}", code.as_str());
        }
        set_language(Language::English);
    }
}
