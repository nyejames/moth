//! Field/member-name parsing and field-access AST construction.
//!
//! WHAT: owns member token parsing and field access node construction.
//! WHY: field reads and inlined const-field reads should not be mixed with builtin or method policy.

use std::cell::RefCell;
use std::rc::Rc;

use super::MemberStepContext;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, NodeKind};
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidFieldAccessReason};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::numeric_text::token::NumericLiteralKind;

use crate::compiler_frontend::compiler_messages::trait_keyword_diagnostics::{
    reserved_trait_keyword_error, reserved_trait_keyword_or_dispatch_mismatch,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::FxHashMap;

#[cfg(test)]
mod tests;

// --------------------------
//  Types
// --------------------------

struct ResolvedFieldMember {
    field_name: StringId,
    type_id: TypeId,
    diagnostic_type: DataType,
    value_mode: ValueMode,
    const_record_state: ConstRecordState,
    const_inline_value: Option<Expression>,
}

struct FieldMemberResolution<'a> {
    receiver_node: &'a AstNode,
    receiver_type_id: TypeId,
    field_name: StringId,
    type_environment: &'a TypeEnvironment,
    resolved_struct_fields_by_path: Option<&'a FxHashMap<InternedPath, Vec<Declaration>>>,
    receiver_is_const_record: bool,
    template_ir_store: Option<&'a Rc<RefCell<TemplateIrStore>>>,
}

// --------------------------
//  Helpers
// --------------------------

fn const_field_value<'a>(
    receiver_type_id: TypeId,
    field_name: StringId,
    type_environment: &TypeEnvironment,
    resolved_struct_fields_by_path: Option<&'a FxHashMap<InternedPath, Vec<Declaration>>>,
) -> Option<&'a Expression> {
    if type_environment
        .generic_instance_key(receiver_type_id)
        .is_some()
    {
        return None;
    }

    let nominal_path = type_environment.nominal_path(receiver_type_id)?;
    let fields = resolved_struct_fields_by_path.and_then(|map| map.get(nominal_path))?;
    let field = fields
        .iter()
        .find(|field| field.id.name() == Some(field_name))?;

    Some(&field.value)
}

fn expression_is_compile_time_constant(
    expression: &Expression,
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
) -> Result<bool, ExpressionParseError> {
    Ok(expression
        .const_value_kind_with_template_classifier(&mut |template| {
            classify_template_from_effective_tir(template, template_ir_store)
        })?
        .is_compile_time_value())
}

fn const_inline_field_value(
    receiver_node: &AstNode,
    receiver_type_id: TypeId,
    field_name: StringId,
    type_environment: &TypeEnvironment,
    resolved_struct_fields_by_path: Option<&FxHashMap<InternedPath, Vec<Declaration>>>,
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
) -> Result<Option<Expression>, ExpressionParseError> {
    if let Some(field_value) =
        const_inline_field_value_from_receiver(receiver_node, field_name, template_ir_store)?
    {
        return Ok(Some(field_value));
    }

    let field_value = const_field_value(
        receiver_type_id,
        field_name,
        type_environment,
        resolved_struct_fields_by_path,
    );

    let Some(field_value) = field_value else {
        return Ok(None);
    };

    if !expression_is_compile_time_constant(field_value, template_ir_store)? {
        return Ok(None);
    }

    let mut inlined_expression = field_value.to_owned();
    inlined_expression.value_mode = ValueMode::ImmutableOwned;
    Ok(Some(inlined_expression))
}

fn const_inline_field_value_from_receiver(
    receiver_node: &AstNode,
    field_name: StringId,
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
) -> Result<Option<Expression>, ExpressionParseError> {
    let receiver_value = match &receiver_node.kind {
        NodeKind::ExpressionStatement(expression) => expression,
        NodeKind::VariableDeclaration(declaration) => &declaration.value,
        _ => return Ok(None),
    };

    let ExpressionKind::StructInstance(fields) = &receiver_value.kind else {
        return Ok(None);
    };

    let Some(field) = fields
        .iter()
        .find(|field| field.id.name() == Some(field_name))
    else {
        return Ok(None);
    };
    let field_value = field.value.to_owned();

    if !expression_is_compile_time_constant(&field_value, template_ir_store)? {
        return Ok(None);
    }

    let mut inlined_expression = field_value;
    inlined_expression.value_mode = ValueMode::ImmutableOwned;
    Ok(Some(inlined_expression))
}

fn resolve_field_member(
    input: FieldMemberResolution<'_>,
) -> Result<Option<ResolvedFieldMember>, ExpressionParseError> {
    let FieldMemberResolution {
        receiver_node,
        receiver_type_id,
        field_name,
        type_environment,
        resolved_struct_fields_by_path,
        receiver_is_const_record,
        template_ir_store,
    } = input;

    // Try canonical TypeEnvironment first; fall back to AST-owned struct shells
    // when the struct was registered with an empty field list during early
    // identity-only registration (e.g. during constant resolution before final
    // field types are resolved).
    let Some(field_type_id) = type_environment
        .field_for(receiver_type_id, field_name)
        .map(|field| field.type_id)
        .or_else(|| {
            let nominal_path = type_environment.nominal_path(receiver_type_id)?;
            let fields = resolved_struct_fields_by_path?.get(nominal_path)?;
            let declaration = fields.iter().find(|f| f.id.name() == Some(field_name))?;
            Some(declaration.value.type_id)
        })
    else {
        return Ok(None);
    };

    let const_field_value = const_field_value(
        receiver_type_id,
        field_name,
        type_environment,
        resolved_struct_fields_by_path,
    );

    let const_inline_value = if let Some(template_ir_store) = template_ir_store {
        // Const records need full declaration values for field inlining. The
        // TypeEnvironment owns semantic field types, while the resolved struct
        // field side table owns foldable default expressions. Prefer the
        // already-inlined receiver instance so const records preserve authored
        // constructor values such as `HtmlDefaults(color = "green")`.
        const_inline_field_value(
            receiver_node,
            receiver_type_id,
            field_name,
            type_environment,
            resolved_struct_fields_by_path,
            template_ir_store,
        )?
    } else {
        None
    };
    let field_value_is_const_record = const_field_value
        .map(Expression::is_const_record_value)
        .unwrap_or(false);
    let field_type_is_struct = matches!(
        type_environment.get(field_type_id),
        Some(TypeDefinition::Struct(_))
    );
    // A struct-valued field read from a const record is still a compile-time
    // member group. It may be chained into another field access, but the field
    // value itself must not be materialized as a runtime struct.
    let const_record_state =
        if receiver_is_const_record && (field_value_is_const_record || field_type_is_struct) {
            ConstRecordState::ConstRecord
        } else {
            ConstRecordState::RuntimeValue
        };

    Ok(Some(ResolvedFieldMember {
        field_name,
        type_id: field_type_id,
        diagnostic_type: diagnostic_type_spelling(field_type_id, type_environment),
        value_mode: ValueMode::ImmutableOwned,
        const_record_state,
        const_inline_value,
    }))
}

pub(super) fn parse_member_name_typed(
    token_stream: &FileTokens,
    _string_table: &StringTable,
) -> Result<StringId, ExpressionParseError> {
    match token_stream.current_token_kind() {
        TokenKind::Symbol(id) => Ok(*id),
        TokenKind::NumericLiteral(token) if token.kind == NumericLiteralKind::WholeNumber => {
            Ok(token.normalized_text)
        }
        TokenKind::Must | TokenKind::TraitThis => {
            let keyword = reserved_trait_keyword_or_dispatch_mismatch(
                token_stream.current_token_kind(),
                token_stream.current_location(),
                "AST Construction",
                "postfix/member parsing",
            )?;

            Err(reserved_trait_keyword_error(keyword, token_stream.current_location()).into())
        }

        _ => Err(CompilerDiagnostic::invalid_field_access(
            InvalidFieldAccessReason::ExpectedNameAfterDot,
            None,
            None,
            Vec::new(),
            token_stream.current_location(),
        )
        .into()),
    }
}

// --------------------------
//  Main parsers
// --------------------------

pub(super) fn parse_field_member_access_typed(
    token_stream: &mut FileTokens,
    context: MemberStepContext<'_>,
    type_interner: &mut AstTypeInterner<'_>,
) -> Result<Option<AstNode>, ExpressionParseError> {
    let MemberStepContext {
        receiver_node,
        receiver_type_id,
        member_name,
        member_location,
        scope_context,
        ..
    } = context;
    let receiver_is_const_record = receiver_node.expression_is_const_record_value()?;
    let field = {
        let template_ir_store = if scope_context.kind.is_constant_context() {
            Some(scope_context.template_ir_store.clone())
        } else {
            None
        };

        resolve_field_member(FieldMemberResolution {
            receiver_node,
            receiver_type_id,
            field_name: member_name,
            type_environment: type_interner.environment(),
            resolved_struct_fields_by_path: scope_context.resolved_struct_fields_by_path.as_deref(),
            receiver_is_const_record,
            template_ir_store: template_ir_store.as_ref(),
        })?
    };

    let Some(field) = field else {
        return Ok(None);
    };

    token_stream.advance();

    if token_stream.current_token_kind() == &TokenKind::OpenParenthesis {
        return Err(CompilerDiagnostic::invalid_field_access(
            InvalidFieldAccessReason::FieldNotMethod,
            Some(member_name),
            Some(receiver_type_id),
            Vec::new(),
            member_location,
        )
        .into());
    }

    let result_expression = if let Some(inlined_expression) = field.const_inline_value {
        inlined_expression
    } else {
        increment_ast_counter(AstCounter::PostfixReceiverNodesCopied);

        let base_expression = match &receiver_node.kind {
            NodeKind::ExpressionStatement(expression) => expression.to_owned(),
            NodeKind::VariableDeclaration(declaration) => declaration.value.to_owned(),
            _ => {
                return Err(CompilerError::compiler_error(format!(
                    "Expected expression receiver node, found {:?}",
                    receiver_node.kind
                ))
                .into());
            }
        };

        let mut expression = Expression::new(
            ExpressionKind::FieldAccess {
                base: Box::new(base_expression),
                field: field.field_name,
            },
            member_location.clone(),
            field.type_id,
            field.diagnostic_type,
            field.value_mode,
        );
        expression.const_record_state = field.const_record_state;
        expression
    };

    Ok(Some(AstNode {
        kind: NodeKind::ExpressionStatement(result_expression),
        scope: scope_context.scope.to_owned(),
        location: member_location,
    }))
}
