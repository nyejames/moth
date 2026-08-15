//! Template-aware generic function body validation.
//!
//! WHAT: parses generic function bodies against their unresolved generic parameter `TypeId`s
//! without emitting executable AST/HIR nodes.
//! WHY: local generic function instantiation must only be enabled after the original template
//! has proven it does not depend on behavior that unconstrained generic parameters cannot
//! guarantee before trait bounds exist.

use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::function_body_to_ast;
use crate::compiler_frontend::ast::generic_functions::GenericFunctionTemplate;
use crate::compiler_frontend::ast::module_ast::scope_context::ScopeContext;
use crate::compiler_frontend::ast::statements::terminality::{
    terminality_policy_for_signature, validate_function_body_terminality,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// Generic body validation executes the ordinary body parser and retains its two-lane failure.
/// A corrupted frozen token table is an internal compiler failure even while validating a source
/// generic template.
type GenericFunctionBodyValidationResult<T> = Result<T, ExpressionParseError>;

/// Input bundle for generic body validation.
///
/// WHAT: keeps the mutable parser services together while the caller owns construction of the
/// stage-appropriate `ScopeContext`.
/// WHY: AST emission already knows file visibility, expected returns, and local parameters;
/// this validator should only own the generic-template body rule itself.
pub(crate) struct GenericFunctionBodyValidationInput<'a, 'environment> {
    pub(crate) template: &'a GenericFunctionTemplate,
    pub(crate) context: ScopeContext,
    pub(crate) type_interner: &'a mut AstTypeInterner<'environment>,
    pub(crate) warnings: &'a mut Vec<CompilerDiagnostic>,
    pub(crate) string_table: &'a mut StringTable,
}

/// Parses the generic function template body for validation only.
///
/// The parsed nodes are intentionally discarded. Concrete instance emission reparses the same
/// immutable template after call-site inference succeeds.
pub(crate) fn validate_generic_function_body(
    input: GenericFunctionBodyValidationInput<'_, '_>,
) -> GenericFunctionBodyValidationResult<()> {
    let GenericFunctionBodyValidationInput {
        template,
        mut context,
        type_interner,
        warnings,
        string_table,
    } = input;

    context.generic_template_validation = true;
    let mut token_stream = template
        .body_tokens
        .as_ref()
        .expect("declaring-module generic validation requires retained body syntax")
        .to_owned();
    let validated_nodes = function_body_to_ast(
        &mut token_stream,
        context,
        type_interner,
        warnings,
        string_table,
    )?;

    let policy = terminality_policy_for_signature(&template.signature, false);
    if let Some(diagnostic) = validate_function_body_terminality(
        &validated_nodes,
        policy,
        template.declaration_location.clone(),
    ) {
        return Err(diagnostic.into());
    }

    Ok(())
}
