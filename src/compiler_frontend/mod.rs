//! The compiler frontend.
//!
//! WHAT: every semantic stage between source text and validated HIR — tokenization, declaration
//!       shells, interface binding, local ordering, AST semantics, public-interface projection,
//!       HIR lowering and borrow validation — plus the named services that sequence them.
//! WHY:  local semantic compilation has one owner. Build and project code chooses one of the
//!       named services below and handles the result; it never composes the stages itself. The
//!       dependency direction is stated in `style-guide.mtf > Production layering and stage
//!       ownership` and checked by `xtask/src/architecture_boundary.rs`.
//!
//! # Production entry points
//! - [`module_compilation::compile_module`]: the canonical service for one ready module, from
//!   retained syntax through borrow validation and generated semantic completion
//! - [`single_source_compilation`]: the two named shorter paths — project config compilation and
//!   direct Moth template compilation — which stop at folded AST values
//!
//! `pipeline::CompilerFrontend` is the stage facade those services drive, not a fourth entry
//! point. It holds the module-local mutable state a service threads through the stages, and its
//! semantic stage methods are visible only inside this module.
//!
//! # Stage owners
//! - [`tokenizer`], [`numeric_text`]: source text to tokens
//! - [`headers`], [`declaration_syntax`], [`paths`]: retained declaration and dependency shells,
//!   file-owned path syntax, and interface binding against provider interfaces
//! - [`module_dependencies`]: local declaration ordering
//! - [`ast`]: AST semantics, constants, generics, templates and TIR
//! - [`public_interface`]: the projected interface a provider publishes
//! - [`hir`]: HIR lowering, validation and reachability
//! - [`analysis`]: borrow validation over validated HIR
//! - [`build_config`]: the compiler-owned typed build-input carriers later command-input and
//!   `#Config` phases fill and consume
//!
//! # What this module does NOT own
//! - Which source belongs to a module, when it is prepared and what happens to a compiled result,
//!   which stay under `build_system`
//! - Project aggregation, entry assembly, output policy and builder capability surfaces

pub(crate) mod ast;
pub(crate) mod declaration_syntax;
pub(crate) mod headers;
pub(crate) mod module_dependencies;
pub(crate) mod numeric_text;
pub(crate) mod plain_markdown;
pub(crate) mod public_call_summary;
pub(crate) mod source_packages;
pub(crate) mod style_directives;
pub(crate) mod tokenizer;

pub(crate) mod build_config;
pub(crate) mod builtins;
pub(crate) mod canonical_type_identity;
pub(crate) mod folded_value;
pub(crate) mod instrumentation;
pub(crate) mod keywords;
pub(crate) mod project_globals;
pub(crate) mod public_interface;
pub(crate) mod semantic_identity;
pub(crate) mod source_module_origin;
pub(crate) mod synthetic_interface_provenance;
pub(crate) mod traits;
pub(crate) mod validated_generic_template_metadata;

pub(crate) mod compiler_messages;

pub(crate) mod symbols {
    pub(crate) mod compiler_symbols;
    pub(crate) mod identifier_policy;
    pub(crate) mod identity;
    pub(crate) mod interned_path;
    pub(crate) mod string_interning;

    #[cfg(test)]
    mod tests;
}

pub(crate) use compiler_messages::compiler_errors;
pub(crate) use compiler_messages::display_messages;
pub(crate) mod datatypes;
pub(crate) mod syntax_errors;
pub(crate) mod type_coercion;
pub(crate) mod utilities;
pub(crate) mod value_mode;

pub(crate) mod external_packages;

pub(crate) mod hir;

pub(crate) mod analysis;
pub(crate) mod arena;

pub(crate) mod module_compilation;
pub(crate) mod module_metadata;
pub(crate) mod paths;
pub(crate) mod single_source_compilation;

mod pipeline;

pub(crate) use pipeline::CompilerFrontend;
pub(crate) use pipeline::{
    AstBuildRequest, FrontendFilePrepareContext, FrontendFilePrepareInput,
    FrontendFilePrepareSource,
};
#[cfg(test)]
pub(crate) use pipeline::{
    file_frontend_prepare_count_for_path_for_test, reset_file_frontend_prepare_count_for_test,
};

/// Flags change the behavior of the core `compiler_frontend` pipeline.
#[derive(PartialEq, Eq, Debug, Clone)]
pub enum Flag {
    Release,
    HtmlWasm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrontendBuildProfile {
    Dev,
    Release,
}

#[cfg(test)]
pub(crate) mod tests {
    pub(crate) mod ast_fixture_support;
    pub(crate) mod borrow_fixture_support;
    mod build_config_tests;
    mod canonical_type_identity_tests;
    pub(crate) mod external_package_support;
    mod frontend_pipeline_tests;
    pub(crate) mod hir_fixture_support;
    mod keyword_tests;
    pub(crate) mod parse_support;
    mod plain_markdown_tests;
    mod public_call_summary_tests;
    mod semantic_identity_tests;
    mod synthetic_interface_provenance_tests;
    pub(crate) mod type_id_fixture_support;
}
