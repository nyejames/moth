//! String coercion policy for the Moth compiler frontend.
//!
//! WHAT: defines what expression types are renderable as string content and
//! provides the coercion logic used at template boundaries.
//! WHY: a shared policy keeps template-boundary conversion explicit and separate from template
//! mechanics.

use crate::compiler_frontend::ast::const_values::store::{
    ConstStringValue, ConstValueId, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExpressionOperationView, ExpressionView,
};
use crate::compiler_frontend::builtins::casts::policies::{
    BuiltinCastLiteral, apply_builtin_cast_policy,
};
use crate::compiler_frontend::builtins::casts::targets::{BuiltinCastPolicyId, BuiltinCastTarget};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// Format completed scalar content through the existing numeric text policy.
///
/// Templates stay with their exact TIR fold owner. Structural anchors cannot be observed as text
/// until the owning output boundary supplies their characters.
pub(crate) fn fold_expression_to_string(
    expression: ExpressionView<'_>,
    const_values: &ConstValueStore,
    string_table: &StringTable,
) -> Result<Option<String>, InfrastructureFailure> {
    match expression.operation() {
        ExpressionOperationView::Constant(value) => {
            fold_constant_to_string(value, const_values, string_table)
        }
        ExpressionOperationView::Coerce { value, .. } => {
            fold_expression_to_string(value, const_values, string_table)
        }
        _ => Ok(None),
    }
}

fn fold_constant_to_string(
    value: ConstValueId,
    const_values: &ConstValueStore,
    string_table: &StringTable,
) -> Result<Option<String>, InfrastructureFailure> {
    let payload = const_values.payload(value).ok_or_else(|| {
        InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "String folding received a foreign constant value ID.",
        )
    })?;
    let text = match payload {
        ConstValuePayload::String(ConstStringValue::Text(string)) => {
            Some(string_table.resolve(*string).to_owned())
        }
        ConstValuePayload::Number(value) => Some(value.to_string()),
        ConstValuePayload::FixedScalar(value) => {
            let Some(scalar) = BuiltinCastTarget::Fixed(value.scalar()).numeric_scalar() else {
                return Ok(None);
            };
            let Ok(BuiltinCastLiteral::String(text)) = apply_builtin_cast_policy(
                BuiltinCastPolicyId::NumericToString(scalar),
                &BuiltinCastLiteral::Fixed(*value),
            ) else {
                return Ok(None);
            };
            Some(text)
        }
        ConstValuePayload::Bool(value) => Some(value.to_string()),
        ConstValuePayload::Char(value) => Some(value.to_string()),
        ConstValuePayload::Coerced(inner) | ConstValuePayload::OptionSome(inner) => {
            return fold_constant_to_string(*inner, const_values, string_table);
        }
        _ => None,
    };
    Ok(text)
}

#[cfg(test)]
#[path = "tests/string_tests.rs"]
mod string_tests;
