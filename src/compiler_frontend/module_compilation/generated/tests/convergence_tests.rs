use super::*;

use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::{CallTarget, ExternalFunctionId};
use crate::compiler_frontend::hir::blocks::HirBlock;
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
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
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
                location: SourceLocation::default(),
            })
            .collect(),
        terminator: HirTerminator::RuntimeFailure {
            message: "test convergence model".to_owned(),
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
    let changes = base_summary_changes(&base_hir, &previous, &next)
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
fn convergence_install_refreshes_active_public_and_preserves_provider_summaries() {
    let active_public = origin("active_public");
    let provider_public = origin("provider_public");
    let stale = summary(FunctionReturnAliasSummary::Fresh);
    let widened = summary(FunctionReturnAliasSummary::Unknown);
    let mut hir = HirModule::new();
    hir.imported_call_summaries
        .insert(active_public.clone(), stale.clone());
    hir.imported_call_summaries
        .insert(provider_public.clone(), stale.clone());

    install_convergence_summaries(
        &mut hir,
        &[],
        &[(active_public.clone(), widened.clone())],
        &[],
    );

    assert_eq!(
        hir.imported_call_summaries.get(&active_public),
        Some(&widened)
    );
    assert_eq!(
        hir.imported_call_summaries.get(&provider_public),
        Some(&stale),
        "provider CrossModule leaves must not be rewritten"
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

    let error = base_summary_changes(&hir, &previous, &next)
        .expect_err("a narrowing base summary must stop convergence");
    assert!(error.msg.contains("narrowed"));
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
