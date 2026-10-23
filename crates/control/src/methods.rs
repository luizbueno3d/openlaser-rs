//! The exact control-method table (§4.5). `control_methods_table_is_exact` pins it. WP0 stub: empty.

pub const METHODS: &[&str] = &[];

pub fn is_read_only(method: &str) -> bool {
    let _ = method;
    false
}
