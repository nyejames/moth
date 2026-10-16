//! Field/member-name parsing and field-access AST construction.
//!
//! WHAT: owns member token parsing and field access node construction.
//! WHY: field reads and inlined const-field reads should not be mixed with builtin or method policy.

use std::cell::RefCell;
use std::rc::Rc;

use super::MemberStepContext;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, NodeKind};
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::const_values::store::{
    ConstValueId, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionConstructionFailure, ExpressionOp,
    ExpressionOperationView, ExpressionView, OwnedExpr, PlaceProjectionView, SuccessShape,
};
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::module_ast::scope_context::{ScopeContext, ScopeDeclarationRef};
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidFieldAccessReason};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::queries::TypeKind;
use moth_lexical::numeric::grammar::NumericLiteralKind;

use crate::compiler_frontend::compiler_messages::trait_keyword_diagnostics::{
    reserved_trait_keyword_error, reserved_trait_keyword_or_dispatch_mismatch_for_tag,
};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::FxHashMap;

#[cfg(test)]
mod tests;

// --------------------------
//  Types
// --------------------------

pub(super) struct ResolvedFieldMember {
    pub(super) field_name: StringId,
    pub(super) type_id: TypeId,
    pub(super) value_mode: ValueMode,
    pub(super) const_record_state: ConstRecordState,
    pub(super) const_inline_value: Option<ExprId>,
}

struct FieldMemberResolution<'a> {
    /// The receiver's authored value, when one exists. A pending receiver (a retained call's
    /// result) has none, so only the type-level field facts apply and no const-record projection
    /// is possible.
    receiver_value: Option<ExprId>,
    receiver_type_id: TypeId,
    field_name: StringId,
    type_environment: &'a TypeEnvironment,
    resolved_struct_fields_by_path: Option<&'a FxHashMap<PathId, Vec<Declaration>>>,
    receiver_is_const_record: bool,
    template_ir_store: Option<&'a Rc<RefCell<TemplateIrStore>>>,
    scope_context: Option<&'a ScopeContext>,
    path_fork: &'a PathInternerFork,
    expression_builder: &'a mut AstExpressionBuilder,
    const_values: &'a ConstValueStore,
}

// --------------------------
//  Helpers
// --------------------------

fn const_field_value(
    receiver_type_id: TypeId,
    field_name: StringId,
    type_environment: &TypeEnvironment,
    resolved_struct_fields_by_path: Option<&FxHashMap<PathId, Vec<Declaration>>>,
    path_fork: &PathInternerFork,
) -> Option<ProjectedField> {
    if matches!(
        type_environment.type_kind(receiver_type_id),
        Some(TypeKind::GenericInstance)
    ) {
        return None;
    }
    let nominal_path = type_environment.nominal_path(receiver_type_id)?;
    let fields = resolved_struct_fields_by_path?.get(nominal_path)?;
    let field = fields
        .iter()
        .find(|field| path_fork.component(field.id) == Some(field_name))?;
    field
        .value
        .default()
        .map(ProjectedField::Constant)
        .or_else(|| field.value.initializer().map(ProjectedField::Expression))
}

fn expression_is_compile_time_constant(
    expression: ExpressionView<'_>,
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
    expression_builder: &AstExpressionBuilder,
    const_values: &ConstValueStore,
) -> Result<bool, ExpressionParseError> {
    Ok(expression
        .const_value_kind_with_template_classifier(const_values, &mut |template| {
            classify_template_from_effective_tir(
                template,
                template_ir_store,
                expression_builder,
                const_values,
            )
        })?
        .is_compile_time_value())
}

#[derive(Clone, Copy)]
enum ProjectedField {
    Expression(ExprId),
    Constant(ConstValueId),
}

impl ProjectedField {
    fn materialise(
        self,
        expression_builder: &mut AstExpressionBuilder,
        const_values: &ConstValueStore,
        span: Option<crate::compiler_frontend::source::SourceSpan>,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        match self {
            Self::Expression(id) => {
                let mut input = expression_builder
                    .view(id)?
                    .to_completed_input(expression_builder)?;
                input.value_mode = ValueMode::ImmutableOwned;
                input.span = span.or(input.span);
                expression_builder.commit_replacement(input)
            }
            Self::Constant(id) => {
                let metadata = const_values.metadata(id).ok_or_else(|| {
                    CompilerError::compiler_error("projected field constant outside owner")
                })?;
                let mut input = expression_builder.completed_input(
                    ExpressionOp::Constant(id),
                    SuccessShape::One(metadata.type_id),
                    span.or(metadata.span),
                    ValueMode::ImmutableOwned,
                )?;
                input.classification.const_record_state = metadata.const_record_state;
                input.classification.contains_regular_division = metadata.contains_regular_division;
                if !metadata.synthetic_interface_provenance.is_empty() {
                    input.occurrence = Some(crate::compiler_frontend::ast::expressions::expression_store::OccurrenceFacts {
                        provenance: metadata.synthetic_interface_provenance.clone(),
                        diagnostic_spelling: None,
                    });
                }
                expression_builder.commit_replacement(input)
            }
        }
    }
}

fn with_const_record_declaration<T>(
    path: &PathId,
    scope_context: Option<&ScopeContext>,
    path_fork: &PathInternerFork,
    visit: impl FnOnce(&Declaration) -> Option<T>,
) -> Option<T> {
    let scope = scope_context?;
    if let Some(declaration) = scope.top_level_declarations.get_by_path(path) {
        return visit(declaration);
    }
    let name = path_fork.component(*path)?;
    match scope.get_reference(&name)? {
        ScopeDeclarationRef::Local { declaration, .. } => visit(&declaration),
        ScopeDeclarationRef::Shared(declaration) => visit(declaration),
        ScopeDeclarationRef::PendingNumeric { .. } | ScopeDeclarationRef::RouteParameter(_) => None,
    }
}

fn const_record_root_and_field_path(
    receiver_value: ExpressionView<'_>,
    field_name: StringId,
) -> (ExpressionView<'_>, Vec<StringId>) {
    let mut fields = vec![field_name];
    let mut current = receiver_value;
    while let ExpressionOperationView::FieldAccess { base, field } = current.operation() {
        fields.push(field);
        current = base;
    }
    fields.reverse();
    (current, fields)
}

fn project_constant_fields(
    mut current: ConstValueId,
    fields: &[StringId],
    const_values: &ConstValueStore,
    path_fork: &PathInternerFork,
) -> Option<ConstValueId> {
    for name in fields {
        let ConstValuePayload::Record(record_fields) = const_values.payload(current)? else {
            return None;
        };
        current = record_fields
            .iter()
            .find(|field| path_fork.component(field.name) == Some(*name))?
            .value;
    }
    Some(current)
}

fn project_const_record_fields(
    current: ExpressionView<'_>,
    fields: &[StringId],
    scope_context: Option<&ScopeContext>,
    path_fork: &PathInternerFork,
    const_values: &ConstValueStore,
) -> Result<Option<ProjectedField>, CompilerError> {
    if fields.is_empty() {
        return Ok(Some(ProjectedField::Expression(current.id())));
    }
    match current.operation() {
        ExpressionOperationView::Struct {
            fields: record_fields,
        } => {
            let Some(field) = record_fields
                .iter()
                .find(|field| path_fork.component(field.name()) == Some(fields[0]))
            else {
                return Ok(None);
            };
            project_const_record_fields(
                field.value(),
                &fields[1..],
                scope_context,
                path_fork,
                const_values,
            )
        }
        ExpressionOperationView::Constant(id) => {
            Ok(project_constant_fields(id, fields, const_values, path_fork)
                .map(ProjectedField::Constant))
        }
        ExpressionOperationView::Reference(path) => {
            let Some(value) =
                with_const_record_declaration(&path, scope_context, path_fork, |declaration| {
                    declaration
                        .value
                        .default()
                        .map(ProjectedField::Constant)
                        .or_else(|| {
                            declaration
                                .value
                                .initializer()
                                .map(ProjectedField::Expression)
                        })
                })
            else {
                return Ok(None);
            };
            match value {
                ProjectedField::Constant(id) => {
                    Ok(project_constant_fields(id, fields, const_values, path_fork)
                        .map(ProjectedField::Constant))
                }
                ProjectedField::Expression(id) => project_const_record_fields(
                    current.bind(id)?,
                    fields,
                    scope_context,
                    path_fork,
                    const_values,
                ),
            }
        }
        _ => Ok(None),
    }
}

fn projected_field_from_receiver(
    receiver: ExpressionView<'_>,
    field_name: StringId,
    scope_context: Option<&ScopeContext>,
    path_fork: &PathInternerFork,
    const_values: &ConstValueStore,
) -> Result<Option<ProjectedField>, CompilerError> {
    let (root, fields) = const_record_root_and_field_path(receiver, field_name);
    project_const_record_fields(root, &fields, scope_context, path_fork, const_values)
}

fn inline_projected_field(
    field: ProjectedField,
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &ConstValueStore,
) -> Result<Option<ExprId>, ExpressionParseError> {
    let is_constant = match field {
        ProjectedField::Expression(id) => expression_is_compile_time_constant(
            expression_builder.view(id)?,
            template_ir_store,
            expression_builder,
            const_values,
        )?,
        ProjectedField::Constant(id) => const_values
            .metadata(id)
            .ok_or_else(|| CompilerError::compiler_error("projected field constant outside owner"))?
            .value_kind
            .is_compile_time_value(),
    };
    if !is_constant {
        return Ok(None);
    }
    Ok(Some(
        field
            .materialise(expression_builder, const_values, None)?
            .id(),
    ))
}

fn receiver_value_expression(receiver_node: &AstNode) -> Option<ExprId> {
    match &receiver_node.kind {
        NodeKind::ExpressionStatement(expression) => Some(*expression),
        NodeKind::VariableDeclaration(declaration) => declaration.value.initializer(),
        _ => None,
    }
}

struct ConstRecordFieldFacts {
    type_id: TypeId,
    is_const_record: bool,
}

fn projected_field_facts(
    field: ProjectedField,
    expression_builder: &AstExpressionBuilder,
    const_values: &ConstValueStore,
) -> Result<ConstRecordFieldFacts, ExpressionParseError> {
    match field {
        ProjectedField::Expression(id) => {
            let view = expression_builder.view(id)?;
            Ok(ConstRecordFieldFacts {
                type_id: view.value_type_id()?,
                is_const_record: view.is_const_record_value(),
            })
        }
        ProjectedField::Constant(id) => {
            let metadata = const_values.metadata(id).ok_or_else(|| {
                CompilerError::compiler_error("projected field constant outside owner")
            })?;
            Ok(ConstRecordFieldFacts {
                type_id: metadata.type_id,
                is_const_record: metadata.const_record_state == ConstRecordState::ConstRecord,
            })
        }
    }
}

/// Project an explicit const-record field without reconstructing a semantic tree.
pub(crate) fn project_explicit_const_record_field(
    expression: OwnedExpr,
    scope_context: &ScopeContext,
    path_fork: &PathInternerFork,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &ConstValueStore,
) -> Result<Option<OwnedExpr>, ExpressionConstructionFailure> {
    let view = expression_builder.view_owned(expression)?;
    let (root_path, fields) = match view.operation() {
        ExpressionOperationView::FieldAccess { base, field } => {
            let (root, fields) = const_record_root_and_field_path(base, field);
            let ExpressionOperationView::Reference(path) = root.operation() else {
                return Ok(None);
            };
            (path, fields)
        }
        ExpressionOperationView::Copy(place) => {
            let mut fields = Vec::new();
            for projection in place.projections() {
                let PlaceProjectionView::Field { field, .. } = projection else {
                    return Ok(None);
                };
                fields.push(field);
            }
            if fields.is_empty() {
                return Ok(None);
            }
            (place.metadata().root, fields)
        }
        _ => return Ok(None),
    };
    let Some(constant) = scope_context.explicit_constant_value(&root_path, path_fork, const_values)
    else {
        return Ok(None);
    };
    let Some(root_metadata) = const_values.metadata(constant) else {
        return Ok(None);
    };
    if root_metadata.const_record_state != ConstRecordState::ConstRecord {
        return Ok(None);
    }
    let Some(projected) = project_constant_fields(constant, &fields, const_values, path_fork)
    else {
        return Ok(None);
    };
    let Some(metadata) = const_values.metadata(projected) else {
        return Ok(None);
    };
    if view.success_types() != [metadata.type_id] {
        return Ok(None);
    }
    let provenance = metadata
        .synthetic_interface_provenance
        .union(&root_metadata.synthetic_interface_provenance);
    let projected =
        ProjectedField::Constant(projected).materialise(expression_builder, const_values, None)?;
    Ok(Some(
        expression_builder.with_synthetic_interface_provenance(projected, provenance)?,
    ))
}

fn resolved_projected_field(
    field_name: StringId,
    field_type_id: TypeId,
    field_value_is_const_record: bool,
    type_environment: &TypeEnvironment,
    receiver_is_const_record: bool,
    const_inline_value: Option<ExprId>,
) -> ResolvedFieldMember {
    let field_type_is_struct = matches!(
        type_environment.get(field_type_id),
        Some(TypeDefinition::Struct(_))
    );
    let const_record_state =
        if field_value_is_const_record || (receiver_is_const_record && field_type_is_struct) {
            ConstRecordState::ConstRecord
        } else {
            ConstRecordState::RuntimeValue
        };

    ResolvedFieldMember {
        field_name,
        type_id: field_type_id,
        value_mode: ValueMode::ImmutableOwned,
        const_record_state,
        const_inline_value,
    }
}

fn resolve_field_member(
    input: FieldMemberResolution<'_>,
) -> Result<Option<ResolvedFieldMember>, ExpressionParseError> {
    let FieldMemberResolution {
        receiver_value,
        receiver_type_id,
        field_name,
        type_environment,
        resolved_struct_fields_by_path,
        receiver_is_const_record,
        template_ir_store,
        scope_context,
        path_fork,
        expression_builder,
        const_values,
    } = input;

    let projected_field = match receiver_value {
        Some(receiver) => projected_field_from_receiver(
            expression_builder.view(receiver)?,
            field_name,
            scope_context,
            path_fork,
            const_values,
        )?,
        None => None,
    };
    let projected_facts = projected_field
        .map(|field| projected_field_facts(field, expression_builder, const_values))
        .transpose()?;
    let const_inline_value = if let Some(template_ir_store) = template_ir_store {
        let candidate = projected_field.or_else(|| {
            const_field_value(
                receiver_type_id,
                field_name,
                type_environment,
                resolved_struct_fields_by_path,
                path_fork,
            )
        });
        match candidate {
            Some(field) => {
                inline_projected_field(field, template_ir_store, expression_builder, const_values)?
            }
            None => None,
        }
    } else {
        None
    };

    if let Some(inlined_expression) = const_inline_value {
        return Ok(Some(resolved_projected_field(
            field_name,
            expression_builder
                .view(inlined_expression)?
                .value_type_id()?,
            expression_builder
                .view(inlined_expression)?
                .is_const_record_value(),
            type_environment,
            receiver_is_const_record,
            Some(inlined_expression),
        )));
    }

    if let Some(field_facts) = projected_facts {
        return Ok(Some(resolved_projected_field(
            field_name,
            field_facts.type_id,
            field_facts.is_const_record,
            type_environment,
            receiver_is_const_record,
            None,
        )));
    }

    // Try canonical TypeEnvironment first; fall back to AST-owned struct shells
    // when the struct was registered with an empty field list during early
    // identity-only registration (e.g. during constant resolution before final
    // field types are resolved).
    let Some(field_type_id) = type_environment
        .field_for(receiver_type_id, field_name, path_fork)
        .map(|field| field.type_id)
        .or_else(|| {
            let nominal_path = type_environment.nominal_path(receiver_type_id)?;
            let fields = resolved_struct_fields_by_path?.get(nominal_path)?;
            let declaration = fields
                .iter()
                .find(|f| path_fork.component(f.id) == Some(field_name))?;
            Some(declaration.type_id)
        })
    else {
        return Ok(None);
    };

    let const_field_value = const_field_value(
        receiver_type_id,
        field_name,
        type_environment,
        resolved_struct_fields_by_path,
        path_fork,
    );

    let field_value_is_const_record = const_field_value
        .map(|field| projected_field_facts(field, expression_builder, const_values))
        .transpose()?
        .is_some_and(|facts| facts.is_const_record);
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
        value_mode: ValueMode::ImmutableOwned,
        const_record_state,
        const_inline_value: None,
    }))
}
/// Resolve one field member for a pending receiver whose concrete type is already known.
///
/// WHAT: runs the same canonical field lookup as the completed chain without a receiver value: no
///       const-record projection is possible, while a declaration-level const field still inlines
///       its compile-time value.
/// WHY: member steps on a retained operand resolve against known receiver structure at the authored
///      site, so their field facts must come from the one field-resolution owner instead of a
///      second lookup beside it.
pub(super) fn resolve_field_member_for_known_receiver(
    receiver_type_id: TypeId,
    field_name: StringId,
    scope_context: &ScopeContext,
    type_interner: &AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &ConstValueStore,
    path_fork: &PathInternerFork,
) -> Result<Option<ResolvedFieldMember>, ExpressionParseError> {
    let template_ir_store = if scope_context.kind.is_constant_context() {
        Some(scope_context.template_ir_store.clone())
    } else {
        None
    };
    resolve_field_member(FieldMemberResolution {
        receiver_value: None,
        receiver_type_id,
        field_name,
        type_environment: type_interner.environment(),
        resolved_struct_fields_by_path: scope_context.resolved_struct_fields_by_path.as_deref(),
        receiver_is_const_record: false,
        template_ir_store: template_ir_store.as_ref(),
        scope_context: Some(scope_context),
        path_fork,
        expression_builder,
        const_values,
    })
}

pub(super) fn parse_member_name_typed(
    token_stream: &AstCursor<'_>,
    string_table: &mut StringTable,
) -> Result<StringId, ExpressionParseError> {
    match token_stream.current_tag() {
        TokenTag::SYMBOL => Ok(token_stream
            .current_string_id_in(string_table)?
            .ok_or_else(|| {
                CompilerError::compiler_error("member symbol token had no string payload")
            })?),
        TokenTag::NUMERIC_LITERAL => {
            let token = token_stream
                .current_numeric_literal_in(string_table)?
                .ok_or_else(|| {
                    CompilerError::compiler_error("member numeric token had no payload")
                })?;
            if token.kind == NumericLiteralKind::WholeNumber {
                Ok(token.normalized_text)
            } else {
                Err(CompilerDiagnostic::invalid_field_access(
                    InvalidFieldAccessReason::ExpectedNameAfterDot,
                    None,
                    None,
                    Vec::new(),
                    Some(token_stream.current_span()),
                )
                .into())
            }
        }
        TokenTag::MUST | TokenTag::TRAIT_THIS => {
            let keyword = reserved_trait_keyword_or_dispatch_mismatch_for_tag(
                token_stream.current_tag(),
                Some(token_stream.current_span()),
                "AST Construction",
                "postfix/member parsing",
            )?;

            Err(reserved_trait_keyword_error(keyword, Some(token_stream.current_span())).into())
        }

        _ => Err(CompilerDiagnostic::invalid_field_access(
            InvalidFieldAccessReason::ExpectedNameAfterDot,
            None,
            None,
            Vec::new(),
            Some(token_stream.current_span()),
        )
        .into()),
    }
}

// --------------------------
//  Main parsers
// --------------------------

pub(super) fn parse_field_member_access_typed(
    token_stream: &mut AstCursor<'_>,
    context: MemberStepContext<'_>,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &ConstValueStore,
    path_fork: &PathInternerFork,
) -> Result<Option<AstNode>, ExpressionParseError> {
    let MemberStepContext {
        receiver_node,
        receiver_type_id,
        member_name,
        member_span,
        scope_context,
        ..
    } = context;
    let receiver_is_const_record =
        receiver_node.expression_is_const_record_value(expression_builder)?;
    let receiver_value = receiver_value_expression(receiver_node);
    let field = {
        let template_ir_store = if scope_context.kind.is_constant_context() {
            Some(scope_context.template_ir_store.clone())
        } else {
            None
        };
        resolve_field_member(FieldMemberResolution {
            receiver_value,
            receiver_type_id,
            field_name: member_name,
            type_environment: type_interner.environment(),
            resolved_struct_fields_by_path: scope_context.resolved_struct_fields_by_path.as_deref(),
            receiver_is_const_record,
            template_ir_store: template_ir_store.as_ref(),
            scope_context: Some(scope_context),
            path_fork,
            expression_builder,
            const_values,
        })?
    };

    let Some(field) = field else {
        return Ok(None);
    };

    token_stream.advance();

    if token_stream.current_tag() == TokenTag::OPEN_PARENTHESIS {
        return Err(CompilerDiagnostic::invalid_field_access(
            InvalidFieldAccessReason::FieldNotMethod,
            Some(member_name),
            Some(receiver_type_id),
            Vec::new(),
            member_span,
        )
        .into());
    }

    let result_expression = if let Some(inlined_expression) = field.const_inline_value {
        let mut input = expression_builder
            .view(inlined_expression)?
            .to_completed_input(expression_builder)?;
        input.span = member_span;
        expression_builder.commit_replacement(input)?
    } else {
        let base_expression = receiver_value_expression(receiver_node).ok_or_else(|| {
            CompilerError::compiler_error("field receiver had no expression initializer")
        })?;
        expression_builder.field_access(
            expression_builder.own(base_expression),
            field.field_name,
            field.type_id,
            member_span,
            field.value_mode,
            field.const_record_state,
        )?
    };

    Ok(Some(AstNode {
        kind: NodeKind::ExpressionStatement(result_expression.id()),
        scope: scope_context.scope.to_owned(),
        span: member_span,
    }))
}
