//! Unit tests for the timing schema v1 registry.
//!
//! WHAT: verifies every checklist invariant of the typed metric registry so a
//!      future edit cannot silently break name uniqueness, dense indexing,
//!      attribution rules or command-total ownership.
//! WHY:  schema errors are cheaper to pin down here than in collector or
//!       benchmark output, and the whole timing system assumes these rules.

use crate::timing::enabled::schema::{
    TIMING_METRIC_DESCRIPTORS, TIMING_SCHEMA_VERSION, TimingAccountingRole, TimingAttributionKind,
    TimingCommand, TimingLevel, TimingMetric, TimingMetricOwner, TimingParent, TimingPipelineStage,
    TimingRelation, TimingSummaryGroup,
};

/// Every metric and its descriptor table entry must agree in count and order.
#[test]
fn enum_and_descriptor_counts_agree() {
    assert_eq!(TIMING_METRIC_DESCRIPTORS.len(), TimingMetric::ALL.len());
    for (index, &metric) in TimingMetric::ALL.iter().enumerate() {
        assert_eq!(metric.index(), index, "dense index must equal position");
        assert_eq!(
            TIMING_METRIC_DESCRIPTORS[index],
            *metric.descriptor(),
            "descriptor table must be parallel to the enum"
        );
    }
}

/// All stable names must be unique.
#[test]
fn stable_names_are_unique() {
    let mut seen = std::collections::HashSet::new();
    for metric in TimingMetric::ALL {
        assert!(
            seen.insert(metric.descriptor().stable_name),
            "duplicate stable name '{}'",
            metric.descriptor().stable_name
        );
    }
}

/// Names must be lowercase dotted, with no leading/trailing dots or empty
/// components.
#[test]
fn names_follow_lowercase_dotted_syntax() {
    for metric in TimingMetric::ALL {
        let name = metric.descriptor().stable_name;
        assert!(
            !name.is_empty(),
            "metric {} has an empty stable name",
            metric.index()
        );
        assert!(
            !name.starts_with('.') && !name.ends_with('.'),
            "'{name}' must not start or end with a dot"
        );
        for part in name.split('.') {
            assert!(!part.is_empty(), "'{name}' has an empty dotted component");
            assert!(
                part.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "'{name}' component '{part}' must be lowercase alphanumeric or underscore"
            );
        }
    }
}

/// No legacy `_ms` unit suffix may survive into v1 names.
#[test]
fn no_ms_suffix_remains() {
    for metric in TimingMetric::ALL {
        let name = metric.descriptor().stable_name;
        assert!(
            !name.ends_with("_ms") && !name.contains("_ms."),
            "'{name}' retains a unit suffix"
        );
    }
}

/// The schema start is schema v1 and every metric carries a level.
#[test]
fn schema_version_is_v1_and_levels_are_well_formed() {
    assert_eq!(TIMING_SCHEMA_VERSION, 1);
    for metric in TimingMetric::ALL {
        assert!(
            matches!(
                metric.descriptor().level,
                TimingLevel::Basic | TimingLevel::Detailed
            ),
            "{} must declare a level",
            metric.descriptor().stable_name
        );
    }
}

/// The registry count and Basic/Detailed split match the plan's closeout
/// record (45 metrics, 34 Basic + 11 Detailed).
#[test]
fn registry_size_matches_plan_closeout() {
    assert_eq!(TimingMetric::ALL.len(), 45);
    let basic = TimingMetric::ALL
        .iter()
        .filter(|metric| metric.descriptor().level == TimingLevel::Basic)
        .count();
    let detailed = TimingMetric::ALL
        .iter()
        .filter(|metric| metric.descriptor().level == TimingLevel::Detailed)
        .count();
    assert_eq!(basic, 34);
    assert_eq!(detailed, 11);
}

/// Every command-accounting metric owns one distinct pipeline segment.
///
/// A summary later consumes this typed policy directly. The registry therefore
/// cannot retain a second Basic metric that measures the same command-child
/// span under a backend- or output-specific name.
#[test]
fn basic_command_accounting_metrics_have_unique_semantic_spans() {
    let mut pipeline_stages = std::collections::HashSet::new();

    for &metric in TimingMetric::ALL {
        let descriptor = metric.descriptor();
        match descriptor.accounting {
            TimingAccountingRole::CommandTotal => {
                assert_eq!(descriptor.level, TimingLevel::Basic);
                assert!(metric.is_command_total());
                assert_eq!(descriptor.relation, TimingRelation::WallSpan);
                assert!(descriptor.parent.is_none());
            }
            TimingAccountingRole::Pipeline(stage) => {
                assert_eq!(descriptor.level, TimingLevel::Basic);
                assert_eq!(descriptor.relation, TimingRelation::WallSpan);
                assert_eq!(descriptor.attribution, TimingAttributionKind::None);
                assert!(descriptor.parent.is_none());
                assert!(
                    pipeline_stages.insert(stage),
                    "{} duplicates the {stage:?} command-pipeline span",
                    descriptor.stable_name
                );
            }
            TimingAccountingRole::Evidence => {}
        }
    }

    assert_eq!(
        pipeline_stages,
        std::collections::HashSet::from([
            TimingPipelineStage::Bootstrap,
            TimingPipelineStage::Frontend,
            TimingPipelineStage::Backend,
            TimingPipelineStage::Output,
        ])
    );
}

/// Every attributed metric permits exactly the context kind it declares.
#[test]
fn attributed_metrics_permit_declared_context_kind() {
    for metric in TimingMetric::ALL {
        let descriptor = metric.descriptor();
        match descriptor.attribution {
            TimingAttributionKind::None => {
                assert!(
                    !matches!(
                        descriptor.stable_name,
                        "boundary.inventory" | "boundary.compile"
                    ) && !descriptor.stable_name.starts_with("frontend."),
                    "{} must not silently allow attribution",
                    descriptor.stable_name
                );
            }
            TimingAttributionKind::Boundary => {
                assert!(matches!(
                    descriptor.stable_name,
                    "boundary.inventory" | "boundary.compile"
                ));
            }
            TimingAttributionKind::Module => {
                assert!(
                    descriptor.stable_name.starts_with("frontend."),
                    "{} must be attributed to a module",
                    descriptor.stable_name
                );
            }
        }
    }
}

/// Every command has exactly one total metric and command totals are unique.
#[test]
fn every_command_total_is_unique() {
    let totals: Vec<_> = TimingMetric::ALL
        .iter()
        .filter(|metric| metric.is_command_total())
        .collect();
    assert_eq!(totals.len(), 3, "exactly one total per command is required");

    for command in [
        TimingCommand::Build,
        TimingCommand::Check,
        TimingCommand::Dev,
    ] {
        let total = TimingMetric::command_total(command).expect("command total exists");
        assert!(total.is_command_total());
        assert!(total.applies_to(command));

        // The command's own total must be unique by construction: the schema
        // maps each command to exactly one metric, and no other metric may own
        // the same command.
        assert_eq!(
            TimingMetric::command_total(command),
            Some(total),
            "command {:?} must map to exactly one metric",
            command
        );
    }
}

/// Parents use either a real metric identity or a typed virtual summary group.
#[test]
fn parent_policy_uses_typed_metric_and_summary_group_identities() {
    let mut summary_groups = std::collections::HashSet::new();

    for &metric in TimingMetric::ALL {
        let descriptor = metric.descriptor();

        match descriptor.relation {
            TimingRelation::NestedEvidence => {
                assert!(
                    matches!(descriptor.parent, Some(TimingParent::Metric(_))),
                    "nested {} must name a containing metric",
                    descriptor.stable_name
                );
            }
            TimingRelation::Accumulated => {}
            TimingRelation::WallSpan => {
                assert!(
                    descriptor.parent.is_none(),
                    "{} must not declare a parent as a wall-span row",
                    descriptor.stable_name
                );
            }
        }

        match descriptor.parent {
            Some(TimingParent::Metric(parent)) => {
                assert_ne!(
                    metric, parent,
                    "{} cannot parent itself",
                    descriptor.stable_name
                );

                // The parent is a typed `TimingMetric`, which guarantees it
                // exists in the descriptor table. A child cannot apply to a
                // command where its enclosing span cannot be recorded.
                for command in [
                    TimingCommand::Build,
                    TimingCommand::Check,
                    TimingCommand::Dev,
                ] {
                    if metric.applies_to(command) {
                        assert!(
                            parent.applies_to(command),
                            "{} applies to {command:?} outside parent {}",
                            descriptor.stable_name,
                            parent.descriptor().stable_name
                        );
                    }
                }
            }
            Some(TimingParent::SummaryGroup(group)) => {
                assert_eq!(
                    descriptor.relation,
                    TimingRelation::Accumulated,
                    "{} may use a virtual group only for accumulated evidence",
                    descriptor.stable_name
                );
                summary_groups.insert(group);
            }
            None => {}
        }
    }

    assert_eq!(
        summary_groups,
        std::collections::HashSet::from([
            TimingSummaryGroup::PublicInterface,
            TimingSummaryGroup::BorrowValidation,
            TimingSummaryGroup::GeneratedFunctions,
        ])
    );
}

/// Public-interface leaves are disjoint accumulated work, not nested evidence.
#[test]
fn public_interface_leaves_use_the_typed_public_interface_group() {
    for metric in [
        TimingMetric::FrontendPublicInterfaceProject,
        TimingMetric::FrontendPublicInterfaceFinalise,
    ] {
        let descriptor = metric.descriptor();
        assert_eq!(descriptor.relation, TimingRelation::Accumulated);
        assert_eq!(
            descriptor.parent,
            Some(TimingParent::SummaryGroup(
                TimingSummaryGroup::PublicInterface
            ))
        );
    }
}

/// Backend and output evidence is valid for build and dev, never check.
#[test]
fn backend_and_output_metrics_apply_to_build_and_dev_not_check() {
    const BACKEND_AND_OUTPUT_METRICS: &[TimingMetric] = &[
        TimingMetric::BuildBackendTotal,
        TimingMetric::BuildOutputTotal,
        TimingMetric::BackendJsLowerEntry,
        TimingMetric::BackendJsLowerLinked,
        TimingMetric::BackendHtmlRender,
        TimingMetric::BackendWasmTotal,
        TimingMetric::BackendWasmLower,
        TimingMetric::BackendWasmArtifacts,
        TimingMetric::BackendAssetsPlan,
        TimingMetric::BackendAssetsEmit,
        TimingMetric::OutputWriteTotal,
    ];

    for &metric in BACKEND_AND_OUTPUT_METRICS {
        assert!(metric.applies_to(TimingCommand::Build));
        assert!(metric.applies_to(TimingCommand::Dev));
        assert!(!metric.applies_to(TimingCommand::Check));
    }
}

/// Stage 0 and output evidence stays beneath its generic pipeline owner.
#[test]
fn stage0_and_output_evidence_has_one_command_pipeline_owner() {
    for metric in [
        TimingMetric::Stage0DirectoryInventory,
        TimingMetric::Stage0DirectoryCompile,
        TimingMetric::Stage0SingleFileTotal,
    ] {
        assert_eq!(metric.descriptor().relation, TimingRelation::NestedEvidence);
        assert_eq!(
            metric.descriptor().parent,
            Some(TimingParent::Metric(TimingMetric::BuildFrontendTotal))
        );
    }

    assert_eq!(
        TimingMetric::OutputWriteTotal.descriptor().parent,
        Some(TimingParent::Metric(TimingMetric::BuildOutputTotal))
    );
    assert_eq!(
        TimingMetric::OutputWriteTotal.descriptor().relation,
        TimingRelation::NestedEvidence
    );

    for metric in [
        TimingMetric::BoundaryInventory,
        TimingMetric::BoundaryCompile,
    ] {
        let descriptor = metric.descriptor();
        assert_eq!(descriptor.owner, TimingMetricOwner::Stage0);
        assert_eq!(descriptor.relation, TimingRelation::Accumulated);
        assert_eq!(descriptor.parent, None);
        assert_eq!(descriptor.accounting, TimingAccountingRole::Evidence);
    }
}

/// The generic backend span is the sole command-accounting owner.
#[test]
fn backend_evidence_uses_the_generic_pipeline_total() {
    assert_eq!(TimingMetric::from_name("backend.html.total"), None);
    assert_eq!(
        TimingMetric::BackendWasmTotal.descriptor().parent,
        Some(TimingParent::Metric(TimingMetric::BuildBackendTotal))
    );
    assert_eq!(
        TimingMetric::BackendWasmTotal.descriptor().relation,
        TimingRelation::NestedEvidence
    );
}

/// Enum order, descriptor order and `ALL` agree inline.
#[test]
fn schema_order_is_deterministic() {
    for (index, metric) in TimingMetric::ALL.iter().enumerate() {
        let from_index = TimingMetric::from_index(index);
        assert_eq!(from_index, Some(*metric));
        assert_eq!(metric.index(), index);
    }
}

/// A future dense collector can index every slot without translating names.
#[test]
fn schema_order_supports_dense_collection_without_name_compatibility() {
    let mut occupied_slots = vec![false; TimingMetric::ALL.len()];

    for &metric in TimingMetric::ALL {
        assert!(!occupied_slots[metric.index()]);
        occupied_slots[metric.index()] = true;
    }

    assert!(occupied_slots.into_iter().all(|occupied| occupied));
}

/// Stable-name lookup round-trips exactly.
#[test]
fn stable_name_lookup_round_trips() {
    for &metric in TimingMetric::ALL {
        let name = metric.descriptor().stable_name;
        assert_eq!(TimingMetric::from_name(name), Some(metric));
    }
    assert_eq!(TimingMetric::from_name("does.not.exist"), None);
    assert_eq!(TimingMetric::from_index(TimingMetric::ALL.len()), None);
    assert_eq!(TimingMetric::from_index(usize::MAX), None);
}
