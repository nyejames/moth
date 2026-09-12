//! Choice payload and option-present capture lowering for HIR match arms.
//!
//! WHAT: allocates capture locals, rewrites guard capture reads, and emits payload extraction
//! assignments for choice-variant and present-option match arms.
//! WHY: capture materialization has different timing from generic CFG lowering: guards are
//! evaluated in the match terminator, while capture assignments execute inside arm blocks.
//!
//! NOTE: payload field aliases (for example `Variant(field as local_name) =>`) are a frontend
//! AST binding concern only. HIR extraction uses `field_index` and `binding_path`; the alias
//! spelling never reaches HIR and does not affect variant layout or payload extraction.

use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::match_patterns::{MatchArm, MatchPattern};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::generic_identity_bridge::TypeIdentityKey;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::blocks::HirLocal;
use crate::compiler_frontend::hir::expression_rewrite::rewrite_expression_bottom_up;
use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirVariantCarrier, ValueKind,
};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::{ChoiceId, LocalId, RegionId};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::return_hir_transformation_error;
use rustc_hash::FxHashMap;

struct MatchCaptureLoweringContext {
    scrutinee_hir: HirExpression,
    choice_id: ChoiceId,
    parent_region: RegionId,
}

impl<'a> HirBuilder<'a> {
    /// Register capture locals for one match arm so guards and bodies can reference them.
    pub(crate) fn register_match_arm_capture_locals(
        &mut self,
        arm: &MatchArm,
        scrutinee_ast: &Expression,
        span: &Option<SourceSpan>,
    ) -> Result<Vec<LocalId>, CompilerError> {
        match &arm.pattern {
            MatchPattern::ChoiceVariant {
                nominal_path,
                captures,
                ..
            } => {
                if captures.is_empty() {
                    return Ok(Vec::new());
                }

                let _choice_id =
                    self.choice_id_for_scrutinee_type(nominal_path, scrutinee_ast.type_id, span)?;
                let region = self.current_region_or_error(span)?;

                let mut local_ids = Vec::with_capacity(captures.len());
                for capture in captures {
                    let field_ty = self.lower_type_id(capture.type_id, &capture.span)?;
                    let local_id = self.allocate_local_id();
                    let block_id = self.current_block_id_or_error(&capture.span)?;

                    self.register_local_in_block(
                        block_id,
                        HirLocal {
                            id: local_id,
                            ty: field_ty,
                            mutable: false,
                            region,
                            span: capture.binding_span,
                        },
                        &capture.span,
                    )?;

                    self.locals_by_name
                        .insert(capture.binding_path.clone(), local_id);
                    self.side_table
                        .bind_local_name(local_id, capture.binding_path.clone());
                    local_ids.push(local_id);
                }

                Ok(local_ids)
            }
            MatchPattern::OptionPresentCapture {
                binding_path,
                inner_type_id,
                span: pattern_span,
                binding_span,
                ..
            } => {
                let binding_fallback_span = (*binding_span).or(*pattern_span);
                let ty = self.lower_type_id(*inner_type_id, &binding_fallback_span)?;
                let region = self.current_region_or_error(&binding_fallback_span)?;
                let local_id = self.allocate_local_id();
                let block_id = self.current_block_id_or_error(&binding_fallback_span)?;

                self.register_local_in_block(
                    block_id,
                    HirLocal {
                        id: local_id,
                        ty,
                        mutable: false,
                        region,
                        span: *binding_span,
                    },
                    &binding_fallback_span,
                )?;

                self.locals_by_name.insert(binding_path.clone(), local_id);
                self.side_table
                    .bind_local_name(local_id, binding_path.clone());

                Ok(vec![local_id])
            }

            _ => Ok(Vec::new()),
        }
    }

    /// Replace guard reads of capture locals with direct payload reads from the scrutinee.
    pub(super) fn substitute_match_guard_captures(
        &mut self,
        guard: &HirExpression,
        arm: &MatchArm,
        capture_locals: &[LocalId],
        scrutinee_ast: &Expression,
        scrutinee_hir: &HirExpression,
        span: &Option<SourceSpan>,
    ) -> Result<HirExpression, CompilerError> {
        let context = self.match_capture_context(arm, scrutinee_ast, scrutinee_hir, span)?;
        let substitutions =
            self.build_guard_capture_substitutions(arm, capture_locals, &context)?;

        if substitutions.is_empty() {
            return Ok(guard.clone());
        }

        Ok(substitute_local_expressions(guard, &substitutions))
    }

    /// Emit `Assign` statements that materialize choice payload captures at arm entry.
    pub(crate) fn emit_match_arm_capture_assignments(
        &mut self,
        arm: &MatchArm,
        capture_locals: &[LocalId],
        scrutinee_hir: &HirExpression,
        scrutinee_ast: &Expression,
        span: &Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        match &arm.pattern {
            MatchPattern::ChoiceVariant { tag, captures, .. } => {
                let context =
                    self.match_capture_context(arm, scrutinee_ast, scrutinee_hir, span)?;

                if captures.is_empty() {
                    return Ok(());
                }

                debug_assert_eq!(
                    captures.len(),
                    capture_locals.len(),
                    "capture count must match registered local count"
                );

                for (capture, &local_id) in captures.iter().zip(capture_locals.iter()) {
                    let field_ty = self.lower_type_id(capture.type_id, &capture.span)?;
                    let payload_get = self.make_capture_payload_get(
                        &context,
                        *tag,
                        capture.field_index,
                        field_ty,
                        &capture.span,
                    );

                    // Authored capture materialization: each arm-entry assignment
                    // carries the capture's binding span, falling back to the
                    // field span when the binding is identity-free.
                    self.emit_statement_kind_with_span(
                        HirStatementKind::Assign {
                            target: HirPlace::Local(local_id),
                            value: payload_get,
                        },
                        &capture.span,
                        capture.binding_span.or(capture.span),
                    )?;
                }

                Ok(())
            }

            MatchPattern::OptionPresentCapture {
                inner_type_id,
                span: pattern_span,
                binding_span,
                ..
            } => {
                if capture_locals.is_empty() {
                    return Ok(());
                }
                let local_id = capture_locals[0];
                let binding_fallback_span = (*binding_span).or(*pattern_span);
                let field_ty = self.lower_type_id(*inner_type_id, &binding_fallback_span)?;
                let region = self.current_region_or_error(&binding_fallback_span)?;
                let payload_get = self.make_expression(
                    &binding_fallback_span,
                    HirExpressionKind::VariantPayloadGet {
                        carrier: HirVariantCarrier::Option,
                        source: Box::new(scrutinee_hir.clone()),
                        variant_index:
                            crate::compiler_frontend::hir::expressions::OPTION_SOME_VARIANT_INDEX,
                        field_index: 0,
                    },
                    field_ty,
                    ValueKind::RValue,
                    region,
                );
                // Authored option capture materialization carries the binding span.
                self.emit_statement_kind_with_span(
                    HirStatementKind::Assign {
                        target: HirPlace::Local(local_id),
                        value: payload_get,
                    },
                    &binding_fallback_span,
                    *binding_span,
                )?;
                Ok(())
            }

            _ => Ok(()),
        }
    }

    /// Lower an arm body while capture names resolve to that arm's local IDs.
    pub(crate) fn with_arm_capture_bindings<T>(
        &mut self,
        arm: &MatchArm,
        capture_locals: &[LocalId],
        f: impl FnOnce(&mut Self) -> Result<T, CompilerError>,
    ) -> Result<T, CompilerError> {
        let bindings = arm_capture_bindings(arm, capture_locals);
        self.with_temporary_local_bindings(bindings, f)
    }

    fn match_capture_context(
        &mut self,
        arm: &MatchArm,
        scrutinee_ast: &Expression,
        scrutinee_hir: &HirExpression,
        span: &Option<SourceSpan>,
    ) -> Result<MatchCaptureLoweringContext, CompilerError> {
        let MatchPattern::ChoiceVariant { nominal_path, .. } = &arm.pattern else {
            return_hir_transformation_error!(
                "Match capture context requires a choice-variant pattern",
                self.hir_error_location(span)
            );
        };

        let choice_id =
            self.choice_id_for_scrutinee_type(nominal_path, scrutinee_ast.type_id, span)?;
        let parent_region = self.current_region_or_error(span)?;

        Ok(MatchCaptureLoweringContext {
            scrutinee_hir: scrutinee_hir.clone(),
            choice_id,
            parent_region,
        })
    }

    fn build_guard_capture_substitutions(
        &mut self,
        arm: &MatchArm,
        capture_locals: &[LocalId],
        context: &MatchCaptureLoweringContext,
    ) -> Result<FxHashMap<LocalId, HirExpression>, CompilerError> {
        let MatchPattern::ChoiceVariant { tag, captures, .. } = &arm.pattern else {
            return Ok(FxHashMap::default());
        };

        if captures.is_empty() {
            return Ok(FxHashMap::default());
        }

        debug_assert_eq!(
            captures.len(),
            capture_locals.len(),
            "capture count must match registered local count"
        );

        let mut substitutions = FxHashMap::default();
        for (capture, &local_id) in captures.iter().zip(capture_locals.iter()) {
            let field_ty = self.lower_type_id(capture.type_id, &capture.span)?;
            let payload_get = self.make_capture_payload_get(
                context,
                *tag,
                capture.field_index,
                field_ty,
                &capture.span,
            );
            substitutions.insert(local_id, payload_get);
        }

        Ok(substitutions)
    }

    fn make_capture_payload_get(
        &mut self,
        context: &MatchCaptureLoweringContext,
        variant_index: usize,
        field_index: usize,
        field_ty: TypeId,
        span: &Option<SourceSpan>,
    ) -> HirExpression {
        self.make_expression(
            span,
            HirExpressionKind::VariantPayloadGet {
                carrier: HirVariantCarrier::Choice {
                    choice_id: context.choice_id,
                },
                source: Box::new(context.scrutinee_hir.clone()),
                variant_index,
                field_index,
            },
            field_ty,
            ValueKind::RValue,
            context.parent_region,
        )
    }

    pub(super) fn choice_id_for_scrutinee_type(
        &mut self,
        nominal_path: &PathId,
        scrutinee_type_id: TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<ChoiceId, CompilerError> {
        if self
            .type_environment
            .variants_for(scrutinee_type_id)
            .is_none()
        {
            return_hir_transformation_error!(
                "Choice pattern capture used with non-choice scrutinee type",
                self.hir_error_location(span)
            );
        }

        let generic_key = match self
            .type_environment
            .type_id_to_type_identity_key(scrutinee_type_id)
        {
            Some(TypeIdentityKey::GenericInstance(key)) => Some(key),
            _ => None,
        };

        match generic_key {
            Some(key) => {
                self.resolve_or_register_generic_choice(&key, nominal_path, scrutinee_type_id, span)
            }
            None => self.resolve_choice_id(nominal_path, span),
        }
    }
}

fn arm_capture_bindings(
    arm: &MatchArm,
    capture_locals: &[LocalId],
) -> Vec<(PathId, LocalId)> {
    match &arm.pattern {
        MatchPattern::ChoiceVariant { captures, .. } => captures
            .iter()
            .zip(capture_locals.iter())
            .map(|(capture, &local_id)| (capture.binding_path.clone(), local_id))
            .collect(),
        MatchPattern::OptionPresentCapture { binding_path, .. } => {
            if let Some(&local_id) = capture_locals.first() {
                vec![(binding_path.clone(), local_id)]
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

pub(super) fn substitute_local_expressions(
    expression: &HirExpression,
    substitutions: &FxHashMap<LocalId, HirExpression>,
) -> HirExpression {
    rewrite_expression_bottom_up(expression, &mut |rewritten| match &rewritten.kind {
        HirExpressionKind::Load(HirPlace::Local(local_id))
        | HirExpressionKind::Copy(HirPlace::Local(local_id)) => {
            substitutions.get(local_id).cloned()
        }
        _ => None,
    })
}
