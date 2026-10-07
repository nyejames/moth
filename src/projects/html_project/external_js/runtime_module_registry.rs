//! Builder-owned registry of core JS runtime modules.
//!
//! WHAT: tracks which JS module specifiers are allowed in `import ... from "..."`
//!       statements inside Moth JS module files, and holds their authored source
//!       for later emission by the HTML builder.
//! WHY: the HTML builder owns the set of core runtime modules; the parser only validates
//!      that JS imports match the registered set. Keeping the registry in `external_js`
//!      lets the parser and builder share one definition while staying within the
//!      HTML-project boundary.

use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use std::fmt::Write as _;

/// A single builder-registered core JS runtime module.
///
/// WHAT: describes a module specifier and its authored JS source that the builder
///       will emit as a runtime asset.
/// WHY: keeps the runtime module contract (specifier + source) separate from
///      the registry that collects them, so v1 and later versions share one shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreJsRuntimeModule {
    pub specifier: String,
    pub source: String,
    /// Symbol names exported by this runtime module that are valid import targets.
    pub exported_names: Vec<String>,
}

impl CoreJsRuntimeModule {
    /// Creates a v1 `@moth/runtime` module with result helpers and builtin error codes.
    ///
    /// WHAT: provides `mothOk` and `mothErr` for fallible JS module functions, plus one
    ///       numeric export per entry in `RUNTIME_ERROR_CODE_EXPORTS`.
    /// WHY: the runtime wrapper contract must match the glue the backend generates, and
    ///      static package assets can only name compiler-owned error codes through imports.
    ///      Generating the export list and declarations from one table keeps the parser's
    ///      allowlist, the first-party audit and the emitted values in agreement.
    pub fn moth_runtime_v1() -> Self {
        let mut source = MOTH_RUNTIME_RESULT_HELPERS_SOURCE.to_owned();
        let mut exported_names = vec!["mothOk".to_owned(), "mothErr".to_owned()];

        for (export_name, code) in RUNTIME_ERROR_CODE_EXPORTS {
            writeln!(
                &mut source,
                "export const {export_name} = {};",
                code.as_u32()
            )
            .expect("writing runtime module source into a String cannot fail");
            exported_names.push(export_name.to_owned());
        }

        Self {
            specifier: "@moth/runtime".to_owned(),
            source,
            exported_names,
        }
    }
}

/// Builtin error codes that annotated JS assets may import from `@moth/runtime`.
///
/// WHAT: the only association between a runtime export name and its `BuiltinErrorCode`.
///       Numeric values come from the enum when the module source is generated.
/// WHY: package assets are static files, so they reach compiler-owned codes by name instead
///      of copying numbers that could drift from the canonical enum.
pub(crate) const RUNTIME_ERROR_CODE_EXPORTS: [(&str, BuiltinErrorCode); 5] = [
    ("MOTH_ERROR_UNSUPPORTED", BuiltinErrorCode::Unsupported),
    (
        "MOTH_ERROR_HOST_INVALID_ARGUMENT",
        BuiltinErrorCode::HostInvalidArgument,
    ),
    (
        "MOTH_ERROR_HOST_RESOURCE_NOT_FOUND",
        BuiltinErrorCode::HostResourceNotFound,
    ),
    (
        "MOTH_ERROR_HOST_RESOURCE_UNAVAILABLE",
        BuiltinErrorCode::HostResourceUnavailable,
    ),
    (
        "MOTH_ERROR_HOST_OPERATION_FAILED",
        BuiltinErrorCode::HostOperationFailed,
    ),
];

/// v1 result helpers for `@moth/runtime`.
///
/// `mothOk(value)` produces a success wrapper.
/// `mothOk()` with no argument also succeeds; `value` is `undefined`.
/// `mothErr(code, message)` produces an error wrapper with enough shape for later
/// dev/debug glue validation.
const MOTH_RUNTIME_RESULT_HELPERS_SOURCE: &str = r#"export function mothOk(value) {
    return { ok: true, value: value };
}

export function mothErr(code, message) {
    return { ok: false, error: { code, message } };
}

"#;

/// Builder-owned registry of allowed core JS runtime module imports.
///
/// WHAT: holds the set of registered core JS runtime modules and their sources.
/// WHY: v1 registers only `@moth/runtime`, but the shape is kept extensible
///      so future phases can register additional core runtime modules without
///      changing the scanner logic.
pub struct RuntimeModuleRegistry {
    modules: Vec<CoreJsRuntimeModule>,
}

impl RuntimeModuleRegistry {
    /// Creates an empty registry with no registered modules for parser/registry tests.
    #[cfg(test)]
    pub fn empty() -> Self {
        Self {
            modules: Vec::new(),
        }
    }

    /// Creates the v1 registry containing only `@moth/runtime`.
    pub fn v1() -> Self {
        Self {
            modules: vec![CoreJsRuntimeModule::moth_runtime_v1()],
        }
    }

    /// Returns true if the given module specifier is registered.
    pub fn is_registered(&self, specifier: &str) -> bool {
        self.modules.iter().any(|m| m.specifier == specifier)
    }

    /// Returns the list of registered runtime modules.
    pub fn registered_modules(&self) -> &[CoreJsRuntimeModule] {
        &self.modules
    }

    /// Returns true if the given name is exported by the registered module specifier.
    pub fn is_exported_name(&self, specifier: &str, name: &str) -> bool {
        self.modules
            .iter()
            .find(|m| m.specifier == specifier)
            .is_some_and(|m| m.exported_names.iter().any(|n| n == name))
    }

    /// Returns the JS source for a registered module specifier, if any.
    pub fn module_source(&self, specifier: &str) -> Option<&str> {
        self.modules
            .iter()
            .find(|m| m.specifier == specifier)
            .map(|m| m.source.as_str())
    }
}
