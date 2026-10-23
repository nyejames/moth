//! Template-aware generic function body validation.
//!
//! WHAT: parses generic function bodies against their unresolved generic parameter `TypeId`s
//! without emitting executable AST/HIR nodes.
//! WHY: local generic function instantiation must only be enabled after the original template
//! has proven it does not depend on behavior that unconstrained generic parameters cannot
//! guarantee before trait bounds exist.

use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::function_body_to_ast;
use crate::compiler_frontend::ast::generic_functions::GenericFunctionTemplate;
use crate::compiler_frontend::ast::module_ast::scope_context::ScopeContext;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::DiagnosticBag;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
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
    pub(crate) expression_builder: &'a mut AstExpressionBuilder,
    pub(crate) const_values: &'a mut ConstValueStore,
    pub(crate) warnings: &'a mut DiagnosticBag,
    pub(crate) string_table: &'a mut StringTable,
    pub(crate) path_fork: &'a mut PathInternerFork,
}

/// Parses and retains a generic function template body for final frontend validation.
///
/// Static selection and terminality consume the retained typed AST before it is discarded.
/// Concrete instance emission still reparses the immutable template after call-site inference
/// succeeds because unresolved generic parameter types cannot enter executable HIR.
pub(crate) fn validate_generic_function_body(
    input: GenericFunctionBodyValidationInput<'_, '_>,
) -> GenericFunctionBodyValidationResult<Vec<AstNode>> {
    let GenericFunctionBodyValidationInput {
        template,
        mut context,
        type_interner,
        expression_builder,
        const_values,
        warnings,
        string_table,
        path_fork,
    } = input;

    context.generic_template_validation = true;

    let Some(body) = template.body_tokens.as_ref() else {
        return Err(ExpressionParseError::Infrastructure(
            crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                "declaring-module generic validation requires retained body syntax",
            )
            .into(),
        ));
    };
    let parse_owner = body.parse_owner()?;
    let (mut body_cursor, _) = parse_owner.cursor()?;
    // The validation context is built as an independent root by the caller, so its body must
    let parsed_body = function_body_to_ast(
        &mut body_cursor,
        context,
        type_interner,
        expression_builder,
        const_values,
        warnings,
        string_table,
        path_fork,
    );
    parsed_body?.expect_complete_nodes("a generic function template body")
}
