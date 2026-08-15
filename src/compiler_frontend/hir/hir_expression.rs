//! HIR Expression Lowering
//!
//! Lowers typed AST expressions into HIR expressions and statement preludes.
//! This file contains the high-level dispatcher and shared expression utilities on `HirBuilder`.
//!
//! ## Cast contract
//!
//! AST resolves all cast targets, evidence, fallibility, and optional wrapping flags before HIR.
//! HIR only carries compiler-owned builtin runtime casts as `HirExpressionKind::Cast` or
//! `HirStatementKind::CastOp`. User-defined cast evidence lowers to a direct user-function call
//! during HIR lowering, and `ResolvedCastEvidence::GenericBound` is validation-only and must not
//! reach HIR.
//!
//! ## Diagnostic boundary
//!
//! `CompilerError` / `return_hir_transformation_error!` in this module means an internal
//! HIR transformation or lowering invariant failure only. Normal user-facing source failures
//! must be emitted as `CompilerDiagnostic` from AST or earlier stages.

use crate::compiler_frontend::ast::expressions::call_argument::{CallAccessMode, CallArgument};
#[cfg(test)]
use crate::compiler_frontend::ast::expressions::expression::FallibleCarrierVariant as AstFallibleCarrierVariant;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, FallibleExpressionHandling, FallibleHandling,
};
use crate::compiler_frontend::ast::expressions::expression_kind::ResolvedCastExpression;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_types::{
    CastHandling, ResolvedCastEvidence,
};
use crate::compiler_frontend::ast::statements::value_production::types::{
    ValueBlock, ValueCatchBlock,
};
use crate::compiler_frontend::builtins::casts::evidence::type_id_for_builtin_target;
use crate::compiler_frontend::builtins::casts::targets::{BuiltinCastPolicyId, BuiltinCastTarget};
use crate::compiler_frontend::compiler_errors::{CompilerError, SourceLocation};
use crate::compiler_frontend::datatypes::generic_identity_bridge::TypeIdentityKey;
use crate::compiler_frontend::datatypes::ids::TypeId as FrontendTypeId;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::blocks::{HirBlock, HirLocal};
use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirMapEntry, HirVariantCarrier, HirVariantField,
    OPTION_SOME_VARIANT_INDEX, ValueKind,
};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::hir_side_table::HirLocalOriginKind;
use crate::compiler_frontend::hir::ids::{LocalId, RegionId};
use crate::compiler_frontend::hir::module::HirChoice;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
#[cfg(test)]
use crate::compiler_frontend::paths::path_format::format_compile_time_path;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::hir_log;
use crate::return_hir_transformation_error;

mod calls;
mod fallible;
mod literals;
mod numeric;
mod operators;
mod option_propagation;
mod places;
mod runtime;
mod templates;
mod types;

use self::fallible::{EmittedFallibleCarrier, FallibleCarrierBranchingContext};
pub(crate) use self::fallible::{ExternalFallibleCallLoweringInput, FallibleBranchingContext};

#[derive(Debug, Clone)]
pub(crate) struct LoweredExpression {
    // WHAT: Statements that must execute before evaluating `value`.
    // WHY: HIR requires expression side effects to be linearized into explicit statements.
    pub prelude: Vec<HirStatement>,
    pub value: HirExpression,
}

impl<'a> HirBuilder<'a> {
    // -------------------------
    //  Expression Lowering
    // -------------------------

    // WHAT: lowers one typed AST expression into a linearized HIR prelude/value pair.
    // WHY: HIR cannot keep nested side effects inside expressions, so every entry point must
    //      return both the value and any statements required to produce it.
    pub(crate) fn lower_expression(
        &mut self,
        expr: &Expression,
    ) -> Result<LoweredExpression, CompilerError> {
        self.log_expression_input(expr);
        self.accumulate_function_provenance(expr);

        let lowered = match &expr.kind {
            ExpressionKind::ChoiceConstruct {
                nominal_path,
                tag,
                fields,
            } => {
                let choice_id = if let Some(TypeIdentityKey::GenericInstance(key)) = self
                    .type_environment
                    .type_id_to_type_identity_key(expr.type_id)
                {
                    self.resolve_or_register_generic_choice(
                        &key,
                        nominal_path,
                        expr.type_id,
                        &expr.location,
                    )?
                } else {
                    self.resolve_choice_id(nominal_path, &expr.location)?
                };
                let region = self.current_region_or_error(&expr.location)?;
                let ty = self.lower_type_id(expr.type_id, &expr.location)?;

                let mut prelude = Vec::new();
                let mut hir_fields = Vec::with_capacity(fields.len());

                for field in fields {
                    let value =
                        self.lower_child_expression_for_parent(&mut prelude, &field.value)?;
                    hir_fields.push(HirVariantField {
                        name: field.id.name(),
                        value,
                    });
                }

                // WHY: classify const-ness from the already-lowered HIR field values
                //      instead of reaching back into AST no-store expression classification.
                //      Each field's `value_kind` is set during lowering and is the HIR-stage
                //      authority for whether that field is a compile-time constant.
                let value_kind = if hir_fields
                    .iter()
                    .all(|field| field.value.value_kind == ValueKind::Const)
                {
                    ValueKind::Const
                } else {
                    ValueKind::RValue
                };

                Ok(LoweredExpression {
                    prelude,
                    value: self.make_expression(
                        &expr.location,
                        HirExpressionKind::VariantConstruct {
                            carrier: HirVariantCarrier::Choice { choice_id },
                            variant_index: *tag,
                            fields: hir_fields,
                        },
                        ty,
                        value_kind,
                        region,
                    ),
                })
            }

            ExpressionKind::Int(value) => self.lower_literal_expression(
                &expr.location,
                expr.type_id,
                HirExpressionKind::Int(*value),
            ),

            ExpressionKind::Float(value) => self.lower_literal_expression(
                &expr.location,
                expr.type_id,
                HirExpressionKind::Float(*value),
            ),

            ExpressionKind::Bool(value) => self.lower_literal_expression(
                &expr.location,
                expr.type_id,
                HirExpressionKind::Bool(*value),
            ),

            ExpressionKind::Char(value) => self.lower_literal_expression(
                &expr.location,
                expr.type_id,
                HirExpressionKind::Char(*value),
            ),

            ExpressionKind::StringSlice(value) => self.lower_literal_expression(
                &expr.location,
                expr.type_id,
                HirExpressionKind::StringLiteral(self.string_table.resolve(*value).to_owned()),
            ),

            #[cfg(test)]
            ExpressionKind::Path(compile_time_path) => {
                // Compile-time path values lower to string literals in HIR.
                // Formatting applies the origin prefix for root-based paths and trailing
                // slash for directories through the shared path formatter.
                let path_string = format_compile_time_path(
                    compile_time_path,
                    &self.path_format_config,
                    self.string_table,
                );

                self.lower_literal_expression(
                    &expr.location,
                    expr.type_id,
                    HirExpressionKind::StringLiteral(path_string),
                )
            }

            ExpressionKind::Cast(cast) => {
                self.lower_cast_expression(cast, expr.type_id, &expr.location)
            }

            ExpressionKind::Reference(name) => {
                self.lower_reference_expression(name, expr.type_id, &expr.location)
            }

            ExpressionKind::Copy(place) => {
                let region = self.current_region_or_error(&expr.location)?;
                let (prelude, place) = self.lower_place_expression_to_hir_place(place)?;
                let ty = self.lower_type_id(expr.type_id, &expr.location)?;

                Ok(LoweredExpression {
                    prelude,
                    value: self.make_expression(
                        &expr.location,
                        HirExpressionKind::Copy(place),
                        ty,
                        ValueKind::RValue,
                        region,
                    ),
                })
            }

            ExpressionKind::Runtime(nodes) => {
                self.lower_runtime_rpn_expression(nodes, &expr.location, expr.type_id)
            }

            ExpressionKind::FieldAccess { base, field } => {
                self.lower_field_access_expression(base, *field, expr.type_id, &expr.location)
            }

            ExpressionKind::MethodCall {
                receiver,
                method_path,
                args,
                result_type_ids,
                location,
            } => self.lower_receiver_method_call_expression(
                method_path,
                receiver,
                args,
                result_type_ids,
                location,
            ),

            ExpressionKind::CollectionBuiltinCall {
                receiver,
                op,
                receiver_requires_mutable,
                args,
                result_type_ids,
                location,
            } => self.lower_collection_builtin_call_expression(
                *op,
                receiver,
                *receiver_requires_mutable,
                args,
                result_type_ids,
                location,
            ),

            ExpressionKind::MapBuiltinCall {
                receiver,
                op,
                receiver_requires_mutable,
                args,
                result_type_ids,
                location,
            } => self.lower_map_builtin_call_expression(
                *op,
                receiver,
                *receiver_requires_mutable,
                args,
                result_type_ids,
                location,
            ),

            ExpressionKind::FunctionCall {
                name,
                args,
                result_type_ids,
            } => {
                let target = self.resolve_call_target_or_error(name, &expr.location)?;
                self.lower_call_expression(target, args, result_type_ids, &expr.location)
            }

            ExpressionKind::HandledFallibleFunctionCall {
                name,
                args,
                result_type_ids,
                handling,
            } => {
                let target = self.resolve_call_target_or_error(name, &expr.location)?;
                self.lower_handled_fallible_call_expression(
                    target,
                    args,
                    result_type_ids,
                    handling,
                    true,
                    &expr.location,
                )
            }

            ExpressionKind::HandledFallibleHostFunctionCall {
                id,
                args,
                result_type_ids,
                error_type_id,
                handling,
            } => self.lower_handled_external_fallible_call_expression(
                ExternalFallibleCallLoweringInput {
                    id: *id,
                    args,
                    result_type_ids,
                    error_type_id: *error_type_id,
                    handling,
                    location: &expr.location,
                },
            ),

            #[cfg(test)]
            ExpressionKind::FallibleCarrierConstruct { variant, value } => {
                let mut prelude = Vec::new();
                let lowered_value = self.lower_child_expression_for_parent(&mut prelude, value)?;
                let region = self.current_region_or_error(&expr.location)?;
                let ty = self.lower_type_id(expr.type_id, &expr.location)?;
                let variant_index = match variant {
                    AstFallibleCarrierVariant::Success => 0,
                    AstFallibleCarrierVariant::Error => 1,
                };
                let value_name = self.string_table.intern("value");

                Ok(LoweredExpression {
                    prelude,
                    value: self.make_expression(
                        &expr.location,
                        HirExpressionKind::VariantConstruct {
                            carrier: HirVariantCarrier::Fallible,
                            variant_index,
                            fields: vec![HirVariantField {
                                name: Some(value_name),
                                value: lowered_value,
                            }],
                        },
                        ty,
                        ValueKind::RValue,
                        region,
                    ),
                })
            }

            ExpressionKind::HandledFallibleExpression { value, handling } => self
                .lower_handled_fallible_expression(value, handling, &expr.location, expr.type_id),

            ExpressionKind::OptionPropagation { value } => {
                self.lower_option_expression_to_present_value(value, &expr.location)
            }

            ExpressionKind::HostFunctionCall {
                id: host_id,
                args,
                result_type_ids,
            } => {
                if self.result_type_ids_are_single_float(result_type_ids) {
                    self.lower_validated_external_call_expression(
                        *host_id,
                        args,
                        result_type_ids,
                        &expr.location,
                    )
                } else {
                    self.lower_call_expression(
                        CallTarget::External(*host_id),
                        args,
                        result_type_ids,
                        &expr.location,
                    )
                }
            }

            ExpressionKind::Collection(items) => {
                let mut prelude = Vec::new();
                let mut lowered_items = Vec::with_capacity(items.len());

                for item in items {
                    let lowered_item =
                        self.lower_child_expression_for_parent(&mut prelude, item)?;
                    lowered_items.push(lowered_item);
                }

                let region = self.current_region_or_error(&expr.location)?;
                let ty = self.lower_type_id(expr.type_id, &expr.location)?;

                Ok(LoweredExpression {
                    prelude,
                    value: self.make_expression(
                        &expr.location,
                        HirExpressionKind::Collection(lowered_items),
                        ty,
                        ValueKind::RValue,
                        region,
                    ),
                })
            }

            ExpressionKind::MapLiteral(entries) => {
                let mut prelude = Vec::new();
                let mut hir_entries = Vec::with_capacity(entries.len());
                for entry in entries {
                    let key = self.lower_child_expression_for_parent(&mut prelude, &entry.key)?;
                    let value =
                        self.lower_child_expression_for_parent(&mut prelude, &entry.value)?;
                    hir_entries.push(HirMapEntry { key, value });
                }
                let region = self.current_region_or_error(&expr.location)?;
                let ty = self.lower_type_id(expr.type_id, &expr.location)?;
                Ok(LoweredExpression {
                    prelude,
                    value: self.make_expression(
                        &expr.location,
                        HirExpressionKind::MapLiteral(hir_entries),
                        ty,
                        ValueKind::RValue,
                        region,
                    ),
                })
            }

            ExpressionKind::Range(start, end) => {
                let mut prelude = Vec::new();
                let lowered_start = self.lower_child_expression_for_parent(&mut prelude, start)?;
                let lowered_end = self.lower_child_expression_for_parent(&mut prelude, end)?;

                let region = self.current_region_or_error(&expr.location)?;
                let ty = self.lower_type_id(expr.type_id, &expr.location)?;

                Ok(LoweredExpression {
                    prelude,
                    value: self.make_expression(
                        &expr.location,
                        HirExpressionKind::Range {
                            start: Box::new(lowered_start),
                            end: Box::new(lowered_end),
                        },
                        ty,
                        ValueKind::RValue,
                        region,
                    ),
                })
            }

            ExpressionKind::StructInstance(args) => {
                // INVARIANT: const-record runtime use should have been rejected in AST.
                // If a const record reaches HIR struct lowering, push validation earlier
                // instead of converting this into a user diagnostic here.
                if expr.is_const_record_value() {
                    return_hir_transformation_error!(
                        "HIR invariant: Const record reached runtime HIR struct lowering; field access should select a member before HIR generation",
                        self.hir_error_location(&expr.location)
                    );
                }

                let Some(nominal_path) = self.type_environment.nominal_path(expr.type_id) else {
                    return_hir_transformation_error!(
                        "Struct instance reached HIR lowering without a nominal struct identity",
                        self.hir_error_location(&expr.location)
                    );
                };
                let nominal_path = nominal_path.to_owned();
                let struct_id = if let Some(TypeIdentityKey::GenericInstance(key)) = self
                    .type_environment
                    .type_id_to_type_identity_key(expr.type_id)
                {
                    self.resolve_or_register_generic_struct(
                        &key,
                        &nominal_path,
                        expr.type_id,
                        &expr.location,
                    )?
                } else {
                    self.resolve_struct_id_from_nominal_path(&nominal_path, &expr.location)?
                };
                let mut prelude = Vec::new();
                let mut fields = Vec::with_capacity(args.len());

                for arg in args {
                    let field_id =
                        self.resolve_field_id_or_error(struct_id, &arg.id, &expr.location)?;
                    let lowered_value =
                        self.lower_child_expression_for_parent(&mut prelude, &arg.value)?;
                    fields.push((field_id, lowered_value));
                }

                let region = self.current_region_or_error(&expr.location)?;
                let ty = self.lower_type_id(expr.type_id, &expr.location)?;

                Ok(LoweredExpression {
                    prelude,
                    value: self.make_expression(
                        &expr.location,
                        HirExpressionKind::StructConstruct { struct_id, fields },
                        ty,
                        ValueKind::RValue,
                        region,
                    ),
                })
            }

            ExpressionKind::Template(_) => {
                // INVARIANT: AST finalization replaces every runtime template with an
                // owned handoff payload before HIR. A raw `ExpressionKind::Template`
                // reaching this dispatcher means AST failed to finish its job.
                return_hir_transformation_error!(
                    "Raw template reached HIR runtime-template lowering after AST finalization.",
                    self.hir_error_location(&expr.location)
                )
            }

            ExpressionKind::RuntimeTemplateHandoff(handoff) => self
                .lower_runtime_template_expression_from_owned_handoff(
                    handoff.as_ref(),
                    &expr.location,
                ),

            ExpressionKind::RuntimeSlotApplicationHandoff(handoff) => self
                .lower_runtime_slot_application_expression_from_owned_handoff(
                    handoff.as_ref(),
                    &expr.location,
                ),

            // Lower the inner value and override the HIR type with the declared
            // coercion target. Numeric coercions are resolved by the code generation
            // backend based on the type annotation. Option coercions materialize
            // `some(value)` here so backends see the real runtime carrier.
            ExpressionKind::Coerced { value, .. } => {
                let mut prelude = Vec::new();
                let mut lowered_value =
                    self.lower_child_expression_for_parent(&mut prelude, value)?;
                let coerced_ty = self.lower_type_id(expr.type_id, &expr.location)?;
                if self.type_environment.option_inner_type(expr.type_id) == Some(lowered_value.ty) {
                    let value_name = self.string_table.intern("value");
                    let region = lowered_value.region;
                    lowered_value = self.make_expression(
                        &expr.location,
                        HirExpressionKind::VariantConstruct {
                            carrier: HirVariantCarrier::Option,
                            variant_index: 1,
                            fields: vec![HirVariantField {
                                name: Some(value_name),
                                value: lowered_value,
                            }],
                        },
                        coerced_ty,
                        ValueKind::RValue,
                        region,
                    );
                    return Ok(LoweredExpression {
                        prelude,
                        value: lowered_value,
                    });
                }

                lowered_value.ty = coerced_ty;
                Ok(LoweredExpression {
                    prelude,
                    value: lowered_value,
                })
            }

            ExpressionKind::Function(_) => {
                return_hir_transformation_error!(
                    "Function expressions are not lowered in this phase",
                    self.hir_error_location(&expr.location)
                )
            }

            ExpressionKind::StructDefinition(_) => {
                return_hir_transformation_error!(
                    "Struct definition expressions are not lowered in this phase",
                    self.hir_error_location(&expr.location)
                )
            }

            ExpressionKind::ValueBlock { block } => {
                self.lower_value_block(block, &expr.location, expr.type_id)
            }

            ExpressionKind::NoValue => {
                let region = self.current_region_or_error(&expr.location)?;
                Ok(LoweredExpression {
                    prelude: vec![],
                    value: self.unit_expression(&expr.location, region),
                })
            }

            ExpressionKind::OptionNone => {
                let region = self.current_region_or_error(&expr.location)?;
                let ty = self.lower_type_id(expr.type_id, &expr.location)?;
                Ok(LoweredExpression {
                    prelude: vec![],
                    value: self.make_expression(
                        &expr.location,
                        HirExpressionKind::VariantConstruct {
                            carrier: HirVariantCarrier::Option,
                            variant_index: 0,
                            fields: vec![],
                        },
                        ty,
                        ValueKind::RValue,
                        region,
                    ),
                })
            }
        }?;

        self.bind_reactive_metadata_for_expression(expr, &lowered.value)?;
        self.log_expression_output(expr, &lowered.value);
        Ok(lowered)
    }

    /// Lower an expression and emit its sequencing work into the active block immediately.
    ///
    /// WHAT: turns expression preludes into current-block statements and, when the expression is
    /// postfix-propagated, emits the success/error HIR edge before returning the unwrapped success
    /// payload.
    /// WHY: nested `expr!` is control flow. Compound expressions and call arguments need a value
    /// to continue with, but the error edge must be visible in the CFG instead of being hidden in
    /// an expression-only propagation helper.
    pub(crate) fn lower_expression_value_to_current_block(
        &mut self,
        expr: &Expression,
    ) -> Result<HirExpression, CompilerError> {
        if let Some(success_value) =
            self.lower_fallible_expression_to_success_value(expr, &expr.location)?
        {
            return Ok(success_value);
        }

        let lowered = self.lower_expression(expr)?;
        for prelude in lowered.prelude {
            self.emit_statement_to_current_block(prelude, &expr.location)?;
        }

        Ok(lowered.value)
    }

    /// Lower a child expression while preserving `lower_expression`'s prelude-returning contract.
    ///
    /// WHAT: ordinary children keep contributing to the parent's pending prelude, while children
    /// that need active CFG mutation first flush that pending prelude into the current block.
    /// WHY: `lower_expression` must still be usable by tests and pure expression callers as a
    /// linearization API, but `expr!` and short-circuit control flow cannot stay hidden inside a
    /// returned expression tree.
    pub(crate) fn lower_child_expression_for_parent(
        &mut self,
        pending_prelude: &mut Vec<HirStatement>,
        expr: &Expression,
    ) -> Result<HirExpression, CompilerError> {
        if self.expression_needs_current_block_lowering(expr) {
            for prelude in pending_prelude.drain(..) {
                self.emit_statement_to_current_block(prelude, &expr.location)?;
            }

            return self.lower_expression_value_to_current_block(expr);
        }

        let lowered = self.lower_expression(expr)?;
        pending_prelude.extend(lowered.prelude);
        Ok(lowered.value)
    }

    pub(crate) fn expression_needs_current_block_lowering(&self, expr: &Expression) -> bool {
        match &expr.kind {
            ExpressionKind::HandledFallibleFunctionCall { args, handling, .. }
            | ExpressionKind::HandledFallibleHostFunctionCall { args, handling, .. } => {
                matches!(handling, FallibleExpressionHandling::Propagate)
                    || args
                        .iter()
                        .any(|arg| self.expression_needs_current_block_lowering(&arg.value))
            }
            ExpressionKind::HandledFallibleExpression { value, handling } => {
                matches!(handling, FallibleExpressionHandling::Propagate)
                    || self.expression_needs_current_block_lowering(value)
            }
            ExpressionKind::Cast(_) => true,

            ExpressionKind::OptionPropagation { .. } => true,
            ExpressionKind::FunctionCall { args, .. } => args
                .iter()
                .any(|arg| self.expression_needs_current_block_lowering(&arg.value)),
            ExpressionKind::HostFunctionCall {
                args,
                result_type_ids,
                ..
            } => {
                self.result_type_ids_are_single_float(result_type_ids)
                    || args
                        .iter()
                        .any(|arg| self.expression_needs_current_block_lowering(&arg.value))
            }
            ExpressionKind::FieldAccess { base, .. } => {
                self.expression_needs_current_block_lowering(base)
            }
            ExpressionKind::MethodCall { receiver, args, .. }
            | ExpressionKind::CollectionBuiltinCall { receiver, args, .. }
            | ExpressionKind::MapBuiltinCall { receiver, args, .. } => {
                self.expression_needs_current_block_lowering(receiver)
                    || args
                        .iter()
                        .any(|arg| self.expression_needs_current_block_lowering(&arg.value))
            }
            #[cfg(test)]
            ExpressionKind::FallibleCarrierConstruct { value, .. } => {
                self.expression_needs_current_block_lowering(value)
            }
            ExpressionKind::Coerced { value, .. } => {
                self.expression_needs_current_block_lowering(value)
            }
            ExpressionKind::Copy(_) => false,
            ExpressionKind::Collection(items) => items
                .iter()
                .any(|item| self.expression_needs_current_block_lowering(item)),
            ExpressionKind::MapLiteral(entries) => entries.iter().any(|entry| {
                self.expression_needs_current_block_lowering(&entry.key)
                    || self.expression_needs_current_block_lowering(&entry.value)
            }),
            ExpressionKind::Range(start, end) => {
                self.expression_needs_current_block_lowering(start)
                    || self.expression_needs_current_block_lowering(end)
            }
            ExpressionKind::StructInstance(fields)
            | ExpressionKind::ChoiceConstruct { fields, .. } => fields
                .iter()
                .any(|field| self.expression_needs_current_block_lowering(&field.value)),
            ExpressionKind::Runtime(nodes) => nodes.items.iter().any(|item| match item {
                ExpressionRpnItem::Operand(expression) => {
                    self.expression_needs_current_block_lowering(expression)
                }
                ExpressionRpnItem::Operator { .. } => false,
            }),
            ExpressionKind::Template(_) => true,
            ExpressionKind::RuntimeTemplateHandoff(_)
            | ExpressionKind::RuntimeSlotApplicationHandoff(_) => true,
            ExpressionKind::ValueBlock { .. } => true,

            ExpressionKind::Int(_)
            | ExpressionKind::Float(_)
            | ExpressionKind::Bool(_)
            | ExpressionKind::Char(_)
            | ExpressionKind::StringSlice(_)
            | ExpressionKind::Reference(_)
            | ExpressionKind::Function(_)
            | ExpressionKind::StructDefinition(_)
            | ExpressionKind::NoValue
            | ExpressionKind::OptionNone => false,
            #[cfg(test)]
            ExpressionKind::Path(_) => false,
        }
    }

    /// Returns true when an external call returns exactly one `Float` success value.
    ///
    /// WHAT: checks that the resolved success return list has one slot and that slot is the
    ///       builtin `Float` type.
    /// WHY: external/backend boundaries must validate a scalar `Float` before ordinary Moth
    ///      code observes it; multi-success or non-Float returns are handled elsewhere.
    fn result_type_ids_are_single_float(&self, result_type_ids: &[FrontendTypeId]) -> bool {
        let [single] = result_type_ids else {
            return false;
        };
        *single == self.type_environment.builtins().float
    }

    // -------------------------
    //  Value-Block Lowering
    // -------------------------

    /// Lowers a value-producing control-flow block into CFG statements.
    ///
    /// WHAT: dispatches on the value-block kind (currently only `If`) and builds the
    ///       prelude statements + result value needed by the expression lowering contract.
    /// WHY: value blocks are expressions that build control flow; they must return a
    ///      `LoweredExpression` so the caller can emit their prelude and use their value.
    pub(super) fn lower_value_block(
        &mut self,
        block: &ValueBlock,
        location: &SourceLocation,
        result_type_id: TypeId,
    ) -> Result<LoweredExpression, CompilerError> {
        match block {
            ValueBlock::If(value_if) => {
                self.lower_value_block_if(value_if, location, result_type_id)
            }
            ValueBlock::Match(value_match) => {
                self.lower_value_block_match(value_match, location, result_type_id)
            }
            ValueBlock::Catch(value_catch) => {
                self.lower_value_block_catch(value_catch, location, result_type_id)
            }
        }
    }

    /// Lowers a catch recovery block whose handler body is owned by `ValueCatchBlock`.
    ///
    /// WHAT: dispatches to the same carrier-branching helpers used by statement catch lowering,
    /// but supplies the handler body from the value block instead of from an expression variant.
    fn lower_value_block_catch(
        &mut self,
        value_catch: &ValueCatchBlock,
        location: &SourceLocation,
        result_type_id: TypeId,
    ) -> Result<LoweredExpression, CompilerError> {
        let result_type_ids = &value_catch.result_type_ids;
        let value_required = !result_type_ids.is_empty();

        match &value_catch.handled_value.kind {
            ExpressionKind::HandledFallibleFunctionCall {
                name,
                args,
                result_type_ids: call_result_type_ids,
                ..
            } => {
                let target = self.resolve_call_target_or_error(name, location)?;
                let (carrier_type, ok_type, err_type) =
                    self.result_call_carrier_slots(&target, location)?;
                let requested_ok_type =
                    self.lower_call_result_type(call_result_type_ids, location)?;
                if requested_ok_type != ok_type {
                    return_hir_transformation_error!(
                        "Value catch call lowered with mismatched success type",
                        self.hir_error_location(location)
                    );
                }

                self.lower_handled_fallible_call_with_branching(
                    target,
                    args,
                    FallibleBranchingContext {
                        result_type_ids,
                        handling: &value_catch.handler,
                        carrier_type,
                        ok_type,
                        err_type,
                        value_required,
                        location,
                        validate_float_success: false,
                    },
                )
            }

            ExpressionKind::HandledFallibleHostFunctionCall {
                id,
                args,
                result_type_ids: call_result_type_ids,
                error_type_id,
                ..
            } => {
                let (carrier_type, ok_type, err_type) = self.fallible_call_carrier_from_slots(
                    call_result_type_ids,
                    *error_type_id,
                    location,
                )?;
                self.lower_handled_fallible_call_with_branching(
                    CallTarget::External(*id),
                    args,
                    FallibleBranchingContext {
                        result_type_ids,
                        handling: &value_catch.handler,
                        carrier_type,
                        ok_type,
                        err_type,
                        value_required,
                        location,
                        validate_float_success: self.type_id_is_float(ok_type),
                    },
                )
            }

            ExpressionKind::HandledFallibleExpression { value, .. } => self
                .lower_recovering_fallible_expression(
                    value,
                    &value_catch.handler,
                    result_type_ids,
                    value_required,
                    location,
                ),

            ExpressionKind::Cast(cast) => self.lower_recovering_cast_expression(
                cast,
                &value_catch.handler,
                result_type_id,
                location,
            ),

            _ => return_hir_transformation_error!(
                "Value catch block did not contain a recoverable expression",
                self.hir_error_location(location)
            ),
        }
    }

    // -------------------------
    //  Casts
    // -------------------------

    /// Lowers a resolved explicit `cast` expression into HIR.
    ///
    /// WHAT: dispatches builtin evidence to a `HirExpressionKind::Cast` or a
    ///      fallible `HirStatementKind::CastOp` with branches, and user-defined
    ///      evidence to a direct user-function call.
    /// WHY: the AST already resolved the target, evidence, fallibility, and optional wrap flag;
    ///      HIR lowering only materializes the resulting value or carrier/control-flow shape.
    ///      `ResolvedCastEvidence::GenericBound` reaching here is a compiler invariant failure.
    fn lower_cast_expression(
        &mut self,
        cast: &ResolvedCastExpression,
        expr_type_id: FrontendTypeId,
        location: &SourceLocation,
    ) -> Result<LoweredExpression, CompilerError> {
        match &cast.evidence {
            ResolvedCastEvidence::Builtin { policy } => match &cast.handling {
                CastHandling::Infallible => self.lower_infallible_builtin_cast_expression(
                    cast,
                    *policy,
                    expr_type_id,
                    location,
                ),
                CastHandling::Propagate | CastHandling::Recover => self
                    .lower_fallible_builtin_cast_expression(cast, *policy, expr_type_id, location),
            },
            ResolvedCastEvidence::UserDefined { method_path, .. } => {
                self.lower_user_defined_cast_expression(cast, method_path, expr_type_id, location)
            }
            ResolvedCastEvidence::GenericBound { .. } => Err(CompilerError::new(
                "Generic-bound cast evidence reached HIR lowering",
                self.hir_error_location(location),
                crate::compiler_frontend::compiler_errors::ErrorType::HirTransformation,
            )),
        }
    }

    /// Lowers an infallible builtin cast as a pure HIR expression.
    fn lower_infallible_builtin_cast_expression(
        &mut self,
        cast: &ResolvedCastExpression,
        policy: BuiltinCastPolicyId,
        expr_type_id: FrontendTypeId,
        location: &SourceLocation,
    ) -> Result<LoweredExpression, CompilerError> {
        let mut prelude = Vec::new();
        let source = self.lower_child_expression_for_parent(&mut prelude, &cast.source)?;

        // `Float -> String` is infallible at the source level because valid Moth `Float` is
        // finite, but it must still lower through the shared `FormatFloat` statement so casts and
        // templates use the same Moth-owned formatter.
        if policy == BuiltinCastPolicyId::FloatToString {
            for prelude_statement in prelude.drain(..) {
                self.emit_statement_to_current_block(prelude_statement, location)?;
            }

            let formatted = self.emit_formatted_float_value(source, location)?;
            let value =
                self.wrap_cast_result_optional_if_needed(formatted, expr_type_id, location)?;
            return Ok(LoweredExpression {
                prelude: vec![],
                value,
            });
        }

        let target_type = self.lower_type_id(cast.target_type_id, location)?;
        let region = self.current_region_or_error(location)?;
        let value = self.make_expression(
            location,
            HirExpressionKind::Cast {
                source: Box::new(source),
                policy,
            },
            target_type,
            ValueKind::RValue,
            region,
        );
        let value = self.wrap_cast_result_optional_if_needed(value, expr_type_id, location)?;

        Ok(LoweredExpression { prelude, value })
    }

    /// Emits the fallible builtin-cast carrier used by propagation and catch recovery.
    fn emit_builtin_cast_carrier(
        &mut self,
        cast: &ResolvedCastExpression,
        policy: BuiltinCastPolicyId,
        location: &SourceLocation,
    ) -> Result<EmittedFallibleCarrier, CompilerError> {
        let lowered_source = self.lower_expression(&cast.source)?;
        for prelude_statement in lowered_source.prelude {
            self.emit_statement_to_current_block(prelude_statement, location)?;
        }

        let ok_type = self.lower_type_id(cast.target_type_id, location)?;
        let err_type = self.builtin_error_type_id(location)?;
        let carrier_type = self
            .type_environment
            .intern_fallible_carrier(ok_type, err_type);
        let result_local = self.allocate_temp_local(carrier_type, Some(location.to_owned()))?;

        let cast_statement = HirStatement {
            id: self.allocate_node_id(),
            kind: HirStatementKind::CastOp {
                policy,
                source: lowered_source.value,
                result: Some(result_local),
            },
            location: location.to_owned(),
        };
        self.side_table.map_statement(location, &cast_statement);
        self.emit_statement_to_current_block(cast_statement, location)?;

        Ok(EmittedFallibleCarrier {
            result_local,
            carrier_type,
            ok_type,
            err_type,
            validate_float_success: false,
        })
    }

    /// Lowers a fallible builtin cast through an explicit carrier statement and branches.
    fn lower_fallible_builtin_cast_expression(
        &mut self,
        cast: &ResolvedCastExpression,
        policy: BuiltinCastPolicyId,
        expr_type_id: FrontendTypeId,
        location: &SourceLocation,
    ) -> Result<LoweredExpression, CompilerError> {
        let carrier = self.emit_builtin_cast_carrier(cast, policy, location)?;

        match &cast.handling {
            CastHandling::Propagate => {
                let success_value =
                    self.lower_fallible_carrier_to_success_value(carrier, location)?;
                let value = self.wrap_cast_result_optional_if_needed(
                    success_value,
                    expr_type_id,
                    location,
                )?;
                Ok(LoweredExpression {
                    prelude: vec![],
                    value,
                })
            }
            CastHandling::Recover => return_hir_transformation_error!(
                "Recovering builtin cast reached HIR outside a value catch block",
                self.hir_error_location(location)
            ),
            CastHandling::Infallible => Err(CompilerError::new(
                "Fallible builtin cast reached HIR with Infallible handling",
                self.hir_error_location(location),
                crate::compiler_frontend::compiler_errors::ErrorType::HirTransformation,
            )),
        }
    }

    /// Lowers a user-defined cast by calling the selected evidence method.
    fn lower_user_defined_cast_expression(
        &mut self,
        cast: &ResolvedCastExpression,
        method_path: &InternedPath,
        expr_type_id: FrontendTypeId,
        location: &SourceLocation,
    ) -> Result<LoweredExpression, CompilerError> {
        let call_target = self.resolve_call_target_or_error(method_path, location)?;
        let source_argument = CallArgument::positional(
            (*cast.source).clone(),
            CallAccessMode::Shared,
            location.to_owned(),
        );

        match &cast.handling {
            CastHandling::Infallible => {
                let result_type_ids = vec![cast.target_type_id];
                let lowered = self.lower_call_expression(
                    call_target,
                    &[source_argument],
                    &result_type_ids,
                    location,
                )?;
                let value = self.wrap_cast_result_optional_if_needed(
                    lowered.value,
                    expr_type_id,
                    location,
                )?;
                Ok(LoweredExpression {
                    prelude: lowered.prelude,
                    value,
                })
            }
            CastHandling::Propagate => {
                let carrier = self.emit_user_defined_cast_call_carrier(
                    call_target,
                    &source_argument,
                    location,
                )?;
                let success_value =
                    self.lower_fallible_carrier_to_success_value(carrier, location)?;
                let value = self.wrap_cast_result_optional_if_needed(
                    success_value,
                    expr_type_id,
                    location,
                )?;
                Ok(LoweredExpression {
                    prelude: vec![],
                    value,
                })
            }
            CastHandling::Recover => return_hir_transformation_error!(
                "Recovering user-defined cast reached HIR outside a value catch block",
                self.hir_error_location(location)
            ),
        }
    }

    /// Lowers `cast ... catch:` using the handler body stored by `ValueCatchBlock`.
    fn lower_recovering_cast_expression(
        &mut self,
        cast: &ResolvedCastExpression,
        handler: &FallibleHandling,
        expr_type_id: FrontendTypeId,
        location: &SourceLocation,
    ) -> Result<LoweredExpression, CompilerError> {
        match &cast.evidence {
            ResolvedCastEvidence::Builtin { policy } => {
                let carrier = self.emit_builtin_cast_carrier(cast, *policy, location)?;
                self.lower_cast_catch_with_optional_wrap(
                    carrier,
                    handler,
                    expr_type_id,
                    cast.target_type_id,
                    cast.requires_optional_wrap_after_cast,
                    location,
                )
            }

            ResolvedCastEvidence::UserDefined { method_path, .. } => {
                let call_target = self.resolve_call_target_or_error(method_path, location)?;
                let source_argument = CallArgument::positional(
                    (*cast.source).clone(),
                    CallAccessMode::Shared,
                    location.to_owned(),
                );
                let carrier = self.emit_user_defined_cast_call_carrier(
                    call_target,
                    &source_argument,
                    location,
                )?;
                self.lower_cast_catch_with_optional_wrap(
                    carrier,
                    handler,
                    expr_type_id,
                    cast.target_type_id,
                    cast.requires_optional_wrap_after_cast,
                    location,
                )
            }

            ResolvedCastEvidence::GenericBound { .. } => Err(CompilerError::new(
                "Generic-bound cast evidence reached recovering HIR lowering",
                self.hir_error_location(location),
                crate::compiler_frontend::compiler_errors::ErrorType::HirTransformation,
            )),
        }
    }

    /// Lowers the catch/recovery path for a fallible cast carrier.
    ///
    /// WHAT: reuses the shared fallible branching helper, optionally wrapping the
    ///      merged result in `some(...)` when the receiving context is an optional type.
    /// WHY: in a `T?` receiving context both the cast success and the catch handler
    ///      produce the inner `T`. Lowering with inner result locals keeps the catch
    ///      handler's `then` value type-compatible with the merge, and wrapping the
    ///      merged inner value ensures both control paths produce `T?`.
    fn lower_cast_catch_with_optional_wrap(
        &mut self,
        carrier: EmittedFallibleCarrier,
        handling: &FallibleHandling,
        expr_type_id: FrontendTypeId,
        target_type_id: FrontendTypeId,
        requires_optional_wrap_after_cast: bool,
        location: &SourceLocation,
    ) -> Result<LoweredExpression, CompilerError> {
        let current_block = self.current_block_id_or_error(location)?;

        if !requires_optional_wrap_after_cast {
            let result_type_ids = self.handled_expression_result_type_ids(expr_type_id);
            return self.lower_fallible_carrier_with_branching(FallibleCarrierBranchingContext {
                current_block,
                result_local: carrier.result_local,
                handled_result: FallibleBranchingContext {
                    result_type_ids: &result_type_ids,
                    handling,
                    carrier_type: carrier.carrier_type,
                    ok_type: carrier.ok_type,
                    err_type: carrier.err_type,
                    value_required: true,
                    location,
                    validate_float_success: false,
                },
            });
        }

        // For an optional receiving context, lower the branching with inner target result
        // locals so the catch handler's `then` value is the same type as the success payload.
        // After the merge, wrap the unified inner value into `some(...)` to produce `T?`.
        let inner_result_type_ids = self.handled_expression_result_type_ids(target_type_id);
        let inner_lowered =
            self.lower_fallible_carrier_with_branching(FallibleCarrierBranchingContext {
                current_block,
                result_local: carrier.result_local,
                handled_result: FallibleBranchingContext {
                    result_type_ids: &inner_result_type_ids,
                    handling,
                    carrier_type: carrier.carrier_type,
                    ok_type: carrier.ok_type,
                    err_type: carrier.err_type,
                    value_required: true,
                    location,
                    validate_float_success: false,
                },
            })?;

        let wrapped_value =
            self.wrap_cast_result_optional_if_needed(inner_lowered.value, expr_type_id, location)?;

        Ok(LoweredExpression {
            prelude: inner_lowered.prelude,
            value: wrapped_value,
        })
    }

    /// Emits a user-defined cast method call that returns a fallible carrier.
    fn emit_user_defined_cast_call_carrier(
        &mut self,
        target: CallTarget,
        source_argument: &CallArgument,
        location: &SourceLocation,
    ) -> Result<EmittedFallibleCarrier, CompilerError> {
        let (carrier_type, ok_type, err_type) =
            self.result_call_carrier_slots(&target, location)?;

        let lowered_argument = self.lower_call_argument_value(source_argument, location, 0)?;
        for prelude_statement in lowered_argument.prelude {
            self.emit_statement_to_current_block(prelude_statement, location)?;
        }

        let result_local = self.allocate_temp_local(carrier_type, Some(location.to_owned()))?;
        let call_statement = HirStatement {
            id: self.allocate_node_id(),
            kind: HirStatementKind::Call {
                target,
                args: vec![lowered_argument.value],
                result: Some(result_local),
            },
            location: location.to_owned(),
        };
        self.side_table.map_statement(location, &call_statement);
        self.emit_statement_to_current_block(call_statement, location)?;

        Ok(EmittedFallibleCarrier {
            result_local,
            carrier_type,
            ok_type,
            err_type,
            validate_float_success: false,
        })
    }

    /// Wraps a cast result in `some(...)` when the receiving context is an optional type.
    fn wrap_cast_result_optional_if_needed(
        &mut self,
        value: HirExpression,
        expr_type_id: FrontendTypeId,
        location: &SourceLocation,
    ) -> Result<HirExpression, CompilerError> {
        let expected_type = self.lower_type_id(expr_type_id, location)?;
        if value.ty == expected_type {
            return Ok(value);
        }

        if self.type_environment.option_inner_type(expected_type) != Some(value.ty) {
            return Err(CompilerError::new(
                format!(
                    "Cast result type {:?} cannot be wrapped to expected optional type {:?}",
                    value.ty, expected_type
                ),
                self.hir_error_location(location),
                crate::compiler_frontend::compiler_errors::ErrorType::HirTransformation,
            ));
        }

        let value_name = self.string_table.intern("value");
        let region = value.region;
        Ok(self.make_expression(
            location,
            HirExpressionKind::VariantConstruct {
                carrier: HirVariantCarrier::Option,
                variant_index: OPTION_SOME_VARIANT_INDEX,
                fields: vec![HirVariantField {
                    name: Some(value_name),
                    value,
                }],
            },
            expected_type,
            ValueKind::RValue,
            region,
        ))
    }

    /// Resolves the builtin `Error` type id for fallible carrier error slots.
    fn builtin_error_type_id(
        &mut self,
        location: &SourceLocation,
    ) -> Result<TypeId, CompilerError> {
        type_id_for_builtin_target(
            BuiltinCastTarget::Error,
            &self.type_environment,
            self.string_table,
        )
        .ok_or_else(|| {
            CompilerError::new(
                "Builtin Error type is not registered in the type environment",
                self.hir_error_location(location),
                crate::compiler_frontend::compiler_errors::ErrorType::HirTransformation,
            )
        })
    }

    // -------------------------
    //  Statement Emission
    // -------------------------

    // WHAT: appends a prebuilt statement to the current block.
    // WHY: expression helpers sometimes manufacture statements outside the main statement
    //      dispatcher but still need to preserve explicit execution order.
    pub(crate) fn emit_statement_to_current_block(
        &mut self,
        statement: HirStatement,
        source_location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let block = self.current_block_mut_or_error(source_location)?;
        block.statements.push(statement);
        Ok(())
    }

    // WHAT: emits one `Assign(Local, value)` statement in the current block.
    // WHY: runtime short-circuit and fallible branching both need consistent temp-local
    //      assignment behavior and source mapping.
    pub(crate) fn emit_assign_local_statement(
        &mut self,
        local: LocalId,
        value: HirExpression,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let assign_statement = HirStatement {
            id: self.allocate_node_id(),
            kind: HirStatementKind::Assign {
                target: HirPlace::Local(local),
                value,
            },
            location: location.to_owned(),
        };

        self.side_table.map_statement(location, &assign_statement);
        self.emit_statement_to_current_block(assign_statement, location)
    }

    // -------------------------
    //  Local Allocation
    // -------------------------

    // WHAT: allocates an unnamed temporary local in the current block.
    // WHY: complex expression lowering needs scratch storage to preserve evaluation order and
    //      explicit place/value distinctions in HIR.
    pub(crate) fn allocate_temp_local(
        &mut self,
        ty: TypeId,
        source_info: Option<SourceLocation>,
    ) -> Result<LocalId, CompilerError> {
        self.allocate_compiler_local(
            ty,
            source_info,
            HirLocalOriginKind::CompilerTemp,
            None,
            None,
        )
    }

    pub(crate) fn allocate_fresh_mutable_call_arg_local(
        &mut self,
        ty: TypeId,
        source_info: Option<SourceLocation>,
        call_location: &SourceLocation,
        argument_index: usize,
    ) -> Result<LocalId, CompilerError> {
        self.allocate_compiler_local(
            ty,
            source_info,
            HirLocalOriginKind::CompilerFreshMutableArg,
            Some(call_location),
            Some(argument_index),
        )
    }

    fn allocate_compiler_local(
        &mut self,
        ty: TypeId,
        source_info: Option<SourceLocation>,
        origin: HirLocalOriginKind,
        call_location: Option<&SourceLocation>,
        argument_index: Option<usize>,
    ) -> Result<LocalId, CompilerError> {
        let location = source_info.to_owned().unwrap_or_default();
        let region = self.current_region_or_error(&location)?;
        let block_id = self.current_block_id_or_error(&location)?;
        let local_id = self.allocate_local_id();

        let local = HirLocal {
            id: local_id,
            ty,
            mutable: true,
            region,
            source_info,
        };

        self.side_table.map_local_source(&local);
        self.register_local_in_block(block_id, local, &location)?;

        let temp_name = format!("__hir_tmp_{}", self.temp_local_counter);
        self.temp_local_counter += 1;
        let temp_name_id = InternedPath::from_single_str(&temp_name, self.string_table);

        // Compiler-introduced temporaries are intentionally excluded from AST symbol resolution.
        // They are named only for diagnostics/debug rendering via the side table.
        self.side_table.bind_local_name(local_id, temp_name_id);
        self.side_table
            .bind_local_origin(local_id, origin, call_location, argument_index);

        Ok(local_id)
    }

    // -------------------------
    //  Expression Construction
    // -------------------------

    // WHAT: returns mutable access to the active block or a structured lowering error.
    // WHY: most expression helpers need to append locals or statements, and failing early
    //      produces clearer diagnostics than assuming block state exists.
    pub(crate) fn current_block_mut_or_error(
        &mut self,
        location: &SourceLocation,
    ) -> Result<&mut HirBlock, CompilerError> {
        let block_id = self.current_block_id_or_error(location)?;
        self.block_mut_by_id_or_error(block_id, location)
    }

    // WHAT: allocates one HIR expression node with its identity and typing metadata attached.
    // WHY: centralizing expression construction keeps IDs, source mappings, and value kinds
    //      uniform across every lowering helper.
    pub(crate) fn make_expression(
        &mut self,
        location: &SourceLocation,
        kind: HirExpressionKind,
        ty: TypeId,
        value_kind: ValueKind,
        region: RegionId,
    ) -> HirExpression {
        let id = self.allocate_value_id();
        self.side_table.map_value(location, id, location);

        HirExpression {
            id,
            kind,
            ty,
            value_kind,
            region,
        }
    }

    // WHAT: creates a canonical load expression for one local.
    // WHY: runtime/result branching paths frequently reconstruct this node shape and should share
    //      one helper for readability and consistency.
    pub(crate) fn make_local_load_expression(
        &mut self,
        local: LocalId,
        ty: TypeId,
        location: &SourceLocation,
        region: RegionId,
    ) -> HirExpression {
        self.make_expression(
            location,
            HirExpressionKind::Load(HirPlace::Local(local)),
            ty,
            ValueKind::RValue,
            region,
        )
    }

    // WHAT: builds the canonical HIR representation of unit.
    // WHY: unit values should lower through the same tuple machinery every other pass expects.
    pub(crate) fn unit_expression(
        &mut self,
        location: &SourceLocation,
        region: RegionId,
    ) -> HirExpression {
        let unit_ty = self.type_environment.builtins().none;
        self.make_expression(
            location,
            HirExpressionKind::TupleConstruct { elements: vec![] },
            unit_ty,
            ValueKind::Const,
            region,
        )
    }

    // -------------------------
    //  Choice Support
    // -------------------------

    /// Register a choice declaration, allocating a stable `ChoiceId`.
    ///
    /// WHAT: called during `prepare_hir_declarations` to build the complete choice registry
    /// before any expression or statement lowering.
    /// WHY: separating registration from lookup keeps choice resolution a pure lookup
    ///      path and prevents lazy-creation ordering bugs.
    pub(crate) fn register_choice_id(
        &mut self,
        nominal_path: &InternedPath,
        location: &SourceLocation,
    ) -> Result<crate::compiler_frontend::hir::ids::ChoiceId, CompilerError> {
        if let Some(&choice_id) = self.choices_by_name.get(nominal_path) {
            return Ok(choice_id);
        }

        let frontend_type_id = self
            .type_environment
            .nominal_id_for_path(nominal_path)
            .and_then(|nominal_id| self.type_environment.type_id_for_nominal_id(nominal_id))
            .ok_or_else(|| {
                crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(format!(
                    "Choice '{}' is not registered in TypeEnvironment during HIR lowering",
                    nominal_path.to_string(self.string_table)
                ))
            })?;

        let choice_id = self.allocate_choice_id();

        // Push a placeholder BEFORE lowering variants so recursive registrations
        // preserve the invariant: ChoiceId(N) maps to module.choices[N].
        self.choices_by_name
            .insert(nominal_path.to_owned(), choice_id);
        self.side_table
            .bind_choice_name(choice_id, nominal_path.to_owned());
        let index = choice_id.0 as usize;
        debug_assert!(index == self.module.choices.len());
        self.module.choices.push(HirChoice {
            id: choice_id,
            frontend_type_id,
            variants: vec![],
        });

        let hir_variants = self.lower_choice_variants_for_type_id(frontend_type_id, location)?;
        self.module.choices[index].variants = hir_variants;

        Ok(choice_id)
    }

    /// Look up a pre-registered choice by its canonical path.
    ///
    /// WHAT: resolves a `ChoiceId` after `prepare_hir_declarations` has registered all choices.
    /// WHY: expression and statement lowering should never create new choice metadata;
    ///      missing entries indicate an AST → HIR contract violation.
    pub(crate) fn resolve_choice_id(
        &self,
        nominal_path: &InternedPath,
        location: &SourceLocation,
    ) -> Result<crate::compiler_frontend::hir::ids::ChoiceId, CompilerError> {
        let Some(choice_id) = self.choices_by_name.get(nominal_path).copied() else {
            return_hir_transformation_error!(
                format!(
                    "Choice '{}' was not pre-registered during HIR declaration preparation",
                    self.symbol_name_for_diagnostics(nominal_path)
                ),
                self.hir_error_location(location)
            );
        };
        Ok(choice_id)
    }

    // -------------------------
    //  Diagnostics & Logging
    // -------------------------

    // WHAT: converts a frontend text location into the shared compiler error-location format.
    // WHY: HIR lowering uses one helper so all transformation errors preserve consistent source metadata.
    pub(crate) fn hir_error_location(&self, location: &SourceLocation) -> SourceLocation {
        location.clone()
    }

    fn log_expression_input(&self, _expr: &Expression) {
        hir_log!(format!(
            "[HIR] Lowering expression {:?} @ {:?}",
            _expr.kind, _expr.location
        ));
    }

    fn log_expression_output(&self, _input: &Expression, _output: &HirExpression) {
        hir_log!(format!(
            "[HIR] Lowered expression {:?} -> {}",
            _input.kind,
            _output.display_with_context(
                &crate::compiler_frontend::hir::hir_display::HirDisplayContext::new(
                    self.string_table,
                )
                .with_side_table(&self.side_table)
                .with_type_environment(&self.type_environment),
            )
        ));
    }

    fn log_call_result_binding(
        &self,
        _location: &SourceLocation,
        _local: Option<LocalId>,
        _value: &HirExpression,
    ) {
        hir_log!(format!(
            "[HIR] Emitted call binding @ {:?}: result={:?}, value={}",
            _location,
            _local,
            _value.display_with_context(
                &crate::compiler_frontend::hir::hir_display::HirDisplayContext::new(
                    self.string_table,
                )
                .with_side_table(&self.side_table)
                .with_type_environment(&self.type_environment),
            )
        ));
    }
}
