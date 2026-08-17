//! Struct field-list shell parsing and default-value validation.
//!
//! WHAT: wraps shared record-body parsing for `Struct = | ... |` field declarations.
//! WHY: struct defaults have extra compile-time constraints that should stay separate from the
//! general shared `| ... |` parsing logic.
//!
//! This module is the authoritative home for the struct shell parser. It returns neutral field
//! syntax; AST type resolution later turns that syntax into typed declarations.

use std::cell::RefCell;
use std::rc::Rc;

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::declaration_syntax::record_body::parse_record_body;
use crate::compiler_frontend::declaration_syntax::signature_members::{
    SignatureMemberContext, SignatureMemberSyntax,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::FileTokens;

/// Boxed diagnostic result for struct shell parsing.
///
/// WHAT: mirrors `RecordBodyParseResult` so the thin `parse_struct_shell` wrapper
///       propagates the already-boxed `parse_record_body` diagnostic without unboxing.
/// WHY: struct shell parsing is a delegation layer; the boxed boundary belongs to
///      record-body parsing, and each plain-`CompilerDiagnostic` caller unboxes once.
type StructShellResult = Result<Vec<SignatureMemberSyntax>, Box<CompilerDiagnostic>>;

/// Parse a struct field-list shell from `| field Type [= default], ... |` syntax.
///
/// WHAT: advances past the opening `|`, parses all fields via `parse_signature_members`,
/// advances past the closing `|`, and validates that any default values are compile-time constants.
/// WHY: this is the single canonical struct field parser. Used by header parsing to populate
/// `StructHeaderMetadata.fields` and by body-declaration parsing for inline struct literals.
pub fn parse_struct_shell(
    token_stream: &mut FileTokens,
    string_table: &mut StringTable,
    warnings: &mut Vec<CompilerDiagnostic>,
    owner_path: &crate::compiler_frontend::symbols::interned_path::InternedPath,
) -> StructShellResult {
    parse_record_body(
        token_stream,
        string_table,
        warnings,
        SignatureMemberContext::StructField,
        owner_path,
    )
}

/// Validates that every struct field default is a compile-time constant.
///
/// WHAT: enforces the invariant that struct defaults must be known at compile time.
/// WHY: called at AST stage only, after constant resolution has run. At header stage,
/// references are unresolved and cannot be validated yet.
///
/// Template constness is classified through the caller's effective view over the module TIR store.
/// A preparation failure preserves its typed infrastructure lane instead of being rendered
/// into the generic non-constant source diagnostic.
pub(crate) fn validate_struct_default_values(
    fields: &[Declaration],
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
) -> Result<(), TemplateError> {
    for field in fields {
        if matches!(field.value.kind, ExpressionKind::NoValue) {
            continue;
        }

        let classification =
            field
                .value
                .const_value_kind_with_template_classifier(&mut |template| {
                    classify_template_from_effective_tir(template, template_ir_store)
                });

        let is_compile_time_constant = classification?.is_compile_time_value();

        if !is_compile_time_constant {
            return Err(CompilerDiagnostic::invalid_struct_default_value(
                field.value.location.clone(),
            )
            .into());
        }
    }

    Ok(())
}
