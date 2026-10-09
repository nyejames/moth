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
//! HIR transformation or lowering invariant failure only. The construction failure lane also
//! carries source diagnostics when authored input exceeds a compact HIR store capacity; other
//! source failures are emitted by AST or earlier stages.

use crate::compiler_frontend::ast::expressions::call_argument::{CallAccessMode, CallArgument};
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, FallibleExpressionHandling,
};
use crate::compiler_frontend::ast::expressions::expression_kind::ResolvedCastExpression;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
#[cfg(test)]
use crate::compiler_frontend::ast::expressions::expression_types::FallibleCarrierVariant as AstFallibleCarrierVariant;
use crate::compiler_frontend::ast::expressions::expression_types::{
    CastHandling, ResolvedCastEvidence,
};
use crate::compiler_frontend::ast::expressions::failure_facts::FailureDisposition;
use crate::compiler_frontend::ast::statements::value_production::types::{
    ValueBlock, ValueCatchBlock,
};
use crate::compiler_frontend::builtins::casts::evidence::type_id_for_builtin_target;
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastPolicyId, BuiltinCastTarget,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::generic_identity_bridge::TypeIdentityKey;
use crate::compiler_frontend::datatypes::ids::TypeId as FrontendTypeId;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::blocks::{HirBlock, HirLocal};
use crate::compiler_frontend::hir::expression_store::{HirConstructionFailure, HirValueRange};
use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirMapEntry, HirVariantCarrier, HirVariantField,
    OPTION_SOME_VARIANT_INDEX, ValueKind,
};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::hir_side_table::HirLocalOriginKind;
use crate::compiler_frontend::hir::ids::{HirValueId, LocalId, RegionId};
use crate::compiler_frontend::hir::module::HirChoice;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatement, HirStatementKind, HirWriteTarget,
};
#[cfg(feature = "benchmark_counters")]
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
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

use self::fallible::EmittedFallibleCarrier;
pub(crate) use self::fallible::{ExternalFallibleCallLoweringInput, FallibleBranchingContext};

#[derive(Debug, Clone)]
pub(crate) struct LoweredExpression {
    // WHAT: Statements that must execute before evaluating `value`.
    // WHY: HIR requires expression side effects to be linearized into explicit statements.
    pub prelude: Vec<HirStatement>,
    pub value: HirValueId,
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
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        self.lower_expression_with_root_cast_result_type(expr, expr.type_id)
    }

    /// Lowers one expression through the shared entry while selecting its root cast result type.
    ///
    /// WHAT: preserves normal expression-entry bookkeeping and allows a direct catch-root cast
    ///       to contribute its inner target value to the catch join.
    /// WHY: the catch owner applies optional wrapping after that join; nested casts still use
    ///      their own AST receiving type through `lower_expression`.
    fn lower_expression_with_root_cast_result_type(
        &mut self,
        expr: &Expression,
        root_cast_result_type_id: FrontendTypeId,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        #[cfg(feature = "benchmark_counters")]
        increment_frontend_counter(FrontendCounter::CensusHirLowerExpressionEntries);
        self.log_expression_input(expr);
        self.accumulate_function_provenance(expr);

        // Only the six immediate scalar payload-copy arms below, not TypeId,
        // span, remap, rewrite or constant-store copy traffic.
        #[cfg(feature = "benchmark_counters")]
        if matches!(
            &expr.kind,
            ExpressionKind::Uint(_)
                | ExpressionKind::Int(_)
                | ExpressionKind::Float(_)
                | ExpressionKind::FixedScalar(_)
                | ExpressionKind::Bool(_)
                | ExpressionKind::Char(_)
        ) {
            increment_frontend_counter(FrontendCounter::CensusHirScalarPayloadCopiesSampled);
        }

        let mut lowered = match &expr.kind {
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
                        &expr.span,
                    )?
                } else {
                    self.resolve_choice_id(nominal_path, &expr.span)?
                };
                let region = self.current_region_or_error(&expr.span)?;
                let ty = self.lower_type_id(expr.type_id, &expr.span)?;

                let mut prelude = Vec::new();
                let mut hir_fields = Vec::with_capacity(fields.len());

                for field in fields {
                    let value =
                        self.lower_child_expression_for_parent(&mut prelude, &field.value)?;
                    hir_fields.push(HirVariantField {
                        name: self.path_fork.component(field.id),
                        value,
                    });
                }

                // WHY: classify const-ness from the already-lowered HIR field values
                //      instead of reaching back into AST no-store expression classification.
                //      Each field's `value_kind` is set during lowering and is the HIR-stage
                //      authority for whether that field is a compile-time constant.
                let value_kind = if hir_fields.iter().all(|field| {
                    self.module.expressions.expression(field.value).value_kind == ValueKind::Const
                }) {
                    ValueKind::Const
                } else {
                    ValueKind::RValue
                };
                let field_range = self
                    .module
                    .expressions
                    .append_variant_fields(&hir_fields, expr.span)?;

                Ok(LoweredExpression {
                    prelude,
                    value: self.make_expression(
                        &expr.span,
                        HirExpressionKind::VariantConstruct {
                            carrier: HirVariantCarrier::Choice { choice_id },
                            variant_index: *tag,
                            fields: field_range,
                        },
                        ty,
                        value_kind,
                        region,
                    )?,
                })
            }

            ExpressionKind::Uint(value) => self.lower_literal_expression(
                &expr.span,
                expr.type_id,
                HirExpressionKind::Uint(*value),
            ),
            ExpressionKind::Int(value) => self.lower_literal_expression(
                &expr.span,
                expr.type_id,
                HirExpressionKind::Int(*value),
            ),
            ExpressionKind::Number(value) => {
                // Sampled NumberValue payload clone at this dispatcher, not a
                // comprehensive ExpressionKind/constant-materialisation clone census.
                #[cfg(feature = "benchmark_counters")]
                increment_frontend_counter(FrontendCounter::CensusHirNumberPayloadClonesSampled);
                self.lower_literal_expression(
                    &expr.span,
                    expr.type_id,
                    HirExpressionKind::Number(value.clone()),
                )
            }

            ExpressionKind::Float(value) => self.lower_literal_expression(
                &expr.span,
                expr.type_id,
                HirExpressionKind::Float(*value),
            ),

            ExpressionKind::FixedScalar(value) => self.lower_literal_expression(
                &expr.span,
                expr.type_id,
                HirExpressionKind::FixedScalar(*value),
            ),

            ExpressionKind::Bool(value) => self.lower_literal_expression(
                &expr.span,
                expr.type_id,
                HirExpressionKind::Bool(*value),
            ),

            ExpressionKind::Char(value) => self.lower_literal_expression(
                &expr.span,
                expr.type_id,
                HirExpressionKind::Char(*value),
            ),

            ExpressionKind::StringSlice(value) => self.lower_literal_expression(
                &expr.span,
                expr.type_id,
                HirExpressionKind::StringLiteral(self.string_table.resolve(*value).to_owned()),
            ),

            ExpressionKind::StructuralString { pieces } => {
                #[cfg(feature = "benchmark_counters")]
                increment_frontend_counter(FrontendCounter::CensusHirStructuralPiecesClonesSampled);
                let region = self.current_region_or_error(&expr.span)?;
                let ty = self.lower_type_id(expr.type_id, &expr.span)?;
                let pieces = self
                    .module
                    .expressions
                    .append_string_pieces(pieces, expr.span)?;
                Ok(LoweredExpression {
                    prelude: vec![],
                    value: self.make_expression(
                        &expr.span,
                        HirExpressionKind::StructuralString { pieces },
                        ty,
                        ValueKind::Const,
                        region,
                    )?,
                })
            }
            ExpressionKind::Cast(cast) => {
                self.lower_cast_expression(cast, root_cast_result_type_id, &expr.span)
            }

            ExpressionKind::Reference(name) => {
                self.lower_reference_expression(name, expr.type_id, &expr.span)
            }

            ExpressionKind::Copy(place) => {
                let region = self.current_region_or_error(&expr.span)?;
                let (prelude, place) = self.lower_place_expression_to_hir_place(place)?;
                let ty = self.lower_type_id(expr.type_id, &expr.span)?;

                Ok(LoweredExpression {
                    prelude,
                    value: self.make_expression(
                        &expr.span,
                        HirExpressionKind::Copy(place),
                        ty,
                        ValueKind::RValue,
                        region,
                    )?,
                })
            }

            ExpressionKind::Runtime(nodes) => {
                // Runtime RPN leaves keep their own spans; generated numeric/branch
                // scaffolding stays spanless and the root span is restored below.
                self.lower_runtime_rpn_expression(nodes, &expr.span, expr.type_id)
            }

            ExpressionKind::FieldAccess { base, field } => {
                self.lower_field_access_expression(base, *field, expr.type_id, &expr.span)
            }

            ExpressionKind::MethodCall {
                receiver,
                method_path,
                args,
                result_type_ids,
                span,
            } => self.lower_receiver_method_call_expression(
                method_path,
                receiver,
                args,
                result_type_ids,
                span,
            ),

            ExpressionKind::CollectionBuiltinCall {
                receiver,
                op,
                args,
                result_type_ids,
                span,
            } => self.lower_collection_builtin_call_expression(
                *op,
                receiver,
                args,
                result_type_ids,
                span,
            ),

            ExpressionKind::MapBuiltinCall {
                receiver,
                op,
                receiver_requires_mutable,
                args,
                result_type_ids,
                span,
            } => self.lower_map_builtin_call_expression(
                *op,
                receiver,
                *receiver_requires_mutable,
                args,
                result_type_ids,
                span,
            ),

            ExpressionKind::FunctionCall {
                name,
                args,
                result_type_ids,
            } => {
                let target = self.resolve_call_target_or_error(name, &expr.span)?;
                self.lower_call_expression(target, args, result_type_ids, &expr.span)
            }

            ExpressionKind::HandledFallibleFunctionCall {
                name,
                args,
                result_type_ids,
                handling,
                ..
            } => {
                let target = self.resolve_call_target_or_error(name, &expr.span)?;
                let propagation_span = expr.propagation_span().or(expr.span);
                self.lower_handled_fallible_call_expression(
                    target,
                    args,
                    result_type_ids,
                    handling,
                    &expr.span,
                    &propagation_span,
                )
            }

            ExpressionKind::HandledFallibleHostFunctionCall {
                id,
                args,
                result_type_ids,
                error_type_id,
                handling,
                ..
            } => {
                let propagation_span = expr.propagation_span().or(expr.span);
                self.lower_handled_external_fallible_call_expression(
                    ExternalFallibleCallLoweringInput {
                        id: *id,
                        args,
                        result_type_ids,
                        error_type_id: *error_type_id,
                        handling,
                        call_span: &expr.span,
                        propagation_span: &propagation_span,
                    },
                )
            }

            #[cfg(test)]
            ExpressionKind::FallibleCarrierConstruct { variant, value } => {
                let mut prelude = Vec::new();
                let lowered_value = self.lower_child_expression_for_parent(&mut prelude, value)?;
                let region = self.current_region_or_error(&expr.span)?;
                let ty = self.lower_type_id(expr.type_id, &expr.span)?;
                let variant_index = match variant {
                    AstFallibleCarrierVariant::Success => 0,
                    AstFallibleCarrierVariant::Error => 1,
                };
                let value_name = self.string_table.intern("value");
                let fields = [HirVariantField {
                    name: Some(value_name),
                    value: lowered_value,
                }];
                let field_range = self
                    .module
                    .expressions
                    .append_variant_fields(&fields, expr.span)?;

                Ok(LoweredExpression {
                    prelude,
                    value: self.make_expression(
                        &expr.span,
                        HirExpressionKind::VariantConstruct {
                            carrier: HirVariantCarrier::Fallible,
                            variant_index,
                            fields: field_range,
                        },
                        ty,
                        ValueKind::RValue,
                        region,
                    )?,
                })
            }

            ExpressionKind::HandledFallibleExpression {
                value, handling, ..
            } => {
                let propagation_span = expr.propagation_span().or(expr.span);
                self.lower_handled_fallible_expression(
                    value,
                    handling,
                    &expr.span,
                    &propagation_span,
                    expr.type_id,
                )
            }

            ExpressionKind::OptionPropagation { value } => {
                self.lower_option_expression_to_present_value(value, &expr.span)
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
                        &expr.span,
                    )
                } else {
                    self.lower_call_expression(
                        CallTarget::External(*host_id),
                        args,
                        result_type_ids,
                        &expr.span,
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

                let region = self.current_region_or_error(&expr.span)?;
                let ty = self.lower_type_id(expr.type_id, &expr.span)?;
                let item_range = self
                    .module
                    .expressions
                    .append_values(&lowered_items, expr.span)?;

                Ok(LoweredExpression {
                    prelude,
                    value: self.make_expression(
                        &expr.span,
                        HirExpressionKind::Collection(item_range),
                        ty,
                        ValueKind::RValue,
                        region,
                    )?,
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
                let region = self.current_region_or_error(&expr.span)?;
                let ty = self.lower_type_id(expr.type_id, &expr.span)?;
                let entry_range = self
                    .module
                    .expressions
                    .append_map_entries(&hir_entries, expr.span)?;
                Ok(LoweredExpression {
                    prelude,
                    value: self.make_expression(
                        &expr.span,
                        HirExpressionKind::MapLiteral(entry_range),
                        ty,
                        ValueKind::RValue,
                        region,
                    )?,
                })
            }

            ExpressionKind::Range(start, end) => {
                let mut prelude = Vec::new();
                let lowered_start = self.lower_child_expression_for_parent(&mut prelude, start)?;
                let lowered_end = self.lower_child_expression_for_parent(&mut prelude, end)?;

                let region = self.current_region_or_error(&expr.span)?;
                let ty = self.lower_type_id(expr.type_id, &expr.span)?;

                Ok(LoweredExpression {
                    prelude,
                    value: self.make_expression(
                        &expr.span,
                        HirExpressionKind::Range {
                            start: lowered_start,
                            end: lowered_end,
                        },
                        ty,
                        ValueKind::RValue,
                        region,
                    )?,
                })
            }

            ExpressionKind::AnonymousConstRecord { .. } => {
                // INVARIANT: anonymous const records are compile-time values. Runtime use is
                // rejected at parse time and folding consumes the record, so whole-record
                // runtime lowering should never be requested.
                return_hir_transformation_error!(
                    "HIR invariant: anonymous const record reached runtime HIR lowering; field access should select a member before HIR generation",
                    self.hir_error_location(&expr.span)
                );
            }
            ExpressionKind::StructInstance(args) => {
                // INVARIANT: const-record runtime use should have been rejected in AST.
                // If a const record reaches HIR struct lowering, push validation earlier
                // instead of converting this into a user diagnostic here.
                if expr.is_const_record_value() {
                    return_hir_transformation_error!(
                        "HIR invariant: Const record reached runtime HIR struct lowering; field access should select a member before HIR generation",
                        self.hir_error_location(&expr.span)
                    );
                }

                let Some(nominal_path) = self.type_environment.nominal_path(expr.type_id) else {
                    return_hir_transformation_error!(
                        "Struct instance reached HIR lowering without a nominal struct identity",
                        self.hir_error_location(&expr.span)
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
                        &expr.span,
                    )?
                } else {
                    self.resolve_struct_id_from_nominal_path(&nominal_path, &expr.span)?
                };
                let mut prelude = Vec::new();
                let mut fields = Vec::with_capacity(args.len());

                for arg in args {
                    let field_id =
                        self.resolve_field_id_or_error(struct_id, &arg.id, &expr.span)?;
                    let lowered_value =
                        self.lower_child_expression_for_parent(&mut prelude, &arg.value)?;
                    fields.push((field_id, lowered_value));
                }

                let region = self.current_region_or_error(&expr.span)?;
                let ty = self.lower_type_id(expr.type_id, &expr.span)?;
                let field_range = self
                    .module
                    .expressions
                    .append_struct_fields(&fields, expr.span)?;

                Ok(LoweredExpression {
                    prelude,
                    value: self.make_expression(
                        &expr.span,
                        HirExpressionKind::StructConstruct {
                            struct_id,
                            fields: field_range,
                        },
                        ty,
                        ValueKind::RValue,
                        region,
                    )?,
                })
            }

            ExpressionKind::Template(_) => {
                // INVARIANT: AST finalization replaces every runtime template with an
                // owned handoff payload before HIR. A raw `ExpressionKind::Template`
                // reaching this dispatcher means AST failed to finish its job.
                return_hir_transformation_error!(
                    "Raw template reached HIR runtime-template lowering after AST finalization.",
                    self.hir_error_location(&expr.span)
                )
            }

            ExpressionKind::RuntimeTemplateHandoff(handoff) => self
                .lower_runtime_template_expression_from_owned_handoff(handoff.as_ref(), &expr.span),

            ExpressionKind::RuntimeSlotApplicationHandoff(handoff) => self
                .lower_runtime_slot_application_expression_from_owned_handoff(
                    handoff.as_ref(),
                    &expr.span,
                ),

            // Lower the inner value and override the HIR type with the declared
            // coercion target. Option coercions materialize `some(value)` here so
            // backends see the real runtime carrier. Implicit numeric promotions
            // (`Int`/`Uint` to `Float`) materialize as an explicit infallible
            // `Cast` instead: backends select the value conversion from the cast
            // policy, so a bare type override would leave the source carrier
            // (notably a BigInt) under the target type.
            ExpressionKind::Coerced { value, .. } => {
                let mut prelude = Vec::new();
                let mut lowered_value =
                    self.lower_child_expression_for_parent(&mut prelude, value)?;
                let coerced_ty = self.lower_type_id(expr.type_id, &expr.span)?;
                let lowered_type = self.module.expressions.expression(lowered_value).ty;
                if self.type_environment.option_inner_type(expr.type_id) == Some(lowered_type) {
                    let value_name = self.string_table.intern("value");
                    let region = self.module.expressions.expression(lowered_value).region;
                    let fields = [HirVariantField {
                        name: Some(value_name),
                        value: lowered_value,
                    }];
                    let field_range = self
                        .module
                        .expressions
                        .append_variant_fields(&fields, expr.span)?;
                    lowered_value = self.make_expression(
                        &expr.span,
                        HirExpressionKind::VariantConstruct {
                            carrier: HirVariantCarrier::Option,
                            variant_index: 1,
                            fields: field_range,
                        },
                        coerced_ty,
                        ValueKind::RValue,
                        region,
                    )?;
                    return Ok(LoweredExpression {
                        prelude,
                        value: lowered_value,
                    });
                }

                let lowered_type = self.module.expressions.expression(lowered_value).ty;
                let source_scalar =
                    NumericScalar::from_type_id(lowered_type, &self.type_environment);
                let target_scalar = NumericScalar::from_type_id(coerced_ty, &self.type_environment);
                if let (Some(source), Some(target)) = (source_scalar, target_scalar)
                    && source != target
                {
                    let converted =
                        self.convert_numeric_operand_to_domain(lowered_value, target, &expr.span)?;
                    return Ok(LoweredExpression {
                        prelude,
                        value: converted,
                    });
                }

                lowered_value = self.replace_expression_metadata(
                    lowered_value,
                    &expr.span,
                    Some(coerced_ty),
                    None,
                )?;
                Ok(LoweredExpression {
                    prelude,
                    value: lowered_value,
                })
            }

            ExpressionKind::Function(_) => {
                return_hir_transformation_error!(
                    "Function expressions are not lowered in this phase",
                    self.hir_error_location(&expr.span)
                )
            }

            ExpressionKind::StructDefinition(_) => {
                return_hir_transformation_error!(
                    "Struct definition expressions are not lowered in this phase",
                    self.hir_error_location(&expr.span)
                )
            }

            ExpressionKind::ValueBlock { block } => {
                // Value blocks are composite control flow; branch scaffolding stays
                // spanless and the root span is restored below.
                self.lower_value_block(block, &expr.span, expr.type_id)
            }

            ExpressionKind::NoValue => {
                let region = self.current_region_or_error(&expr.span)?;
                Ok(LoweredExpression {
                    prelude: vec![],
                    value: self.unit_expression(&expr.span, region)?,
                })
            }

            ExpressionKind::OptionNone => {
                let region = self.current_region_or_error(&expr.span)?;
                let ty = self.lower_type_id(expr.type_id, &expr.span)?;
                let field_range = self
                    .module
                    .expressions
                    .append_variant_fields(&[], expr.span)?;
                Ok(LoweredExpression {
                    prelude: vec![],
                    value: self.make_expression(
                        &expr.span,
                        HirExpressionKind::VariantConstruct {
                            carrier: HirVariantCarrier::Option,
                            variant_index: 0,
                            fields: field_range,
                        },
                        ty,
                        ValueKind::RValue,
                        region,
                    )?,
                })
            }
        }?;

        // The root HIR value represents this authored AST expression. Child values retain
        // their own spans; constructors used for compiler scaffolding remain span-free. Some
        // lowering paths return a generated root value, so restoring its authored span must also
        // install the corresponding side-table mappings.
        lowered.value = self.replace_expression_metadata(lowered.value, &expr.span, None, None)?;
        self.log_expression_output(expr, lowered.value);
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
    ) -> Result<HirValueId, HirConstructionFailure> {
        if let Some(success_value) =
            self.lower_fallible_expression_to_success_value(expr, &expr.span)?
        {
            return Ok(success_value);
        }

        let lowered = self.lower_expression(expr)?;
        for prelude in lowered.prelude {
            self.emit_statement_to_current_block(prelude, &expr.span)?;
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
    ) -> Result<HirValueId, HirConstructionFailure> {
        if self.expression_needs_current_block_lowering(expr) {
            for prelude in pending_prelude.drain(..) {
                self.emit_statement_to_current_block(prelude, &expr.span)?;
            }

            return self.lower_expression_value_to_current_block(expr);
        }

        let lowered = self.lower_expression(expr)?;
        pending_prelude.extend(lowered.prelude);
        Ok(lowered.value)
    }

    pub(crate) fn expression_needs_current_block_lowering(&self, expr: &Expression) -> bool {
        // A recovering producer routes its error edge to the enclosing handler while it
        // lowers, so it emits into the current block instead of returning a pending
        // prelude. Earlier sibling operands must flush first to keep source order.
        let recovers_into_active_handler = self.active_catch_handler.is_some();
        match &expr.kind {
            ExpressionKind::HandledFallibleFunctionCall { args, handling, .. }
            | ExpressionKind::HandledFallibleHostFunctionCall { args, handling, .. } => {
                matches!(handling, FallibleExpressionHandling::Propagate)
                    || (recovers_into_active_handler
                        && matches!(handling, FallibleExpressionHandling::Recover))
                    || args
                        .iter()
                        .any(|arg| self.expression_needs_current_block_lowering(&arg.value))
            }
            ExpressionKind::HandledFallibleExpression {
                value, handling, ..
            } => {
                matches!(handling, FallibleExpressionHandling::Propagate)
                    || (recovers_into_active_handler
                        && matches!(handling, FallibleExpressionHandling::Recover))
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
            | ExpressionKind::AnonymousConstRecord { fields }
            | ExpressionKind::ChoiceConstruct { fields, .. } => fields
                .iter()
                .any(|field| self.expression_needs_current_block_lowering(&field.value)),
            ExpressionKind::Runtime(nodes) => {
                self.active_catch_handler.is_some()
                    || nodes.items.iter().any(|item| match item {
                        ExpressionRpnItem::Operand(expression) => {
                            self.expression_needs_current_block_lowering(expression)
                        }
                        ExpressionRpnItem::Operator { .. } => false,
                        // Pending syntax never survives evaluation; flag it like the completed
                        // lowering tree builder does so a debug build fails at the first invalid
                        // boundary instead of silently treating it as lowering-inert.
                        ExpressionRpnItem::PendingNumericLiteral { .. }
                        | ExpressionRpnItem::PendingGroup { .. } => {
                            debug_assert!(
                                false,
                                "pending expression syntax reached HIR lowering query"
                            );
                            false
                        }
                    })
            }
            ExpressionKind::Template(_) => true,
            ExpressionKind::RuntimeTemplateHandoff(_)
            | ExpressionKind::RuntimeSlotApplicationHandoff(_) => true,
            ExpressionKind::ValueBlock { .. } => true,

            ExpressionKind::Uint(_)
            | ExpressionKind::Int(_)
            | ExpressionKind::Float(_)
            | ExpressionKind::FixedScalar(_)
            | ExpressionKind::Number(_)
            | ExpressionKind::Bool(_)
            | ExpressionKind::Char(_)
            | ExpressionKind::StringSlice(_)
            | ExpressionKind::StructuralString { .. }
            | ExpressionKind::Reference(_)
            | ExpressionKind::Function(_)
            | ExpressionKind::StructDefinition(_)
            | ExpressionKind::NoValue
            | ExpressionKind::OptionNone => false,
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
        span: &Option<SourceSpan>,
        result_type_id: TypeId,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        match block {
            ValueBlock::If(value_if) => self.lower_value_block_if(value_if, span, result_type_id),
            ValueBlock::LexicalScope(value_lexical_scope) => {
                self.lower_value_lexical_scope(value_lexical_scope, span, result_type_id)
            }
            ValueBlock::Match(value_match) => {
                self.lower_value_block_match(value_match, span, result_type_id)
            }
            ValueBlock::Catch(value_catch) => {
                self.lower_value_block_catch(value_catch, span, result_type_id)
            }
        }
    }

    /// Installs one continuation before lowering the entire protected expression.
    fn lower_value_block_catch(
        &mut self,
        value_catch: &ValueCatchBlock,
        span: &Option<SourceSpan>,
        result_type_id: TypeId,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        let protected = &value_catch.handled_value;
        if protected.failure_facts.is_statically_infallible_catch() {
            // AST validated the authored handler. Converged facts show no error edge can run,
            // so only the protected value remains executable HIR.
            return self.lower_expression(protected);
        }
        // The parser owns catch compatibility; its validated disposition, not the protected
        // root shape, decides which payload arrives in the handler.
        let FailureDisposition::HandledByCatch { error_type_id } =
            protected.failure_facts.disposition
        else {
            return_hir_transformation_error!(
                "Value catch block did not contain a recoverable expression",
                self.hir_error_location(span)
            );
        };
        let err_type = self.lower_type_id(error_type_id, span)?;

        // The direct catch root joins cast target values before applying an optional receiving
        // wrap. Nested casts keep their receiving type because a parent expression may need it.
        let cast_with_optional_receiving_wrap = match &protected.kind {
            ExpressionKind::Cast(cast) if cast.requires_optional_wrap_after_cast => Some(cast),
            _ => None,
        };
        let inner_result_type_ids;
        let result_type_ids = if let Some(cast) = cast_with_optional_receiving_wrap {
            inner_result_type_ids = self.handled_expression_result_type_ids(cast.target_type_id);
            &inner_result_type_ids
        } else {
            &value_catch.result_type_ids
        };
        let lowered = self.lower_fallible_carrier_with_branching(
            FallibleBranchingContext {
                result_type_ids,
                handling: &value_catch.handler,
                err_type,
                span,
            },
            // Every protected shape lowers through the ordinary expression path while the
            // handler is active; producers route their error edges to that handler themselves.
            |builder| match cast_with_optional_receiving_wrap {
                Some(cast) => builder
                    .lower_expression_with_root_cast_result_type(protected, cast.target_type_id),
                None => builder.lower_expression(protected),
            },
        )?;
        if cast_with_optional_receiving_wrap.is_some() {
            let value =
                self.wrap_cast_result_optional_if_needed(lowered.value, result_type_id, span)?;
            return Ok(LoweredExpression {
                prelude: lowered.prelude,
                value,
            });
        }
        Ok(lowered)
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
        span: &Option<SourceSpan>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        match &cast.evidence {
            ResolvedCastEvidence::Builtin { policy } => match (cast.fallibility, cast.handling) {
                (BuiltinCastFallibility::Infallible, _) => {
                    self.lower_infallible_builtin_cast_expression(cast, *policy, expr_type_id, span)
                }
                (BuiltinCastFallibility::Fallible, CastHandling::StoreConversion) => {
                    self.lower_store_conversion_cast_expression(cast, *policy, expr_type_id, span)
                }
                (
                    BuiltinCastFallibility::Fallible,
                    CastHandling::Implicit | CastHandling::Recover,
                ) => self.lower_fallible_builtin_cast_expression(cast, *policy, expr_type_id, span),
            },
            ResolvedCastEvidence::UserDefined { method_path, .. } => {
                self.lower_user_defined_cast_expression(cast, method_path, expr_type_id, span)
            }
            ResolvedCastEvidence::GenericBound { .. } => Err(CompilerError::new(
                "Generic-bound cast evidence reached HIR lowering",
                self.hir_error_location(span),
                crate::compiler_frontend::compiler_errors::ErrorType::HirTransformation,
            )
            .into()),
        }
    }

    /// Lowers an infallible builtin cast as a pure HIR expression.
    fn lower_infallible_builtin_cast_expression(
        &mut self,
        cast: &ResolvedCastExpression,
        policy: BuiltinCastPolicyId,
        expr_type_id: FrontendTypeId,
        span: &Option<SourceSpan>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        let mut prelude = Vec::new();
        let source = self.lower_child_expression_for_parent(&mut prelude, &cast.source)?;

        // `Float -> String` is infallible at the source level because valid Moth `Float` is
        // finite, but it must still lower through the shared `FormatFloat` statement so casts and
        // templates use the same Moth-owned formatter. Fixed binary floats take the ordinary cast
        // path: their string conversion is a numeric text policy the backends lower or reject.
        if policy == BuiltinCastPolicyId::NumericToString(NumericScalar::Float) {
            for prelude_statement in prelude.drain(..) {
                self.emit_statement_to_current_block(prelude_statement, span)?;
            }

            let formatted = self.emit_formatted_float_value(source, span)?;
            let value = self.wrap_cast_result_optional_if_needed(formatted, expr_type_id, span)?;
            return Ok(LoweredExpression {
                prelude: vec![],
                value,
            });
        }

        let target_type = self.lower_type_id(cast.target_type_id, span)?;
        let region = self.current_region_or_error(span)?;
        let value = self.make_expression(
            span,
            HirExpressionKind::Cast { source, policy },
            target_type,
            ValueKind::RValue,
            region,
        )?;
        let value = self.wrap_cast_result_optional_if_needed(value, expr_type_id, span)?;

        Ok(LoweredExpression { prelude, value })
    }

    /// Emits the fallible builtin-cast carrier used by propagation, recovery, and store conversion.
    fn emit_builtin_cast_carrier(
        &mut self,
        cast: &ResolvedCastExpression,
        policy: BuiltinCastPolicyId,
        span: &Option<SourceSpan>,
    ) -> Result<EmittedFallibleCarrier, HirConstructionFailure> {
        let lowered_source = self.lower_expression(&cast.source)?;
        for prelude_statement in lowered_source.prelude {
            self.emit_statement_to_current_block(prelude_statement, span)?;
        }

        let ok_type = self.lower_type_id(cast.target_type_id, span)?;
        let err_type = self.builtin_error_type_id(span)?;
        let carrier_type = self
            .type_environment
            .intern_fallible_carrier(ok_type, err_type);
        let result_local = self.allocate_temp_local(carrier_type, None)?;

        let cast_statement = HirStatement {
            id: self.allocate_node_id(),
            kind: HirStatementKind::CastOp {
                policy,
                source: lowered_source.value,
                result: Some(HirLocalDestination::Define(result_local)),
            },
            span: *span,
        };
        self.side_table.map_statement(*span, &cast_statement);
        self.emit_statement_to_current_block(cast_statement, span)?;

        Ok(EmittedFallibleCarrier {
            result_local,
            carrier_type,
            ok_type,
            err_type,
            validate_float_success: false,
        })
    }

    /// Lowers a compiler-inserted numeric conversion at a compound-assignment store.
    ///
    /// WHAT: emits the checked builtin cast carrier and selects the enclosing function's numeric
    ///       failure edge before returning the success payload.
    /// WHY: this conversion belongs to the assignment write-back boundary, so its failure stays
    ///      attached to the assignment and the write is committed only on success.
    fn lower_store_conversion_cast_expression(
        &mut self,
        cast: &ResolvedCastExpression,
        policy: BuiltinCastPolicyId,
        expr_type_id: FrontendTypeId,
        span: &Option<SourceSpan>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        let carrier = self.emit_builtin_cast_carrier(cast, policy, span)?;
        let failure_mode = self.select_numeric_failure_mode(span)?;
        let success_value =
            self.lower_store_conversion_carrier_to_success_value(carrier, failure_mode, span)?;
        let value = self.wrap_cast_result_optional_if_needed(success_value, expr_type_id, span)?;

        Ok(LoweredExpression {
            prelude: vec![],
            value,
        })
    }

    /// Lowers a fallible builtin cast through an explicit carrier statement and branches.
    fn lower_fallible_builtin_cast_expression(
        &mut self,
        cast: &ResolvedCastExpression,
        policy: BuiltinCastPolicyId,
        expr_type_id: FrontendTypeId,
        span: &Option<SourceSpan>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        let carrier = self.emit_builtin_cast_carrier(cast, policy, span)?;

        let success_value = match cast.handling {
            CastHandling::Implicit => {
                self.lower_cast_carrier_with_implicit_delivery(carrier, span)?
            }
            CastHandling::Recover => {
                if self.active_catch_handler.is_none() {
                    return_hir_transformation_error!(
                        "Recovering builtin cast reached HIR outside a value catch block",
                        self.hir_error_location(span)
                    );
                }
                self.lower_carrier_to_active_catch_success(carrier, span)?
            }
            CastHandling::StoreConversion => return_hir_transformation_error!(
                "Store conversion cast bypassed compound-assignment lowering",
                self.hir_error_location(span)
            ),
        };
        let value = self.wrap_cast_result_optional_if_needed(success_value, expr_type_id, span)?;
        Ok(LoweredExpression {
            prelude: vec![],
            value,
        })
    }

    fn lower_cast_carrier_with_implicit_delivery(
        &mut self,
        carrier: EmittedFallibleCarrier,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        if self.active_catch_handler.is_some() {
            return self.lower_carrier_to_active_catch_success(carrier, span);
        }
        let failure_mode = self.select_numeric_failure_mode(span)?;
        self.lower_implicit_cast_carrier_to_success_value(carrier, failure_mode, span)
    }
    /// Lowers a user-defined cast by calling the selected evidence method.
    fn lower_user_defined_cast_expression(
        &mut self,
        cast: &ResolvedCastExpression,
        method_path: &PathId,
        expr_type_id: FrontendTypeId,
        span: &Option<SourceSpan>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        let call_target = self.resolve_call_target_or_error(method_path, span)?;
        let source_argument =
            CallArgument::positional((*cast.source).clone(), CallAccessMode::Shared, *span);

        match (cast.fallibility, cast.handling) {
            (
                BuiltinCastFallibility::Infallible,
                CastHandling::Implicit | CastHandling::Recover,
            ) => {
                let result_type_ids = vec![cast.target_type_id];
                let lowered = self.lower_call_expression(
                    call_target,
                    &[source_argument],
                    &result_type_ids,
                    span,
                )?;
                let value =
                    self.wrap_cast_result_optional_if_needed(lowered.value, expr_type_id, span)?;
                Ok(LoweredExpression {
                    prelude: lowered.prelude,
                    value,
                })
            }
            (BuiltinCastFallibility::Fallible, CastHandling::Implicit) => {
                let carrier =
                    self.emit_user_defined_cast_call_carrier(call_target, &source_argument, span)?;
                let success_value =
                    self.lower_cast_carrier_with_implicit_delivery(carrier, span)?;
                let value =
                    self.wrap_cast_result_optional_if_needed(success_value, expr_type_id, span)?;
                Ok(LoweredExpression {
                    prelude: vec![],
                    value,
                })
            }
            (BuiltinCastFallibility::Fallible, CastHandling::Recover) => {
                if self.active_catch_handler.is_none() {
                    return_hir_transformation_error!(
                        "Recovering user-defined cast reached HIR outside a value catch block",
                        self.hir_error_location(span)
                    );
                }
                let carrier =
                    self.emit_user_defined_cast_call_carrier(call_target, &source_argument, span)?;
                let success_value = self.lower_carrier_to_active_catch_success(carrier, span)?;
                let value =
                    self.wrap_cast_result_optional_if_needed(success_value, expr_type_id, span)?;
                Ok(LoweredExpression {
                    prelude: vec![],
                    value,
                })
            }
            (_, CastHandling::StoreConversion) => return_hir_transformation_error!(
                "Store conversion cast reached HIR with non-builtin evidence",
                self.hir_error_location(span)
            ),
        }
    }

    /// Emits a user-defined cast method call that returns a fallible carrier.
    fn emit_user_defined_cast_call_carrier(
        &mut self,
        target: CallTarget,
        source_argument: &CallArgument,
        span: &Option<SourceSpan>,
    ) -> Result<EmittedFallibleCarrier, HirConstructionFailure> {
        let (carrier_type, ok_type, err_type) = self.result_call_carrier_slots(&target, span)?;

        let lowered_argument = self.lower_call_argument_value(source_argument, span, 0)?;
        for prelude_statement in lowered_argument.prelude {
            self.emit_statement_to_current_block(prelude_statement, span)?;
        }

        let result_local = self.allocate_temp_local(carrier_type, None)?;
        let call_statement = HirStatement {
            id: self.allocate_node_id(),
            kind: HirStatementKind::Call {
                target,
                args: self
                    .module
                    .expressions
                    .append_values(&[lowered_argument.value], *span)?,
                result: Some(HirLocalDestination::Define(result_local)),
            },
            span: *span,
        };
        self.side_table.map_statement(*span, &call_statement);
        self.emit_statement_to_current_block(call_statement, span)?;

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
        value: HirValueId,
        expr_type_id: FrontendTypeId,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let expected_type = self.lower_type_id(expr_type_id, span)?;
        let (source_type, region) = {
            let source = self.module.expressions.expression(value);
            (source.ty, source.region)
        };
        if source_type == expected_type {
            return Ok(value);
        }

        if self.type_environment.option_inner_type(expected_type) != Some(source_type) {
            return Err(CompilerError::new(
                format!(
                    "Cast result type {:?} cannot be wrapped to expected optional type {:?}",
                    source_type, expected_type
                ),
                self.hir_error_location(span),
                crate::compiler_frontend::compiler_errors::ErrorType::HirTransformation,
            )
            .into());
        }

        let value_name = self.string_table.intern("value");
        let fields = [HirVariantField {
            name: Some(value_name),
            value,
        }];
        let field_range = self
            .module
            .expressions
            .append_variant_fields(&fields, *span)?;
        self.make_expression(
            span,
            HirExpressionKind::VariantConstruct {
                carrier: HirVariantCarrier::Option,
                variant_index: OPTION_SOME_VARIANT_INDEX,
                fields: field_range,
            },
            expected_type,
            ValueKind::RValue,
            region,
        )
    }

    /// Resolves the builtin `Error` type id for fallible carrier error slots.
    fn builtin_error_type_id(
        &mut self,
        span: &Option<SourceSpan>,
    ) -> Result<TypeId, HirConstructionFailure> {
        Ok(type_id_for_builtin_target(
            BuiltinCastTarget::Error,
            &self.type_environment,
            self.string_table,
            self.path_fork,
        )
        .ok_or_else(|| {
            CompilerError::new(
                "Builtin Error type is not registered in the type environment",
                self.hir_error_location(span),
                crate::compiler_frontend::compiler_errors::ErrorType::HirTransformation,
            )
        })?)
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
        span: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let block = self.current_block_mut_or_error(span)?;
        block.statements.push(statement);
        Ok(())
    }

    // WHAT: emits one explicitly classified local write in the current block.
    // WHY: compiler-generated definitions and updates share statement emission but carry
    //      different binding semantics into analysis and backends.
    pub(crate) fn emit_write_statement(
        &mut self,
        target: HirWriteTarget,
        value: HirValueId,
        span: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let write_statement = HirStatement {
            id: self.allocate_node_id(),
            kind: HirStatementKind::Write { target, value },
            span: None,
        };

        self.side_table.map_statement(None, &write_statement);
        self.emit_statement_to_current_block(write_statement, span)
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
        source_info: Option<SourceSpan>,
    ) -> Result<LocalId, HirConstructionFailure> {
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
        source_info: Option<SourceSpan>,
        call_span: Option<SourceSpan>,
        argument_index: usize,
    ) -> Result<LocalId, HirConstructionFailure> {
        self.allocate_compiler_local(
            ty,
            source_info,
            HirLocalOriginKind::CompilerFreshMutableArg,
            call_span,
            Some(argument_index),
        )
    }

    fn allocate_compiler_local(
        &mut self,
        ty: TypeId,
        source_info: Option<SourceSpan>,
        origin: HirLocalOriginKind,
        call_span: Option<SourceSpan>,
        argument_index: Option<usize>,
    ) -> Result<LocalId, HirConstructionFailure> {
        let span = source_info;
        let region = self.current_region_or_error(&span)?;
        let block_id = self.current_block_id_or_error(&span)?;
        let local_id = self.allocate_local_id();

        let local = HirLocal {
            id: local_id,
            ty,
            mutable: true,
            region,
            span: source_info,
        };

        self.side_table.map_local_source(&local);
        self.register_local_in_block(block_id, local, &span)?;

        let temp_name = format!("__hir_tmp_{}", self.temp_local_counter);
        self.temp_local_counter += 1;
        let temp_component = self.string_table.intern(&temp_name);
        let temp_name_id = self
            .path_fork
            .try_intern_child(PathId::ROOT, temp_component)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "path table exhausted while naming a compiler-generated HIR local",
                )
            })?;

        // Compiler-introduced temporaries are intentionally excluded from AST symbol resolution.
        // They are named only for diagnostics/debug rendering via the side table.
        self.side_table.bind_local_name(local_id, temp_name_id);
        self.side_table
            .bind_local_origin(local_id, origin, call_span, argument_index);

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
        span: &Option<SourceSpan>,
    ) -> Result<&mut HirBlock, HirConstructionFailure> {
        let block_id = self.current_block_id_or_error(span)?;
        Ok(self.block_mut_by_id_or_error(block_id, span)?)
    }

    // WHAT: allocates one HIR expression node with its identity and typing metadata attached.
    // WHY: centralizing expression construction keeps IDs, source mappings, and value kinds
    //      uniform across every lowering helper.
    pub(crate) fn make_expression(
        &mut self,
        span: &Option<SourceSpan>,
        kind: HirExpressionKind,
        ty: TypeId,
        value_kind: ValueKind,
        region: RegionId,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let id = self.module.expressions.append_expression(HirExpression {
            kind,
            ty,
            value_kind,
            region,
            span: *span,
        })?;
        self.side_table.map_value(*span, id, *span);
        Ok(id)
    }

    // WHAT: returns an expression root with selected scalar metadata updated.
    // WHY: HIR rows are immutable because multiple edges may share an ID; changed metadata must
    //      receive a new row while keeping its source mapping attached to the new identity.
    pub(crate) fn replace_expression_metadata(
        &mut self,
        id: HirValueId,
        span: &Option<SourceSpan>,
        ty: Option<TypeId>,
        region: Option<RegionId>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let (ty, region, previous_span) = {
            let expression = self.module.expressions.get_expression(id).ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "HIR expression ID {:?} is outside its dense row store",
                    id
                ))
            })?;
            (
                ty.unwrap_or(expression.ty),
                region.unwrap_or(expression.region),
                expression.span,
            )
        };
        let replacement = self
            .module
            .expressions
            .copy_expression_with_metadata(id, *span, ty, region)?;
        if replacement != id {
            let (ast_span, source_span) = if previous_span == *span {
                (
                    self.side_table.value_ast_span(id).or(previous_span),
                    self.side_table.value_source_span(id).or(previous_span),
                )
            } else {
                (*span, *span)
            };
            self.side_table
                .map_value(ast_span, replacement, source_span);
        }
        Ok(replacement)
    }

    // WHAT: creates a canonical load expression for one local.
    // WHY: runtime/result branching paths frequently reconstruct this node shape and should share
    //      one helper for readability and consistency.
    pub(crate) fn make_local_load_expression(
        &mut self,
        local: LocalId,
        ty: TypeId,
        span: &Option<SourceSpan>,
        region: RegionId,
    ) -> Result<HirValueId, HirConstructionFailure> {
        self.make_expression(
            span,
            HirExpressionKind::Load(HirPlace::local(local)),
            ty,
            ValueKind::RValue,
            region,
        )
    }

    // WHAT: builds the canonical HIR representation of unit.
    // WHY: unit values should lower through the same tuple machinery every other pass expects.
    pub(crate) fn unit_expression(
        &mut self,
        span: &Option<SourceSpan>,
        region: RegionId,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let unit_ty = self.type_environment.builtins().none;
        self.make_expression(
            span,
            HirExpressionKind::TupleConstruct {
                elements: HirValueRange::empty(),
            },
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
        nominal_path: &PathId,
        span: &Option<SourceSpan>,
    ) -> Result<crate::compiler_frontend::hir::ids::ChoiceId, HirConstructionFailure> {
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
                    self.symbol_name_for_diagnostics(nominal_path)
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

        let hir_variants = self.lower_choice_variants_for_type_id(frontend_type_id, span)?;
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
        nominal_path: &PathId,
        span: &Option<SourceSpan>,
    ) -> Result<crate::compiler_frontend::hir::ids::ChoiceId, HirConstructionFailure> {
        let Some(choice_id) = self.choices_by_name.get(nominal_path).copied() else {
            return_hir_transformation_error!(
                format!(
                    "Choice '{}' was not pre-registered during HIR declaration preparation",
                    self.symbol_name_for_diagnostics(nominal_path)
                ),
                self.hir_error_location(span)
            );
        };
        Ok(choice_id)
    }

    // -------------------------
    //  Diagnostics & Logging
    // -------------------------

    // WHAT: preserves exact source-span provenance for HIR transformation errors.
    // WHY: HIR lowering uses one helper so all transformation errors preserve consistent source metadata.
    pub(crate) fn hir_error_location(&self, span: &Option<SourceSpan>) -> Option<SourceSpan> {
        *span
    }

    fn log_expression_input(&self, _expr: &Expression) {
        hir_log!(format!(
            "[HIR] Lowering expression {:?} @ {:?}",
            _expr.kind, _expr.span
        ));
    }

    fn log_expression_output(&self, _input: &Expression, _output: HirValueId) {
        hir_log!(format!(
            "[HIR] Lowered expression {:?} -> {}",
            _input.kind,
            crate::compiler_frontend::hir::hir_display::HirDisplayContext::new(
                self.string_table,
                self.path_fork,
            )
            .with_side_table(&self.side_table)
            .with_type_environment(&self.type_environment)
            .render_expression(&self.module.expressions, _output)
        ));
    }

    fn log_call_result_binding(
        &self,
        _span: &Option<SourceSpan>,
        _local: Option<LocalId>,
        _value: HirValueId,
    ) {
        hir_log!(format!(
            "[HIR] Emitted call binding @ {:?}: result={:?}, value={}",
            _span,
            _local,
            crate::compiler_frontend::hir::hir_display::HirDisplayContext::new(
                self.string_table,
                self.path_fork,
            )
            .with_side_table(&self.side_table)
            .with_type_environment(&self.type_environment)
            .render_expression(&self.module.expressions, _value)
        ));
    }
}
