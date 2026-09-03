//! Multi-bind parsing and target resolution helpers.
//!
//! WHAT: parses `a, b = pair()` style statements and resolves each target into the AST shape that HIR
//! lowering consumes later.
//! WHY: multi-bind syntax mixes statement parsing, declaration rules, and target validation, so it
//! deserves a dedicated module instead of living inside the general function-body dispatcher.
//!
//! INVARIANT: multi-bind is a special-purpose surface for explicit multi-value-producing
//! expressions. It is NOT a generic destructuring mechanism. The right-hand side must belong to
//! an explicitly supported multi-bind-producing surface (currently: multi-return function calls).
//! Future surfaces (e.g. pattern-match blocks) should be added by extending the classifier, not by
//! broadening regular declaration syntax.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{
    AstNode, Declaration, MultiBindTarget, MultiBindTargetKind, NodeKind,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression;
use crate::compiler_frontend::ast::statements::value_production::try_parse_multi_bind_value_block;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::type_resolution::{
    TypeResolutionContext, TypeResolutionContextInputs, resolve_diagnostic_type_to_type_id_checked,
    resolve_parsed_type_annotation,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidAssignmentTargetReason, InvalidMultiBindReason, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::declaration_shell::{
    BindingTargetSyntax, parse_binding_target_syntax,
};
use crate::compiler_frontend::symbols::identifier_policy::{
    IdentifierNamingKind, ensure_not_keyword_shadow_identifier, naming_warning_for_identifier,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::utilities::token_scan::has_top_level_comma_before_statement_end;
use crate::compiler_frontend::value_mode::ValueMode;
use std::collections::HashSet;

/// Multi-bind is a closed receiver that may recursively parse value-producing bodies. Preserve
/// those body's infrastructure failures until the AST emission boundary instead of reducing them
/// to a diagnostic inside this statement-local helper.
type MultiBindResult<T> = Result<T, ExpressionParseError>;

struct ResolvedMultiBindTargets {
    targets: Vec<MultiBindTarget>,
    new_declarations: Vec<Declaration>,
}

// --------------------------
//  Parsing
// --------------------------

pub(crate) fn parse_multi_bind_statement(
    token_stream: &mut FileTokens,
    context: &mut ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> MultiBindResult<Option<AstNode>> {
    if !has_top_level_comma_before_statement_end(token_stream) {
        return Ok(None);
    }

    let Some(parsed_targets) = parse_target_list(token_stream, string_table)? else {
        return Ok(None);
    };

    validate_unique_target_names(&parsed_targets, string_table)?;
    validate_multi_bind_target_identifiers(&parsed_targets, context, string_table)?;
    let target_names = parsed_targets
        .iter()
        .map(|target| target.name)
        .collect::<Vec<_>>();
    let mut rhs_context = context.with_pending_catch_assignment_targets(&target_names);

    let known_slot_types = resolve_known_slot_types(
        &parsed_targets,
        &mut rhs_context,
        type_interner,
        string_table,
    )?;

    let rhs_expression = if token_stream.current_token_kind() == &TokenKind::If {
        match try_parse_multi_bind_value_block(
            token_stream,
            &rhs_context,
            type_interner,
            parsed_targets.len(),
            &known_slot_types,
            string_table,
        ) {
            Some(Ok(expr)) => expr,
            Some(Err(diagnostic)) => return Err(diagnostic),
            None => parse_multi_bind_rhs_expression(
                token_stream,
                &rhs_context,
                type_interner,
                string_table,
            )?,
        }
    } else {
        parse_multi_bind_rhs_expression(token_stream, &rhs_context, type_interner, string_table)?
    };

    let rhs_slots = extract_rhs_slot_types(&rhs_expression, type_interner.environment())?;

    if rhs_slots.len() != parsed_targets.len() {
        return Err(CompilerDiagnostic::invalid_multi_bind(
            InvalidMultiBindReason::ArityMismatch {
                expected: parsed_targets.len(),
                found: rhs_slots.len(),
            },
            None,
            rhs_expression.location.clone(),
        )
        .into());
    }

    let resolved_targets = resolve_multi_bind_targets(
        &parsed_targets,
        &rhs_slots,
        &mut *context,
        type_interner,
        string_table,
    )?;

    register_new_declarations(context, resolved_targets.new_declarations);

    Ok(Some(AstNode {
        kind: NodeKind::MultiBind {
            targets: resolved_targets.targets,
            value: rhs_expression,
        },
        location: token_stream.current_location(),
        scope: context.scope.clone(),
    }))
}

/// Check that every target name is legal for assignment and emit naming warnings
/// for newly introduced identifiers.
fn validate_multi_bind_target_identifiers(
    parsed_targets: &[BindingTargetSyntax],
    context: &ScopeContext,
    string_table: &mut StringTable,
) -> MultiBindResult<()> {
    for target in parsed_targets {
        ensure_not_keyword_shadow_identifier(
            target.name,
            target.location.to_owned(),
            string_table,
        )?;

        if context.is_assignment_target_unavailable(target.name) {
            return Err(CompilerDiagnostic::invalid_assignment_target(
                InvalidAssignmentTargetReason::UnavailableInCatchRecovery,
                Some(target.name),
                None,
                None,
                None,
                None,
                target.location.to_owned(),
            )
            .into());
        }

        if context.get_reference(&target.name).is_none()
            && let Some(warning) = naming_warning_for_identifier(
                target.name,
                target.location.to_owned(),
                IdentifierNamingKind::ValueLike,
                string_table,
            )
        {
            context.emit_warning(warning);
        }
    }

    Ok(())
}

/// Consume a comma-separated list of binding targets and the `=` separator.
///
/// WHY: this is the only place that knows how to backtrack when the stream is not a multi-bind.
fn parse_target_list(
    token_stream: &mut FileTokens,
    string_table: &mut StringTable,
) -> MultiBindResult<Option<Vec<BindingTargetSyntax>>> {
    let start_index = token_stream.index;
    let mut parsed_targets = Vec::new();
    let mut saw_comma = false;
    let mut continuation_comma = None;

    loop {
        if token_stream.current_token_kind() == &TokenKind::This {
            return Err(CompilerDiagnostic::invalid_multi_bind_syntax(
                InvalidMultiBindReason::ThisTargetReserved,
                token_stream.current_location(),
            )
            .into());
        }

        let TokenKind::Symbol(name) = token_stream.current_token_kind().to_owned() else {
            return Err(CompilerDiagnostic::invalid_multi_bind_syntax(
                if continuation_comma.is_some() {
                    InvalidMultiBindReason::MissingTargetAfterComma
                } else {
                    InvalidMultiBindReason::ExpectedTargetName
                },
                continuation_comma.unwrap_or_else(|| token_stream.current_location()),
            )
            .into());
        };
        token_stream.advance();
        let target_syntax = parse_binding_target_syntax(name, token_stream, string_table)?;
        validate_target_mutability(&target_syntax, string_table)?;
        parsed_targets.push(target_syntax);

        match token_stream.current_token_kind() {
            TokenKind::Comma => {
                saw_comma = true;
                let comma_location = token_stream.current_location();
                continuation_comma = Some(comma_location.clone());
                token_stream.advance();

                while token_stream.current_token_kind() == &TokenKind::Newline {
                    token_stream.advance();
                }

                if matches!(
                    token_stream.current_token_kind(),
                    TokenKind::Comma | TokenKind::Assign | TokenKind::End | TokenKind::Eof
                ) {
                    return Err(CompilerDiagnostic::invalid_multi_bind_syntax(
                        InvalidMultiBindReason::MissingTargetAfterComma,
                        comma_location,
                    )
                    .into());
                }
            }

            TokenKind::Assign => break,

            TokenKind::Newline | TokenKind::End | TokenKind::Eof => {
                return Err(CompilerDiagnostic::invalid_multi_bind_syntax(
                    InvalidMultiBindReason::MissingAssignmentOperator,
                    token_stream.current_location(),
                )
                .into());
            }

            _ => {
                return Err(CompilerDiagnostic::invalid_multi_bind_syntax(
                    InvalidMultiBindReason::InvalidTokenAfterTarget,
                    token_stream.current_location(),
                )
                .into());
            }
        }
    }

    if !saw_comma || parsed_targets.len() < 2 {
        token_stream.index = start_index;
        return Ok(None);
    }

    token_stream.advance();
    Ok(Some(parsed_targets))
}

/// Reject mutable multi-bind targets that lack an explicit type annotation.
fn validate_target_mutability(
    target_syntax: &BindingTargetSyntax,
    _string_table: &StringTable,
) -> MultiBindResult<()> {
    if target_syntax.binding_mode.is_mutable()
        && target_syntax.type_annotation.eq(&ParsedTypeRef::Inferred)
    {
        return Err(CompilerDiagnostic::invalid_multi_bind(
            InvalidMultiBindReason::MutableTargetNeedsExplicitType,
            Some(target_syntax.name),
            target_syntax.location.clone(),
        )
        .into());
    }

    Ok(())
}

/// Ensure no two targets in the same multi-bind share the same name.
fn validate_unique_target_names(
    parsed_targets: &[BindingTargetSyntax],
    _string_table: &StringTable,
) -> MultiBindResult<()> {
    let mut seen_target_names = HashSet::new();
    for target in parsed_targets {
        if !seen_target_names.insert(target.name) {
            return Err(CompilerDiagnostic::invalid_multi_bind(
                InvalidMultiBindReason::DuplicateTarget,
                Some(target.name),
                target.location.clone(),
            )
            .into());
        }
    }

    Ok(())
}

/// Parse the single expression on the right-hand side of a multi-bind.
fn parse_multi_bind_rhs_expression(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> MultiBindResult<Expression> {
    if matches!(
        token_stream.current_token_kind(),
        TokenKind::Newline | TokenKind::End | TokenKind::Eof
    ) {
        return Err(CompilerDiagnostic::invalid_multi_bind_syntax(
            InvalidMultiBindReason::MissingRightHandExpression,
            token_stream.current_location(),
        )
        .into());
    }

    let mut inferred_rhs_type = ExpectedType::Infer;
    let rhs_expression = create_expression(
        token_stream,
        context,
        type_interner,
        &mut inferred_rhs_type,
        &ValueMode::ImmutableOwned,
        false,
        string_table,
    )?;

    if token_stream.current_token_kind() == &TokenKind::Comma {
        return Err(CompilerDiagnostic::invalid_multi_bind_syntax(
            InvalidMultiBindReason::MultipleRightHandExpressions,
            token_stream.current_location(),
        )
        .into());
    }

    Ok(rhs_expression)
}

/// Pre-resolve the types that are already known before parsing the RHS.
///
/// WHAT: for each target, returns `Some(TypeId)` if the type is known from an explicit
/// annotation or from an existing mutable local, otherwise `None`.
/// WHY: multi-bind value-if parsing needs this to decide whether it can delegate to the
/// standard receiver helper (all known) or must use the inferred path.
fn resolve_known_slot_types(
    parsed_targets: &[BindingTargetSyntax],
    context: &mut ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> MultiBindResult<Vec<Option<TypeId>>> {
    let mut known = Vec::with_capacity(parsed_targets.len());

    for target_syntax in parsed_targets {
        let explicit_type =
            resolve_target_explicit_type(target_syntax, context, type_interner, string_table)?;
        if let Some((type_id, _)) = explicit_type {
            known.push(Some(type_id));
            continue;
        }

        if let Some(existing) = context.get_reference(&target_syntax.name) {
            known.push(Some(existing.value.type_id));
            continue;
        }

        known.push(None);
    }

    Ok(known)
}

/// Verify that the RHS expression belongs to a whitelisted multi-value-producing surface.
fn classify_multi_bind_rhs(
    expression: &Expression,
    type_environment: &TypeEnvironment,
) -> MultiBindResult<()> {
    match &expression.kind {
        ExpressionKind::FunctionCall { .. }
        | ExpressionKind::HandledFallibleFunctionCall { .. }
        | ExpressionKind::HandledFallibleHostFunctionCall { .. }
        | ExpressionKind::HostFunctionCall { .. } => Ok(()),

        ExpressionKind::ValueBlock { .. } => {
            let Some(tuple_fields) = type_environment.tuple_field_ids(expression.type_id) else {
                return Err(CompilerDiagnostic::invalid_multi_bind(
                    InvalidMultiBindReason::RhsNotMultiValue,
                    None,
                    expression.location.clone(),
                )
                .into());
            };

            if tuple_fields.len() < 2 {
                return Err(CompilerDiagnostic::invalid_multi_bind(
                    InvalidMultiBindReason::RhsNotMultiValue,
                    None,
                    expression.location.clone(),
                )
                .into());
            }

            Ok(())
        }

        _ => Err(CompilerDiagnostic::invalid_multi_bind(
            InvalidMultiBindReason::UnsupportedRhs,
            None,
            expression.location.clone(),
        )
        .into()),
    }
}

/// Extract the ordered slot `TypeId`s from a classified multi-value RHS expression.
fn extract_rhs_slot_types(
    rhs_expression: &Expression,
    type_environment: &TypeEnvironment,
) -> MultiBindResult<Vec<TypeId>> {
    classify_multi_bind_rhs(rhs_expression, type_environment)?;

    let Some(tuple_fields) = type_environment.tuple_field_ids(rhs_expression.type_id) else {
        return Err(CompilerDiagnostic::invalid_multi_bind(
            InvalidMultiBindReason::RhsNotMultiValue,
            None,
            rhs_expression.location.clone(),
        )
        .into());
    };

    Ok(tuple_fields.to_vec())
}

// WHAT: resolves each parsed binding target into either a fresh declaration target or an existing
// mutable assignment target.
// WHY: multi-bind is intentionally allowed to mix new names with existing mutable locals, and HIR
// needs that distinction preserved instead of rediscovering it later.
fn resolve_multi_bind_targets(
    parsed_targets: &[BindingTargetSyntax],
    rhs_slots: &[TypeId],
    context: &mut ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> MultiBindResult<ResolvedMultiBindTargets> {
    let mut resolved_bindings = Vec::with_capacity(parsed_targets.len());
    let mut new_declarations = Vec::new();

    for (slot_index, (slot_type, target_syntax)) in
        rhs_slots.iter().zip(parsed_targets.iter()).enumerate()
    {
        let explicit_type = resolve_target_explicit_type(
            target_syntax,
            &mut *context,
            type_interner,
            string_table,
        )?;
        let explicit_type_id = explicit_type.as_ref().map(|(type_id, _)| *type_id);
        let explicit_diagnostic_type = explicit_type.map(|(_, diagnostic_type)| diagnostic_type);

        let Some(existing_declaration) = context.get_reference(&target_syntax.name) else {
            let target_data_type = resolve_new_target_data_type(
                target_syntax,
                explicit_type_id,
                explicit_diagnostic_type.as_ref(),
                *slot_type,
                slot_index,
                string_table,
                type_interner.environment(),
            )?;
            let target_ownership = binding_target_ownership(target_syntax);
            let target_id = context.scope.append(target_syntax.name);

            // Build the declaration with the canonical slot TypeId directly.
            // diagnostic_type is display-only; semantic identity comes from type_id.
            new_declarations.push(Declaration {
                id: target_id.to_owned(),
                value: Expression::new(
                    ExpressionKind::NoValue,
                    target_syntax.location.clone(),
                    *slot_type,
                    target_data_type.to_owned(),
                    target_ownership.to_owned(),
                ),
                config_qualifier: None,
            });

            resolved_bindings.push(MultiBindTarget {
                id: target_id,
                type_id: *slot_type,
                value_mode: target_ownership,
                kind: MultiBindTargetKind::Declaration,
                location: target_syntax.location.clone(),
            });
            continue;
        };

        // Existing mutable target — validate compatibility and produce an assignment target.
        resolved_bindings.push(resolve_existing_target(
            target_syntax,
            *slot_type,
            explicit_type_id.as_ref(),
            existing_declaration.as_declaration(),
            slot_index,
            string_table,
            type_interner.environment(),
        )?);
    }

    Ok(ResolvedMultiBindTargets {
        targets: resolved_bindings,
        new_declarations,
    })
}

// --------------------------
//  Target resolution
// --------------------------

/// Validate that an existing mutable local is compatible with the corresponding RHS slot.
fn resolve_existing_target(
    target_syntax: &BindingTargetSyntax,
    slot_type: TypeId,
    explicit_type_id: Option<&TypeId>,
    existing_declaration: &Declaration,
    _slot_index: usize,
    _string_table: &StringTable,
    _type_environment: &TypeEnvironment,
) -> MultiBindResult<MultiBindTarget> {
    if target_syntax.binding_mode.is_mutable() {
        return Err(CompilerDiagnostic::invalid_multi_bind(
            InvalidMultiBindReason::ExistingTargetMutableMarker,
            Some(target_syntax.name),
            target_syntax.location.clone(),
        )
        .into());
    }

    if !existing_declaration.value.value_mode.is_mutable() {
        return Err(CompilerDiagnostic::invalid_multi_bind(
            InvalidMultiBindReason::ExistingTargetImmutable,
            Some(target_syntax.name),
            target_syntax.location.clone(),
        )
        .into());
    }

    if let Some(explicit_type_id) = explicit_type_id
        && *explicit_type_id != existing_declaration.value.type_id
    {
        return Err(CompilerDiagnostic::type_mismatch(
            existing_declaration.value.type_id,
            *explicit_type_id,
            TypeMismatchContext::General,
            target_syntax.location.clone(),
        )
        .into());
    }

    if existing_declaration.value.type_id != slot_type {
        return Err(CompilerDiagnostic::type_mismatch(
            existing_declaration.value.type_id,
            slot_type,
            TypeMismatchContext::General,
            target_syntax.location.clone(),
        )
        .into());
    }

    Ok(MultiBindTarget {
        id: existing_declaration.id.to_owned(),
        type_id: slot_type,
        value_mode: existing_declaration.value.value_mode.to_owned(),
        kind: MultiBindTargetKind::Assignment,
        location: target_syntax.location.clone(),
    })
}

/// Determine the diagnostic `DataType` for a new declaration target.
///
/// If the target has an explicit annotation, it is checked against the corresponding RHS slot type.
fn resolve_new_target_data_type(
    target_syntax: &BindingTargetSyntax,
    explicit_type_id: Option<TypeId>,
    explicit_diagnostic_type: Option<&DataType>,
    slot_type: TypeId,
    _slot_index: usize,
    _string_table: &StringTable,
    type_environment: &TypeEnvironment,
) -> MultiBindResult<DataType> {
    let Some(explicit_type_id) = explicit_type_id else {
        // Display-only diagnostic spelling for the declaration.
        return Ok(diagnostic_type_spelling(slot_type, type_environment));
    };

    if explicit_type_id != slot_type {
        return Err(CompilerDiagnostic::type_mismatch(
            explicit_type_id,
            slot_type,
            TypeMismatchContext::General,
            target_syntax.location.clone(),
        )
        .into());
    }

    // Return the explicit annotation's diagnostic spelling if available.
    // Inferred targets derive display spelling from the canonical TypeId.
    Ok(explicit_diagnostic_type
        .cloned()
        .unwrap_or_else(|| diagnostic_type_spelling(slot_type, type_environment)))
}

/// Map the parsed binding mode to the value mode used for the target declaration.
fn binding_target_ownership(target_syntax: &BindingTargetSyntax) -> ValueMode {
    target_syntax.binding_mode.value_mode()
}

/// Insert freshly resolved declarations into the current scope context.
fn register_new_declarations(context: &mut ScopeContext, new_declarations: Vec<Declaration>) {
    for declaration in new_declarations {
        let binding_location = declaration.value.location.clone();
        context.add_var(declaration, binding_location);
    }
}

// --------------------------
//  Helpers
// --------------------------

fn resolve_target_explicit_type(
    target_syntax: &BindingTargetSyntax,
    context: &mut ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> MultiBindResult<Option<(TypeId, DataType)>> {
    if target_syntax.type_annotation.eq(&ParsedTypeRef::Inferred) {
        return Ok(None);
    }

    let resolved_annotation = {
        let mut type_resolution_context =
            TypeResolutionContext::from_inputs(TypeResolutionContextInputs {
                declaration_table: &context.top_level_declarations,
                visible_declaration_ids: context.visible_declaration_ids.as_ref(),
                visible_external_symbols: context
                    .file_visibility
                    .as_ref()
                    .map(|fv| &fv.visible_external_symbols),
                visible_source_bindings: context
                    .file_visibility
                    .as_ref()
                    .map(|fv| &fv.visible_source_names),
                visible_type_aliases: context
                    .file_visibility
                    .as_ref()
                    .map(|fv| &fv.visible_type_alias_names),
                resolved_type_aliases: context.resolved_type_aliases.as_deref(),
                generic_declarations_by_path: context.generic_declarations_by_path.as_deref(),
                resolved_struct_fields_by_path: context.resolved_struct_fields_by_path.as_deref(),
                type_environment: type_interner.environment_mut_for_derived_types(),
                visible_namespace_records: context
                    .file_visibility
                    .as_ref()
                    .map(|fv| &fv.visible_namespace_records),
                trait_environment: Some(context.trait_environment()),
                trait_evidence_environment: Some(context.trait_evidence_environment()),
                visible_trait_names: context
                    .file_visibility
                    .as_ref()
                    .map(|fv| &fv.visible_trait_names),
            })
            .with_active_generic_type_context(context.active_generic_type_context());

        resolve_parsed_type_annotation(
            target_syntax.type_annotation.clone(),
            &target_syntax.location,
            &mut type_resolution_context,
            string_table,
            Some(context),
        )?
    };

    if matches!(resolved_annotation.diagnostic_type, DataType::Inferred) {
        return Ok(None);
    }

    let type_id = resolve_diagnostic_type_to_type_id_checked(
        &resolved_annotation.diagnostic_type,
        type_interner.environment_mut_for_derived_types(),
        &target_syntax.location,
    )?;

    Ok(Some((type_id, resolved_annotation.diagnostic_type)))
}
