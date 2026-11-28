use super::super::service::{
    BoracleDump, BoracleExperiment, BoracleReferencePromotionStatus, BoracleReferenceRuleSet,
    BoracleRuleSelection, BoracleServiceOptions, format_experiment_names, run_hir_module,
};
use super::{SharedSubsetKind, hir_shared_subset};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::module_compilation::BoracleModuleInput;
use std::path::PathBuf;
use std::sync::Arc;

#[test]
fn dump_and_experiment_names_are_stable() {
    assert_eq!("last-use".parse::<BoracleDump>(), Ok(BoracleDump::LastUse));
    assert_eq!(
        "differential".parse::<BoracleDump>(),
        Ok(BoracleDump::Differential)
    );
    assert_eq!(
        "relations".parse::<BoracleDump>(),
        Ok(BoracleDump::Relations)
    );
    assert_eq!(
        "precision".parse::<BoracleDump>(),
        Ok(BoracleDump::Precision)
    );
    assert_eq!(
        "dead-exclusive-loan".parse::<BoracleExperiment>(),
        Ok(BoracleExperiment::DeadExclusiveLoan)
    );
}

#[test]
fn default_rule_selection_is_reference_without_experiments() {
    let selection = BoracleRuleSelection::default();
    assert_eq!(selection.reference_rule_set, BoracleReferenceRuleSet::V1);
    assert!(selection.experiments.is_empty());
    assert!(selection.validate().is_ok());
}

#[test]
fn experiment_names_render_none_for_reference_mode() {
    assert_eq!(BoracleRuleSelection::default().experiment_names(), "none");
    assert_eq!(format_experiment_names([]), "none");
}

#[test]
fn experiment_names_dedupe_a_union_across_selections() {
    let repeated = [
        BoracleExperiment::DeadExclusiveLoan,
        BoracleExperiment::DeadExclusiveLoan,
    ];
    assert_eq!(format_experiment_names(repeated), "dead-exclusive-loan");
}

#[test]
fn dead_exclusive_loan_metadata_is_not_reference_promoted() {
    let metadata = BoracleExperiment::DeadExclusiveLoan.metadata();
    assert_eq!(metadata.name, "dead-exclusive-loan");
    assert!(metadata.may_change_legality);
    assert!(metadata.prerequisites.is_empty());
    assert_eq!(
        metadata.reference_promotion,
        BoracleReferencePromotionStatus::NotPromoted
    );
}

#[test]
fn module_report_header_renders_entry_path_as_portable_text() {
    let (mut module, function) = hir_shared_subset(SharedSubsetKind::Copy);
    module.functions.push(function);
    let input = BoracleModuleInput {
        hir: module,
        type_environment: crate::compiler_frontend::datatypes::environment::TypeEnvironment::new(),
        external_package_registry: Arc::new(ExternalPackageRegistry::new()),
        source_path: PathBuf::from("/project/back\\slash.moth"),
    };

    let report = run_hir_module(
        &input,
        BoracleServiceOptions {
            dump: BoracleDump::Problem,
            rule_selection: BoracleRuleSelection::default(),
        },
    )
    .expect("fixture module should produce a report");

    let header = report
        .lines()
        .find(|line| line.starts_with("entry = "))
        .expect("report should retain its entry header");
    assert_eq!(header, "entry = /project/back/slash.moth");
}
