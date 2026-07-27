//! Compiler frontend pipeline.
//!
//! WHAT: tokenization, header parsing, dependency sorting, AST/HIR construction, and borrow
//! validation, wired into the stage flow described in the compiler design overview.

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

pub(crate) mod builtins;
pub(crate) mod canonical_type_identity;
pub(crate) mod folded_value;
pub(crate) mod instrumentation;
pub(crate) mod keywords;
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

pub(crate) mod module_metadata;
pub(crate) mod paths;

mod pipeline;

pub use pipeline::CompilerFrontend;
pub(crate) use pipeline::{
    FrontendFilePrepareContext, FrontendFilePrepareInput, FrontendFilePrepareSource,
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
    mod canonical_type_identity_tests;
    pub(crate) mod external_package_support;
    pub(crate) mod hir_fixture_support;
    mod keyword_tests;
    pub(crate) mod parse_support;
    mod plain_markdown_tests;
    mod semantic_identity_tests;
    mod synthetic_interface_provenance_tests;
    pub(crate) mod type_id_fixture_support;
}
