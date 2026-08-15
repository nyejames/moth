//! Focused invariants for indexed interface closure.
//!
//! WHAT: proves the transient interface view and declaration/evidence work queue close deep
//! re-export chains exactly once, copy no unrelated provider facts, fail deterministically on
//! disagreeing publishers and duplicate keys, and publish in an order independent of provider
//! dependency order.
//! WHY: these hidden join invariants are not visible through end-to-end output.

use super::super::model::{ConcreteCallSummaryRecord, PublicAliasSemantics, PublicFieldTypeSlot};
use super::super::{
    LocalPublicInterface, PublicDeclarationRecord, PublicDeclarationSemantics,
    PublicEvidenceOwnership, PublicEvidenceRecord, PublicEvidenceRequirementMapping,
    PublicFunctionCategory, PublicFunctionSemantics, PublicInterfaceDraft, PublicParameterTypeSlot,
    PublicSemanticInterface, PublicStructSemantics, PublicTraitSemantics, SourceProviderDependency,
    SourceProviderDependencySet,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalEvidenceIdentity, CanonicalTraitIdentity, CanonicalTypeIdentity,
    StableTraitRequirementIdentity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallSummary,
};
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, ModuleRootRole, OriginDeclarationId, OriginFunctionId, OriginTraitId,
    OriginTypeCategory, OriginTypeId, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::identity::{DependencyShellId, FileId};

fn provider_origin(module_name: &str) -> StableModuleOriginIdentity {
    StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local(module_name),
        module_name.to_owned(),
        ModuleRootRole::Normal,
    )
}

fn type_origin(module: &StableModuleOriginIdentity, name: &str) -> OriginTypeId {
    OriginTypeId::new(module.clone(), name.to_owned(), OriginTypeCategory::Struct)
}

fn alias_type_origin(module: &StableModuleOriginIdentity, name: &str) -> OriginTypeId {
    OriginTypeId::new(
        module.clone(),
        name.to_owned(),
        OriginTypeCategory::TransparentAlias,
    )
}

fn function_origin(module: &StableModuleOriginIdentity, name: &str) -> OriginFunctionId {
    OriginFunctionId::new_free(module.clone(), name.to_owned())
}

fn trait_origin(module: &StableModuleOriginIdentity, name: &str) -> OriginTraitId {
    OriginTraitId::new(module.clone(), name.to_owned())
}

fn empty_summary() -> PublicCallSummary {
    PublicCallSummary {
        parameters: Vec::new(),
        return_alias: FunctionReturnAliasSummary::Fresh,
    }
}

fn function_record(
    module: &StableModuleOriginIdentity,
    name: &str,
    parameter_count: usize,
) -> PublicDeclarationRecord {
    PublicDeclarationRecord {
        origin: OriginDeclarationId::Function(function_origin(module, name)),
        semantics: PublicDeclarationSemantics::Function(PublicFunctionSemantics {
            category: PublicFunctionCategory::ConcreteLocal,
            parameters: (0..parameter_count)
                .map(|index| PublicParameterTypeSlot {
                    name: Some(format!("arg{index}")),
                    type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int),
                    access: crate::compiler_frontend::public_call_summary::PublicCallParameterAccess::Shared,
                    folded_default: None,
                })
                .collect(),
            returns: Vec::new(),
            error_return: None,
        }),
    }
}

fn struct_record(
    module: &StableModuleOriginIdentity,
    name: &str,
    fields: Vec<PublicFieldTypeSlot>,
) -> PublicDeclarationRecord {
    PublicDeclarationRecord {
        origin: OriginDeclarationId::Type(type_origin(module, name)),
        semantics: PublicDeclarationSemantics::Struct(PublicStructSemantics {
            generic_parameters: Vec::new(),
            fields,
            receiver_methods: Vec::new(),
        }),
    }
}

fn alias_record(
    module: &StableModuleOriginIdentity,
    name: &str,
    target: CanonicalTypeIdentity,
) -> PublicDeclarationRecord {
    PublicDeclarationRecord {
        origin: OriginDeclarationId::Type(alias_type_origin(module, name)),
        semantics: PublicDeclarationSemantics::TransparentAlias(PublicAliasSemantics {
            target_type_identity: target,
        }),
    }
}

fn trait_record(module: &StableModuleOriginIdentity, name: &str) -> PublicDeclarationRecord {
    PublicDeclarationRecord {
        origin: OriginDeclarationId::Trait(trait_origin(module, name)),
        semantics: PublicDeclarationSemantics::Trait(PublicTraitSemantics {
            requirements: Vec::new(),
            incompatibilities: Vec::new(),
        }),
    }
}

fn evidence_record(
    target: OriginTypeId,
    trait_identity: CanonicalTraitIdentity,
) -> PublicEvidenceRecord {
    PublicEvidenceRecord {
        identity: CanonicalEvidenceIdentity::new(
            CanonicalTypeIdentity::SourceNominal(target),
            trait_identity,
        ),
        ownership: PublicEvidenceOwnership::SourceCanonical,
        requirement_mappings: Vec::new(),
    }
}

fn provider_interface(
    module: &StableModuleOriginIdentity,
    declarations: Vec<PublicDeclarationRecord>,
    summaries: Vec<ConcreteCallSummaryRecord>,
    evidence: Vec<PublicEvidenceRecord>,
) -> PublicSemanticInterface {
    PublicSemanticInterface {
        module_origin: module.clone(),
        export_bindings: Vec::new(),
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations,
        reusable_evidence: evidence,
        concrete_call_summaries: summaries,
    }
}

fn local_interface(bindings: Vec<ExportBinding>) -> LocalPublicInterface {
    LocalPublicInterface {
        draft: PublicInterfaceDraft {
            module_origin: provider_origin("facade"),
            export_bindings: bindings,
            export_diagnostic_provenance: Vec::new(),
            binding_exports: Vec::new(),
            declarations: Vec::new(),
            reusable_evidence: Vec::new(),
        },
        concrete_call_summaries: Vec::new(),
    }
}

fn close(
    bindings: Vec<ExportBinding>,
    providers: Vec<&PublicSemanticInterface>,
) -> Result<PublicSemanticInterface, CompilerError> {
    let provider_dependencies = SourceProviderDependencySet::new(
        providers
            .into_iter()
            .enumerate()
            .map(|(index, interface)| SourceProviderDependency {
                kind:
                    crate::compiler_frontend::public_interface::ProviderDependencyKind::Authored {
                        shell: DependencyShellId::new(FileId(0), index as u32),
                    },
                interface,
            })
            .collect(),
    )
    .expect("distinct provider shells should register");

    PublicSemanticInterface::close_from_local(
        local_interface(bindings),
        &provider_dependencies,
        &ExternalPackageRegistry::default(),
    )
}

/// One deep re-export fixture: the facade exports `Card` and `make_card` from the `alpha`
/// provider, whose struct field references the hidden `Label` alias in `beta` and whose
/// evidence names the `DISPLAY_TEXT` trait declared by `alpha`.
struct DeepReexportFixture {
    alpha: PublicSemanticInterface,
    beta: PublicSemanticInterface,
    card_origin: OriginTypeId,
    make_origin: OriginFunctionId,
    label_origin: OriginTypeId,
    display_text_origin: OriginTraitId,
    card_evidence_identity: CanonicalEvidenceIdentity,
}

impl DeepReexportFixture {
    fn new() -> Self {
        let alpha = provider_origin("alpha");
        let beta = provider_origin("beta");
        let card_origin = type_origin(&alpha, "Card");
        let make_origin = function_origin(&alpha, "make_card");
        let label_origin = alias_type_origin(&beta, "Label");
        let display_text_origin = trait_origin(&alpha, "DISPLAY_TEXT");
        let card_evidence_identity = CanonicalEvidenceIdentity::new(
            CanonicalTypeIdentity::SourceNominal(card_origin.clone()),
            CanonicalTraitIdentity::Source(display_text_origin.clone()),
        );

        let alpha = provider_interface(
            &alpha,
            vec![
                struct_record(
                    &alpha,
                    "Card",
                    vec![PublicFieldTypeSlot {
                        name: "label".to_owned(),
                        type_identity: CanonicalTypeIdentity::SourceNominal(label_origin.clone()),
                        folded_default: None,
                    }],
                ),
                function_record(&alpha, "make_card", 0),
                trait_record(&alpha, "DISPLAY_TEXT"),
                struct_record(&alpha, "Unused", Vec::new()),
                function_record(&alpha, "unused_fn", 0),
            ],
            vec![
                ConcreteCallSummaryRecord {
                    origin: make_origin.clone(),
                    summary: empty_summary(),
                },
                ConcreteCallSummaryRecord {
                    origin: function_origin(&alpha, "unused_fn"),
                    summary: empty_summary(),
                },
            ],
            vec![
                evidence_record(
                    card_origin.clone(),
                    CanonicalTraitIdentity::Source(display_text_origin.clone()),
                ),
                evidence_record(
                    type_origin(&alpha, "Unused"),
                    CanonicalTraitIdentity::Source(display_text_origin.clone()),
                ),
            ],
        );

        let beta = provider_interface(
            &beta,
            vec![
                alias_record(
                    &beta,
                    "Label",
                    CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
                ),
                alias_record(
                    &beta,
                    "Other",
                    CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int),
                ),
            ],
            Vec::new(),
            Vec::new(),
        );

        Self {
            alpha,
            beta,
            card_origin,
            make_origin,
            label_origin,
            display_text_origin,
            card_evidence_identity,
        }
    }

    fn facade_bindings(&self) -> Vec<ExportBinding> {
        vec![
            ExportBinding::new(
                provider_origin("facade"),
                "Card".to_owned(),
                OriginDeclarationId::Type(self.card_origin.clone()),
            ),
            ExportBinding::new(
                provider_origin("facade"),
                "make_card".to_owned(),
                OriginDeclarationId::Function(self.make_origin.clone()),
            ),
        ]
    }
}

#[test]
fn deep_reexport_closure_visits_each_record_once() {
    let fixture = DeepReexportFixture::new();
    let closed = close(
        fixture.facade_bindings(),
        vec![&fixture.alpha, &fixture.beta],
    )
    .expect("deep re-export closure should succeed");

    let declaration_origins = closed
        .declarations
        .iter()
        .map(|declaration| declaration.origin.clone())
        .collect::<Vec<_>>();
    for origin in [
        OriginDeclarationId::Type(fixture.card_origin.clone()),
        OriginDeclarationId::Function(fixture.make_origin.clone()),
        OriginDeclarationId::Type(fixture.label_origin.clone()),
        OriginDeclarationId::Trait(fixture.display_text_origin.clone()),
    ] {
        assert_eq!(
            declaration_origins
                .iter()
                .filter(|candidate| **candidate == origin)
                .count(),
            1,
            "each reachable declaration must be closed exactly once: {declaration_origins:?}"
        );
    }

    assert_eq!(
        closed
            .concrete_call_summaries
            .iter()
            .filter(|summary| summary.origin == fixture.make_origin)
            .count(),
        1,
        "the reachable concrete summary must be copied exactly once"
    );
    assert_eq!(
        closed
            .reusable_evidence
            .iter()
            .filter(|evidence| evidence.identity == fixture.card_evidence_identity)
            .count(),
        1,
        "the reachable evidence record must be copied exactly once"
    );
}

#[test]
fn unrelated_provider_facts_are_not_copied() {
    let fixture = DeepReexportFixture::new();
    let closed = close(
        fixture.facade_bindings(),
        vec![&fixture.alpha, &fixture.beta],
    )
    .expect("deep re-export closure should succeed");

    let declaration_origins = closed
        .declarations
        .iter()
        .map(|declaration| declaration.origin.clone())
        .collect::<Vec<_>>();
    assert!(
        !declaration_origins.contains(&OriginDeclarationId::Type(type_origin(
            &fixture.alpha.module_origin,
            "Unused"
        ))),
        "unrelated provider declarations must not be copied"
    );
    assert!(
        !declaration_origins.contains(&OriginDeclarationId::Type(type_origin(
            &fixture.beta.module_origin,
            "Other"
        ))),
        "unrelated provider aliases must not be copied"
    );
    assert!(
        closed
            .concrete_call_summaries
            .iter()
            .all(|summary| summary.origin
                != function_origin(&fixture.alpha.module_origin, "unused_fn")),
        "unrelated provider summaries must not be copied"
    );
    assert_eq!(
        closed.reusable_evidence.len(),
        1,
        "only the evidence for the closed card surface may be copied"
    );
}

#[test]
fn closure_output_is_independent_of_provider_dependency_order() {
    let fixture = DeepReexportFixture::new();
    let first = close(
        fixture.facade_bindings(),
        vec![&fixture.alpha, &fixture.beta],
    )
    .expect("closure with alpha first should succeed");
    let second = close(
        fixture.facade_bindings(),
        vec![&fixture.beta, &fixture.alpha],
    )
    .expect("closure with beta first should succeed");

    assert_eq!(first.declarations, second.declarations);
    assert_eq!(first.reusable_evidence, second.reusable_evidence);
    assert_eq!(
        first.concrete_call_summaries,
        second.concrete_call_summaries
    );
}

#[test]
fn disagreeing_provider_declarations_fail_deterministically() {
    let module = provider_origin("shared");
    let first = provider_interface(
        &module,
        vec![function_record(&module, "make_card", 1)],
        Vec::new(),
        Vec::new(),
    );
    let second = provider_interface(
        &module,
        vec![function_record(&module, "make_card", 0)],
        Vec::new(),
        Vec::new(),
    );

    let error = SourceProviderDependencySet::new(vec![
        SourceProviderDependency {
            kind: crate::compiler_frontend::public_interface::ProviderDependencyKind::Authored {
                shell: DependencyShellId::new(FileId(0), 0),
            },
            interface: &first,
        },
        SourceProviderDependency {
            kind: crate::compiler_frontend::public_interface::ProviderDependencyKind::Authored {
                shell: DependencyShellId::new(FileId(0), 1),
            },
            interface: &second,
        },
    ])
    .expect_err("two providers cannot publish different contents for one origin");

    assert!(
        error
            .msg
            .contains("disagrees with an equal-origin provider interface"),
        "unexpected error: {}",
        error.msg
    );
}

#[test]
fn disagreeing_provider_summaries_fail_deterministically() {
    let module = provider_origin("shared");
    let origin = function_origin(&module, "make_card");
    let mut differing_summary = empty_summary();
    differing_summary.return_alias = FunctionReturnAliasSummary::AliasParams(vec![0]);

    let first = provider_interface(
        &module,
        vec![function_record(&module, "make_card", 0)],
        vec![ConcreteCallSummaryRecord {
            origin: origin.clone(),
            summary: empty_summary(),
        }],
        Vec::new(),
    );
    let second = provider_interface(
        &module,
        vec![function_record(&module, "make_card", 0)],
        vec![ConcreteCallSummaryRecord {
            origin: origin.clone(),
            summary: differing_summary,
        }],
        Vec::new(),
    );

    let error = SourceProviderDependencySet::new(vec![
        SourceProviderDependency {
            kind: crate::compiler_frontend::public_interface::ProviderDependencyKind::Authored {
                shell: DependencyShellId::new(FileId(0), 0),
            },
            interface: &first,
        },
        SourceProviderDependency {
            kind: crate::compiler_frontend::public_interface::ProviderDependencyKind::Authored {
                shell: DependencyShellId::new(FileId(0), 1),
            },
            interface: &second,
        },
    ])
    .expect_err("two providers cannot publish different summaries for one callable");

    assert!(
        error
            .msg
            .contains("disagrees with an equal-origin provider interface"),
        "unexpected error: {}",
        error.msg
    );
}

#[test]
fn duplicate_keys_in_one_interface_fail_at_view_construction() {
    let module = provider_origin("duplicate");
    let duplicate = provider_interface(
        &module,
        vec![
            function_record(&module, "make_card", 0),
            function_record(&module, "make_card", 0),
        ],
        Vec::new(),
        Vec::new(),
    );

    let error = SourceProviderDependencySet::new(vec![SourceProviderDependency {
        kind: crate::compiler_frontend::public_interface::ProviderDependencyKind::Authored {
            shell: DependencyShellId::new(FileId(0), 0),
        },
        interface: &duplicate,
    }])
    .expect_err("a malformed successful interface must fail while its binding view is built");

    assert!(
        error.msg.contains("duplicate declaration origin"),
        "unexpected error: {}",
        error.msg
    );
}

#[test]
fn disagreeing_evidence_records_fail_in_both_provider_orders() {
    let alpha = provider_origin("alpha");
    let beta = provider_origin("beta");
    let card = type_origin(&alpha, "Card");
    let trait_id = CanonicalTraitIdentity::Source(trait_origin(&alpha, "DISPLAY_TEXT"));
    let first = evidence_record(card.clone(), trait_id.clone());
    let mut second = evidence_record(card.clone(), trait_id.clone());
    second
        .requirement_mappings
        .push(PublicEvidenceRequirementMapping {
            requirement_identity: StableTraitRequirementIdentity::new(
                trait_id.clone(),
                "show".to_owned(),
            ),
            method_origin: function_origin(&alpha, "show"),
        });

    let alpha_provider = provider_interface(
        &alpha,
        vec![
            struct_record(&alpha, "Card", Vec::new()),
            trait_record(&alpha, "DISPLAY_TEXT"),
        ],
        Vec::new(),
        vec![first],
    );
    let beta_provider = provider_interface(
        &beta,
        vec![struct_record(&beta, "Card", Vec::new())],
        Vec::new(),
        vec![second],
    );
    let bindings = vec![ExportBinding::new(
        provider_origin("facade"),
        "Card".to_owned(),
        OriginDeclarationId::Type(card),
    )];

    for providers in [
        vec![&alpha_provider, &beta_provider],
        vec![&beta_provider, &alpha_provider],
    ] {
        let error = close(bindings.clone(), providers)
            .expect_err("differing evidence records with one identity must fail in either order");
        assert!(
            error.msg.contains("disagree on reusable evidence identity"),
            "unexpected error: {}",
            error.msg
        );
    }
}

#[test]
fn equal_repeated_evidence_materialises_once() {
    let alpha = provider_origin("alpha");
    let beta = provider_origin("beta");
    let card = type_origin(&alpha, "Card");
    let evidence = evidence_record(
        card.clone(),
        CanonicalTraitIdentity::Source(trait_origin(&alpha, "DISPLAY_TEXT")),
    );
    let alpha_provider = provider_interface(
        &alpha,
        vec![
            struct_record(&alpha, "Card", Vec::new()),
            trait_record(&alpha, "DISPLAY_TEXT"),
        ],
        Vec::new(),
        vec![evidence.clone()],
    );
    let beta_provider = provider_interface(
        &beta,
        vec![struct_record(&beta, "Card", Vec::new())],
        Vec::new(),
        vec![evidence.clone()],
    );
    let bindings = vec![ExportBinding::new(
        provider_origin("facade"),
        "Card".to_owned(),
        OriginDeclarationId::Type(card),
    )];

    let closed = close(bindings, vec![&alpha_provider, &beta_provider])
        .expect("equal repeated evidence records should agree");
    assert_eq!(
        closed
            .reusable_evidence
            .iter()
            .filter(|record| record.identity == evidence.identity)
            .count(),
        1,
        "equal repeated evidence must materialise exactly once"
    );
}
