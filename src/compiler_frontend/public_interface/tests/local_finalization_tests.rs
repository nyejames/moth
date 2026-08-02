//! Focused hidden-invariant tests for the post-borrow-validation local finalization join.
//!
//! WHAT: exercises the invariants of the completed-phase join in [`PublicInterfaceDraft`]
//! that integration output cannot inspect: free and receiver call summaries joined once,
//! generic templates carrying no base concrete summary, private and start summary
//! exclusion, missing, extra and duplicate public-origin rejection, free-receiver category
//! mismatch rejection, signature-summary shape mismatch rejection, declared access and
//! effect drift rejection, and deterministic origin-sorted concrete summary emission.
//! WHY: these are finalization invariants owned by
//! `compiler_frontend::public_interface::local_finalization`, so they own a focused test
//! beside the module rather than an end-to-end case.

use super::super::{
    PublicDeclarationRecord, PublicDeclarationSemantics, PublicFunctionCategory,
    PublicFunctionSemantics, PublicGenericTemplateDescriptor, PublicInterfaceDraft,
    PublicParameterTypeSlot, PublicReceiverMethodCategory, PublicReceiverMethodSemantics,
    PublicStructSemantics,
};
use super::test_support::{module_origin, struct_origin};

use crate::compiler_frontend::analysis::borrow_checker::BorrowAnalysis;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::hir::ids::FunctionId;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallMutationEffect, PublicCallParameterAccess,
    PublicCallParameterSummary, PublicCallReactiveEffect, PublicCallSummary,
    PublicCallTransferEffect, PublicCallTransferEligibility,
};
use crate::compiler_frontend::semantic_identity::{
    OriginDeclarationId, OriginFunctionId, OriginTypeId,
};

fn empty_public_call_summary() -> PublicCallSummary {
    PublicCallSummary {
        parameters: vec![],
        return_alias: FunctionReturnAliasSummary::Fresh,
    }
}

fn public_call_summary_for_access(access: PublicCallParameterAccess) -> PublicCallSummary {
    let (mutation, transfer_eligibility, transfer_effect, reactive_effect) = match access {
        PublicCallParameterAccess::Shared => (
            PublicCallMutationEffect::NoWrite,
            PublicCallTransferEligibility::Eligible,
            PublicCallTransferEffect::MayConsume,
            PublicCallReactiveEffect::None,
        ),
        PublicCallParameterAccess::Mutable => (
            PublicCallMutationEffect::Writes,
            PublicCallTransferEligibility::Eligible,
            PublicCallTransferEffect::MayConsume,
            PublicCallReactiveEffect::None,
        ),
        PublicCallParameterAccess::Reactive => (
            PublicCallMutationEffect::NoWrite,
            PublicCallTransferEligibility::Ineligible,
            PublicCallTransferEffect::NeverConsumes,
            PublicCallReactiveEffect::Subscribes,
        ),
    };

    PublicCallSummary {
        parameters: vec![PublicCallParameterSummary {
            access,
            mutation,
            transfer_eligibility,
            transfer_effect,
            reactive_effect,
        }],
        return_alias: FunctionReturnAliasSummary::Fresh,
    }
}

fn public_parameter() -> PublicParameterTypeSlot {
    PublicParameterTypeSlot {
        name: None,
        type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int),
        access: PublicCallParameterAccess::Shared,
        folded_default: None,
    }
}

fn public_hir_for_origins(origins: &[(FunctionId, OriginFunctionId)]) -> HirModule {
    let mut hir = HirModule::new();
    for (function_id, origin) in origins {
        hir.function_ids_by_origin
            .insert(origin.clone(), *function_id);
    }
    hir
}

fn function_draft_record(
    origin: OriginFunctionId,
    parameters: Vec<PublicParameterTypeSlot>,
) -> PublicDeclarationRecord {
    PublicDeclarationRecord {
        origin: OriginDeclarationId::Function(origin),
        semantics: PublicDeclarationSemantics::Function(PublicFunctionSemantics {
            category: PublicFunctionCategory::ConcreteLocal,
            parameters,
            returns: vec![],
            error_return: None,
        }),
    }
}

fn receiver_draft_record(
    receiver_origin: OriginTypeId,
    method_origin: OriginFunctionId,
) -> PublicDeclarationRecord {
    receiver_draft_record_with_template(receiver_origin, method_origin, false)
}

fn receiver_draft_record_with_template(
    receiver_origin: OriginTypeId,
    method_origin: OriginFunctionId,
    is_generic: bool,
) -> PublicDeclarationRecord {
    PublicDeclarationRecord {
        origin: OriginDeclarationId::Type(receiver_origin),
        semantics: PublicDeclarationSemantics::Struct(PublicStructSemantics {
            generic_parameters: vec![],
            fields: vec![],
            receiver_methods: vec![PublicReceiverMethodSemantics {
                method_origin,
                category: if is_generic {
                    PublicReceiverMethodCategory::GenericTemplate
                } else {
                    PublicReceiverMethodCategory::ConcreteLocal
                },
                parameters: vec![],
                returns: vec![],
                error_return: None,
            }],
        }),
    }
}

fn generic_function_draft_record(origin: OriginFunctionId) -> PublicDeclarationRecord {
    PublicDeclarationRecord {
        origin: OriginDeclarationId::Function(origin),
        semantics: PublicDeclarationSemantics::Function(PublicFunctionSemantics {
            category: PublicFunctionCategory::GenericTemplate(PublicGenericTemplateDescriptor {
                generic_parameters: vec![],
            }),
            parameters: vec![],
            returns: vec![],
            error_return: None,
        }),
    }
}

fn draft_with_records(records: Vec<PublicDeclarationRecord>) -> PublicInterfaceDraft {
    PublicInterfaceDraft {
        module_origin: module_origin(),
        export_bindings: vec![],
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: records,
        reusable_evidence: vec![],
    }
}
#[test]
fn finalizes_free_and_receiver_call_summaries_once() {
    let free_origin = OriginFunctionId::new_free(module_origin(), "render".to_owned());
    let receiver_origin = struct_origin("Counter");
    let method_origin = OriginFunctionId::new_receiver(
        module_origin(),
        "reset".to_owned(),
        receiver_origin.clone(),
    );
    let draft = draft_with_records(vec![
        function_draft_record(free_origin.clone(), vec![]),
        receiver_draft_record(receiver_origin, method_origin.clone()),
    ]);
    let mut borrow_analysis = BorrowAnalysis::default();
    borrow_analysis
        .public_call_summaries
        .insert(FunctionId(4), empty_public_call_summary());
    borrow_analysis
        .public_call_summaries
        .insert(FunctionId(7), empty_public_call_summary());
    let hir = public_hir_for_origins(&[
        (FunctionId(4), free_origin.clone()),
        (FunctionId(7), method_origin.clone()),
    ]);

    let draft = draft
        .finalize_after_borrow_validation(&borrow_analysis, &hir)
        .expect("free and receiver summaries should join");

    assert_eq!(draft.concrete_call_summaries.len(), 2);
    assert!(
        draft
            .concrete_call_summaries
            .iter()
            .any(|record| record.origin == free_origin)
    );
    assert!(
        draft
            .concrete_call_summaries
            .iter()
            .any(|record| record.origin == method_origin)
    );
}

#[test]
fn generic_template_has_no_base_concrete_summary() {
    let generic_origin = OriginFunctionId::new_free(module_origin(), "identity".to_owned());
    let draft = draft_with_records(vec![generic_function_draft_record(generic_origin)])
        .finalize_after_borrow_validation(&BorrowAnalysis::default(), &HirModule::new())
        .expect(
            "generic templates remain declaration categories until R5 generated sidecars exist",
        );

    let PublicDeclarationSemantics::Function(function) = &draft.draft.declarations[0].semantics
    else {
        panic!("expected generic free-function declaration record");
    };
    assert!(matches!(
        function.category,
        PublicFunctionCategory::GenericTemplate(_)
    ));
    assert!(draft.concrete_call_summaries.is_empty());
}

#[test]
fn generic_receiver_template_has_no_base_concrete_summary() {
    let receiver_origin = struct_origin("Box");
    let method_origin =
        OriginFunctionId::new_receiver(module_origin(), "get".to_owned(), receiver_origin.clone());
    let draft = draft_with_records(vec![receiver_draft_record_with_template(
        receiver_origin,
        method_origin,
        true,
    )])
    .finalize_after_borrow_validation(&BorrowAnalysis::default(), &HirModule::new())
        .expect("generic receiver templates remain declaration categories until R5 generated sidecars exist");

    let PublicDeclarationSemantics::Struct(receiver) = &draft.draft.declarations[0].semantics
    else {
        panic!("expected generic receiver declaration record");
    };
    assert!(matches!(
        receiver.receiver_methods[0].category,
        PublicReceiverMethodCategory::GenericTemplate
    ));
    assert!(draft.concrete_call_summaries.is_empty());
}

#[test]
fn finalization_excludes_private_and_start_summaries() {
    // Three local functions carry borrow-observed call summaries: the implicit start
    // (FunctionId 0), one exported public free function (FunctionId 1) and one private
    // free function (FunctionId 2). Only the public origin enters the draft, so the completed
    // phase must retain exactly that origin's summary and exclude both local-only summaries.
    let start_id = FunctionId(0);
    let public_id = FunctionId(1);
    let private_id = FunctionId(2);
    let public_origin = OriginFunctionId::new_free(module_origin(), "render".to_owned());

    let mut borrow_analysis = BorrowAnalysis::default();
    borrow_analysis
        .public_call_summaries
        .insert(start_id, empty_public_call_summary());
    borrow_analysis
        .public_call_summaries
        .insert(public_id, empty_public_call_summary());
    borrow_analysis
        .public_call_summaries
        .insert(private_id, empty_public_call_summary());
    let hir = public_hir_for_origins(&[(public_id, public_origin.clone())]);

    let completed = draft_with_records(vec![function_draft_record(public_origin.clone(), vec![])])
        .finalize_after_borrow_validation(&borrow_analysis, &hir)
        .expect("private and start summaries should remain local-only");
    assert_eq!(completed.draft.declarations.len(), 1);
    assert_eq!(
        completed.concrete_call_summaries.len(),
        1,
        "only the exported public origin receives a concrete summary record"
    );
    assert_eq!(
        completed.concrete_call_summaries[0].origin, public_origin,
        "the single retained summary must belong to the exported public origin"
    );
    assert!(
        !completed
            .concrete_call_summaries
            .iter()
            .any(|record| record.origin != public_origin),
        "private and start summaries must not enter the completed public phase"
    );
}

#[test]
fn finalization_rejects_missing_public_origin() {
    let free_origin = OriginFunctionId::new_free(module_origin(), "render".to_owned());
    let result = draft_with_records(vec![function_draft_record(free_origin, vec![])])
        .finalize_after_borrow_validation(&BorrowAnalysis::default(), &HirModule::new());
    assert!(
        result.is_err(),
        "a declared public origin with no matching local FunctionId must fail"
    );
}

#[test]
fn finalization_rejects_extra_public_origin() {
    let free_origin = OriginFunctionId::new_free(module_origin(), "render".to_owned());
    let extra_origin = OriginFunctionId::new_free(module_origin(), "extra".to_owned());
    let mut borrow_analysis = BorrowAnalysis::default();
    borrow_analysis
        .public_call_summaries
        .insert(FunctionId(1), empty_public_call_summary());
    borrow_analysis
        .public_call_summaries
        .insert(FunctionId(2), empty_public_call_summary());
    let hir = public_hir_for_origins(&[
        (FunctionId(1), free_origin.clone()),
        (FunctionId(2), extra_origin),
    ]);
    let result = draft_with_records(vec![function_draft_record(free_origin, vec![])])
        .finalize_after_borrow_validation(&borrow_analysis, &hir);
    assert!(
        result.is_err(),
        "a local FunctionId with no matching declared public origin must fail"
    );
}

#[test]
fn finalization_rejects_duplicate_public_origin() {
    let free_origin = OriginFunctionId::new_free(module_origin(), "render".to_owned());
    let result = draft_with_records(vec![
        function_draft_record(free_origin.clone(), vec![]),
        function_draft_record(free_origin.clone(), vec![]),
    ])
    .finalize_after_borrow_validation(
        &BorrowAnalysis {
            public_call_summaries: [(FunctionId(1), empty_public_call_summary())]
                .into_iter()
                .collect(),
            ..BorrowAnalysis::default()
        },
        &public_hir_for_origins(&[(FunctionId(1), free_origin)]),
    );
    assert!(
        result.is_err(),
        "two concrete-local records sharing one stable callable origin must fail"
    );
}

#[test]
fn finalization_rejects_free_receiver_category_mismatch() {
    let receiver_origin = struct_origin("Counter");
    let receiver_method_origin =
        OriginFunctionId::new_receiver(module_origin(), "render".to_owned(), receiver_origin);
    let mut borrow_analysis = BorrowAnalysis::default();
    borrow_analysis
        .public_call_summaries
        .insert(FunctionId(1), empty_public_call_summary());
    let result = draft_with_records(vec![function_draft_record(
        receiver_method_origin.clone(),
        vec![],
    )])
    .finalize_after_borrow_validation(
        &borrow_analysis,
        &public_hir_for_origins(&[(FunctionId(1), receiver_method_origin)]),
    );
    assert!(
        result.is_err(),
        "a receiver-method origin joined as a free-function declaration must fail"
    );
}

#[test]
fn finalization_rejects_signature_summary_shape_mismatch() {
    let free_origin = OriginFunctionId::new_free(module_origin(), "render".to_owned());
    let mut borrow_analysis = BorrowAnalysis::default();
    borrow_analysis
        .public_call_summaries
        .insert(FunctionId(1), empty_public_call_summary());
    let result = draft_with_records(vec![function_draft_record(
        free_origin.clone(),
        vec![public_parameter()],
    )])
    .finalize_after_borrow_validation(
        &borrow_analysis,
        &public_hir_for_origins(&[(FunctionId(1), free_origin)]),
    );
    assert!(
        result.is_err(),
        "a declaration with one parameter joined to a parameterless summary must fail"
    );
}

#[test]
fn finalization_rejects_declared_access_and_effect_drift() {
    let function_origin = OriginFunctionId::new_free(module_origin(), "render".to_owned());
    let hir = public_hir_for_origins(&[(FunctionId(1), function_origin.clone())]);

    let mut access_mismatch_analysis = BorrowAnalysis::default();
    access_mismatch_analysis.public_call_summaries.insert(
        FunctionId(1),
        public_call_summary_for_access(PublicCallParameterAccess::Mutable),
    );
    let access_mismatch = draft_with_records(vec![function_draft_record(
        function_origin.clone(),
        vec![public_parameter()],
    )])
    .finalize_after_borrow_validation(&access_mismatch_analysis, &hir);
    assert!(
        access_mismatch.is_err(),
        "borrow-observed access must agree with the declaration-owned access contract"
    );

    let mut invalid_effect_summary =
        public_call_summary_for_access(PublicCallParameterAccess::Shared);
    invalid_effect_summary.parameters[0].transfer_eligibility =
        PublicCallTransferEligibility::Ineligible;
    invalid_effect_summary.parameters[0].transfer_effect = PublicCallTransferEffect::NeverConsumes;
    let mut invalid_effect_analysis = BorrowAnalysis::default();
    invalid_effect_analysis
        .public_call_summaries
        .insert(FunctionId(1), invalid_effect_summary);
    let invalid_effect = draft_with_records(vec![function_draft_record(
        function_origin,
        vec![public_parameter()],
    )])
    .finalize_after_borrow_validation(&invalid_effect_analysis, &hir);
    assert!(
        invalid_effect.is_err(),
        "invalid optional-transfer facts must fail at the summary join boundary"
    );
}

#[test]
fn finalization_emits_origin_sorted_concrete_summaries() {
    // Provide concrete-local callables whose stable origins sort in the opposite order to their
    // declaration record order. The completed phase must emit origin-sorted records without
    // relying on hash-map iteration order.
    let later_origin = OriginFunctionId::new_free(module_origin(), "zebra".to_owned());
    let earlier_origin = OriginFunctionId::new_free(module_origin(), "alpha".to_owned());
    let draft = draft_with_records(vec![
        function_draft_record(later_origin.clone(), vec![]),
        function_draft_record(earlier_origin.clone(), vec![]),
    ]);
    let mut borrow_analysis = BorrowAnalysis::default();
    borrow_analysis
        .public_call_summaries
        .insert(FunctionId(1), empty_public_call_summary());
    borrow_analysis
        .public_call_summaries
        .insert(FunctionId(2), empty_public_call_summary());
    let hir = public_hir_for_origins(&[
        (FunctionId(1), later_origin.clone()),
        (FunctionId(2), earlier_origin.clone()),
    ]);

    let completed = draft
        .finalize_after_borrow_validation(&borrow_analysis, &hir)
        .expect("two distinct concrete origins should join");

    assert_eq!(completed.concrete_call_summaries.len(), 2);
    assert_eq!(completed.concrete_call_summaries[0].origin, earlier_origin);
    assert_eq!(completed.concrete_call_summaries[1].origin, later_origin);
    assert!(
        completed
            .concrete_call_summaries
            .windows(2)
            .all(|window| window[0].origin <= window[1].origin),
        "concrete call summary records must be sorted by stable origin"
    );
}
