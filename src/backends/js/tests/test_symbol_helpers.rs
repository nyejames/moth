//! Shared test helpers for verifying deterministic symbol names in JS output.
//!
//! WHAT: mirrors the dev-mode naming logic in `JsEmitter::build_symbol_raw`.
//! WHY: multiple test modules need to predict emitted symbol names without duplicating logic.

pub(crate) fn sanitize_hint(raw: &str) -> String {
    let mut result = String::new();

    for ch in raw.chars() {
        if ch == '_' || ch == '$' || ch.is_ascii_alphanumeric() {
            result.push(ch);
        } else {
            result.push('_');
        }
    }

    if result.is_empty() {
        String::from("value")
    } else {
        result
    }
}

pub(crate) fn expected_dev_function_name(leaf: &str, id: u32) -> String {
    format!("moth_{}_fn{}", sanitize_hint(leaf), id)
}

pub(crate) fn expected_dev_local_name(leaf: &str, id: u32) -> String {
    format!("moth_{}_l{}", sanitize_hint(leaf), id)
}

pub(crate) fn expected_dev_field_name(leaf: &str, _id: u32) -> String {
    let encoded = leaf
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("moth_field_{encoded}")
}
