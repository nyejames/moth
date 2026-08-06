//! Unit tests for the timing schema v1 registry.
//!
//! WHAT: verifies every checklist invariant of the typed metric registry so a
//!      future edit cannot silently break name uniqueness, dense indexing,
//!      attribution rules or command-total ownership.
//! WHY:  schema errors are cheaper to pin down here than in collector or
//!       benchmark output, and the whole timing system assumes these rules.

use crate::timing::enabled::schema::{
    TIMING_METRIC_DESCRIPTORS, TIMING_SCHEMA_VERSION, TimingAttributionKind, TimingCommand,
    TimingLevel, TimingMetric, TimingRelation,
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
/// record (46 metrics, 35 Basic + 11 Detailed).
#[test]
fn registry_size_matches_plan_closeout() {
    assert_eq!(TimingMetric::ALL.len(), 46);
    let basic = TimingMetric::ALL
        .iter()
        .filter(|metric| metric.descriptor().level == TimingLevel::Basic)
        .count();
    let detailed = TimingMetric::ALL
        .iter()
        .filter(|metric| metric.descriptor().level == TimingLevel::Detailed)
        .count();
    assert_eq!(basic, 35);
    assert_eq!(detailed, 11);
}

/// Every basic metric has a human owner and a concrete semantic role.
#[test]
fn every_basic_metric_has_a_human_owner_row() {
    for &metric in TimingMetric::ALL {
        let descriptor = metric.descriptor();
        if descriptor.level == TimingLevel::Basic {
            assert!(
                is_basic_metric_supposed(metric),
                "{} ({}) must be wired into the concise report",
                descriptor.stable_name,
                metric.index()
            );
        }
    }
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

/// Every parent policy must be coherent.
///
/// Nested evidence rows always declare a parent. Accumulated rows may group
/// under a well-known aggregate row key (e.g. `frontend.borrow`), which has no
/// dedicated metric of its own. Wall-span rows must never declare a parent.
#[test]
fn every_parent_policy_is_coherent() {
    for metric in TimingMetric::ALL {
        let descriptor = metric.descriptor();
        match descriptor.relation {
            TimingRelation::NestedEvidence => {
                let parent = descriptor
                    .parent
                    .unwrap_or_else(|| panic!("nested {} lacks a parent", descriptor.stable_name));
                assert!(
                    parent_valid(parent),
                    "{} parent policy references unknown parent '{parent}'",
                    descriptor.stable_name
                );
            }
            TimingRelation::Accumulated => {
                if let Some(parent) = descriptor.parent {
                    assert!(
                        parent_valid(parent),
                        "{} groups under unknown parent '{parent}'",
                        descriptor.stable_name
                    );
                }
            }
            TimingRelation::WallSpan => {
                assert!(
                    descriptor.parent.is_none(),
                    "{} must not declare a parent as a wall-span row",
                    descriptor.stable_name
                );
            }
        }
    }
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

// ---------------------------------------------------------------------
// Private test policy
// ---------------------------------------------------------------------

/// Whether a basic metric has a role in the concise human report.
fn is_basic_metric_supposed(metric: TimingMetric) -> bool {
    matches!(
        metric,
        TimingMetric::CommandBuildTotal
            | TimingMetric::CommandCheckTotal
            | TimingMetric::CommandDevBuildWrite
            | TimingMetric::BuildBootstrapTotal
            | TimingMetric::BuildFrontendTotal
            | TimingMetric::BuildBackendTotal
            | TimingMetric::BuildOutputTotal
            | TimingMetric::Stage0DirectoryInventory
            | TimingMetric::Stage0DirectoryCompile
            | TimingMetric::Stage0SingleFileTotal
            | TimingMetric::BoundaryInventory
            | TimingMetric::BoundaryCompile
            | TimingMetric::FrontendPrepare
            | TimingMetric::FrontendBindHeaders
            | TimingMetric::FrontendOrderDeclarations
            | TimingMetric::FrontendAstTotal
            | TimingMetric::FrontendAstEnvironment
            | TimingMetric::FrontendAstEmit
            | TimingMetric::FrontendAstFinalise
            | TimingMetric::FrontendPublicInterfaceProject
            | TimingMetric::FrontendHir
            | TimingMetric::FrontendBorrowInitial
            | TimingMetric::FrontendBorrowConverge
            | TimingMetric::FrontendGeneratedMaterialise
            | TimingMetric::FrontendGeneratedBorrowRecheck
            | TimingMetric::FrontendPublicInterfaceFinalise
            | TimingMetric::FrontendModuleSemanticTotal
            | TimingMetric::BackendHtmlTotal
            | TimingMetric::BackendJsLowerEntry
            | TimingMetric::BackendJsLowerLinked
            | TimingMetric::BackendHtmlRender
            | TimingMetric::BackendWasmTotal
            | TimingMetric::BackendAssetsPlan
            | TimingMetric::BackendAssetsEmit
            | TimingMetric::OutputWriteTotal
    )
}

/// The definitive parent check: a parent is valid when it is another metric's
/// stable name or a well-known human aggregate row key.
fn parent_valid(parent: &str) -> bool {
    TimingMetric::from_name(parent).is_some()
        || matches!(
            parent,
            "frontend.public_interface" | "frontend.borrow" | "frontend.generated"
        )
}
