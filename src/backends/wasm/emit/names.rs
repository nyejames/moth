//! Optional custom name-section emission.
//!
//! The current emitter keeps this lightweight: it emits a valid `name` custom-section envelope so
//! debug tooling can detect that names were intentionally requested.

use wasm_encoder::CustomSection;

pub(crate) fn build_name_custom_section() -> CustomSection<'static> {
    // WHAT: emit a valid `name` custom-section envelope.
    // WHY: debug tooling detects a requested name section from the envelope alone; function and
    // local name subsections are not emitted.
    CustomSection {
        name: "name".into(),
        data: Vec::<u8>::new().into(),
    }
}
