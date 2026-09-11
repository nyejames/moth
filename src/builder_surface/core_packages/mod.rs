//! Core package registrations.
//!
//! WHAT: registers the builtin core packages that builders may provide.
//! WHY: keeps package definitions in one place so frontend and backend
//! can reference the same canonical metadata.

mod collections;
mod io;
mod math;
mod prelude;
mod random;
mod text;
mod time;

use crate::compiler_frontend::external_packages::{ExternalJsLowering, ExternalPackageRegistry};

/// Optional core packages that builders may expose explicitly.
///
/// WHAT: these are compiler-known package identities, but not mandatory builder surface.
/// WHY: Stage 0 can report "unsupported by builder" for these paths instead of treating them
/// as missing source files.
pub const OPTIONAL_CORE_PACKAGE_PATHS: &[&str] =
    &["@core/math", "@core/text", "@core/random", "@core/time"];

pub use collections::register_core_collections_package;
pub use io::register_core_io_package;
pub use math::register_core_math_package;
pub use prelude::register_core_prelude;
pub use random::register_core_random_package;
pub use text::register_core_text_package;
pub use time::register_core_time_package;

/// Registers the optional Core binding packages offered by the HTML builder.
pub(crate) fn register_optional_core_packages(registry: &mut ExternalPackageRegistry) {
    register_core_math_package(registry);
    register_core_text_package(registry);
    register_core_random_package(registry);
    register_core_time_package(registry);
}

/// Inline JavaScript lowering templates from Core binding packages.
///
/// WHAT: names each `InlineExpression` by package path and symbol path after
///       constructing the mandatory builtin Core registry and registering the
///       optional packages the HTML builder exposes.
/// WHY: first-party validation must inspect the emitted templates without a
///      second copy of the lowering strings or a frontend harvest API.
pub(crate) fn core_javascript_inline_expressions() -> Vec<(String, String)> {
    let mut registry = ExternalPackageRegistry::new();
    register_optional_core_packages(&mut registry);

    let mut expressions = Vec::new();
    for (id, definition) in registry.functions() {
        let Some(ExternalJsLowering::InlineExpression(source)) = definition.lowerings.js.as_ref()
        else {
            continue;
        };

        let package = registry
            .resolve_function_package(id)
            .expect("core function must have a registered package path");
        let symbol = registry
            .resolve_function_symbol_path(id)
            .expect("core function must have a registered symbol path")
            .to_string();
        expressions.push((format!("{package}::{symbol}"), source.clone()));
    }

    expressions.sort_by(|left, right| left.0.cmp(&right.0));
    expressions
}
