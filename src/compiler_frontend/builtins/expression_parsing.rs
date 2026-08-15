//! Builtin expression parsing helpers.
//!
//! WHAT: parses compiler-owned expression forms such as collection literals.
//! WHY: builtin parsing logic should live with builtin metadata so extending language-owned
//! surfaces does not keep bloating the generic expression parser.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::statements::collections::new_curly_literal;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, TypeMismatchContext};
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::FileTokens;
use crate::compiler_frontend::type_coercion::parse_context::{
    ExpectedCollectionContext, ExpectedCurlyLiteralContext, ExpectedMapContext, ExpectedType,
};
use crate::compiler_frontend::value_mode::ValueMode;

/// Parses collection literal expressions (`{...}`) for declared and inferred collection types.
/// Parses curly-brace literal expressions (`{...}`) for collections, maps, and inferred targets.
///
/// WHAT: validates that `{...}` literals are used with a compatible expected type and dispatches
///       to the correct collection or map parser.
/// WHY: curly-brace syntax introduces both homogeneous collections and ordered maps; the builtin
///      parsing helper must own the dispatch so the expression parser stays flat.
pub(crate) fn parse_curly_literal_expression(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expected_type: &ExpectedType,
    value_mode: &ValueMode,
    expression: &mut Vec<ExpressionRpnItem>,
    string_table: &mut StringTable,
) -> Result<(), ExpressionParseError> {
    let curly_context = match expected_type {
        ExpectedType::Known(type_id) => {
            let type_environment = type_interner.environment();
            if let Some(map_shape) = type_environment.map_shape(*type_id) {
                ExpectedCurlyLiteralContext::Map(ExpectedMapContext {
                    key_type_id: map_shape.key_type,
                    value_type_id: map_shape.value_type,
                    key_diagnostic_type: diagnostic_type_spelling(
                        map_shape.key_type,
                        type_environment,
                    ),
                    value_diagnostic_type: diagnostic_type_spelling(
                        map_shape.value_type,
                        type_environment,
                    ),
                    map_type_id: Some(*type_id),
                })
            } else if let Some(shape) = type_environment.collection_shape(*type_id) {
                ExpectedCurlyLiteralContext::Collection(ExpectedCollectionContext::Explicit {
                    collection_type_id: *type_id,
                    element_type_id: shape.element_type,
                    fixed_capacity: shape.fixed_capacity,
                })
            } else {
                return Err(CompilerDiagnostic::type_mismatch(
                    *type_id,
                    type_environment.builtins().string,
                    TypeMismatchContext::General,
                    token_stream.current_location(),
                )
                .into());
            }
        }

        ExpectedType::Infer => ExpectedCurlyLiteralContext::Infer,
    };

    expression.push(ExpressionRpnItem::Operand(new_curly_literal(
        token_stream,
        curly_context,
        context,
        type_interner,
        value_mode,
        string_table,
    )?));
    Ok(())
}
