use super::*;

use crate::build_system::BuildProfile;
use crate::build_system::create_project_modules::ProjectFrontendCompilation;
use crate::build_system::create_project_modules::compile_project_frontend;
use crate::builder_surface::BuilderSurface;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_messages::DiagnosticPayload;
use crate::compiler_frontend::compiler_messages::diagnostic_bag::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::TypeMismatchView;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::diagnostics::legacy_adapter::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, TypeMismatchContext};
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
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use crate::compiler_frontend::module_compilation::generated::test_fixtures::{
    PublishedBoundary, test_sidecar,
};
use crate::compiler_frontend::module_compilation::generated::transaction::{
    GeneratedFunctionTransaction, GeneratedRequestEntry, GeneratedRequestFacts,
};
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallSummary,
};
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, GeneratedFunctionIdentity, ModulePrivateExecutableCategory,
    ModulePrivateExecutableIdentity, ModuleRootRole, OriginFunctionId, StableModuleOriginIdentity,
    StablePackageIdentity,
};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::external_js::js_import_provider::JsExternalImportProvider;
use crate::projects::settings::Config;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use std::collections::VecDeque;
use std::fs;

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
        Box::new([CanonicalTypeIdentity::Builtin(
            CanonicalBuiltinType::FixedScalar(FixedScalar::I32),
        )]),
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
        returns: Box::new([]),
        error_return: None,
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
                    args: crate::compiler_frontend::hir::expression_store::HirValueRange::empty(),
                    results: Vec::new(),
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
            returns: Box::new([]),
            error_return: None,
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
        CallTarget::Generated(Box::new(alpha.clone())),
        CallTarget::Generated(Box::new(unknown)),
        CallTarget::ModulePrivate(private.clone()),
        CallTarget::Local(FunctionId(0)),
        CallTarget::CrossModule(cross_module),
        CallTarget::External(ExternalFunctionId::Synthetic(1)),
    ]);
    let alpha_facts = link_facts_for_calls(vec![CallTarget::Generated(Box::new(beta.clone()))]);
    let beta_facts = link_facts_for_calls(vec![
        CallTarget::Generated(Box::new(alpha.clone())),
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
        HirBuiltinFailureSource::NumericOperation => vec![BuiltinErrorCode::IntOverflow],
        HirBuiltinFailureSource::CompoundWriteBack { .. } => {
            vec![BuiltinErrorCode::IntCastOutOfRange]
        }
        HirBuiltinFailureSource::AuthoredCastConversion | HirBuiltinFailureSource::Call(_) => {
            vec![]
        }
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
        infallible_cast_evidence: None,
    }
}

#[test]
fn numeric_failure_contributor_rejects_empty_or_non_implicit_catalog_codes() {
    for codes in [
        vec![],
        vec![BuiltinErrorCode::Unsupported],
        vec![BuiltinErrorCode::IntParseInvalidFormat],
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
fn authored_cast_failure_contributor_accepts_cast_policy_codes_only() {
    for (code, accepted) in [
        (BuiltinErrorCode::IntParseInvalidFormat, true),
        (BuiltinErrorCode::IntCastOutOfRange, true),
        (BuiltinErrorCode::IntOverflow, false),
        (BuiltinErrorCode::FloatFormatInvariant, false),
    ] {
        let mut hir = base_hir(&[], &[private_identity("producer")]);
        let mut facts = failure_facts(
            HirBuiltinFailureBoundary::InferPrivate,
            vec![HirBuiltinFailureSource::AuthoredCastConversion],
        );
        facts.contributors[0].codes = vec![code];
        hir.function_failure_facts.insert(FunctionId(0), facts);
        let mut report = report([(FunctionId(0), summary(FunctionReturnAliasSummary::Fresh))]);

        assert_eq!(
            infer_builtin_failure_summaries(&hir, &mut report).is_ok(),
            accepted,
            "unexpected implicit cast classification for {code:?}"
        );
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
        HirBuiltinFailureSource::Call(CallTarget::Generated(Box::new(generated.clone()))),
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
        CallTarget::Generated(Box::new(generated)),
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
            &CallTarget::Generated(Box::new(generated_identity("missing"))),
        )
        .is_err()
    );
}

#[test]
fn convergence_callers_queue_reaches_generated_cycle_without_duplicate_entries() {
    let generated_a = generated_identity("generated_a");
    let generated_b = generated_identity("generated_b");
    let base_facts = link_facts_for_calls(Vec::new());
    let generated_a_facts =
        link_facts_for_calls(vec![CallTarget::Generated(Box::new(generated_b.clone()))]);
    let generated_b_facts =
        link_facts_for_calls(vec![CallTarget::Generated(Box::new(generated_a.clone()))]);
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
        capture_counters_for_test, log_frontend_counters, reset_frontend_counters,
    };
    use crate::compiler_frontend::module_compilation::GeneratedFunctionSidecar;
    use crate::timing::start_benchmark_collection;

    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _counter_capture = capture_counters_for_test();
    reset_frontend_counters();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");

    let identity = generated_identity("stable");
    let stable = summary(FunctionReturnAliasSummary::Fresh);
    let published = PublishedBoundary::with_sidecar(
        identity.clone(),
        stable.clone(),
        GeneratedFunctionSidecar::new(identity.clone(), counter_test_module(), Default::default()),
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

#[test]
fn reachable_generated_sidecars_follow_final_live_call_edges() {
    let alpha = generated_identity("alpha");
    let beta = generated_identity("beta");
    let orphan = generated_identity("orphan");
    let base_facts = link_facts_for_calls(vec![CallTarget::Generated(Box::new(alpha.clone()))]);
    let alpha_facts = link_facts_for_calls(vec![CallTarget::Generated(Box::new(beta.clone()))]);
    let beta_facts = link_facts_for_calls(vec![CallTarget::Generated(Box::new(alpha.clone()))]);
    let orphan_facts = link_facts_for_calls(vec![CallTarget::Generated(Box::new(
        generated_identity("orphan_child"),
    ))]);
    let known = PublishedBoundary::empty();
    let mut transaction = GeneratedFunctionTransaction::new(known.view());
    let request = |identity: GeneratedFunctionIdentity, display_name: &str| GeneratedRequestFacts {
        identity,
        display_name: display_name.to_owned(),
        call_span: None,
    };
    let request_ids = transaction.register_requests([
        request(alpha.clone(), "alpha"),
        request(beta.clone(), "beta"),
        request(orphan.clone(), "orphan"),
    ]);

    for request_id in request_ids {
        let identity = transaction.identity(request_id).unwrap().clone();
        assert_eq!(
            transaction.enter(request_id).unwrap(),
            GeneratedRequestEntry::Materialise
        );
        let mut sidecar =
            test_sidecar(identity.clone(), summary(FunctionReturnAliasSummary::Fresh));
        sidecar.module.link_facts.functions = if identity == alpha {
            alpha_facts.clone()
        } else if identity == beta {
            beta_facts.clone()
        } else {
            orphan_facts.clone()
        };
        sidecar.module.executable.hir.expressions.freeze();
        transaction
            .complete(
                request_id,
                summary(FunctionReturnAliasSummary::Fresh),
                sidecar,
            )
            .unwrap();
    }

    let reachable = reachable_generated_sidecars(&base_facts, &transaction);
    assert_eq!(
        reachable,
        FxHashSet::from_iter([alpha.clone(), beta.clone()])
    );

    let mut hir = HirModule::new();
    let expected_summary = summary(FunctionReturnAliasSummary::Fresh);
    hir.generated_call_summaries
        .insert(alpha.clone(), expected_summary.clone());
    hir.generated_call_summaries
        .insert(orphan.clone(), expected_summary);
    retain_live_call_summaries(&mut hir, &base_facts);
    assert_eq!(hir.generated_call_summaries.len(), 1);
    assert!(hir.generated_call_summaries.contains_key(&alpha));

    transaction.retain_reachable_sidecars(&reachable).unwrap();
    assert!(transaction.sidecar_mut(&orphan).is_err());
    let delta = transaction.finish().unwrap();
    assert_eq!(delta.records().len(), 2);
}

fn compile_owned_service_fixture(
    source: &str,
    single_file: bool,
    external_sources: &[(&str, &str)],
) -> (ProjectFrontendCompilation, StringTable) {
    let directory = tempfile::tempdir().expect("the service fixture directory should exist");
    let source_root = directory.path().join("src");
    fs::create_dir_all(&source_root).expect("the service source root should exist");
    let source_path = source_root.join("@page.moth");
    fs::write(&source_path, source).expect("the retained-fact fixture should be written");
    fs::write(
        directory.path().join("config.moth"),
        "project #= (name = \"release\", entry_root = \"src\")\nhtml #= ()\n",
    )
    .expect("the directory fixture config should be written");
    for (path, contents) in external_sources {
        fs::write(source_root.join(path), contents)
            .expect("the external service fixture should be written");
    }
    let entry = if single_file {
        source_path.clone()
    } else {
        directory.path().to_path_buf()
    };
    let mut config = Config::new(entry);
    let mut strings = StringTable::new();
    let mut surface = BuilderSurface::with_mandatory_core();
    if !external_sources.is_empty() {
        surface
            .external_import_providers
            .register(std::sync::Arc::new(JsExternalImportProvider::new()));
    }
    let frontend = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &StyleDirectiveRegistry::built_ins(),
        &mut surface,
        &mut strings,
    )
    .expect("the canonical service should return its complete retained-fact outcome");
    fs::write(&source_path, "no semantic source remains\n")
        .expect("the original source should be replaced");
    (frontend, strings)
}

#[test]
fn module_and_single_file_results_reconverge_recursive_failure_facts_after_service_exit() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    const SOURCE: &str = "helper |value I32, ready Bool| -> I32:\n\
        if ready:\n        return helper(value, ready)\n    ;\n    return value * 2\n;\n\
        last |value I32, ready Bool| -> I32:\n    return helper(value, ready)\n;\n\
        inner |value I32, ready Bool| -> I32:\n    return last(value, ready)\n;\n\
        middle |value I32, ready Bool| -> I32:\n    return inner(value, ready)\n;\n\
        outer |value I32, ready Bool| -> I32:\n    return middle(value, ready)\n;\n\
        forward type T |marker T, value I32, ready Bool| -> I32:\n\
            return outer(value, ready)\n;\n\
        Failure = | message String |\n\
        message type T |value T| -> T:\n    return value\n;\n\
        check |value String|:\n    assert(true, message(value))\n;\n\
        operand type T |value T| -> T:\n    return value\n;\n\
        load |value I32| -> I32, Failure!:\n    return value\n;\n\
        recover |value I32| -> I32:\n\
            return load(operand(value)) catch |failure| then 0\n;\n\
        check(\"safe\")\n\
        recovered = recover(1)\n\
        result = forward(1, recovered, false) catch then 0\n[: result=[result]]\n";

    for single_file in [false, true] {
        let (frontend, strings) = compile_owned_service_fixture(SOURCE, single_file, &[]);
        assert!(!frontend.has_diagnosed_or_blocked());

        let modules = frontend
            .project
            .successful_artefacts_in_module_id_order()
            .map(|artefact| &artefact.module)
            .collect::<Vec<_>>();
        assert_eq!(modules.len(), 1);
        let base = &modules[0].executable.hir;
        let helper = base
            .function_ids_by_private_origin
            .iter()
            .find(|(identity, _)| identity.defining_name() == "helper")
            .map(|(_, function)| *function)
            .expect("the base module retains its recursive helper identity");
        let facts = &base.function_failure_facts[&helper];
        assert_eq!(facts.contributors.len(), 2);
        assert!(matches!(
            facts.contributors[0].source,
            HirBuiltinFailureSource::Call(CallTarget::Local(function)) if function == helper
        ));
        assert!(matches!(
            facts.contributors[1].source,
            HirBuiltinFailureSource::NumericOperation
        ));
        assert_eq!(facts.contributors[1].codes, [BuiltinErrorCode::IntOverflow]);
        let recursive_span = facts.contributors[0]
            .span
            .expect("the retained recursive call has an authored span");
        let origin_span = facts.contributors[1]
            .span
            .expect("the retained arithmetic origin has an authored span");
        let recursive_range = recursive_span
            .resolve_with(ExtendedSpanBuilder::new().resolver_for(recursive_span.source()));
        let origin_range =
            origin_span.resolve_with(ExtendedSpanBuilder::new().resolver_for(origin_span.source()));
        assert!(recursive_range.start() < origin_range.start());
        assert_eq!(
            &SOURCE[origin_range.start() as usize..origin_range.end() as usize],
            "*"
        );

        // Fresh false failure bits force the owned semantic facts to establish the recursive
        // fixed point again. Neither preparation, AST nor TIR is available after the service.
        let mut base_report = report(
            base.functions
                .iter()
                .map(|function| (function.id, summary(FunctionReturnAliasSummary::Fresh))),
        );
        infer_builtin_failure_summaries(base, &mut base_report)
            .expect("retained base facts should reconverge");
        assert!(base_report.analysis.public_call_summaries[&helper].escapes_builtin_failure);
        let named_base_function = |name: &str| {
            base.function_ids_by_private_origin
                .iter()
                .find(|(identity, _)| identity.defining_name() == name)
                .map(|(_, function)| *function)
                .unwrap_or_else(|| panic!("the owned base module retains {name}"))
        };
        let outer = named_base_function("outer");
        let witness = builtin_failure_witness(
            base,
            &base_report,
            outer,
            &base.function_failure_facts[&outer].contributors[0],
        )
        .expect("the retained private chain should select its concrete recursive origin");
        assert_eq!(
            witness.call_spans,
            ["outer", "middle", "inner"].map(|name| base.function_failure_facts
                [&named_base_function(name)]
                .contributors[0]
                .span)
        );
        assert_eq!(witness.origin_span, Some(origin_span));
        assert_eq!(witness.codes, [BuiltinErrorCode::IntOverflow]);
        assert_eq!(witness.elided_call_hops, 1);
        assert!(!witness.call_spans.contains(&Some(recursive_span)));

        let check = named_base_function("check");
        assert!(!base_report.analysis.public_call_summaries[&check].escapes_builtin_failure);
        assert!(
            builtin_failure_diagnostic(base, &base_report, check)
                .expect("the concrete inactive message should be validated")
                .is_none()
        );
        assert!(
            !modules[0]
                .link_facts
                .functions
                .direct_call_targets()
                .into_iter()
                .any(|(_, target)| {
                    matches!(target, CallTarget::Generated(identity)
                if identity.declaration().defining_name() == "message")
                })
        );
        let recover = named_base_function("recover");
        assert!(!base_report.analysis.public_call_summaries[&recover].escapes_builtin_failure);
        assert!(
            builtin_failure_diagnostic(base, &base_report, recover)
                .expect("the infallible concrete operand should remain lawful with a custom catch")
                .is_none()
        );
        let load = named_base_function("load");
        let custom_type = base
            .functions
            .iter()
            .find(|function| function.id == load)
            .expect("the typed producer survives the service")
            .error_return
            .expect("load retains its custom failure channel");
        let diagnostic = CompilerDiagnostic::type_mismatch(
            custom_type,
            custom_type,
            TypeMismatchContext::Assignment,
            None,
        );
        let prepared = PreparedDiagnosticBatch::capture_from_bag(
            DiagnosticBag::from(diagnostic),
            strings.clone(),
            Some(&modules[0].executable.type_environment),
            true,
            Some(std::sync::Arc::clone(&modules[0].executable.path_table)),
        );
        let mut accumulator = BoundaryAccumulator::new();
        accumulator.append_prepared(prepared, None);
        let failure = accumulator.finish(InstallMode::Frozen);
        let (diagnostic_report, view) =
            failure.reports.iter().next().expect("captured custom type");
        let type_mismatch = view
            .family_view::<TypeMismatchView>()
            .expect("the diagnostic must retain type-mismatch facts");
        let type_context = DiagnosticRenderContext::for_report(diagnostic_report);
        assert_eq!(
            view.types().display_for_render(
                type_mismatch.expected,
                type_context.string_table,
                type_context.path_table(),
            ),
            "Failure"
        );
        let (typed_span, failure_block, error_local) = base
            .blocks
            .iter()
            .find_map(|block| match &block.terminator {
                HirTerminator::Invoke {
                    span,
                    operation:
                        HirFallibleOperation::Call {
                            target: CallTarget::Local(function),
                            ..
                        },
                    failure,
                    error_result,
                    ..
                } if *function == load => Some((
                    span.expect("the typed producer retains its authored site"),
                    *failure,
                    *error_result,
                )),
                _ => None,
            })
            .expect("the lawful custom catch retains load's routed typed error edge");
        let typed_range =
            typed_span.resolve_with(ExtendedSpanBuilder::new().resolver_for(typed_span.source()));
        assert_eq!(
            typed_range.start() as usize,
            SOURCE
                .find("load(operand(value))")
                .expect("the fixture contains one typed producer")
        );
        let error = base
            .blocks
            .iter()
            .flat_map(|block| &block.locals)
            .find(|local| local.id == error_local)
            .expect("the typed failure edge retains its error local");
        assert_eq!(error.ty, custom_type);
        let private_span = base
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .find_map(|statement| match &statement.kind {
                HirStatementKind::Call {
                    target: CallTarget::Generated(identity),
                    ..
                } if identity.declaration().defining_name() == "operand" => statement.span,
                _ => None,
            })
            .expect("the infallible protected operand remains a plain native call");
        let private_range = private_span
            .resolve_with(ExtendedSpanBuilder::new().resolver_for(private_span.source()));
        assert_eq!(
            private_range.start() as usize,
            SOURCE
                .find("operand(value)")
                .expect("the fixture contains one protected private call")
        );
        assert_ne!(private_span, typed_span);
        let fallback_start = SOURCE
            .find("catch |failure| then 0")
            .expect("the fixture contains one authored custom handler")
            + "catch |failure| then ".len();
        let mut handler_block = failure_block;
        let mut visited = FxHashSet::default();
        let handler_span = loop {
            assert!(
                visited.insert(handler_block),
                "the fallback route must not loop"
            );
            let block = base
                .blocks
                .iter()
                .find(|block| block.id == handler_block)
                .expect("the typed error route retains each target block");
            let fallback = block.statements.iter().find_map(|statement| {
                let HirStatementKind::Write { value, .. } = statement.kind else {
                    return None;
                };
                let span = base.expressions.expression(value).span?;
                let range =
                    span.resolve_with(ExtendedSpanBuilder::new().resolver_for(span.source()));
                (range.start() as usize == fallback_start).then_some(span)
            });
            if let Some(span) = fallback {
                break span;
            }
            let HirTerminator::Jump { target, .. } = block.terminator else {
                panic!("the custom failure route must reach its authored fallback");
            };
            handler_block = target;
        };
        assert_ne!(handler_span, typed_span);
        assert_ne!(handler_span, private_span);

        let sidecars = frontend.project.generated.sidecars().collect::<Vec<_>>();
        assert_eq!(sidecars.len(), 2);
        assert!(
            !sidecars
                .iter()
                .any(|sidecar| { sidecar.identity.declaration().defining_name() == "message" })
        );
        let operand = sidecars
            .iter()
            .find(|sidecar| sidecar.identity.declaration().defining_name() == "operand")
            .expect("the protected generic operand has completed concrete work");
        let operand_root =
            operand.module.executable.hir.function_ids_by_generated[&operand.identity];
        assert!(
            !operand
                .module
                .executable
                .borrow_analysis
                .analysis
                .public_call_summaries[&operand_root]
                .escapes_builtin_failure
        );
        assert!(
            operand
                .module
                .executable
                .hir
                .functions
                .iter()
                .find(|function| function.id == operand_root)
                .expect("the concrete operand retains its native contract")
                .error_return
                .is_none()
        );
        let sidecar = sidecars
            .iter()
            .find(|sidecar| sidecar.identity.declaration().defining_name() == "forward")
            .expect("the private recursive forwarding request completed");
        assert_eq!(
            sidecar.identity.type_arguments(),
            [CanonicalTypeIdentity::Builtin(
                CanonicalBuiltinType::FixedScalar(FixedScalar::I32,)
            )]
        );
        let generated = &sidecar.module.executable.hir;
        let root = generated.function_ids_by_generated[&sidecar.identity];
        let generated_facts = &generated.function_failure_facts[&root];
        assert_eq!(generated_facts.contributors.len(), 1);
        assert!(matches!(
            &generated_facts.contributors[0].source,
            HirBuiltinFailureSource::Call(CallTarget::ModulePrivate(identity))
                if identity.defining_name() == "outer"
        ));
        let mut generated_report = report(
            generated
                .functions
                .iter()
                .map(|function| (function.id, summary(FunctionReturnAliasSummary::Fresh))),
        );
        infer_builtin_failure_summaries(generated, &mut generated_report)
            .expect("generated facts should consume the retained base helper summary");
        assert!(generated_report.analysis.public_call_summaries[&root].escapes_builtin_failure);
        assert!(
            sidecar
                .module
                .executable
                .borrow_analysis
                .analysis
                .public_call_summaries[&root]
                .escapes_builtin_failure
        );
        let error_type = generated
            .functions
            .iter()
            .find(|function| function.id == root)
            .expect("the generated root remains executable")
            .error_return
            .expect("the generated private lane returns builtin failure");
        assert_eq!(
            sidecar
                .module
                .executable
                .type_environment
                .canonical_identity_for_type_id(error_type),
            Some(&CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error))
        );
    }
}

#[test]
fn completed_service_diagnoses_retained_generic_assertion_and_custom_catch_sites() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let assertion_source = "message type T |marker T, value I32| -> String:\n\
        number = value * 2\n    return \"unused\"\n;\n\
        check |value String|:\n    assert(true, message(value, 1))\n;\ncheck(\"safe\")\n";
    let catch_source = "Failure = | message String |\n\
        operand type T |marker T, value I32| -> I32:\n    return value * 2\n;\n\
        load |value I32| -> I32, Failure!:\n    return value\n;\n\
        recover |value I32| -> I32:\n\
            return load(operand(value, value)) catch |failure| then 0\n;\n\
        recovered = recover(1)\n";

    for single_file in [false, true] {
        for (source, assertion) in [(assertion_source, true), (catch_source, false)] {
            let (frontend, mut strings) = compile_owned_service_fixture(source, single_file, &[]);
            assert!(frontend.has_diagnosed_or_blocked());
            let failure = frontend.into_boundary_failure(&mut strings, None, None);
            assert_eq!(failure.error_count(), 1);
            let (report, view) = failure
                .reports
                .iter_severity(
                    crate::compiler_frontend::compiler_messages::DiagnosticSeverity::Error,
                )
                .next()
                .expect("concrete generated failure should retain one diagnostic");
            let expected_reason_key = if assertion {
                "invalid_fallible_handling.assertion_message_cannot_escape"
            } else {
                "invalid_fallible_handling.custom_error_mixed_with_implicit_failure"
            };
            assert_eq!(view.reason_key(), Some(expected_reason_key));
            let identity = report.context();
            let context = DiagnosticRenderContext::for_report(report);
            let primary_site = view.primary_site();
            let span_start = |site: crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef<'_>| {
                match site {
                    crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef::Local(span) => {
                        span.byte_range(identity).start() as usize
                    }
                    crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef::Foreign {
                        span,
                        context,
                    } => span
                        .byte_range(context.get().expect("foreign site identity should be installed"))
                        .start() as usize,
                    crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef::Absent => {
                        panic!("the diagnosed semantic site retains an exact source span")
                    }
                }
            };
            let source_id = match primary_site {
                crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef::Local(span)
                | crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef::Foreign {
                    span,
                    ..
                } => span.source(),
                crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef::Absent => {
                    panic!("the source diagnosis retains its primary source")
                }
            };
            let snapshot = identity
                .line_index(source_id)
                .expect("the completed diagnosis retains its source line index");
            assert_eq!(
                (0..snapshot.line_count())
                    .map(|line| snapshot
                        .line_text(line)
                        .expect("each retained source line should resolve"))
                    .collect::<Vec<_>>(),
                source.lines().collect::<Vec<_>>()
            );
            if assertion {
                assert_eq!(
                    span_start(primary_site),
                    source
                        .find("message(value, 1)")
                        .expect("the fixture contains its inert message call")
                );
                assert_eq!(
                    context
                        .site_position(primary_site)
                        .expect("the retained inert message span resolves after source deletion")
                        .line,
                    "    assert(true, message(value, 1))"
                );
            } else {
                assert_eq!(
                    span_start(primary_site),
                    source
                        .find("catch |failure| then 0")
                        .expect("the fixture contains its custom handler")
                );
                let labels = view.durable_labels().collect::<Vec<_>>();
                assert_eq!(labels.len(), 2);
                assert!(matches!(
                    labels[0].message,
                    crate::compiler_frontend::compiler_messages::diagnostics::labels::DurableLabelMessage::TypedFailureProducer { .. }
                ));
                assert!(matches!(
                    labels[1].message,
                    crate::compiler_frontend::compiler_messages::diagnostics::labels::DurableLabelMessage::ImplicitFailureProducer
                ));
                assert_eq!(
                    span_start(labels[0].site),
                    source
                        .find("load(operand(value, value))")
                        .expect("the fixture contains its typed producer")
                );
                assert_eq!(
                    span_start(labels[1].site),
                    source
                        .find("operand(value, value)")
                        .expect("the fixture contains its generated private operand")
                );
                let label_text = crate::compiler_frontend::compiler_messages::render::terminal::format_label_messages_view(
                    view,
                    context,
                )
                .join("\n");
                assert!(
                    label_text.contains("Failure"),
                    "the typed producer label should name the custom error type: {label_text}"
                );
            }
        }
    }
}

#[test]
fn active_and_inert_generic_message_share_one_completed_service_identity() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    const SOURCE: &str = "message type T |value T| -> T:\n    return value\n;\n\
        check |value String|:\n    assert(true, message(value))\n;\n\
        check(\"safe\")\nactive_message = message(\"live\")\n[: result=[active_message]]\n";
    for single_file in [false, true] {
        let (frontend, _) = compile_owned_service_fixture(SOURCE, single_file, &[]);
        assert!(!frontend.has_diagnosed_or_blocked());
        let artefacts = frontend
            .project
            .successful_artefacts_in_module_id_order()
            .collect::<Vec<_>>();
        assert_eq!(artefacts.len(), 1);
        let base = &artefacts[0].module.executable.hir;
        let check = base
            .function_ids_by_private_origin
            .iter()
            .find(|(identity, _)| identity.defining_name() == "check")
            .map(|(_, function)| *function)
            .expect("the known-true assertion retains its checked function");
        let start = base
            .start_function
            .expect("the active use belongs to the normal root");
        let check_entry = base
            .functions
            .iter()
            .find(|function| function.id == check)
            .expect("the checked function retains its executable root")
            .entry;
        let check_block = base
            .blocks
            .iter()
            .find(|block| block.id == check_entry)
            .expect("the checked function retains its entry block");
        assert!(check_block.statements.is_empty());
        assert!(
            matches!(check_block.terminator, HirTerminator::Return(values)
            if base.expressions.values(values).is_empty())
        );
        let sidecars = frontend.project.generated.sidecars().collect::<Vec<_>>();
        assert_eq!(sidecars.len(), 1);
        let message = sidecars[0];
        assert_eq!(message.identity.declaration().defining_name(), "message");
        assert_eq!(
            message.identity.type_arguments(),
            [CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String)]
        );
        let root = message.module.executable.hir.function_ids_by_generated[&message.identity];
        assert!(
            !message
                .module
                .executable
                .borrow_analysis
                .analysis
                .public_call_summaries[&root]
                .escapes_builtin_failure
        );
        let callers = artefacts[0]
            .module
            .link_facts
            .functions
            .direct_call_targets()
            .into_iter()
            .filter_map(|(function, target)| {
                matches!(&target, CallTarget::Generated(identity)
                    if identity.as_ref() == &message.identity)
                .then_some(function)
            })
            .collect::<Vec<_>>();
        assert_eq!(callers, [start]);
        assert!(!callers.contains(&check));
        let active_spans = base
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter_map(|statement| match &statement.kind {
                HirStatementKind::Call {
                    target: CallTarget::Generated(identity),
                    ..
                } if identity.as_ref() == &message.identity => statement.span,
                _ => None,
            })
            .collect::<Vec<_>>();
        let [span] = active_spans.as_slice() else {
            panic!("the shared identity has exactly one executable message producer");
        };
        assert_eq!(
            span.resolve_with(ExtendedSpanBuilder::new().resolver_for(span.source()))
                .start() as usize,
            SOURCE
                .find("message(\"live\")")
                .expect("the fixture has one active message call")
        );
        assert_ne!(
            span.resolve_with(ExtendedSpanBuilder::new().resolver_for(span.source()))
                .start() as usize,
            SOURCE
                .find("message(value)")
                .expect("the fixture has one inert message occurrence")
        );
    }
}

#[test]
fn final_generated_failure_lanes_keep_external_candidates_without_validation_only_sidecars() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // Materialisation records the later private helper's protected call without a CFG failure
    // edge. Convergence must add the handler-only provider once that call becomes an Invoke.
    const SOURCE: &str = "@live.js touch, read, ready\n@dormant.js dormant\n\
        @recovery.js recover\n@active_message.js assertion_message\n\
        double |value I32| -> I32:\n    return value * 2\n;\n\
        execute type T |marker T, value I32| -> I32:\n\
            doubled = double(value)\n    touch()\n    io.line(\"live\")\n\
            observed = read() catch then 0\n\
            recovered = late(value) catch then recover()\n\
            return doubled + observed + recovered\n;\n\
        late |value I32| -> I32:\n    return value + 2147483647\n;\n\
        message type T |marker T| -> String:\n    return dormant()\n;\n\
        active_message type T |marker T| -> String:\n    return assertion_message()\n;\n\
        assert(ready(), active_message(\"active assertion\"))\n\
        assert(true, message(\"checked\"))\n\
        result = execute(\"active\", 2) catch then 0\n[:[result]]\n";
    const LIVE: &str = "/** @moth.sig touch || -> */\n\
        export function touch() {}\n\
        /** @moth.sig ready || -> Bool */\n\
        export function ready() { return true; }\n\
        /** @moth.sig read || -> I32, Error! */\n\
        export function read() { return { ok: true, value: 3 }; }\n";
    const DORMANT: &str = "/** @moth.sig dormant || -> String */\n\
        export function dormant() { return \"unused\"; }\n";
    const RECOVERY: &str = "/** @moth.sig recover || -> I32 */\n\
        export function recover() { return 0; }\n";
    const ACTIVE_MESSAGE: &str = "/** @moth.sig assertion_message || -> String */\n\
        export function assertion_message() { return \"active assertion provider\"; }\n";

    for single_file in [false, true] {
        let (frontend, _) = compile_owned_service_fixture(
            SOURCE,
            single_file,
            &[
                ("live.js", LIVE),
                ("dormant.js", DORMANT),
                ("recovery.js", RECOVERY),
                ("active_message.js", ACTIVE_MESSAGE),
            ],
        );
        assert!(!frontend.has_diagnosed_or_blocked());
        let sidecars = frontend.project.generated.sidecars().collect::<Vec<_>>();
        assert_eq!(
            sidecars.len(),
            2,
            "only the executed and active-message generic instances should be published"
        );
        let sidecar = sidecars
            .iter()
            .find(|sidecar| sidecar.identity.declaration().defining_name() == "execute")
            .expect("the executed generic instance should be published");
        let active_message = sidecars
            .iter()
            .find(|sidecar| sidecar.identity.declaration().defining_name() == "active_message")
            .expect("the active assertion-message generic instance should be published");
        let hir = &sidecar.module.executable.hir;
        let root = hir.function_ids_by_generated[&sidecar.identity];
        let root_function = hir
            .functions
            .iter()
            .find(|function| function.id == root)
            .expect("the selected generated root should survive convergence");
        assert!(
            root_function.error_return.is_some(),
            "the private helper's inferred failure must rewrite the generated root's call lane"
        );
        assert!(hir.blocks.iter().any(|block| matches!(
            &block.terminator,
            HirTerminator::Invoke {
                operation: HirFallibleOperation::Call {
                    target: CallTarget::ModulePrivate(_),
                    ..
                },
                ..
            }
        )));
        assert!(
            hir.blocks.iter().any(|block| matches!(
                &block.terminator,
                HirTerminator::Invoke {
                    operation: HirFallibleOperation::Call {
                        target: CallTarget::ModulePrivate(identity),
                        ..
                    },
                    ..
                } if identity.defining_name() == "late"
            )),
            "the initially pending protected call must acquire its inferred failure edge"
        );
        let statement_external = hir
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter_map(|statement| match &statement.kind {
                HirStatementKind::Call {
                    target: CallTarget::External(function),
                    ..
                } => Some(*function),
                _ => None,
            })
            .collect::<FxHashSet<_>>();
        let invoked_external = hir
            .blocks
            .iter()
            .filter_map(|block| match &block.terminator {
                HirTerminator::Invoke {
                    operation:
                        HirFallibleOperation::Call {
                            target: CallTarget::External(function),
                            ..
                        },
                    ..
                } => Some(*function),
                _ => None,
            })
            .collect::<FxHashSet<_>>();
        assert_eq!(
            statement_external.len(),
            3,
            "touch, Core IO and the newly reachable recovery provider remain calls"
        );
        assert!(statement_external.contains(&ExternalFunctionId::IoLine));
        assert_eq!(
            invoked_external.len(),
            1,
            "read retains its typed error edge"
        );

        let reachability = collect_reachability_from_function_link_facts(
            &sidecar.module.link_facts.functions,
            &[root],
        )
        .expect("final generated link facts should match its rewritten root");
        let registry = &sidecar.module.link_facts.external_package_registry;
        let candidates = &sidecar.module.link_facts.external_import_candidates;
        assert_eq!(
            reachability.reachable_external_functions,
            statement_external
                .union(&invoked_external)
                .copied()
                .collect()
        );
        let assets = candidates
            .iter()
            .filter_map(|candidate| candidate.runtime_asset.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(
            assets.len(),
            2,
            "only the live and recovery providers should supply generated runtime assets"
        );
        assert!(
            assets
                .iter()
                .any(|asset| asset.canonical_source_path.ends_with("live.js"))
        );
        assert!(
            assets
                .iter()
                .any(|asset| asset.canonical_source_path.ends_with("recovery.js")),
            "final candidate refresh must retain the provider first reached through the new failure edge"
        );
        assert!(assets.iter().all(|asset| asset.asset_kind == "js"));
        for call in &reachability.reachable_external_calls {
            assert!(registry.get_function_by_id(call.function_id).is_some());
            let package = registry
                .resolve_function_package_id(call.function_id)
                .expect("each final external call retains its registry owner");
            assert!(registry.get_package_by_id(package).is_some());
            if call.function_id == ExternalFunctionId::IoLine {
                assert!(
                    candidates
                        .iter()
                        .filter(|candidate| candidate.package_id == package)
                        .all(|candidate| candidate.runtime_asset.is_none()),
                    "a registered Core helper is valid without a provider asset"
                );
            } else {
                assert!(
                    candidates
                        .iter()
                        .any(|candidate| candidate.package_id == package),
                    "both external Call and Invoke must retain their provider candidate"
                );
            }
        }
        let message_hir = &active_message.module.executable.hir;
        let message_root = message_hir.function_ids_by_generated[&active_message.identity];
        let message_reachability = collect_reachability_from_function_link_facts(
            &active_message.module.link_facts.functions,
            &[message_root],
        )
        .expect("final active-message link facts should retain its provider call");
        assert_eq!(message_reachability.reachable_external_calls.len(), 1);
        assert_eq!(message_reachability.reachable_external_functions.len(), 1);
        let message_registry = &active_message.module.link_facts.external_package_registry;
        let message_call = &message_reachability.reachable_external_calls[0];
        assert!(
            message_registry
                .get_function_by_id(message_call.function_id)
                .is_some()
        );
        let message_package = message_registry
            .resolve_function_package_id(message_call.function_id)
            .expect("the active-message call retains its issuing registry");
        assert!(
            message_registry
                .get_package_by_id(message_package)
                .is_some()
        );
        let message_candidates = &active_message.module.link_facts.external_import_candidates;
        assert_eq!(message_candidates.len(), 1);
        assert_eq!(message_candidates[0].package_id, message_package);
        let message_asset = message_candidates[0]
            .runtime_asset
            .as_ref()
            .expect("the active-message provider must supply its sole runtime asset");
        assert!(
            message_asset
                .canonical_source_path
                .ends_with("active_message.js")
        );
        assert_eq!(message_asset.asset_kind, "js");
        assert_eq!(
            frontend
                .successful_module_views()
                .flat_map(|module| module.link_facts.functions.direct_call_targets())
                .filter_map(|(_, target)| match target {
                    CallTarget::Generated(identity) => {
                        Some(identity.declaration().defining_name().to_owned())
                    }
                    _ => None,
                })
                .collect::<FxHashSet<_>>(),
            FxHashSet::from_iter(["execute".to_owned(), "active_message".to_owned()]),
            "the published source facts retain both executable demands and no inert message"
        );
        for module in frontend.successful_module_views() {
            assert!(
                module
                    .link_facts
                    .functions
                    .direct_call_targets()
                    .into_iter()
                    .all(|(_, target)| !matches!(
                        target,
                        CallTarget::Generated(identity)
                            if identity.declaration().defining_name() == "message"
                    )),
                "validation-only generic demand must not remain live"
            );
        }
    }
}
