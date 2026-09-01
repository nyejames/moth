use super::super::service::{
    BoracleDump, BoracleExperiment, BoracleReferencePromotionStatus, BoracleReferenceRuleSet,
    BoracleRuleSelection, format_experiment_names,
};

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
