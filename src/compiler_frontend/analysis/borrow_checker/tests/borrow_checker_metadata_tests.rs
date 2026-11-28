//! Borrow metadata traversal regression tests.
//!
//! WHAT: checks that assertion-message expressions contribute their local loads to terminator
//!       metadata.
//! WHY: the message is a runtime expression whose loads must contribute to terminal liveness.

use crate::compiler_frontend::analysis::borrow_checker::{
    BorrowCheckReport, check_borrows, refresh_call_allocations, seed_call_summaries,
    stabilize_call_metadata, summarize_call_allocations_with_projection,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expression_store::HirExpressionStore;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId, RegionId};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::private_failure_lane::install_private_failure_lanes_with_receipt;
use crate::compiler_frontend::hir::reachability::collect_module_function_link_facts;
use crate::compiler_frontend::hir::statements::{HirStatementKind, HirWriteTarget};
use crate::compiler_frontend::hir::terminators::{HirAssertionMessageEvaluation, HirTerminator};
use crate::compiler_frontend::module_compilation::generated::infer_builtin_failure_summaries;
use crate::compiler_frontend::public_call_summary::FunctionReturnAliasSummary;
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, GeneratedFunctionIdentity, ModulePrivateExecutableCategory,
    ModulePrivateExecutableIdentity, ModuleRootRole, StableModuleOriginIdentity,
    StablePackageIdentity,
};
use crate::compiler_frontend::tests::hir_fixture_support::PreparedHirFixture;
use rustc_hash::FxHashSet;

#[test]
fn assertion_failure_message_contributes_loaded_local_metadata() {
    let mut expressions = HirExpressionStore::default();
    let message = crate::compiler_frontend::tests::hir_fixture_support::expression(
        HirExpressionKind::Load(HirPlace::local(LocalId(7))),
        builtin_type_ids::STRING,
        RegionId(0),
        ValueKind::RValue,
        &mut expressions,
    );
    let terminator = HirTerminator::AssertFailure {
        message,
        message_evaluation: HirAssertionMessageEvaluation::Runtime,
    };
    let mut loaded_locals = FxHashSet::default();

    super::collect_terminator_loaded_locals(&expressions, &terminator, &mut |local| {
        loaded_locals.insert(local);
    });

    assert_eq!(loaded_locals, FxHashSet::from_iter([LocalId(7)]));
}

fn generated(name: &str) -> GeneratedFunctionIdentity {
    GeneratedFunctionIdentity::new(
        GeneratedDeclarationIdentity::ModulePrivate(ModulePrivateExecutableIdentity::new(
            StableModuleOriginIdentity::from_portable_path(
                StablePackageIdentity::project_local("borrow-metadata-tests"),
                "main".to_owned(),
                ModuleRootRole::Normal,
            ),
            "@entry.moth".to_owned(),
            ModulePrivateExecutableCategory::GenericFunction,
            name.to_owned(),
            None,
        )),
        Box::new([]),
        Box::new([]),
    )
}

fn function(fixture: &PreparedHirFixture, name: &str) -> FunctionId {
    fixture
        .hir
        .functions
        .iter()
        .find(|function| {
            fixture.hir.side_table.resolve_function_name(
                function.id,
                &fixture.paths,
                &fixture.strings,
            ) == Some(name)
        })
        .expect("fixture function")
        .id
}

fn seed(fixture: &PreparedHirFixture) -> BorrowCheckReport {
    seed_call_summaries(&fixture.hir, &fixture.paths, &fixture.strings)
        .expect("fixture declarations should seed")
}

fn stabilize(fixture: &PreparedHirFixture, report: &mut BorrowCheckReport) {
    stabilize_call_metadata(
        &fixture.hir,
        &fixture.hir.type_environment,
        &fixture.packages,
        &fixture.paths,
        &fixture.strings,
        &fixture.type_projection(),
        report,
    )
    .expect("exact metadata work should converge");
}

fn cold(fixture: &PreparedHirFixture) -> BorrowCheckReport {
    summarize_call_allocations_with_projection(
        &fixture.hir,
        &fixture.hir.type_environment,
        &fixture.packages,
        &fixture.paths,
        &fixture.strings,
        &fixture.type_projection(),
    )
    .expect("cold comparator should converge")
}

#[test]
fn exact_metadata_rebuilds_after_real_remap_and_call_edit() {
    let mut fixture = PreparedHirFixture::new(
        "leaf || -> I32:\n    return 7\n;\n\
         other_leaf || -> I32:\n    return 8\n;\n\
         caller || -> I32:\n    return leaf()\n;\n",
    );
    let leaf = function(&fixture, "leaf");
    let other = function(&fixture, "other_leaf");
    let caller = function(&fixture, "caller");
    let mut unused = fixture.hir.blocks[0].clone();
    unused.id = BlockId(fixture.hir.blocks.len() as u32);
    fixture.hir.blocks.insert(0, unused);
    fixture.hir.compact_block_ids();
    let mut report = seed(&fixture);
    assert!(report.metadata.pending().contains(&caller));
    stabilize(&fixture, &mut report);
    assert_eq!(report.metadata.visits[&caller], 1);
    let leaf_summary = report.analysis.public_call_summaries[&leaf].clone();
    let old_entry = report.metadata.bodies[&caller]
        .schema
        .as_ref()
        .unwrap()
        .entry;
    assert_eq!(report.metadata.callers[&leaf].len(), 1);

    fixture.hir.blocks.remove(0);
    let remap = fixture.hir.compact_block_ids();
    assert_ne!(remap.get(old_entry), Some(old_entry));
    report.metadata.apply_block_remap(&remap);
    assert!(report.metadata.pending().is_empty());
    stabilize(&fixture, &mut report);
    assert!(report.metadata.visits.is_empty());
    let body = &report.metadata.bodies[&caller];
    assert_eq!(
        body.schema.as_ref().unwrap().entry,
        remap.get(old_entry).unwrap()
    );
    assert_eq!(body.indexed_revision, Some(body.revision));
    assert_eq!(report.analysis.public_call_summaries[&leaf], leaf_summary);

    let target = fixture
        .hir
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.statements)
        .find_map(|statement| match &mut statement.kind {
            HirStatementKind::Call { target, .. } if *target == CallTarget::Local(leaf) => {
                Some(target)
            }
            _ => None,
        })
        .expect("caller has a local call");
    *target = CallTarget::Local(other);
    let revision = report.metadata.bodies[&caller].revision;
    report.metadata.enqueue([caller, caller]);
    assert_eq!(report.metadata.pending(), &FxHashSet::from_iter([caller]));
    assert_eq!(report.metadata.bodies[&caller].revision, revision);
    report.invalidate_call_metadata([caller]);
    assert_eq!(report.metadata.bodies[&caller].revision, revision + 1);
    assert!(!report.metadata.indices.contains_key(&caller));
    assert!(!report.metadata.callers.contains_key(&leaf));
    assert_eq!(
        report.analysis.public_call_summaries[&caller].return_alias,
        FunctionReturnAliasSummary::Pending
    );
    stabilize(&fixture, &mut report);
    assert!(report.metadata.pending().is_empty());
    assert_eq!(report.metadata.visits[&caller], 1);
    assert_eq!(report.metadata.callers[&other].len(), 1);
    assert_eq!(
        report.analysis.public_call_summaries,
        cold(&fixture).analysis.public_call_summaries
    );
}

#[test]
fn inferred_failure_and_allocation_survive_validation_of_a_generated_root() {
    let mut fixture =
        PreparedHirFixture::new("failer || -> I32:\n    value ~I32 = 2\n    return value * 2\n;\n");
    let root = function(&fixture, "failer");
    let identity = generated("failer");
    fixture
        .hir
        .function_ids_by_generated
        .insert(identity.clone(), root);
    assert!(
        fixture
            .hir
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .any(|statement| {
                matches!(
                    statement.kind,
                    HirStatementKind::NumericOp {
                        failure_mode:
                            crate::compiler_frontend::hir::numeric::NumericFailureMode::Trap,
                        ..
                    }
                )
            })
    );
    let mut report = seed(&fixture);
    assert!(report.metadata.pending().contains(&root));
    assert_eq!(
        report.analysis.public_call_summaries[&root].return_alias,
        FunctionReturnAliasSummary::Pending
    );

    infer_builtin_failure_summaries(&fixture.hir, &mut report);
    assert!(report.analysis.public_call_summaries[&root].escapes_builtin_failure);
    let receipt = install_private_failure_lanes_with_receipt(
        &mut fixture.hir.hir_module,
        &report,
        &mut fixture.hir.type_environment,
        &fixture.packages,
    )
    .expect("inferred private lane should install");
    assert!(receipt.outcome_changes.contains(&root));
    assert!(receipt.producer_splits.iter().any(|&(id, _)| id == root));
    report.invalidate_call_metadata([root]);
    report.metadata.apply_block_remap(&receipt.block_remap);
    stabilize(&fixture, &mut report);

    let generated_root = fixture.hir.function_ids_by_generated[&identity];
    let settled = report.analysis.public_call_summaries[&generated_root].clone();
    assert!(settled.escapes_builtin_failure);
    let facts = settled
        .return_alias
        .facts()
        .expect("generated root settled");
    assert!(facts.success.can_return());
    assert!(facts.error.can_return());
    report.metadata.enqueue([root]);
    let checked = check_borrows(
        &fixture.hir,
        &fixture.hir.type_environment,
        &fixture.packages,
        &fixture.paths,
        &fixture.strings,
        Some((
            std::mem::take(&mut report.analysis.public_call_summaries),
            &fixture.type_projection(),
        )),
    )
    .expect("validation should accept the live settled inputs");
    report.replace_validation(checked);
    assert_eq!(
        report.analysis.public_call_summaries[&generated_root],
        settled
    );
    assert_eq!(report.metadata.pending(), &FxHashSet::from_iter([root]));
    assert!(report.metadata.indices.contains_key(&root));
    assert!(report.analysis.function_summaries.contains_key(&root));
    assert_eq!(report.stats.functions_analyzed, fixture.hir.functions.len());
}

#[test]
fn dependency_ready_exact_work_extracts_acyclic_bodies_once_and_revisits_a_cycle() {
    let fixture = PreparedHirFixture::new(
        "top || -> I32:\n    return middle()\n;\n\
         middle || -> I32:\n    return leaf()\n;\n\
         leaf || -> I32:\n    return 7\n;\n\
         left |choose Bool| -> I32:\n    if choose:\n        return 1\n    ;\n    return right(choose)\n;\n\
         right |choose Bool| -> I32:\n    return left(choose)\n;\n",
    );
    let mut report = seed(&fixture);
    let top = function(&fixture, "top");
    let leaf = function(&fixture, "leaf");
    assert!(report.metadata.bodies[&top].position < report.metadata.bodies[&leaf].position);
    stabilize(&fixture, &mut report);
    for name in ["top", "middle", "leaf"] {
        assert_eq!(
            report.metadata.visits[&function(&fixture, name)],
            1,
            "{name}"
        );
    }
    for name in ["left", "right"] {
        assert_eq!(
            report.metadata.visits[&function(&fixture, name)],
            2,
            "{name}"
        );
    }
    assert_eq!(
        report.analysis.public_call_summaries,
        cold(&fixture).analysis.public_call_summaries
    );
}

#[test]
fn refreshed_nonlocal_path_reindexes_local_dependents_without_resetting_summaries() {
    let mut fixture = PreparedHirFixture::new(
        "Inner = |item I32, other I32|\n\
         Outer = |inner Inner|\n\
         provider |value Outer| -> I32:\n    return value.inner.item\n;\n\
         relay |value Outer| -> I32:\n    return provider(value)\n;\n\
         caller |value Outer| -> I32:\n    return relay(value)\n;\n",
    );
    let provider = function(&fixture, "provider");
    let relay = function(&fixture, "relay");
    let caller = function(&fixture, "caller");
    let identity = generated("provider");
    let mut report = seed(&fixture);
    let settled_input = cold(&fixture)
        .analysis
        .public_call_summaries
        .remove(&provider)
        .unwrap();
    let target = fixture
        .hir
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.statements)
        .find_map(|statement| match &mut statement.kind {
            HirStatementKind::Call { target, .. } if *target == CallTarget::Local(provider) => {
                Some(target)
            }
            _ => None,
        })
        .expect("relay has a local call");
    *target = CallTarget::Generated(Box::new(identity.clone()));
    fixture.hir.generated_call_summaries.insert(
        identity.clone(),
        report.analysis.public_call_summaries[&provider].clone(),
    );
    stabilize(&fixture, &mut report);
    let prior = report.analysis.public_call_summaries[&relay].clone();
    assert!(prior.return_alias.facts().unwrap().is_transfer_bottom());
    let mut legacy = BorrowCheckReport::default();
    legacy.analysis.public_call_summaries = report.analysis.public_call_summaries.clone();
    let caller_ports_before = report.metadata.indices[&caller].entry_ports().to_vec();

    fixture
        .hir
        .generated_call_summaries
        .insert(identity, settled_input);
    report.metadata.refresh_inputs([relay]);
    assert_eq!(report.analysis.public_call_summaries[&relay], prior);
    stabilize(&fixture, &mut report);
    let links = collect_module_function_link_facts(&fixture.hir).unwrap();
    refresh_call_allocations(
        &fixture.hir,
        &fixture.hir.type_environment,
        &fixture.packages,
        &fixture.paths,
        &fixture.strings,
        (
            &links,
            &FxHashSet::from_iter([relay]),
            &fixture.type_projection(),
        ),
        &mut legacy,
    )
    .expect("legacy re-index and widening should accept the available path");
    assert_ne!(
        report.metadata.indices[&caller].entry_ports(),
        caller_ports_before.as_slice()
    );
    assert_eq!(report.metadata.bodies[&relay].revision, 0);
    assert_eq!(report.metadata.bodies[&caller].revision, 0);
    assert_eq!(report.metadata.visits[&relay], 1);
    assert_eq!(report.metadata.visits[&caller], 1);
    assert_eq!(
        report.analysis.public_call_summaries,
        legacy.analysis.public_call_summaries
    );
    assert_eq!(
        report.analysis.public_call_summaries,
        cold(&fixture).analysis.public_call_summaries
    );
}

#[test]
fn changing_result_only_supports_reindexes_callers_before_extraction() {
    let mut fixture = PreparedHirFixture::new(concat!(
        "leaf || -> I32:\n",
        "    scratch ~I32 = 0\n",
        "    ~scratch = 1\n",
        "    return 7\n;\n",
        "caller || -> I32:\n    return leaf()\n;\n",
    ));
    let leaf = function(&fixture, "leaf");
    let caller = function(&fixture, "caller");
    let mut report = seed(&fixture);
    stabilize(&fixture, &mut report);
    let old_index = report.metadata.indices[&leaf].clone();
    let entry = fixture
        .hir
        .functions
        .iter()
        .find(|f| f.id == leaf)
        .unwrap()
        .entry;
    let block = &mut fixture.hir.blocks[entry.0 as usize];
    assert!(matches!(block.terminator, HirTerminator::Return(_)));
    let before = block.statements.len();
    block.statements.retain(|statement| {
        !matches!(
            statement.kind,
            HirStatementKind::Write {
                target: HirWriteTarget::AssignPlace(_),
                ..
            }
        )
    });
    assert_eq!(block.statements.len() + 1, before);

    // The entry's independent local write used the first site support. Dropping it
    // renumbers the literal result without changing provenance or return reach.
    // Retain the old schema for the fingerprint comparison, but mark its coordinates stale.
    report.metadata.bodies.get_mut(&leaf).unwrap().revision += 1;
    report.metadata.refresh_inputs([leaf]);
    // The old typed row names the previous result support; keep the stale index only so
    // index refresh can compare its routing, not as a same-epoch summary baseline.
    report
        .analysis
        .public_call_summaries
        .get_mut(&leaf)
        .unwrap()
        .reset_graph_payload();
    assert_eq!(report.metadata.pending(), &FxHashSet::from_iter([leaf]));
    let mut checker = super::BorrowChecker::new(
        &fixture.hir,
        &fixture.hir.type_environment,
        &fixture.packages,
        &fixture.paths,
        &fixture.strings,
    )
    .with_type_projection(&fixture.type_projection());
    checker.public_call_summaries = std::mem::take(&mut report.analysis.public_call_summaries);
    report
        .metadata
        .refresh_indices(&checker)
        .expect("rebuild the edited index");
    report.analysis.public_call_summaries = checker.public_call_summaries;
    assert!(report.metadata.indices[&leaf].only_result_supports_changed(&old_index));
    assert!(report.metadata.pending().contains(&caller));
    stabilize(&fixture, &mut report);
    assert_eq!(
        report.analysis.public_call_summaries,
        cold(&fixture).analysis.public_call_summaries
    );
}
