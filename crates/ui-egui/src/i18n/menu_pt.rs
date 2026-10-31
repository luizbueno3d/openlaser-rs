//! Portuguese (Brazil) for the menu titles and every label in `menu_catalog`, keyed by the English label.

pub(super) fn get(en: &str) -> Option<&'static str> {
    Some(match en {
        "English" => "English",
        "Português (Brasil)" => "Português (Brasil)",
        _ => return None,
    })
}
