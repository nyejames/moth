//! Local finalization: the completed direct-interface phase after borrow validation.
//!
//! WHAT: owns [`PublicInterfaceDraft::finalize_after_borrow_validation`], which joins each
//! exported concrete free function and receiver method to exactly one stable origin/local
//! `FunctionId` mapping and retains the matching complete local [`PublicCallSummary`] as one
//! [`ConcreteCallSummaryRecord`] in the completed [`LocalPublicInterface`].
//!
//! WHY: AST owns the public signature and declared parameter access while borrow validation
//! owns mutation, transfer, reactive and return-alias effects. This is the sole production join
//! point from the pre-HIR draft to the completed phase. Keeping finalization in its own module
//! separates the post-borrow-validation step from the pre-HIR projection modules.

use super::model::PublicParameterTypeSlot;
use super::model::{
    ConcreteCallSummaryRecord, LocalPublicInterface, PublicDeclarationSemantics,
    PublicFunctionCategory, PublicInterfaceDraft, PublicReceiverMethodCategory,
    PublicReceiverMethodSemantics,
};
use crate::compiler_frontend::analysis::borrow_checker::BorrowAnalysis;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::public_call_summary::{
    PublicCallSummary, validate_public_call_summary,
};
use crate::compiler_frontend::semantic_identity::{
    FunctionOriginKind, OriginDeclarationId, OriginFunctionId, OriginTypeId,
};
use rustc_hash::{FxHashMap, FxHashSet};

impl PublicInterfaceDraft {
    /// Finalize direct callable declaration records after borrow validation.
    ///
    /// WHAT: joins each exported concrete free function and receiver method to exactly one
    /// stable origin/local `FunctionId` mapping and retains the matching complete local
    /// [`PublicCallSummary`] as one [`ConcreteCallSummaryRecord`] in the completed phase.
    /// Generic declarations never enter that table.
    /// WHY: AST owns the public signature and declared parameter access while borrow validation
    /// owns mutation, transfer, reactive and return-alias effects. This is the sole production
    /// join point, so private functions and implicit start remain ordinary local summaries and
    /// never become consumer-visible public records. This consumes the pre-HIR draft and returns
    /// the completed [`LocalPublicInterface`]; the draft cannot be finalized twice.
    pub(crate) fn finalize_after_borrow_validation(
        self,
        borrow_analysis: &BorrowAnalysis,
        hir_module: &HirModule,
    ) -> Result<LocalPublicInterface, CompilerError> {
        // Transient construction-time indexes: validate uniqueness and join summaries by origin.
        // They are dropped before the `LocalPublicInterface` boundary, which stores only the
        // contiguous, origin-ordered `Vec<ConcreteCallSummaryRecord>`.
        let mut summaries_by_origin: FxHashMap<OriginFunctionId, PublicCallSummary> =
            FxHashMap::default();
        let mut expected_origins = FxHashSet::default();
        for record in &self.declarations {
            match &record.semantics {
                PublicDeclarationSemantics::Function(semantics) => {
                    let OriginDeclarationId::Function(origin) = &record.origin else {
                        return Err(CompilerError::compiler_error(
                            "public-interface finalization found function semantics under a non-function declaration origin",
                        ));
                    };
                    if !matches!(origin.kind(), FunctionOriginKind::Free) {
                        return Err(CompilerError::compiler_error(format!(
                            "public-interface finalization found receiver origin {:?} in a free-function record",
                            origin
                        )));
                    }
                    match &semantics.category {
                        PublicFunctionCategory::ConcreteLocal => {
                            finalize_callable_summary(
                                origin,
                                &semantics.parameters,
                                &mut summaries_by_origin,
                                &mut expected_origins,
                                borrow_analysis,
                                hir_module,
                            )?;
                        }
                        PublicFunctionCategory::GenericTemplate(_) => {
                            validate_generic_template_callable(
                                origin,
                                &summaries_by_origin,
                                hir_module,
                            )?;
                        }
                    }
                }
                PublicDeclarationSemantics::Struct(semantics) => {
                    let OriginDeclarationId::Type(receiver_origin) = &record.origin else {
                        return Err(CompilerError::compiler_error(
                            "public-interface finalization found struct semantics under a non-type declaration origin",
                        ));
                    };
                    finalize_receiver_method_summaries(
                        receiver_origin,
                        &semantics.receiver_methods,
                        &mut summaries_by_origin,
                        &mut expected_origins,
                        borrow_analysis,
                        hir_module,
                    )?;
                }
                PublicDeclarationSemantics::Choice(semantics) => {
                    let OriginDeclarationId::Type(receiver_origin) = &record.origin else {
                        return Err(CompilerError::compiler_error(
                            "public-interface finalization found choice semantics under a non-type declaration origin",
                        ));
                    };
                    finalize_receiver_method_summaries(
                        receiver_origin,
                        &semantics.receiver_methods,
                        &mut summaries_by_origin,
                        &mut expected_origins,
                        borrow_analysis,
                        hir_module,
                    )?;
                }
                PublicDeclarationSemantics::TransparentAlias(_)
                | PublicDeclarationSemantics::Constant(_)
                | PublicDeclarationSemantics::Trait(_) => {}
            }
        }

        for origin in hir_module.function_ids_by_origin.keys() {
            if !expected_origins.contains(origin) {
                return Err(CompilerError::compiler_error(format!(
                    "public-interface finalization found extra stable function origin {:?} not present in the direct declaration draft",
                    origin
                )));
            }
        }

        // Produce the contiguous, origin-ordered record table and drop the transient map. The
        // sort is explicit by stable origin so the completed phase is deterministic without
        // relying on hash-map iteration order.
        let mut concrete_call_summaries: Vec<ConcreteCallSummaryRecord> = summaries_by_origin
            .into_iter()
            .map(|(origin, summary)| ConcreteCallSummaryRecord { origin, summary })
            .collect();
        concrete_call_summaries.sort_by(|left, right| left.origin.cmp(&right.origin));

        Ok(LocalPublicInterface {
            draft: self,
            concrete_call_summaries,
        })
    }
}

fn finalize_receiver_method_summaries(
    receiver_origin: &OriginTypeId,
    methods: &[PublicReceiverMethodSemantics],
    concrete_call_summaries: &mut FxHashMap<OriginFunctionId, PublicCallSummary>,
    expected_origins: &mut FxHashSet<OriginFunctionId>,
    borrow_analysis: &BorrowAnalysis,
    hir_module: &HirModule,
) -> Result<(), CompilerError> {
    for method in methods {
        let Some(method_receiver) = method.method_origin.receiver() else {
            return Err(CompilerError::compiler_error(format!(
                "public-interface finalization found free-function origin {:?} in receiver {:?}",
                method.method_origin, receiver_origin
            )));
        };
        if method_receiver != receiver_origin {
            return Err(CompilerError::compiler_error(format!(
                "public-interface finalization found receiver origin {:?} attached to {:?}",
                method_receiver, receiver_origin
            )));
        }
        match method.category {
            PublicReceiverMethodCategory::ConcreteLocal => {
                finalize_callable_summary(
                    &method.method_origin,
                    &method.parameters,
                    concrete_call_summaries,
                    expected_origins,
                    borrow_analysis,
                    hir_module,
                )?;
            }
            PublicReceiverMethodCategory::GenericTemplate => {
                validate_generic_template_callable(
                    &method.method_origin,
                    concrete_call_summaries,
                    hir_module,
                )?;
            }
        }
    }
    Ok(())
}

fn finalize_callable_summary(
    origin: &OriginFunctionId,
    signature_parameters: &[PublicParameterTypeSlot],
    concrete_call_summaries: &mut FxHashMap<OriginFunctionId, PublicCallSummary>,
    expected_origins: &mut FxHashSet<OriginFunctionId>,
    borrow_analysis: &BorrowAnalysis,
    hir_module: &HirModule,
) -> Result<(), CompilerError> {
    if !expected_origins.insert(origin.clone()) {
        return Err(CompilerError::compiler_error(format!(
            "public-interface finalization found duplicate callable origin {:?}",
            origin
        )));
    }
    let local_function_id = hir_module
        .function_ids_by_origin
        .get(origin)
        .copied()
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "public-interface finalization is missing the local FunctionId for stable origin {:?}",
                origin
            ))
        })?;
    if Some(local_function_id) == hir_module.start_function {
        return Err(CompilerError::compiler_error(format!(
            "public-interface finalization mapped callable origin {:?} to the implicit start function",
            origin
        )));
    }

    let summary = borrow_analysis
        .public_call_summaries
        .get(&local_function_id)
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "public-interface finalization is missing the borrow call summary for stable origin {:?} and local function {:?}",
                origin, local_function_id
            ))
        })?;
    let declared_parameter_access = signature_parameters
        .iter()
        .map(|parameter| parameter.access)
        .collect::<Vec<_>>();
    validate_public_call_summary(&declared_parameter_access, summary).map_err(|error| {
        CompilerError::compiler_error(format!(
            "public-interface finalization found an invalid call summary for stable origin {:?}: {}",
            origin, error.msg
        ))
    })?;

    if concrete_call_summaries
        .insert(origin.clone(), summary.clone())
        .is_some()
    {
        return Err(CompilerError::compiler_error(format!(
            "public-interface finalization found duplicate concrete call summary for callable origin {:?}",
            origin
        )));
    }
    Ok(())
}

fn validate_generic_template_callable(
    origin: &OriginFunctionId,
    concrete_call_summaries: &FxHashMap<OriginFunctionId, PublicCallSummary>,
    hir_module: &HirModule,
) -> Result<(), CompilerError> {
    if concrete_call_summaries.contains_key(origin) {
        return Err(CompilerError::compiler_error(format!(
            "public-interface finalization found a concrete base summary for generic template origin {:?}; generated summaries belong to sidecars",
            origin
        )));
    }

    if hir_module.function_ids_by_origin.contains_key(origin) {
        return Err(CompilerError::compiler_error(format!(
            "public-interface finalization found a local HIR FunctionId for generic template origin {:?}",
            origin
        )));
    }

    Ok(())
}
