use super::*;

use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_messages::DiagnosticPayload;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::{CallTarget, ExternalFunctionId};
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::failure_facts::HirFunctionFailureFacts;
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirNodeId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::{
    HirModuleLinkFacts, collect_module_function_link_facts,
};
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::module_compilation::generated::test_fixtures::PublishedBoundary;
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallSummary,
};
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, GeneratedFunctionIdentity, ModulePrivateExecutableCategory,
    ModulePrivateExecutableIdentity, ModuleRootRole, OriginFunctionId, StableModuleOriginIdentity,
    StablePackageIdentity,
};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use std::collections::VecDeque;

fn module_origin() -> StableModuleOriginIdentity {
    StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("convergence-tests"),
        "main".to_owned(),
        ModuleRootRole::Normal,
    )
}

fn generated_identity(name: &str) -> GeneratedFunctionIdentity {
    GeneratedFunctionIdentity::new(
        GeneratedDeclarationIdentity::ModulePrivate(ModulePrivateExecutableIdentity::new(
            module_origin(),
            "@page.moth".to_owned(),
            ModulePrivateExecutableCategory::GenericFunction,
            name.to_owned(),
            None,
        )),
        Box::new([CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int)]),
        Box::new([]),
    )
}

fn private_identity(name: &str) -> ModulePrivateExecutableIdentity {
    ModulePrivateExecutableIdentity::new(
        module_origin(),
        "@page.moth".to_owned(),
        ModulePrivateExecutableCategory::FreeFunction,
        name.to_owned(),
        None,
    )
}

fn origin(name: &str) -> OriginFunctionId {
    OriginFunctionId::new_free(module_origin(), name.to_owned())
}

fn summary(return_alias: FunctionReturnAliasSummary) -> PublicCallSummary {
    PublicCallSummary {
        parameters: Vec::new(),
        return_alias,
        escapes_builtin_failure: false,
    }
}

fn link_facts_for_calls(targets: Vec<CallTarget>) -> HirModuleLinkFacts {
    let mut module = HirModule::new();
    module.functions.push(HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: Vec::new(),
        return_type: TypeId(0),
    });
    module
        .function_provenance
        .insert(FunctionId(0), Default::default());
    module.blocks.push(HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: Vec::new(),
        statements: targets
            .into_iter()
            .enumerate()
            .map(|(index, target)| HirStatement {
                id: HirNodeId(index as u32),
                kind: HirStatementKind::Call {
                    target,
                    args: Vec::new(),
                    result: None,
                },
                span: None,
            })
            .collect(),
        terminator: HirTerminator::RuntimeFailure {
            message: "test convergence model".to_owned(),
            cause: None,
        },
    });
    collect_module_function_link_facts(&module).expect("test HIR should produce link facts")
}

fn base_hir(
    public_origins: &[OriginFunctionId],
    private_identities: &[ModulePrivateExecutableIdentity],
) -> HirModule {
    let mut hir = HirModule::new();
    let function_count = public_origins.len() + private_identities.len();
    for index in 0..function_count {
        hir.functions.push(HirFunction {
            id: FunctionId(index as u32),
            entry: BlockId(0),
            params: Vec::new(),
            return_type: TypeId(0),
        });
        hir.function_provenance
            .insert(FunctionId(index as u32), Default::default());
    }
    for (index, origin) in public_origins.iter().enumerate() {
        hir.function_ids_by_origin
            .insert(origin.clone(), FunctionId(index as u32));
    }
    for (offset, identity) in private_identities.iter().enumerate() {
        hir.function_ids_by_private_origin.insert(
            identity.clone(),
            FunctionId((public_origins.len() + offset) as u32),
        );
    }
    hir
}

fn report(
    summaries: impl IntoIterator<Item = (FunctionId, PublicCallSummary)>,
) -> BorrowCheckReport {
    let mut report = BorrowCheckReport::default();
    report.analysis.public_call_summaries.extend(summaries);
    report
}

#[test]
fn convergence_model_sorts_nodes_and_classifies_validated_call_targets() {
    let alpha = generated_identity("alpha");
    let beta = generated_identity("beta");
    let unknown = generated_identity("unknown");
    let private = private_identity("private");
    let cross_module = OriginFunctionId::new_free(module_origin(), "cross".to_owned());

    let base_facts = link_facts_for_calls(vec![
        CallTarget::Generated(alpha.clone()),
        CallTarget::Generated(unknown),
        CallTarget::ModulePrivate(private.clone()),
        CallTarget::Local(FunctionId(0)),
        CallTarget::CrossModule(cross_module),
        CallTarget::External(ExternalFunctionId::Synthetic(1)),
    ]);
    let alpha_facts = link_facts_for_calls(vec![CallTarget::Generated(beta.clone())]);
    let beta_facts = link_facts_for_calls(vec![
        CallTarget::Generated(alpha.clone()),
        CallTarget::ModulePrivate(private),
    ]);

    let model = ConvergenceModel::from_link_facts(
        &base_facts,
        vec![(&beta, &beta_facts), (&alpha, &alpha_facts)],
    )
    .unwrap();

    assert_eq!(model.node_count(), 3);
    assert_eq!(
        model.node(ConvergenceNodeId(0)),
        Some(&ConvergenceNode::BaseModule)
    );
    assert_eq!(
        model.node(ConvergenceNodeId(1)),
        Some(&ConvergenceNode::Generated(Box::new(alpha.clone())))
    );
    assert_eq!(
        model.node(ConvergenceNodeId(2)),
        Some(&ConvergenceNode::Generated(Box::new(beta.clone())))
    );
    assert_eq!(
        model.callers(ConvergenceNodeId(1)),
        Some(&[ConvergenceNodeId(0), ConvergenceNodeId(2)][..])
    );
    assert_eq!(
        model.callers(ConvergenceNodeId(2)),
        Some(&[ConvergenceNodeId(1)][..])
    );
    assert_eq!(
        model.callers(ConvergenceNodeId(0)),
        Some(&[ConvergenceNodeId(2)][..])
    );
    assert_eq!(
        model.active_public_callees(ConvergenceNodeId(0)),
        Some(&[][..]),
        "a provider CrossModule target remains a fixed leaf"
    );
    assert_eq!(
        model.active_public_callees(ConvergenceNodeId(1)),
        Some(&[][..]),
        "an unknown CrossModule target remains a fixed leaf"
    );
    assert_eq!(
        model.dirty_nodes([ConvergenceNodeId(1)]),
        vec![
            ConvergenceNodeId(0),
            ConvergenceNodeId(1),
            ConvergenceNodeId(2)
        ]
    );
    assert_eq!(
        model.dirty_nodes([ConvergenceNodeId(0)]),
        vec![
            ConvergenceNodeId(0),
            ConvergenceNodeId(1),
            ConvergenceNodeId(2)
        ]
    );
}

#[test]
fn convergence_model_keeps_provider_private_calls_as_fixed_leaves() {
    let generated = generated_identity("generated");
    let base_private = private_identity("base_private");
    let provider_private = private_identity("provider_private");
    let generated_facts = link_facts_for_calls(vec![
        CallTarget::ModulePrivate(base_private.clone()),
        CallTarget::ModulePrivate(provider_private.clone()),
    ]);
    let base_facts = link_facts_for_calls(Vec::new());
    let mut base_private_identities = rustc_hash::FxHashSet::default();
    base_private_identities.insert(base_private.clone());

    let model = ConvergenceModel::from_link_facts_for_base_callees(
        &base_facts,
        vec![(&generated, &generated_facts)],
        &rustc_hash::FxHashSet::default(),
        &base_private_identities,
    )
    .unwrap();

    assert_eq!(
        model.callers(ConvergenceNodeId(0)),
        Some(&[ConvergenceNodeId(1)][..])
    );
    assert_eq!(
        model.module_private_callees(ConvergenceNodeId(1)),
        Some(&[base_private][..])
    );
}

#[test]
fn convergence_model_classifies_active_base_public_cross_module_calls() {
    let generated = generated_identity("generated");
    let active_public = OriginFunctionId::new_free(module_origin(), "public_helper".to_owned());
    let generated_facts =
        link_facts_for_calls(vec![CallTarget::CrossModule(active_public.clone())]);
    let base_facts = link_facts_for_calls(Vec::new());
    let mut base_public_origins = rustc_hash::FxHashSet::default();
    base_public_origins.insert(active_public.clone());

    let model = ConvergenceModel::from_link_facts_for_base_callees(
        &base_facts,
        vec![(&generated, &generated_facts)],
        &base_public_origins,
        &rustc_hash::FxHashSet::default(),
    )
    .unwrap();

    assert_eq!(
        model.callers(ConvergenceNodeId(0)),
        Some(&[ConvergenceNodeId(1)][..])
    );
    assert_eq!(
        model.active_public_callees(ConvergenceNodeId(1)),
        Some(&[active_public][..])
    );
}

#[test]
fn convergence_models_keep_equal_identities_local_to_each_boundary() {
    let identity = generated_identity("shared");
    let first_base = link_facts_for_calls(Vec::new());
    let second_base = link_facts_for_calls(Vec::new());
    let first_generated = link_facts_for_calls(Vec::new());
    let second_generated = link_facts_for_calls(Vec::new());

    let first = ConvergenceModel::from_link_facts(&first_base, vec![(&identity, &first_generated)])
        .unwrap();
    let second =
        ConvergenceModel::from_link_facts(&second_base, vec![(&identity, &second_generated)])
            .unwrap();

    assert_eq!(
        first.node_id(&ConvergenceNode::Generated(Box::new(identity.clone()))),
        Some(ConvergenceNodeId(1))
    );
    assert_eq!(
        second.node_id(&ConvergenceNode::Generated(Box::new(identity))),
        Some(ConvergenceNodeId(1))
    );
    assert_eq!(first.callers(ConvergenceNodeId(1)), Some(&[][..]));
    assert_eq!(second.callers(ConvergenceNodeId(1)), Some(&[][..]));
}

#[test]
fn convergence_model_rejects_duplicate_local_generated_identities() {
    let identity = generated_identity("duplicate");
    let base_facts = link_facts_for_calls(Vec::new());
    let first_generated = link_facts_for_calls(Vec::new());
    let second_generated = link_facts_for_calls(Vec::new());

    let error = ConvergenceModel::from_link_facts(
        &base_facts,
        vec![
            (&identity, &first_generated),
            (&identity, &second_generated),
        ],
    )
    .unwrap_err();

    assert!(error.msg.contains("duplicate generated identity"));
}

#[test]
fn convergence_base_changes_enqueue_only_callers_with_changed_direct_inputs() {
    let public_a = origin("public_a");
    let public_b = origin("public_b");
    let private_a = private_identity("private_a");
    let private_b = private_identity("private_b");
    let generated_a = generated_identity("generated_a");
    let generated_b = generated_identity("generated_b");
    let base_hir = base_hir(
        &[public_a.clone(), public_b.clone()],
        &[private_a.clone(), private_b.clone()],
    );
    let previous = report([
        (FunctionId(0), summary(FunctionReturnAliasSummary::Fresh)),
        (FunctionId(1), summary(FunctionReturnAliasSummary::Fresh)),
        (FunctionId(2), summary(FunctionReturnAliasSummary::Fresh)),
        (FunctionId(3), summary(FunctionReturnAliasSummary::Fresh)),
    ]);
    let next = report([
        (FunctionId(0), summary(FunctionReturnAliasSummary::Unknown)),
        (FunctionId(1), summary(FunctionReturnAliasSummary::Fresh)),
        (FunctionId(2), summary(FunctionReturnAliasSummary::Unknown)),
        (FunctionId(3), summary(FunctionReturnAliasSummary::Fresh)),
    ]);
    let changes = base_summary_changes(
        &base_hir,
        &previous.analysis.public_call_summaries,
        &next.analysis.public_call_summaries,
    )
    .expect("a widening base summary should be accepted");
    assert_eq!(changes.public, vec![public_a.clone()]);
    assert_eq!(changes.module_private, vec![private_a.clone()]);

    let base_facts = link_facts_for_calls(Vec::new());
    let generated_a_facts = link_facts_for_calls(vec![
        CallTarget::CrossModule(public_a.clone()),
        CallTarget::ModulePrivate(private_a.clone()),
    ]);
    let generated_b_facts = link_facts_for_calls(vec![
        CallTarget::CrossModule(public_b.clone()),
        CallTarget::ModulePrivate(private_b.clone()),
    ]);
    let mut base_public_origins = rustc_hash::FxHashSet::default();
    base_public_origins.insert(public_a);
    base_public_origins.insert(public_b);
    let mut base_private_identities = rustc_hash::FxHashSet::default();
    base_private_identities.insert(private_a);
    base_private_identities.insert(private_b);
    let model = ConvergenceModel::from_link_facts_for_base_callees(
        &base_facts,
        vec![
            (&generated_b, &generated_b_facts),
            (&generated_a, &generated_a_facts),
        ],
        &base_public_origins,
        &base_private_identities,
    )
    .expect("test convergence model should build");

    let mut queue = VecDeque::new();
    let mut queued_nodes = vec![false; model.node_count()];
    enqueue_base_dependents(&model, &changes, &mut queue, &mut queued_nodes)
        .expect("base dependents should enqueue");
    let expected_node = model
        .node_id(&ConvergenceNode::Generated(Box::new(generated_a)))
        .expect("generated A should have a node");
    assert_eq!(
        queue.into_iter().collect::<Vec<_>>(),
        vec![expected_node],
        "only the sidecar calling the widened public/private summaries should be rechecked"
    );
}

#[test]
fn convergence_install_reports_changes_and_preserves_provider_summaries() {
    let active_public = origin("active_public");
    let provider_public = origin("provider_public");
    let active_private = private_identity("active_private");
    let provider_private = private_identity("provider_private");
    let generated = generated_identity("generated");
    let stale = summary(FunctionReturnAliasSummary::Fresh);
    let widened = summary(FunctionReturnAliasSummary::Unknown);
    let mut hir = HirModule::new();
    hir.imported_call_summaries
        .insert(active_public.clone(), stale.clone());
    hir.imported_call_summaries
        .insert(provider_public.clone(), stale.clone());
    hir.module_private_call_summaries
        .insert(provider_private.clone(), stale.clone());

    // Each case installs a complete direct-summary set and expects the change result that
    // decides whether the node's borrow report may be reused.
    let generated_stale = [(generated.clone(), stale.clone())];
    let generated_widened = [(generated.clone(), widened.clone())];
    let public_widened = [(active_public.clone(), widened.clone())];
    let private_stale = [(active_private.clone(), stale.clone())];
    let private_widened = [(active_private.clone(), widened.clone())];
    let cases: [(&str, &[_], &[_], &[_], bool); 9] = [
        ("public replacement", &[], &public_widened, &[], true),
        ("unchanged public", &[], &public_widened, &[], false),
        (
            "generated insertion",
            &generated_stale,
            &public_widened,
            &[],
            true,
        ),
        (
            "unchanged generated",
            &generated_stale,
            &public_widened,
            &[],
            false,
        ),
        (
            "generated replacement",
            &generated_widened,
            &public_widened,
            &[],
            true,
        ),
        ("generated removal", &[], &public_widened, &[], true),
        (
            "private insertion",
            &[],
            &public_widened,
            &private_stale,
            true,
        ),
        (
            "private replacement",
            &[],
            &public_widened,
            &private_widened,
            true,
        ),
        (
            "unchanged private",
            &[],
            &public_widened,
            &private_widened,
            false,
        ),
    ];
    for (case, generated_summaries, public_summaries, private_summaries, expected) in cases {
        assert_eq!(
            install_convergence_summaries(
                &mut hir,
                generated_summaries,
                public_summaries,
                private_summaries,
            ),
            expected,
            "{case}"
        );
    }

    assert!(hir.generated_call_summaries.is_empty());
    assert_eq!(
        hir.imported_call_summaries.get(&active_public),
        Some(&widened)
    );
    assert_eq!(
        hir.module_private_call_summaries.get(&active_private),
        Some(&widened)
    );
    assert_eq!(
        hir.imported_call_summaries.get(&provider_public),
        Some(&stale),
        "provider CrossModule leaves must not be rewritten"
    );
    assert_eq!(
        hir.module_private_call_summaries.get(&provider_private),
        Some(&stale),
        "provider-private leaves must not be rewritten"
    );
}

#[test]
fn convergence_direct_summaries_read_exact_active_base_report_facts() {
    let active_public = origin("active_public");
    let active_private = private_identity("active_private");
    let generated = generated_identity("generated");
    let base_hir = base_hir(
        std::slice::from_ref(&active_public),
        std::slice::from_ref(&active_private),
    );
    let base_report = report([
        (FunctionId(0), summary(FunctionReturnAliasSummary::Unknown)),
        (FunctionId(1), summary(FunctionReturnAliasSummary::Fresh)),
    ]);
    let base_facts = link_facts_for_calls(Vec::new());
    let generated_facts = link_facts_for_calls(vec![
        CallTarget::CrossModule(active_public.clone()),
        CallTarget::ModulePrivate(active_private.clone()),
    ]);
    let mut base_public_origins = rustc_hash::FxHashSet::default();
    base_public_origins.insert(active_public.clone());
    let mut base_private_identities = rustc_hash::FxHashSet::default();
    base_private_identities.insert(active_private.clone());
    let model = ConvergenceModel::from_link_facts_for_base_callees(
        &base_facts,
        vec![(&generated, &generated_facts)],
        &base_public_origins,
        &base_private_identities,
    )
    .expect("test active-base model should build");
    let node = model
        .node_id(&ConvergenceNode::Generated(Box::new(generated)))
        .expect("generated node should exist");
    let published = PublishedBoundary::empty();
    let transaction = GeneratedFunctionTransaction::new(published.view());

    let direct = direct_convergence_summaries(&model, node, &transaction, &base_hir, &base_report)
        .expect("active-base summaries should resolve from the base report");

    assert_eq!(direct.generated, Vec::new());
    assert_eq!(
        direct.active_public,
        vec![(active_public, summary(FunctionReturnAliasSummary::Unknown))]
    );
    assert_eq!(
        direct.module_private,
        vec![(active_private, summary(FunctionReturnAliasSummary::Fresh))]
    );
}

#[test]
fn convergence_base_changes_reject_a_narrowing_report() {
    let public_origin = origin("public");
    let hir = base_hir(&[public_origin], &[]);
    let previous = report([(FunctionId(0), summary(FunctionReturnAliasSummary::Unknown))]);
    let next = report([(FunctionId(0), summary(FunctionReturnAliasSummary::Fresh))]);

    let error = base_summary_changes(
        &hir,
        &previous.analysis.public_call_summaries,
        &next.analysis.public_call_summaries,
    )
    .expect_err("a narrowing base summary must stop convergence");
    assert!(error.msg.contains("narrowed"));
}

#[test]
fn builtin_failure_widening_uses_existing_base_change_lane() {
    let public = origin("public");
    let private = private_identity("private");
    let hir = base_hir(
        std::slice::from_ref(&public),
        std::slice::from_ref(&private),
    );
    let initial = summary(FunctionReturnAliasSummary::Fresh);
    let mut escaping = initial.clone();
    escaping.escapes_builtin_failure = true;
    let previous = report([(FunctionId(0), initial.clone()), (FunctionId(1), initial)]);
    let next = report([(FunctionId(0), escaping.clone()), (FunctionId(1), escaping)]);

    let changes = base_summary_changes(
        &hir,
        &previous.analysis.public_call_summaries,
        &next.analysis.public_call_summaries,
    )
    .unwrap();
    assert_eq!(changes.public, vec![public]);
    assert_eq!(changes.module_private, vec![private]);
    assert!(
        base_summary_changes(
            &hir,
            &next.analysis.public_call_summaries,
            &previous.analysis.public_call_summaries
        )
        .is_err()
    );
}

#[test]
fn missing_base_summary_is_not_an_infallible_summary() {
    let hir = base_hir(&[origin("public")], &[]);
    let complete = report([(FunctionId(0), summary(FunctionReturnAliasSummary::Fresh))]);
    let missing = BorrowCheckReport::default();

    assert!(
        base_summary_changes(
            &hir,
            &missing.analysis.public_call_summaries,
            &complete.analysis.public_call_summaries
        )
        .is_err()
    );
    assert!(
        base_summary_changes(
            &hir,
            &complete.analysis.public_call_summaries,
            &missing.analysis.public_call_summaries
        )
        .is_err()
    );
}

#[test]
fn builtin_failure_inference_requires_semantic_facts_and_call_summary() {
    let hir = base_hir(&[origin("public")], &[]);
    let mut complete = report([(FunctionId(0), summary(FunctionReturnAliasSummary::Fresh))]);
    let mut missing = BorrowCheckReport::default();

    assert!(infer_builtin_failure_summaries(&hir, &mut missing).is_err());
    assert!(infer_builtin_failure_summaries(&hir, &mut complete).is_err());
}

fn failure_contributor(source: HirBuiltinFailureSource) -> HirBuiltinFailureContributor {
    let codes = match &source {
        HirBuiltinFailureSource::NumericOperation
        | HirBuiltinFailureSource::CompoundWriteBack { .. } => {
            vec![BuiltinErrorCode::IntOverflow]
        }
        HirBuiltinFailureSource::Call(_) => vec![],
    };
    HirBuiltinFailureContributor {
        source,
        span: None,
        codes,
    }
}

fn failure_facts(
    boundary: HirBuiltinFailureBoundary,
    sources: Vec<HirBuiltinFailureSource>,
) -> HirFunctionFailureFacts {
    HirFunctionFailureFacts {
        span: None,
        boundary,
        contributors: sources.into_iter().map(failure_contributor).collect(),
        assertion_message_calls: vec![],
        deferred_custom_catches: vec![],
    }
}

#[test]
fn numeric_failure_contributor_rejects_empty_or_non_implicit_catalog_codes() {
    for codes in [
        vec![],
        vec![BuiltinErrorCode::Unsupported],
        vec![BuiltinErrorCode::FloatBoundaryNonFinite],
        vec![BuiltinErrorCode::FloatFormatInvariant],
        vec![
            BuiltinErrorCode::IntOverflow,
            BuiltinErrorCode::FloatFormatInvariant,
        ],
    ] {
        let mut hir = base_hir(&[], &[private_identity("producer")]);
        let mut facts = failure_facts(
            HirBuiltinFailureBoundary::InferPrivate,
            vec![HirBuiltinFailureSource::NumericOperation],
        );
        facts.contributors[0].codes = codes;
        hir.function_failure_facts.insert(FunctionId(0), facts);
        let mut report = report([(FunctionId(0), summary(FunctionReturnAliasSummary::Fresh))]);
        assert!(infer_builtin_failure_summaries(&hir, &mut report).is_err());
    }
}

#[test]
fn builtin_failure_inference_reaches_local_recursive_fixed_point() {
    let mut hir = base_hir(
        &[],
        &[
            private_identity("first"),
            private_identity("second"),
            private_identity("producer"),
        ],
    );
    for index in 0..3 {
        let mut sources = vec![HirBuiltinFailureSource::Call(CallTarget::Local(
            FunctionId((index + 1) % 3),
        ))];
        if index == 2 {
            sources.push(HirBuiltinFailureSource::NumericOperation);
        }
        hir.function_failure_facts.insert(
            FunctionId(index),
            failure_facts(HirBuiltinFailureBoundary::InferPrivate, sources),
        );
    }
    let mut report = report((0..3).map(|index| {
        (
            FunctionId(index),
            summary(FunctionReturnAliasSummary::Fresh),
        )
    }));
    infer_builtin_failure_summaries(&hir, &mut report).unwrap();
    assert!(
        report
            .analysis
            .public_call_summaries
            .values()
            .all(|summary| { summary.escapes_builtin_failure })
    );
}

#[test]
fn declared_error_and_export_boundaries_never_forward_builtin_failure_bit() {
    for boundary in [
        HirBuiltinFailureBoundary::BuiltinErrorSlot,
        HirBuiltinFailureBoundary::CustomErrorSlot(TypeId(7)),
        HirBuiltinFailureBoundary::ExportedNoSlot,
    ] {
        let mut hir = base_hir(&[origin("boundary")], &[]);
        hir.function_failure_facts.insert(
            FunctionId(0),
            failure_facts(boundary, vec![HirBuiltinFailureSource::NumericOperation]),
        );
        let mut report = report([(FunctionId(0), summary(FunctionReturnAliasSummary::Fresh))]);
        infer_builtin_failure_summaries(&hir, &mut report).unwrap();
        assert!(!report.analysis.public_call_summaries[&FunctionId(0)].escapes_builtin_failure);
        assert_eq!(
            builtin_failure_diagnostic(&hir, &report, FunctionId(0))
                .unwrap()
                .is_some(),
            boundary != HirBuiltinFailureBoundary::BuiltinErrorSlot,
        );
    }
}

fn failure_witness_span(index: u32) -> Option<SourceSpan> {
    let mut builder = ExtendedSpanBuilder::new();
    let local = LocalSpan::exact(index * 10, 3, &mut builder).unwrap();
    Some(SourceSpan::new(
        SourceId::from_index(index as usize + 1),
        local,
    ))
}

fn exported_failure_witness(hir: &HirModule, report: &BorrowCheckReport) -> BuiltinFailureWitness {
    let diagnostic = builtin_failure_diagnostic(hir, report, FunctionId(0))
        .unwrap()
        .unwrap();
    assert_eq!(diagnostic.primary_span, failure_witness_span(0));
    let DiagnosticPayload::InvalidFallibleHandling {
        reason: InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction { witness },
    } = diagnostic.payload
    else {
        panic!("expected exported builtin failure reason");
    };
    let mut spans: Vec<_> = witness.call_spans.iter().copied().flatten().collect();
    spans.extend(witness.origin_span);
    assert_eq!(
        diagnostic
            .labels
            .iter()
            .filter_map(|label| label.span)
            .collect::<Vec<_>>(),
        spans
    );
    *witness
}

#[test]
fn builtin_failure_witness_resolves_local_private_and_generated_hops_in_order() {
    let private = private_identity("second");
    let generated = generated_identity("producer");
    let mut hir = base_hir(
        &[origin("boundary")],
        &[
            private_identity("first"),
            private.clone(),
            private_identity("producer"),
        ],
    );
    hir.function_ids_by_generated
        .insert(generated.clone(), FunctionId(3));
    let sources = [
        HirBuiltinFailureSource::Call(CallTarget::Local(FunctionId(1))),
        HirBuiltinFailureSource::Call(CallTarget::ModulePrivate(private.clone())),
        HirBuiltinFailureSource::Call(CallTarget::Generated(generated.clone())),
        HirBuiltinFailureSource::NumericOperation,
    ];
    for (index, source) in sources.into_iter().enumerate() {
        let boundary = if index == 0 {
            HirBuiltinFailureBoundary::ExportedNoSlot
        } else {
            HirBuiltinFailureBoundary::InferPrivate
        };
        let mut facts = failure_facts(boundary, vec![source]);
        facts.span = failure_witness_span(index as u32);
        facts.contributors[0].span = failure_witness_span(index as u32 + 4);
        if index == 3 {
            facts.contributors[0].codes = vec![
                BuiltinErrorCode::DivideByZero,
                BuiltinErrorCode::IntOverflow,
                BuiltinErrorCode::InvalidRangeStep,
                BuiltinErrorCode::RangeStepNoProgress,
            ];
        }
        hir.function_failure_facts
            .insert(FunctionId(index as u32), facts);
    }
    let mut escaping = summary(FunctionReturnAliasSummary::Fresh);
    escaping.escapes_builtin_failure = true;
    hir.module_private_call_summaries
        .insert(private, escaping.clone());
    hir.generated_call_summaries.insert(generated, escaping);
    let mut report = report((0..4).map(|index| {
        (
            FunctionId(index),
            summary(FunctionReturnAliasSummary::Fresh),
        )
    }));
    infer_builtin_failure_summaries(&hir, &mut report).unwrap();

    let witness = exported_failure_witness(&hir, &report);
    assert_eq!(
        witness.call_spans,
        (4..7).map(failure_witness_span).collect::<Vec<_>>()
    );
    assert_eq!(witness.origin_span, failure_witness_span(7));
    assert_eq!(
        witness.codes,
        hir.function_failure_facts[&FunctionId(3)].contributors[0].codes
    );
    assert_eq!(witness.elided_call_hops, 0);
}

#[test]
fn builtin_failure_witness_keeps_three_calls_and_origin_after_elided_middle_hops() {
    let mut hir = base_hir(
        &[origin("boundary")],
        &(1..6)
            .map(|index| private_identity(&format!("helper_{index}")))
            .collect::<Vec<_>>(),
    );
    for index in 0..6 {
        let boundary = if index == 0 {
            HirBuiltinFailureBoundary::ExportedNoSlot
        } else {
            HirBuiltinFailureBoundary::InferPrivate
        };
        let source = if index == 5 {
            HirBuiltinFailureSource::NumericOperation
        } else {
            HirBuiltinFailureSource::Call(CallTarget::Local(FunctionId(index + 1)))
        };
        let mut facts = failure_facts(boundary, vec![source]);
        facts.span = failure_witness_span(index);
        facts.contributors[0].span = failure_witness_span(index + 6);
        hir.function_failure_facts.insert(FunctionId(index), facts);
    }
    let mut report = report((0..6).map(|index| {
        (
            FunctionId(index),
            summary(FunctionReturnAliasSummary::Fresh),
        )
    }));
    infer_builtin_failure_summaries(&hir, &mut report).unwrap();

    let witness = exported_failure_witness(&hir, &report);
    assert_eq!(
        witness.call_spans,
        (6..9).map(failure_witness_span).collect::<Vec<_>>()
    );
    assert_eq!(witness.origin_span, failure_witness_span(11));
    assert_eq!(witness.codes, vec![BuiltinErrorCode::IntOverflow]);
    assert_eq!(witness.elided_call_hops, 2);
}

#[test]
fn builtin_failure_witness_skips_recursive_back_edge_to_reach_later_origin() {
    let mut hir = base_hir(&[origin("boundary")], &[private_identity("recursive")]);
    let mut boundary = failure_facts(
        HirBuiltinFailureBoundary::ExportedNoSlot,
        vec![HirBuiltinFailureSource::Call(CallTarget::Local(
            FunctionId(1),
        ))],
    );
    boundary.span = failure_witness_span(0);
    boundary.contributors[0].span = failure_witness_span(2);
    hir.function_failure_facts.insert(FunctionId(0), boundary);
    let mut recursive = failure_facts(
        HirBuiltinFailureBoundary::InferPrivate,
        vec![
            HirBuiltinFailureSource::Call(CallTarget::Local(FunctionId(1))),
            HirBuiltinFailureSource::NumericOperation,
        ],
    );
    recursive.contributors[0].span = failure_witness_span(3);
    recursive.contributors[1].span = failure_witness_span(4);
    hir.function_failure_facts.insert(FunctionId(1), recursive);
    let mut report = report((0..2).map(|index| {
        (
            FunctionId(index),
            summary(FunctionReturnAliasSummary::Fresh),
        )
    }));
    infer_builtin_failure_summaries(&hir, &mut report).unwrap();

    // The self-recursive call precedes the numeric producer in source order, but the
    // witness skips only the back-edge: the earlier boundary hop stays visible while the
    // recursive hop contributes no label and the later origin is retained.
    let witness = exported_failure_witness(&hir, &report);
    assert_eq!(witness.call_spans, vec![failure_witness_span(2)]);
    assert_eq!(witness.origin_span, failure_witness_span(4));
    assert_eq!(witness.codes, vec![BuiltinErrorCode::IntOverflow]);
    assert_eq!(witness.elided_call_hops, 0);
}

#[test]
fn builtin_failure_witness_skips_mutual_recursion_to_reach_later_origin() {
    let mut hir = base_hir(
        &[origin("boundary")],
        &[private_identity("first"), private_identity("second")],
    );
    let mut boundary = failure_facts(
        HirBuiltinFailureBoundary::ExportedNoSlot,
        vec![HirBuiltinFailureSource::Call(CallTarget::Local(
            FunctionId(1),
        ))],
    );
    boundary.span = failure_witness_span(0);
    boundary.contributors[0].span = failure_witness_span(2);
    hir.function_failure_facts.insert(FunctionId(0), boundary);
    let mut first = failure_facts(
        HirBuiltinFailureBoundary::InferPrivate,
        vec![
            HirBuiltinFailureSource::Call(CallTarget::Local(FunctionId(2))),
            HirBuiltinFailureSource::NumericOperation,
        ],
    );
    first.contributors[0].span = failure_witness_span(3);
    first.contributors[1].span = failure_witness_span(4);
    hir.function_failure_facts.insert(FunctionId(1), first);
    let mut second = failure_facts(
        HirBuiltinFailureBoundary::InferPrivate,
        vec![HirBuiltinFailureSource::Call(CallTarget::Local(
            FunctionId(1),
        ))],
    );
    second.contributors[0].span = failure_witness_span(5);
    hir.function_failure_facts.insert(FunctionId(2), second);
    let mut report = report((0..3).map(|index| {
        (
            FunctionId(index),
            summary(FunctionReturnAliasSummary::Fresh),
        )
    }));
    infer_builtin_failure_summaries(&hir, &mut report).unwrap();

    // The mutual cycle (first -> second -> first) carries no origin of its own: the witness
    // backtracks past the second hop and resolves the later numeric producer in the first
    // helper, so only the entry hop stays visible.
    let witness = exported_failure_witness(&hir, &report);
    assert_eq!(witness.call_spans, vec![failure_witness_span(2)]);
    assert_eq!(witness.origin_span, failure_witness_span(4));
    assert_eq!(witness.codes, vec![BuiltinErrorCode::IntOverflow]);
    assert_eq!(witness.elided_call_hops, 0);
}

#[test]
fn builtin_failure_witness_expands_each_recursive_body_once_before_late_origin() {
    // WHAT: every private body calls every other body before body 1's only numeric producer,
    //       so the origin is reachable only after the whole mutually recursive component.
    // WHY: path-only backtracking re-entered exhausted bodies through every simple path. The
    //      bounded search enters each body once, matching the AST witness for the same shape.
    const BODY_COUNT: u32 = 12;
    let private_names = (1..=BODY_COUNT)
        .map(|body| private_identity(&format!("body_{body}")))
        .collect::<Vec<_>>();
    let mut hir = base_hir(&[origin("boundary")], &private_names);
    let mut boundary = failure_facts(
        HirBuiltinFailureBoundary::ExportedNoSlot,
        vec![HirBuiltinFailureSource::Call(CallTarget::Local(
            FunctionId(1),
        ))],
    );
    boundary.span = failure_witness_span(0);
    boundary.contributors[0].span = failure_witness_span(1);
    hir.function_failure_facts.insert(FunctionId(0), boundary);
    for body in 1..=BODY_COUNT {
        let mut sources = (1..=BODY_COUNT)
            .filter(|callee| *callee != body)
            .map(|callee| HirBuiltinFailureSource::Call(CallTarget::Local(FunctionId(callee))))
            .collect::<Vec<_>>();
        if body == 1 {
            sources.push(HirBuiltinFailureSource::NumericOperation);
        }
        let mut facts = failure_facts(HirBuiltinFailureBoundary::InferPrivate, sources);
        if body == 1 {
            let origin = facts.contributors.last_mut().expect("body 1 has an origin");
            origin.span = failure_witness_span(2);
        }
        hir.function_failure_facts.insert(FunctionId(body), facts);
    }
    let mut report = report((0..=BODY_COUNT).map(|index| {
        (
            FunctionId(index),
            summary(FunctionReturnAliasSummary::Fresh),
        )
    }));
    infer_builtin_failure_summaries(&hir, &mut report).unwrap();

    let boundary_call = &hir.function_failure_facts[&FunctionId(0)].contributors[0];
    let mut entered = FxHashSet::from_iter([FunctionId(0)]);
    let witness =
        builtin_failure_witness_search(&hir, &report, boundary_call, &mut entered).unwrap();
    assert_eq!(entered.len(), BODY_COUNT as usize + 1);
    assert_eq!(witness.call_spans, vec![failure_witness_span(1)]);
    assert_eq!(witness.origin_span, failure_witness_span(2));
    assert_eq!(witness.codes, vec![BuiltinErrorCode::IntOverflow]);
    assert_eq!(witness.elided_call_hops, 0);
    assert_eq!(exported_failure_witness(&hir, &report), witness);
}

#[test]
fn builtin_failure_witness_never_classifies_candidates_after_an_early_terminal_leaf() {
    // WHAT: a helper's first candidate is an escaping cross-module leaf, followed by many calls
    //       whose summaries are deliberately absent.
    // WHY: classifying a call without its exact summary is an infrastructure error, so the
    //      search succeeding proves it stopped at the leaf instead of ordering the whole body.
    const UNUSED_CALL_COUNT: usize = 10_000;
    let foreign = origin("foreign");
    let mut hir = base_hir(&[origin("boundary")], &[private_identity("helper")]);
    let mut boundary = failure_facts(
        HirBuiltinFailureBoundary::ExportedNoSlot,
        vec![HirBuiltinFailureSource::Call(CallTarget::Local(
            FunctionId(1),
        ))],
    );
    boundary.contributors[0].span = failure_witness_span(1);
    hir.function_failure_facts.insert(FunctionId(0), boundary);
    let mut sources = vec![HirBuiltinFailureSource::Call(CallTarget::CrossModule(
        foreign.clone(),
    ))];
    sources.extend((0..UNUSED_CALL_COUNT).map(|index| {
        HirBuiltinFailureSource::Call(CallTarget::CrossModule(origin(&format!(
            "unsummarised_{index}"
        ))))
    }));
    let mut helper = failure_facts(HirBuiltinFailureBoundary::InferPrivate, sources);
    helper.contributors[0].span = failure_witness_span(2);
    hir.function_failure_facts.insert(FunctionId(1), helper);
    let mut escaping = summary(FunctionReturnAliasSummary::Fresh);
    escaping.escapes_builtin_failure = true;
    hir.imported_call_summaries
        .insert(foreign, escaping.clone());
    // Inference would classify every helper contributor, so the escape bit is installed directly.
    let report = report([
        (FunctionId(0), summary(FunctionReturnAliasSummary::Fresh)),
        (FunctionId(1), escaping),
    ]);

    let boundary_call = &hir.function_failure_facts[&FunctionId(0)].contributors[0];
    let mut entered = FxHashSet::from_iter([FunctionId(0)]);
    let witness =
        builtin_failure_witness_search(&hir, &report, boundary_call, &mut entered).unwrap();
    assert_eq!(
        witness.call_spans,
        vec![failure_witness_span(1), failure_witness_span(2)]
    );
    assert_eq!(witness.origin_span, None);
    assert!(witness.codes.is_empty());
}

#[test]
fn builtin_failure_witness_keeps_origin_free_recursion_free_of_numeric_failure() {
    let mut hir = base_hir(&[origin("boundary")], &[private_identity("recursive")]);
    let mut boundary = failure_facts(
        HirBuiltinFailureBoundary::ExportedNoSlot,
        vec![HirBuiltinFailureSource::Call(CallTarget::Local(
            FunctionId(1),
        ))],
    );
    boundary.span = failure_witness_span(0);
    boundary.contributors[0].span = failure_witness_span(2);
    hir.function_failure_facts.insert(FunctionId(0), boundary);
    let mut recursive = failure_facts(
        HirBuiltinFailureBoundary::InferPrivate,
        vec![HirBuiltinFailureSource::Call(CallTarget::Local(
            FunctionId(1),
        ))],
    );
    recursive.contributors[0].span = failure_witness_span(3);
    hir.function_failure_facts.insert(FunctionId(1), recursive);
    let mut report = report((0..2).map(|index| {
        (
            FunctionId(index),
            summary(FunctionReturnAliasSummary::Fresh),
        )
    }));
    infer_builtin_failure_summaries(&hir, &mut report).unwrap();

    // Skipping back-edges must not invent an origin: a purely recursive graph converges to
    // no escaping failure, so the exported boundary carries no rejection at all.
    assert!(!report.analysis.public_call_summaries[&FunctionId(0)].escapes_builtin_failure);
    assert!(
        builtin_failure_diagnostic(&hir, &report, FunctionId(0))
            .unwrap()
            .is_none()
    );
}

#[test]
fn builtin_failure_witness_stops_at_cross_module_summary_and_requires_that_summary() {
    let foreign = origin("foreign");
    let mut hir = base_hir(&[origin("boundary")], &[]);
    let mut facts = failure_facts(
        HirBuiltinFailureBoundary::ExportedNoSlot,
        vec![HirBuiltinFailureSource::Call(CallTarget::CrossModule(
            foreign.clone(),
        ))],
    );
    facts.span = failure_witness_span(0);
    facts.contributors[0].span = failure_witness_span(1);
    hir.function_failure_facts.insert(FunctionId(0), facts);
    let mut escaping = summary(FunctionReturnAliasSummary::Fresh);
    escaping.escapes_builtin_failure = true;
    hir.imported_call_summaries
        .insert(foreign.clone(), escaping);
    let report = report([(FunctionId(0), summary(FunctionReturnAliasSummary::Fresh))]);

    let witness = exported_failure_witness(&hir, &report);
    assert_eq!(witness.call_spans, vec![failure_witness_span(1)]);
    assert_eq!(witness.origin_span, None);
    assert!(witness.codes.is_empty());
    assert_eq!(witness.elided_call_hops, 0);

    hir.imported_call_summaries.remove(&foreign);
    assert!(builtin_failure_diagnostic(&hir, &report, FunctionId(0)).is_err());
}

#[test]
fn builtin_failure_witness_stops_at_unavailable_call_before_later_origin() {
    // WHAT: an active cross-module leaf precedes a local numeric producer in source order.
    // WHY: the leaf is terminal, not skippable: the later producer would invent a connected
    //      origin across the hop already recorded as the visible boundary.
    let foreign = origin("foreign");
    let mut hir = base_hir(&[origin("boundary")], &[private_identity("helper")]);
    let mut boundary = failure_facts(
        HirBuiltinFailureBoundary::ExportedNoSlot,
        vec![HirBuiltinFailureSource::Call(CallTarget::Local(
            FunctionId(1),
        ))],
    );
    boundary.span = failure_witness_span(0);
    boundary.contributors[0].span = failure_witness_span(2);
    hir.function_failure_facts.insert(FunctionId(0), boundary);
    let mut helper = failure_facts(
        HirBuiltinFailureBoundary::InferPrivate,
        vec![
            HirBuiltinFailureSource::Call(CallTarget::CrossModule(foreign.clone())),
            HirBuiltinFailureSource::NumericOperation,
        ],
    );
    helper.contributors[0].span = failure_witness_span(3);
    helper.contributors[1].span = failure_witness_span(4);
    hir.function_failure_facts.insert(FunctionId(1), helper);
    let mut escaping = summary(FunctionReturnAliasSummary::Fresh);
    escaping.escapes_builtin_failure = true;
    hir.imported_call_summaries.insert(foreign, escaping);
    let mut report = report((0..2).map(|index| {
        (
            FunctionId(index),
            summary(FunctionReturnAliasSummary::Fresh),
        )
    }));
    infer_builtin_failure_summaries(&hir, &mut report).unwrap();

    let witness = exported_failure_witness(&hir, &report);
    assert_eq!(
        witness.call_spans,
        vec![failure_witness_span(2), failure_witness_span(3)]
    );
    assert_eq!(witness.origin_span, None);
    assert!(witness.codes.is_empty());
    assert_eq!(witness.elided_call_hops, 0);
}

#[test]
fn assertion_message_candidates_validate_without_forwarding_failure() {
    let mut hir = base_hir(
        &[],
        &[private_identity("assertion"), private_identity("producer")],
    );
    let mut assertion = failure_facts(HirBuiltinFailureBoundary::InferPrivate, vec![]);
    assertion
        .assertion_message_calls
        .push(failure_contributor(HirBuiltinFailureSource::Call(
            CallTarget::Local(FunctionId(1)),
        )));
    hir.function_failure_facts.insert(FunctionId(0), assertion);
    hir.function_failure_facts.insert(
        FunctionId(1),
        failure_facts(
            HirBuiltinFailureBoundary::InferPrivate,
            vec![HirBuiltinFailureSource::NumericOperation],
        ),
    );
    let mut report = report((0..2).map(|index| {
        (
            FunctionId(index),
            summary(FunctionReturnAliasSummary::Fresh),
        )
    }));
    infer_builtin_failure_summaries(&hir, &mut report).unwrap();
    assert!(!report.analysis.public_call_summaries[&FunctionId(0)].escapes_builtin_failure);
    assert!(
        builtin_failure_diagnostic(&hir, &report, FunctionId(0))
            .unwrap()
            .is_some()
    );
    report
        .analysis
        .public_call_summaries
        .get_mut(&FunctionId(1))
        .unwrap()
        .escapes_builtin_failure = false;
    assert!(
        builtin_failure_diagnostic(&hir, &report, FunctionId(0))
            .unwrap()
            .is_none()
    );
    report.analysis.public_call_summaries.remove(&FunctionId(1));
    assert!(
        builtin_failure_diagnostic(&hir, &report, FunctionId(0))
            .unwrap()
            .is_some()
    );
}

#[test]
fn omitted_assertion_calls_use_existing_generated_reverse_dependencies() {
    use crate::compiler_frontend::module_compilation::generated::test_fixtures::test_sidecar;
    use crate::compiler_frontend::module_compilation::generated::transaction::GeneratedRequestFacts;

    let identity = generated_identity("assertion");
    let private = private_identity("producer");
    let initial = summary(FunctionReturnAliasSummary::Fresh);
    let mut sidecar = test_sidecar(identity.clone(), initial.clone());
    let mut facts = failure_facts(HirBuiltinFailureBoundary::InferPrivate, vec![]);
    facts
        .assertion_message_calls
        .push(failure_contributor(HirBuiltinFailureSource::Call(
            CallTarget::ModulePrivate(private.clone()),
        )));
    sidecar
        .module
        .executable
        .hir
        .function_failure_facts
        .insert(FunctionId(0), facts);
    let published = PublishedBoundary::empty();
    let mut transaction = GeneratedFunctionTransaction::new(published.view());
    let request = transaction.register_requests([GeneratedRequestFacts {
        identity: identity.clone(),
        display_name: "assertion".to_owned(),
        call_span: None,
    }])[0];
    transaction.enter(request).unwrap();
    transaction
        .complete(request, initial.clone(), sidecar)
        .unwrap();
    let base = base_hir(&[], std::slice::from_ref(&private));
    let base_links = link_facts_for_calls(vec![]);
    let private_identities = FxHashSet::from_iter([private.clone()]);
    let public_origins = FxHashSet::default();
    let mut model = ConvergenceModel::from_link_facts_for_base_callees(
        &base_links,
        transaction.completed_link_facts(),
        &public_origins,
        &private_identities,
    )
    .unwrap();
    model
        .include_deferred_validation_dependencies(
            &base,
            &mut transaction,
            &public_origins,
            &private_identities,
        )
        .unwrap();
    assert_eq!(
        model.callers(ConvergenceNodeId(0)),
        Some(&[ConvergenceNodeId(1)][..])
    );
    let mut queue = VecDeque::new();
    let mut queued = vec![false; model.node_count()];
    enqueue_base_dependents(
        &model,
        &BaseSummaryChanges {
            public: vec![],
            module_private: vec![private],
        },
        &mut queue,
        &mut queued,
    )
    .unwrap();
    assert_eq!(
        queue.into_iter().collect::<Vec<_>>(),
        vec![ConvergenceNodeId(1)]
    );

    let mut escaping = initial.clone();
    escaping.escapes_builtin_failure = true;
    assert!(update_generated_summary(&mut transaction, &identity, escaping).unwrap());
    assert!(
        transaction
            .summary(&identity)
            .unwrap()
            .escapes_builtin_failure
    );
    assert!(update_generated_summary(&mut transaction, &identity, initial).is_err());
}

#[test]
fn builtin_failure_call_resolution_preserves_exact_summary_bits() {
    let private = private_identity("private");
    let generated = generated_identity("generated");
    let imported = origin("imported");
    let mut escaping = summary(FunctionReturnAliasSummary::Fresh);
    escaping.escapes_builtin_failure = true;
    let mut hir = HirModule::new();
    install_convergence_summaries(
        &mut hir,
        &[(generated.clone(), escaping.clone())],
        &[(imported.clone(), escaping.clone())],
        &[(private.clone(), escaping.clone())],
    );
    let report = report([(FunctionId(0), escaping)]);

    for target in [
        CallTarget::Local(FunctionId(0)),
        CallTarget::CrossModule(imported),
        CallTarget::ModulePrivate(private),
        CallTarget::Generated(generated),
    ] {
        assert!(call_escapes_builtin_failure(&hir, &report, &target).unwrap());
    }
    assert!(
        call_escapes_builtin_failure(&hir, &report, &CallTarget::Local(FunctionId(1))).is_err()
    );
    assert!(
        call_escapes_builtin_failure(
            &hir,
            &report,
            &CallTarget::Generated(generated_identity("missing")),
        )
        .is_err()
    );
}

#[test]
fn convergence_callers_queue_reaches_generated_cycle_without_duplicate_entries() {
    let generated_a = generated_identity("generated_a");
    let generated_b = generated_identity("generated_b");
    let base_facts = link_facts_for_calls(Vec::new());
    let generated_a_facts = link_facts_for_calls(vec![CallTarget::Generated(generated_b.clone())]);
    let generated_b_facts = link_facts_for_calls(vec![CallTarget::Generated(generated_a.clone())]);
    let model = ConvergenceModel::from_link_facts(
        &base_facts,
        vec![
            (&generated_b, &generated_b_facts),
            (&generated_a, &generated_a_facts),
        ],
    )
    .expect("test generated cycle should build");
    let node_a = model
        .node_id(&ConvergenceNode::Generated(Box::new(generated_a)))
        .expect("generated A should have a node");
    let node_b = model
        .node_id(&ConvergenceNode::Generated(Box::new(generated_b)))
        .expect("generated B should have a node");

    let mut queue = VecDeque::new();
    let mut queued_nodes = vec![false; model.node_count()];
    enqueue_convergence_callers(&model, node_b, &mut queue, &mut queued_nodes)
        .expect("B callers should enqueue");
    enqueue_convergence_callers(&model, node_a, &mut queue, &mut queued_nodes)
        .expect("A callers should enqueue");

    assert_eq!(
        queue.into_iter().collect::<Vec<_>>(),
        vec![node_a, node_b],
        "the cycle should enqueue each generated caller once"
    );
}

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
fn counter_test_module() -> crate::compiler_frontend::module_compilation::Module {
    use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
    use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
    use crate::compiler_frontend::hir::reachability::HirModuleLinkFacts;
    use crate::compiler_frontend::module_compilation::ModuleRootActivity;
    use crate::compiler_frontend::module_compilation::artefact::{
        ModuleCompilerMetadata, ModuleExecutable, ModuleLinkFacts,
    };
    use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
    use std::path::PathBuf;
    use std::sync::Arc;

    crate::compiler_frontend::module_compilation::Module {
        executable: ModuleExecutable {
            hir: HirModule::new(),
            resource_table: ModuleResourceTable::new(),
            type_environment: TypeEnvironment::new(),
            borrow_analysis: BorrowCheckReport::default(),
            numeric_proofs:
                crate::compiler_frontend::analysis::numeric_proofs::NumericProofs::default(),
            path_table: Arc::new(
                crate::compiler_frontend::symbols::path_interner::PathInternerBuilder::new()
                    .freeze(),
            ),
        },
        link_facts: ModuleLinkFacts {
            external_package_registry: Arc::new(ExternalPackageRegistry::new()),
            external_import_candidates: Vec::new(),
            functions: HirModuleLinkFacts::default(),
        },
        metadata: ModuleCompilerMetadata {
            entry_point: PathBuf::new(),
            warnings: Vec::new(),
            const_top_level_fragments: Vec::new(),
            root_activity: ModuleRootActivity::default(),
            doc_fragments: Vec::new(),
            materialisation_context: None,
        },
    }
}

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
#[test]
fn unchanged_generated_summary_counts_comparison_without_change() {
    use crate::compiler_frontend::instrumentation::{
        capture_frontend_counters_for_test, log_frontend_counters, reset_frontend_counters,
    };
    use crate::compiler_frontend::module_compilation::GeneratedFunctionSidecar;
    use crate::timing::start_benchmark_collection;

    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _counter_capture = capture_frontend_counters_for_test();
    reset_frontend_counters();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");

    let identity = generated_identity("stable");
    let stable = summary(FunctionReturnAliasSummary::Fresh);
    let published = PublishedBoundary::with_sidecar(
        identity.clone(),
        stable.clone(),
        GeneratedFunctionSidecar::new(identity.clone(), counter_test_module()),
    );
    let mut transaction = GeneratedFunctionTransaction::new(published.view());

    assert!(
        !super::update_generated_summary(&mut transaction, &identity, stable)
            .expect("an unchanged summary comparison should succeed")
    );

    log_frontend_counters();
    let observations = timing_session.finish();
    let counter_value = |name: &str| {
        observations
            .counters
            .iter()
            .find(|counter| counter.name == name)
            .map(|counter| counter.value)
            .unwrap_or(-1.0)
    };
    assert_eq!(counter_value("convergence_summary_comparisons"), 1.0);
    assert_eq!(counter_value("convergence_summary_changes"), 0.0);
}

#[test]
fn builtin_failure_witness_writeback_priority_is_local_to_its_arithmetic_at_each_hop() {
    for earlier in [false, true] {
        for private_hop in [false, true] {
            let mut hir = base_hir(&[origin("boundary")], &[private_identity("helper")]);
            let arithmetic_span = failure_witness_span(2);
            let mut helper = failure_facts(
                if private_hop {
                    HirBuiltinFailureBoundary::InferPrivate
                } else {
                    HirBuiltinFailureBoundary::ExportedNoSlot
                },
                vec![
                    HirBuiltinFailureSource::NumericOperation,
                    HirBuiltinFailureSource::CompoundWriteBack {
                        target: TypeId(8),
                        arithmetic_span,
                    },
                ],
            );
            helper.span = failure_witness_span(0);
            helper.contributors[0].span = arithmetic_span;
            helper.contributors[1].span = failure_witness_span(3);
            if earlier {
                let mut unrelated = failure_contributor(HirBuiltinFailureSource::NumericOperation);
                unrelated.span = failure_witness_span(1);
                helper.contributors.insert(0, unrelated);
            }
            let helper_id = FunctionId(u32::from(private_hop));
            hir.function_failure_facts.insert(helper_id, helper);
            if private_hop {
                let mut boundary = failure_facts(
                    HirBuiltinFailureBoundary::ExportedNoSlot,
                    vec![HirBuiltinFailureSource::Call(CallTarget::Local(helper_id))],
                );
                boundary.span = failure_witness_span(0);
                boundary.contributors[0].span = failure_witness_span(4);
                hir.function_failure_facts.insert(FunctionId(0), boundary);
            } else {
                hir.function_failure_facts.insert(
                    FunctionId(1),
                    failure_facts(HirBuiltinFailureBoundary::InferPrivate, vec![]),
                );
            }
            let mut report = report((0..2).map(|index| {
                (
                    FunctionId(index),
                    summary(FunctionReturnAliasSummary::Fresh),
                )
            }));
            infer_builtin_failure_summaries(&hir, &mut report).unwrap();
            let witness = exported_failure_witness(&hir, &report);
            assert_eq!(
                witness.origin_span,
                failure_witness_span(if earlier { 1 } else { 3 })
            );
            assert_eq!(
                witness.origin,
                if earlier {
                    BuiltinFailureOriginKind::Operation
                } else {
                    BuiltinFailureOriginKind::CompoundWriteBack { target: TypeId(8) }
                }
            );
            assert_eq!(witness.call_spans.len(), usize::from(private_hop));
        }
    }
}

#[test]
fn builtin_failure_witness_skipped_back_edge_keeps_writeback_origin() {
    // WHAT: the recursive call precedes one compound update's arithmetic and matching
    //       write-back in source order.
    // WHY: skipping the back-edge must reapply the write-back policy instead of walking
    //      the remainder in raw order (which would report the arithmetic as Operation).
    let mut hir = base_hir(&[origin("boundary")], &[private_identity("recursive")]);
    let mut boundary = failure_facts(
        HirBuiltinFailureBoundary::ExportedNoSlot,
        vec![HirBuiltinFailureSource::Call(CallTarget::Local(
            FunctionId(1),
        ))],
    );
    boundary.span = failure_witness_span(0);
    boundary.contributors[0].span = failure_witness_span(2);
    hir.function_failure_facts.insert(FunctionId(0), boundary);
    let arithmetic_span = failure_witness_span(3);
    let mut recursive = failure_facts(
        HirBuiltinFailureBoundary::InferPrivate,
        vec![
            HirBuiltinFailureSource::Call(CallTarget::Local(FunctionId(1))),
            HirBuiltinFailureSource::NumericOperation,
            HirBuiltinFailureSource::CompoundWriteBack {
                target: TypeId(8),
                arithmetic_span,
            },
        ],
    );
    recursive.contributors[0].span = failure_witness_span(5);
    recursive.contributors[1].span = arithmetic_span;
    recursive.contributors[2].span = failure_witness_span(6);
    hir.function_failure_facts.insert(FunctionId(1), recursive);
    let mut report = report((0..2).map(|index| {
        (
            FunctionId(index),
            summary(FunctionReturnAliasSummary::Fresh),
        )
    }));
    infer_builtin_failure_summaries(&hir, &mut report).unwrap();

    let witness = exported_failure_witness(&hir, &report);
    assert_eq!(witness.call_spans, vec![failure_witness_span(2)]);
    assert_eq!(witness.origin_span, failure_witness_span(6));
    assert_eq!(
        witness.origin,
        BuiltinFailureOriginKind::CompoundWriteBack { target: TypeId(8) }
    );
    assert_eq!(witness.elided_call_hops, 0);
}
// `builtin_failure_witness_keeps_three_calls_and_origin_after_elided_middle_hops`
// above owns the exact synthetic witness assertions (ordered call spans, origin
// span, typed codes, elided count). D02 determinism over real source lives with
// the frontend pipeline owner in
// `src/build_system/tests/compile_project_frontend_tests/package_materialisation_tests.rs`
// (`exported_private_chain_witness_rendering_is_stable_across_repeated_compilation`),
// which compiles real Moth source through the real pipeline and the real terse
// renderer; the synthetic harness here cannot vary declaration order or workers
// (its `Local`-call branch never reads `function_ids_by_private_origin`, and
// semantic jobs stay serial).
