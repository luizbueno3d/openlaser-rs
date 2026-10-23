//! Every LightBurn menu item from CAT §3, in LightBurn order, "---" separators, ids converted to camelCase.
//! WP0 stub: empty (WP8 fills it).

pub struct Row {
    pub path: &'static [&'static str],
    pub label: &'static str,
    pub shortcut: Option<&'static str>,
    pub id: &'static str,
}

pub static CATALOG: &[Row] = &[];
