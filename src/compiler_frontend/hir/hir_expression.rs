//! HIR Expression Lowering
//!
//! Lowers typed AST expressions into HIR expressions and statement preludes.
//! This file contains the high-level dispatcher and shared expression utilities on `HirBuilder`.
//!
//! ## Cast contract
//!
//! AST resolves all cast targets, evidence, fallibility, and optional wrapping flags before HIR.
//! HIR carries builtin casts as pure `Cast` expressions or native fallible `Invoke` operations.
//! User-defined cast evidence lowers to a direct user-function call
//! during HIR lowering, and `ResolvedCastEvidence::GenericBound` is validation-only and must not
//! reach HIR.
//!
//! ## Diagnostic boundary
//!
//! `CompilerError` / `return_hir_transformation_error!` in this module means an internal
//! HIR transformation or lowering invariant failure only. The construction failure lane also
//! carries source diagnostics when authored input exceeds a compact HIR store capacity; other
//! source failures are emitted by AST or earlier stages.

use crate::compiler_frontend::ast::expressions::expression_store::{
    ArgumentListView, CallView, CastFacts, CollectionCall, ExpressionListView,
    ExpressionOperationView, ExpressionView, FieldListView, HandlingRecord, MapCall, MapListView,
    PlaceView, ResolvedCallableTarget,
};
use crate::compiler_frontend::ast::expressions::expression_types::{
    CastHandling, FallibleExpressionHandling, ResolvedCastEvidence,
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
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirMapEntry, HirVariantCarrier, HirVariantField,
    OPTION_SOME_VARIANT_INDEX, ValueKind,
};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::hir_side_table::HirLocalOriginKind;
use crate::compiler_frontend::hir::ids::{HirValueId, LocalId, RegionId};
use crate::compiler_frontend::hir::module::HirChoice;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind, HirWriteTarget};
#[cfg(feature = "benchmark_counters")]
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::hir_log;
use crate::return_hir_transformation_error;

mod calls;
mod expression_walk;
mod fallible;
mod numeric;
mod operators;
mod option_propagation;
mod places;
mod runtime;
mod templates;
mod types;

use self::fallible::EmittedFallibleResults;
pub(crate) use self::fallible::FallibleBranchingContext;

#[derive(Debug, Clone)]
pub(crate) struct LoweredExpression {
    // WHAT: Statements that must execute before evaluating `value`.
    // WHY: HIR requires expression side effects to be linearized into explicit statements.
    pub prelude: Vec<HirStatement>,
    pub value: HirValueId,
}

/// Ordered producer values, with no tuple type or carrier value.
pub(crate) struct LoweredResults {
    pub prelude: Vec<HirStatement>,
    pub values: Vec<HirValueId>,
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
        expr: ExpressionView<'_>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        let lowered = self.lower_expression_results(expr)?;
        let value = self.single_result_value(lowered.values, &expr.span())?;
        Ok(LoweredExpression {
            prelude: lowered.prelude,
            value,
        })
    }

    pub(crate) fn lower_expression_results_to_current_block(
        &mut self,
        expr: ExpressionView<'_>,
    ) -> Result<Vec<HirValueId>, HirConstructionFailure> {
        let lowered = self.lower_expression_results(expr)?;
        for statement in lowered.prelude {
            self.emit_statement_to_current_block(statement, &expr.span())?;
        }
        Ok(lowered.values)
    }

    pub(crate) fn lower_expression_results(
        &mut self,
        expr: ExpressionView<'_>,
    ) -> Result<LoweredResults, HirConstructionFailure> {
        let types = expr.success_types();
        let span = &expr.span();
        let native_producer = matches!(
            expr.operation(),
            ExpressionOperationView::ValueBlock(_)
                | ExpressionOperationView::Call(_)
                | ExpressionOperationView::MethodCall { .. }
                | ExpressionOperationView::CollectionBuiltin { .. }
                | ExpressionOperationView::MapBuiltin { .. }
                | ExpressionOperationView::HandleFallible { .. }
        );
        if native_producer {
            #[cfg(feature = "benchmark_counters")]
            increment_frontend_counter(FrontendCounter::CensusHirLowerExpressionEntries);
            self.log_expression_input(expr);
            self.accumulate_function_provenance(expr);
        }
        let mut lowered = match expr.operation() {
            ExpressionOperationView::ValueBlock(block) => {
                self.lower_value_block(block, expr, span, types)?
            }
            ExpressionOperationView::Call(call) => {
                self.lower_bound_call_results(expr, call, None)?
            }
            ExpressionOperationView::MethodCall { receiver, call } => {
                self.lower_bound_call_results(expr, call, Some(receiver))?
            }
            ExpressionOperationView::CollectionBuiltin {
                receiver,
                facts,
                arguments,
            } if expr.error_type().is_none() => {
                self.lower_collection_builtin_results(receiver, facts, arguments, types, span)?
            }
            ExpressionOperationView::MapBuiltin {
                receiver,
                facts,
                arguments,
            } if expr.error_type().is_none() => {
                self.lower_map_builtin_results(receiver, facts, arguments, types, span)?
            }
            ExpressionOperationView::HandleFallible { value, handling } => {
                self.lower_handle_fallible_results(value, handling, span)?
            }
            _ if expr.error_type().is_some()
                && !matches!(expr.operation(), ExpressionOperationView::Cast { .. }) =>
            {
                self.lower_error_typed_expression_results(expr, span)?
            }
            _ => self.lower_default_expression_results(expr, types)?,
        };
        if native_producer {
            for value in &mut lowered.values {
                *value = self.replace_expression_metadata(*value, span, None, None)?;
                self.log_expression_output(expr, *value);
            }
        }
        Ok(lowered)
    }

    /// Lowers a collection builtin producer through the external call path.
    fn lower_collection_builtin_results(
        &mut self,
        receiver: ExpressionView<'_>,
        facts: &CollectionCall,
        arguments: ArgumentListView<'_>,
        types: &[TypeId],
        span: &Option<SourceSpan>,
    ) -> Result<LoweredResults, HirConstructionFailure> {
        self.lower_call_results(
            CallTarget::External(facts.operation.external_function_id()),
            arguments,
            Some(receiver),
            types,
            span,
        )
    }

    /// Lowers a map builtin producer, forwarding its receiver mutability contract.
    fn lower_map_builtin_results(
        &mut self,
        receiver: ExpressionView<'_>,
        facts: &MapCall,
        arguments: ArgumentListView<'_>,
        types: &[TypeId],
        span: &Option<SourceSpan>,
    ) -> Result<LoweredResults, HirConstructionFailure> {
        self.lower_map_builtin_call_results(
            facts.operation,
            receiver,
            facts.receiver_requires_mutable,
            arguments,
            types,
            span,
        )
    }

    /// Emits a fallible producer and finishes its explicit handling.
    fn lower_handle_fallible_results(
        &mut self,
        value: ExpressionView<'_>,
        handling: &HandlingRecord,
        span: &Option<SourceSpan>,
    ) -> Result<LoweredResults, HirConstructionFailure> {
        let results = self.emit_result_expression_to_current_block(value)?;
        self.finish_handled_fallible_results(
            results,
            &handling.handling,
            &handling.propagation_span.or(*span),
        )
    }

    /// Collects success values from an expression whose error branch is already fallible.
    fn lower_error_typed_expression_results(
        &mut self,
        expr: ExpressionView<'_>,
        span: &Option<SourceSpan>,
    ) -> Result<LoweredResults, HirConstructionFailure> {
        let results = self.emit_result_expression_to_current_block(expr)?;
        let values = if self.active_catch_handler.is_some() {
            self.lower_results_to_active_catch_success_values(results, span)?
        } else {
            self.lower_fallible_results_to_success_values(results, span)?
        };
        Ok(LoweredResults {
            prelude: Vec::new(),
            values,
        })
    }

    /// Lowers a plain expression as one producer value, or none when it has no success type.
    fn lower_default_expression_results(
        &mut self,
        expr: ExpressionView<'_>,
        types: &[TypeId],
    ) -> Result<LoweredResults, HirConstructionFailure> {
        let lowered = self.lower_expression_with_root_cast_result_type(expr, None)?;
        let values = if types.is_empty() {
            vec![]
        } else {
            vec![lowered.value]
        };
        Ok(LoweredResults {
            prelude: lowered.prelude,
            values,
        })
    }

    /// Keeps the call occurrence's native slot shape and executes its arguments once.
    fn lower_bound_call_results(
        &mut self,
        expression: ExpressionView<'_>,
        call: CallView<'_>,
        receiver: Option<ExpressionView<'_>>,
    ) -> Result<LoweredResults, HirConstructionFailure> {
        let metadata = call.metadata();
        let span = &expression.span();
        let types = expression.success_types();
        let target = match metadata.target {
            ResolvedCallableTarget::Function(path) => {
                self.resolve_call_target_or_error(&path, span)?
            }
            ResolvedCallableTarget::Host(id) => CallTarget::External(id),
        };
        if let Some(handling) = call.handling() {
            let results = match metadata.target {
                ResolvedCallableTarget::Host(id) => {
                    let error = metadata.error_type.or(handling.error_type).ok_or_else(|| {
                        CompilerError::compiler_error("Handled host call has no error contract")
                    })?;
                    self.emit_external_result_call_to_current_block(
                        id,
                        call.arguments(),
                        receiver,
                        types,
                        metadata.requires_external_float_validation,
                        error,
                        span,
                    )?
                }
                ResolvedCallableTarget::Function(_) => self.emit_result_call_to_current_block(
                    target,
                    call.arguments(),
                    receiver,
                    types,
                    span,
                )?,
            };
            return self.finish_handled_fallible_results(
                results,
                &handling.handling,
                &handling.propagation_span.or(*span),
            );
        }
        if expression.error_type().is_some() {
            let results = self.emit_result_expression_to_current_block(expression)?;
            let values = if self.active_catch_handler.is_some() {
                self.lower_results_to_active_catch_success_values(results, span)?
            } else {
                self.lower_fallible_results_to_success_values(results, span)?
            };
            return Ok(LoweredResults {
                prelude: Vec::new(),
                values,
            });
        }
        if let ResolvedCallableTarget::Host(id) = metadata.target
            && metadata.requires_external_float_validation
        {
            let lowered = self.lower_validated_external_call_expression(
                id,
                call.arguments(),
                receiver,
                types,
                span,
            )?;
            return Ok(LoweredResults {
                prelude: lowered.prelude,
                values: vec![lowered.value],
            });
        }
        self.lower_call_results(target, call.arguments(), receiver, types, span)
    }

    /// Lowers one expression through the shared entry while selecting its root cast result type.
    ///
    /// WHAT: preserves normal expression-entry bookkeeping and allows a direct catch-root cast
    ///       to contribute its inner target value to the catch join.
    /// WHY: the catch owner applies optional wrapping after that join; nested casts still use
    ///      their own AST receiving type through `lower_expression`.
    fn lower_expression_with_root_cast_result_type(
        &mut self,
        expr: ExpressionView<'_>,
        root_cast_result_type_id: Option<FrontendTypeId>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        if expression_walk::is_operator_expression(expr) {
            return self.lower_operator_expression(expr);
        }

        #[cfg(feature = "benchmark_counters")]
        increment_frontend_counter(FrontendCounter::CensusHirLowerExpressionEntries);
        self.log_expression_input(expr);
        self.accumulate_function_provenance(expr);

        // Constant content crosses into HIR only through the constant-store visitor.
        let mut lowered = match expr.operation() {
            ExpressionOperationView::Choice { tag, fields } => {
                self.lower_choice_expression(expr, tag, fields, None)
            }
            ExpressionOperationView::RouteConstruct {
                tag,
                fields,
                future_input,
            } => self.lower_choice_expression(expr, tag, fields, Some(future_input)),

            ExpressionOperationView::Constant(value) => {
                let value = self.lower_const_store_expression(value, &expr.span())?;
                Ok(LoweredExpression {
                    prelude: Vec::new(),
                    value,
                })
            }
            ExpressionOperationView::Cast { source, facts } => self.lower_cast_expression(
                source,
                facts,
                root_cast_result_type_id.unwrap_or(expr.value_type_id()?),
                &expr.span(),
            ),

            ExpressionOperationView::Reference(name) => {
                self.lower_reference_expression(&name, expr.value_type_id()?, &expr.span())
            }

            ExpressionOperationView::Copy(place) => self.lower_copy_expression(expr, place),

            ExpressionOperationView::NumericUnary { .. }
            | ExpressionOperationView::NumericBinary { .. }
            | ExpressionOperationView::LogicalNot { .. }
            | ExpressionOperationView::LogicalBinary { .. }
            | ExpressionOperationView::Compare { .. }
            | ExpressionOperationView::Range { .. } => {
                return_hir_transformation_error!(
                    "Operator expression reached the generic HIR dispatcher",
                    self.hir_error_location(&expr.span())
                );
            }

            ExpressionOperationView::FieldAccess { base, field } => {
                self.lower_field_access_expression(base, field, expr.value_type_id()?, &expr.span())
            }

            ExpressionOperationView::Call(_)
            | ExpressionOperationView::MethodCall { .. }
            | ExpressionOperationView::CollectionBuiltin { .. }
            | ExpressionOperationView::MapBuiltin { .. }
            | ExpressionOperationView::HandleFallible { .. } => {
                self.lower_single_result_expression(expr)
            }

            ExpressionOperationView::PropagateOption { value } => {
                self.lower_option_expression_to_present_value(value, &expr.span())
            }

            ExpressionOperationView::Collection(items) => {
                self.lower_collection_expression(expr, items)
            }

            ExpressionOperationView::Map(entries) => self.lower_map_expression(expr, entries),

            ExpressionOperationView::Struct { fields: args } => {
                self.lower_struct_expression(expr, args)
            }

            ExpressionOperationView::Template(_) => {
                // Finalization must eliminate construction-only template operations.
                return_hir_transformation_error!(
                    "Raw template reached HIR runtime-template lowering after AST finalization.",
                    self.hir_error_location(&expr.span())
                )
            }

            ExpressionOperationView::RuntimeTemplate(handoff) => self
                .lower_runtime_template_expression_from_owned_handoff(handoff, expr, &expr.span()),

            ExpressionOperationView::RuntimeSlotApplication(handoff) => self
                .lower_runtime_slot_application_expression_from_owned_handoff(
                    handoff,
                    expr,
                    &expr.span(),
                ),

            // Lower the inner value and override the HIR type with the declared
            // coercion target. Option coercions materialize `some(value)` here so
            // backends see the real runtime carrier. Contextual numeric promotions
            // materialize as an explicit infallible `Cast` instead: backends select
            // the value conversion from the cast policy, so a bare type override
            // would leave the source carrier under the target type.
            ExpressionOperationView::Coerce { value, .. } => {
                let mut prelude = Vec::new();
                let mut lowered_value =
                    self.lower_child_expression_for_parent(&mut prelude, value)?;
                let coerced_ty = self.lower_type_id(expr.value_type_id()?, &expr.span())?;
                let lowered_type = self.module.expressions.expression(lowered_value).ty;
                if self
                    .type_environment
                    .option_inner_type(expr.value_type_id()?)
                    == Some(lowered_type)
                {
                    let value_name = self.string_table.intern("value");
                    let region = self.module.expressions.expression(lowered_value).region;
                    let fields = [HirVariantField {
                        name: Some(value_name),
                        value: lowered_value,
                    }];
                    let field_range = self
                        .module
                        .expressions
                        .append_variant_fields(&fields, expr.span())?;
                    lowered_value = self.make_expression(
                        &expr.span(),
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
                    let converted = self.convert_numeric_operand_to_domain(
                        lowered_value,
                        target,
                        &expr.span(),
                    )?;
                    return Ok(LoweredExpression {
                        prelude,
                        value: converted,
                    });
                }

                lowered_value = self.replace_expression_metadata(
                    lowered_value,
                    &expr.span(),
                    Some(coerced_ty),
                    None,
                )?;
                Ok(LoweredExpression {
                    prelude,
                    value: lowered_value,
                })
            }

            ExpressionOperationView::ValueBlock(block) => {
                self.lower_value_block_expression(expr, block)
            }
        }?;

        // The root HIR value represents this authored AST expression. Child values retain
        // their own spans; constructors used for compiler scaffolding remain span-free. Some
        // lowering paths return a generated root value, so restoring its authored span must also
        // install the corresponding side-table mappings.
        lowered.value =
            self.replace_expression_metadata(lowered.value, &expr.span(), None, None)?;
        self.log_expression_output(expr, lowered.value);
        Ok(lowered)
    }

    /// Lowers an authored place read as an HIR copy of the projected place.
    fn lower_copy_expression(
        &mut self,
        expr: ExpressionView<'_>,
        place: PlaceView<'_>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        let region = self.current_region_or_error(&expr.span())?;
        let (prelude, place) = self.lower_place_expression_to_hir_place(place)?;
        let ty = self.lower_type_id(expr.value_type_id()?, &expr.span())?;

        Ok(LoweredExpression {
            prelude,
            value: self.make_expression(
                &expr.span(),
                HirExpressionKind::Copy(place),
                ty,
                ValueKind::RValue,
                region,
            )?,
        })
    }

    /// Lowers a call-like producer expression by taking its single result value.
    fn lower_single_result_expression(
        &mut self,
        expr: ExpressionView<'_>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        let results = self.lower_expression_results(expr)?;
        let value = self.single_result_value(results.values, &expr.span())?;
        Ok(LoweredExpression {
            prelude: results.prelude,
            value,
        })
    }

    /// Lowers a value block to its single result. Branch scaffolding stays spanless; the
    /// caller restores the authored root span.
    fn lower_value_block_expression(
        &mut self,
        expr: ExpressionView<'_>,
        block: &ValueBlock,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        let lowered = self.lower_value_block(block, expr, &expr.span(), expr.success_types())?;
        let value = self.single_result_value(lowered.values, &expr.span())?;
        Ok(LoweredExpression {
            prelude: lowered.prelude,
            value,
        })
    }

    /// Lowers an authored choice constructor or route into a variant carrier.
    fn lower_choice_expression(
        &mut self,
        expr: ExpressionView<'_>,
        tag: u32,
        fields: FieldListView<'_>,
        route_future_input: Option<bool>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        let type_id = expr.value_type_id()?;
        let nominal_path = self
            .type_environment
            .nominal_path(type_id)
            .ok_or_else(|| CompilerError::compiler_error("Choice has no nominal identity"))?
            .to_owned();
        let choice_id = if let Some(TypeIdentityKey::GenericInstance(key)) = self
            .type_environment
            .type_id_to_type_identity_key(expr.value_type_id()?)
        {
            self.resolve_or_register_generic_choice(
                &key,
                &nominal_path,
                expr.value_type_id()?,
                &expr.span(),
            )?
        } else {
            self.resolve_choice_id(&nominal_path, &expr.span())?
        };
        let region = self.current_region_or_error(&expr.span())?;
        let ty = self.lower_type_id(expr.value_type_id()?, &expr.span())?;

        let mut prelude = Vec::new();
        let mut hir_fields = Vec::with_capacity(fields.len());

        for field in fields.iter() {
            let value = self.lower_child_expression_for_parent(&mut prelude, field.value())?;
            hir_fields.push(HirVariantField {
                name: self.path_fork.component(field.name()),
                value,
            });
        }

        // WHY: classify const-ness from the already-lowered HIR field values
        //      instead of reaching back into AST no-store expression classification.
        //      Each field's `value_kind` is set during lowering and is the HIR-stage
        //      authority for whether that field is a compile-time constant.
        let fields_are_const = hir_fields.iter().all(|field| {
            self.module.expressions.expression(field.value).value_kind == ValueKind::Const
        });
        let value_kind = if route_future_input.is_none() && fields_are_const {
            ValueKind::Const
        } else {
            ValueKind::RValue
        };
        let field_range = self
            .module
            .expressions
            .append_variant_fields(&hir_fields, expr.span())?;

        Ok(LoweredExpression {
            prelude,
            value: self.make_expression(
                &expr.span(),
                match route_future_input {
                    Some(future_input) => HirExpressionKind::RouteConstruct {
                        choice_id,
                        variant_index: tag as usize,
                        fields: field_range,
                        future_input,
                    },
                    None => HirExpressionKind::VariantConstruct {
                        carrier: HirVariantCarrier::Choice { choice_id },
                        variant_index: tag as usize,
                        fields: field_range,
                    },
                },
                ty,
                value_kind,
                region,
            )?,
        })
    }

    /// Lowers an authored collection literal into an item list over its lowered elements.
    fn lower_collection_expression(
        &mut self,
        expr: ExpressionView<'_>,
        items: ExpressionListView<'_>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        let mut prelude = Vec::new();
        let mut lowered_items = Vec::with_capacity(items.len());

        for item in items.iter() {
            let lowered_item = self.lower_child_expression_for_parent(&mut prelude, item)?;
            lowered_items.push(lowered_item);
        }

        let region = self.current_region_or_error(&expr.span())?;
        let ty = self.lower_type_id(expr.value_type_id()?, &expr.span())?;
        let item_range = self
            .module
            .expressions
            .append_values(&lowered_items, expr.span())?;

        Ok(LoweredExpression {
            prelude,
            value: self.make_expression(
                &expr.span(),
                HirExpressionKind::Collection(item_range),
                ty,
                ValueKind::RValue,
                region,
            )?,
        })
    }

    /// Lowers an authored map literal into an entry list over its lowered key/value pairs.
    fn lower_map_expression(
        &mut self,
        expr: ExpressionView<'_>,
        entries: MapListView<'_>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        let mut prelude = Vec::new();
        let mut hir_entries = Vec::with_capacity(entries.len());
        for entry in entries.iter() {
            let key = self.lower_child_expression_for_parent(&mut prelude, entry.key())?;
            let value = self.lower_child_expression_for_parent(&mut prelude, entry.value())?;
            hir_entries.push(HirMapEntry { key, value });
        }
        let region = self.current_region_or_error(&expr.span())?;
        let ty = self.lower_type_id(expr.value_type_id()?, &expr.span())?;
        let entry_range = self
            .module
            .expressions
            .append_map_entries(&hir_entries, expr.span())?;
        Ok(LoweredExpression {
            prelude,
            value: self.make_expression(
                &expr.span(),
                HirExpressionKind::MapLiteral(entry_range),
                ty,
                ValueKind::RValue,
                region,
            )?,
        })
    }

    /// Lowers an authored struct literal into a struct construct over its lowered fields.
    fn lower_struct_expression(
        &mut self,
        expr: ExpressionView<'_>,
        args: FieldListView<'_>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        // INVARIANT: const-record runtime use should have been rejected in AST.
        // If a const record reaches HIR struct lowering, push validation earlier
        // instead of converting this into a user diagnostic here.
        if expr.is_const_record_value() {
            return_hir_transformation_error!(
                "HIR invariant: Const record reached runtime HIR struct lowering; field access should select a member before HIR generation",
                self.hir_error_location(&expr.span())
            );
        }

        let Some(nominal_path) = self.type_environment.nominal_path(expr.value_type_id()?) else {
            return_hir_transformation_error!(
                "Struct instance reached HIR lowering without a nominal struct identity",
                self.hir_error_location(&expr.span())
            );
        };
        let nominal_path = nominal_path.to_owned();
        let struct_id = if let Some(TypeIdentityKey::GenericInstance(key)) = self
            .type_environment
            .type_id_to_type_identity_key(expr.value_type_id()?)
        {
            self.resolve_or_register_generic_struct(
                &key,
                &nominal_path,
                expr.value_type_id()?,
                &expr.span(),
            )?
        } else {
            self.resolve_struct_id_from_nominal_path(&nominal_path, &expr.span())?
        };
        let mut prelude = Vec::new();
        let mut fields = Vec::with_capacity(args.len());

        for arg in args.iter() {
            let field_id = self.resolve_field_id_or_error(struct_id, &arg.name(), &expr.span())?;
            let lowered_value =
                self.lower_child_expression_for_parent(&mut prelude, arg.value())?;
            fields.push((field_id, lowered_value));
        }

        let region = self.current_region_or_error(&expr.span())?;
        let ty = self.lower_type_id(expr.value_type_id()?, &expr.span())?;
        let field_range = self
            .module
            .expressions
            .append_struct_fields(&fields, expr.span())?;

        Ok(LoweredExpression {
            prelude,
            value: self.make_expression(
                &expr.span(),
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
        expr: ExpressionView<'_>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let lowered = self.lower_expression(expr)?;
        for prelude in lowered.prelude {
            self.emit_statement_to_current_block(prelude, &expr.span())?;
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
        expr: ExpressionView<'_>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        if self.expression_needs_current_block_lowering(expr) {
            for prelude in pending_prelude.drain(..) {
                self.emit_statement_to_current_block(prelude, &expr.span())?;
            }

            return self.lower_expression_value_to_current_block(expr);
        }

        let lowered = self.lower_expression(expr)?;
        pending_prelude.extend(lowered.prelude);
        Ok(lowered.value)
    }

    pub(crate) fn expression_needs_current_block_lowering(&self, expr: ExpressionView<'_>) -> bool {
        // Checks the root first, so a decided node never allocates the work stack.
        if self.expression_node_needs_current_block_lowering(expr) {
            return true;
        }
        let mut pending = Vec::new();
        expr.visit_children(|child| pending.push(child));
        while let Some(node) = pending.pop() {
            if self.expression_node_needs_current_block_lowering(node) {
                return true;
            }
            node.visit_children(|child| pending.push(child));
        }
        false
    }

    /// Checks one expression node. Child traversal lives in `expression_needs_current_block_lowering`.
    fn expression_node_needs_current_block_lowering(&self, expr: ExpressionView<'_>) -> bool {
        if expr.error_type().is_some() {
            return true;
        }
        use ExpressionOperationView as Operation;
        let handling_needs_block = |handling: &FallibleExpressionHandling| {
            matches!(handling, FallibleExpressionHandling::Propagate)
                || (self.active_catch_handler.is_some()
                    && matches!(handling, FallibleExpressionHandling::Recover))
        };
        match expr.operation() {
            Operation::Call(call) | Operation::MethodCall { call, .. } => {
                call.metadata().requires_external_float_validation
                    || call
                        .handling()
                        .is_some_and(|record| handling_needs_block(&record.handling))
            }
            Operation::HandleFallible { handling, .. } => handling_needs_block(&handling.handling),
            Operation::Cast { .. }
            | Operation::PropagateOption { .. }
            | Operation::LogicalBinary { .. }
            | Operation::ValueBlock(_)
            | Operation::Template(_)
            | Operation::RuntimeTemplate(_)
            | Operation::RuntimeSlotApplication(_)
            | Operation::NumericUnary { .. }
            | Operation::NumericBinary { .. } => true,
            Operation::Compare { .. } => self.active_catch_handler.is_some(),
            _ => false,
        }
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
        owner: ExpressionView<'_>,
        span: &Option<SourceSpan>,
        result_type_ids: &[TypeId],
    ) -> Result<LoweredResults, HirConstructionFailure> {
        match block {
            ValueBlock::If(value_if) => {
                self.lower_value_block_if(owner.store(), value_if, span, result_type_ids)
            }
            ValueBlock::LexicalScope(value_lexical_scope) => self.lower_value_lexical_scope(
                owner.store(),
                value_lexical_scope,
                span,
                result_type_ids,
            ),
            ValueBlock::Match(value_match) => {
                self.lower_value_block_match(owner.store(), value_match, span, result_type_ids)
            }
            ValueBlock::Catch(value_catch) => {
                self.lower_value_block_catch(value_catch, owner, span, result_type_ids)
            }
        }
    }

    /// Installs one continuation before lowering the entire protected expression.
    fn lower_value_block_catch(
        &mut self,
        value_catch: &ValueCatchBlock,
        owner: ExpressionView<'_>,
        span: &Option<SourceSpan>,
        result_type_ids: &[TypeId],
    ) -> Result<LoweredResults, HirConstructionFailure> {
        let protected = owner.bind(value_catch.handled_value)?;
        let facts = protected.failure_facts().ok_or_else(|| {
            CompilerError::compiler_error("Catch protected expression has no failure facts")
        })?;
        if facts.is_statically_infallible_catch() {
            // AST validated the authored handler. Converged facts show no error edge can run,
            // so only the protected value remains executable HIR.
            return self.lower_expression_results(protected);
        }
        // The parser owns catch compatibility; its validated disposition, not the protected
        // root shape, decides which payload arrives in the handler.
        let FailureDisposition::HandledByCatch { error_type_id } = facts.disposition else {
            return_hir_transformation_error!(
                "Value catch block did not contain a recoverable expression",
                self.hir_error_location(span)
            );
        };
        let err_type = self.lower_type_id(error_type_id, span)?;

        // The direct catch root joins cast target values before applying an optional receiving
        // wrap. Nested casts keep their receiving type because a parent expression may need it.
        let cast_with_optional_receiving_wrap = match protected.operation() {
            ExpressionOperationView::Cast { facts, .. }
                if facts.requires_optional_wrap_after_cast =>
            {
                Some(facts)
            }
            _ => None,
        };
        let inner_result_type_ids;
        let result_type_ids = if let Some(cast) = cast_with_optional_receiving_wrap {
            inner_result_type_ids = [cast.target_type_id];
            inner_result_type_ids.as_slice()
        } else {
            result_type_ids
        };
        let lowered = self.lower_fallible_results_with_branching(
            FallibleBranchingContext {
                result_type_ids,
                expressions: owner.store(),
                handling: &value_catch.handler,
                err_type,
                span,
            },
            // Every protected shape lowers through the ordinary expression path while the
            // handler is active; producers route their error edges to that handler themselves.
            |builder| match cast_with_optional_receiving_wrap {
                Some(cast) => {
                    let lowered = builder.lower_expression_with_root_cast_result_type(
                        protected,
                        Some(cast.target_type_id),
                    )?;
                    Ok(LoweredResults {
                        prelude: lowered.prelude,
                        values: vec![lowered.value],
                    })
                }
                None => builder.lower_expression_results(protected),
            },
        )?;
        if cast_with_optional_receiving_wrap.is_some() {
            let value = self.wrap_value_to_optional_type(
                lowered.values[0],
                protected.value_type_id()?,
                span,
            )?;
            return Ok(LoweredResults {
                prelude: lowered.prelude,
                values: vec![value],
            });
        }
        Ok(lowered)
    }

    // -------------------------
    //  Casts
    // -------------------------

    /// Lowers a resolved explicit `cast` expression into HIR.
    ///
    /// WHAT: dispatches builtin evidence to an infallible `HirExpressionKind::Cast` or
    ///      `HirFallibleOperation::CastOp` delivered through `Invoke`. User-defined evidence
    ///      calls the selected method through the same native success/error channels.
    /// WHY: AST owns the target, evidence, fallibility and optional wrapping facts;
    ///      HIR only makes their resolved value and control-flow delivery explicit.
    ///      `ResolvedCastEvidence::GenericBound` reaching here is a compiler invariant failure.
    fn lower_cast_expression(
        &mut self,
        source: ExpressionView<'_>,
        cast: &CastFacts,
        expr_type_id: FrontendTypeId,
        span: &Option<SourceSpan>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        match &cast.evidence {
            ResolvedCastEvidence::Builtin { policy } => match (cast.fallibility, cast.handling) {
                (BuiltinCastFallibility::Infallible, _) => self
                    .lower_infallible_builtin_cast_expression(
                        source,
                        cast,
                        *policy,
                        expr_type_id,
                        span,
                    ),
                (BuiltinCastFallibility::Fallible, CastHandling::StoreConversion) => self
                    .lower_store_conversion_cast_expression(
                        source,
                        cast,
                        *policy,
                        expr_type_id,
                        span,
                    ),
                (
                    BuiltinCastFallibility::Fallible,
                    CastHandling::Implicit | CastHandling::Recover,
                ) => self.lower_fallible_builtin_cast_expression(
                    source,
                    cast,
                    *policy,
                    expr_type_id,
                    span,
                ),
            },
            ResolvedCastEvidence::UserDefined { method_path, .. } => self
                .lower_user_defined_cast_expression(source, cast, method_path, expr_type_id, span),
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
        source: ExpressionView<'_>,
        cast: &CastFacts,
        policy: BuiltinCastPolicyId,
        expr_type_id: FrontendTypeId,
        span: &Option<SourceSpan>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        let mut prelude = Vec::new();
        let source = self.lower_child_expression_for_parent(&mut prelude, source)?;

        // Every numeric domain now takes the ordinary cast path: fixed binary floats lower
        // through their numeric text policy so casts and templates share the same formatter
        // selection.
        let target_type = self.lower_type_id(cast.target_type_id, span)?;
        let region = self.current_region_or_error(span)?;
        let value = self.make_expression(
            span,
            HirExpressionKind::Cast { source, policy },
            target_type,
            ValueKind::RValue,
            region,
        )?;
        let value = self.wrap_value_to_optional_type(value, expr_type_id, span)?;

        Ok(LoweredExpression { prelude, value })
    }

    /// Emits a checked cast with native success/error edge locals.
    fn emit_builtin_cast_results(
        &mut self,
        source: ExpressionView<'_>,
        cast: &CastFacts,
        policy: BuiltinCastPolicyId,
        span: &Option<SourceSpan>,
    ) -> Result<EmittedFallibleResults, HirConstructionFailure> {
        let lowered_source = self.lower_expression(source)?;
        for prelude_statement in lowered_source.prelude {
            self.emit_statement_to_current_block(prelude_statement, span)?;
        }

        let ok_type = self.lower_type_id(cast.target_type_id, span)?;
        let err_type = self.builtin_error_type_id(span)?;
        self.emit_fallible_operation(
            crate::compiler_frontend::hir::terminators::HirFallibleOperation::CastOp {
                policy,
                source: lowered_source.value,
            },
            &[ok_type],
            err_type,
            span,
        )
    }

    /// Lowers a compiler-inserted numeric conversion at a compound-assignment store.
    ///
    /// WHAT: invokes the checked cast and selects the enclosing function's failure delivery
    ///       before exposing the scalar success slot.
    /// WHY: this conversion belongs to the assignment write-back boundary, so its failure stays
    ///      attached to the assignment and the write is committed only on success.
    fn lower_store_conversion_cast_expression(
        &mut self,
        source: ExpressionView<'_>,
        cast: &CastFacts,
        policy: BuiltinCastPolicyId,
        expr_type_id: FrontendTypeId,
        span: &Option<SourceSpan>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        let results = self.emit_builtin_cast_results(source, cast, policy, span)?;
        let success_value = self.lower_conversion_results(results, true, span)?;
        let value = self.wrap_value_to_optional_type(success_value, expr_type_id, span)?;

        Ok(LoweredExpression {
            prelude: vec![],
            value,
        })
    }

    /// Lowers a fallible builtin cast through explicit native success/error continuations.
    fn lower_fallible_builtin_cast_expression(
        &mut self,
        source: ExpressionView<'_>,
        cast: &CastFacts,
        policy: BuiltinCastPolicyId,
        expr_type_id: FrontendTypeId,
        span: &Option<SourceSpan>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        let results = self.emit_builtin_cast_results(source, cast, policy, span)?;

        let success_value = match cast.handling {
            CastHandling::Implicit => {
                self.lower_cast_results_with_implicit_delivery(results, span)?
            }
            CastHandling::Recover => {
                if self.active_catch_handler.is_none() {
                    return_hir_transformation_error!(
                        "Recovering builtin cast reached HIR outside a value catch block",
                        self.hir_error_location(span)
                    );
                }
                self.lower_results_to_active_catch_success_value(results, span)?
            }
            CastHandling::StoreConversion => return_hir_transformation_error!(
                "Store conversion cast bypassed compound-assignment lowering",
                self.hir_error_location(span)
            ),
        };
        let value = self.wrap_value_to_optional_type(success_value, expr_type_id, span)?;
        Ok(LoweredExpression {
            prelude: vec![],
            value,
        })
    }

    fn lower_cast_results_with_implicit_delivery(
        &mut self,
        results: EmittedFallibleResults,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        if self.active_catch_handler.is_some() {
            return self.lower_results_to_active_catch_success_value(results, span);
        }
        self.lower_conversion_results(results, false, span)
    }

    fn lower_conversion_results(
        &mut self,
        results: EmittedFallibleResults,
        store: bool,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        if self.numeric_failure_is_recoverable(span)? {
            return self.lower_fallible_results_to_success_value(results, span);
        }
        use crate::compiler_frontend::hir::terminators::{HirTerminator, RuntimeFailureCause};
        let cause = if store {
            RuntimeFailureCause::StoreConversion {
                error: results.error_local,
            }
        } else {
            RuntimeFailureCause::AuthoredCastConversion {
                error: results.error_local,
            }
        };
        self.emit_terminator_with_span(
            results.error_block,
            HirTerminator::RuntimeFailure {
                message: if store {
                    "Compound assignment conversion failed"
                } else {
                    "Cast conversion failed"
                }
                .to_owned(),
                cause: Some(cause),
            },
            span,
            *span,
        )?;
        let values = self.fallible_success_values(&results, span)?;
        self.single_result_value(values, span)
    }
    /// Lowers a user-defined cast by calling the selected evidence method.
    fn lower_user_defined_cast_expression(
        &mut self,
        source: ExpressionView<'_>,
        cast: &CastFacts,
        method_path: &PathId,
        expr_type_id: FrontendTypeId,
        span: &Option<SourceSpan>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        let call_target = self.resolve_call_target_or_error(method_path, span)?;

        match (cast.fallibility, cast.handling) {
            (
                BuiltinCastFallibility::Infallible,
                CastHandling::Implicit | CastHandling::Recover,
            ) => {
                let mut prelude = Vec::new();
                let source = self.lower_child_expression_for_parent(&mut prelude, source)?;
                let lowered = self.lower_call_results_from_values(
                    call_target,
                    &[source],
                    prelude,
                    &[cast.target_type_id],
                    span,
                )?;
                let value = self.single_result_value(lowered.values, span)?;
                let value = self.wrap_value_to_optional_type(value, expr_type_id, span)?;
                Ok(LoweredExpression {
                    prelude: lowered.prelude,
                    value,
                })
            }
            (BuiltinCastFallibility::Fallible, CastHandling::Implicit) => {
                let carrier =
                    self.emit_user_defined_cast_call_results(call_target, source, span)?;
                let success_value =
                    self.lower_cast_results_with_implicit_delivery(carrier, span)?;
                let value = self.wrap_value_to_optional_type(success_value, expr_type_id, span)?;
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
                    self.emit_user_defined_cast_call_results(call_target, source, span)?;
                let success_value =
                    self.lower_results_to_active_catch_success_value(carrier, span)?;
                let value = self.wrap_value_to_optional_type(success_value, expr_type_id, span)?;
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

    /// Invokes the selected fallible evidence method without a result carrier.
    fn emit_user_defined_cast_call_results(
        &mut self,
        target: CallTarget,
        source: ExpressionView<'_>,
        span: &Option<SourceSpan>,
    ) -> Result<EmittedFallibleResults, HirConstructionFailure> {
        let (types, error_type) = self.result_call_slots(&target, span)?;
        let source = self.lower_expression_value_to_current_block(source)?;
        let args = self.module.expressions.append_values(&[source], *span)?;
        let types = self.lower_call_result_types(&types, span)?;
        let error_type = self.lower_type_id(error_type, span)?;
        self.emit_fallible_operation(
            crate::compiler_frontend::hir::terminators::HirFallibleOperation::Call { target, args },
            &types,
            error_type,
            span,
        )
    }

    /// Wraps an already validated value when its receiver adds an optional layer.
    pub(crate) fn wrap_value_to_optional_type(
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
                    "Value type {:?} cannot be wrapped to expected optional type {:?}",
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

    /// Resolves the builtin `Error` type id for recoverable failure edges and error returns.
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

    fn log_expression_input(&self, _expression: ExpressionView<'_>) {
        hir_log!(format!(
            "[HIR] Lowering expression @ {:?}, results {:?}",
            _expression.span(),
            _expression.success_types()
        ));
    }

    fn log_expression_output(&self, _input: ExpressionView<'_>, _output: HirValueId) {
        hir_log!(format!(
            "[HIR] Lowered expression @ {:?} -> {}",
            _input.span(),
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
