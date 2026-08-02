//! Focused publication-invariant tests for completed public semantic interfaces.
//!
//! WHAT: verifies that malformed successful surfaces fail before provider publication when a
//! binding lacks its declaration, declaration categories disagree, concrete free or receiver
//! summaries are missing, or a generic declaration carries an impossible concrete summary.
//! WHY: consumers treat a successful provider interface as trusted compiler-owned input, so
//! these failures belong at the publication boundary rather than in source import diagnostics.

use super::super::model::{
    ConcreteCallSummaryRecord, PublicBindingExport, PublicEvidenceOwnership, PublicEvidenceRecord,
    PublicEvidenceRequirementMapping, PublicFieldTypeSlot, PublicFunctionSemantics,
    PublicParameterTypeSlot, PublicReturnTypeSlot, PublicTraitReceiverAccess,
    PublicTraitRequirementSurface,
};
use super::super::{
    LocalPublicInterface, PublicDeclarationRecord, PublicDeclarationSemantics,
    PublicFunctionCategory, PublicGenericTemplateDescriptor, PublicInterfaceDraft,
    PublicReceiverMethodCategory, PublicReceiverMethodSemantics, PublicSemanticInterface,
    PublicStructSemantics, PublicTraitSemantics, SourceProviderImport, SourceProviderImportSet,
};
use super::test_support::{module_origin, struct_origin};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalCoreTraitIdentity, CanonicalEvidenceIdentity,
    CanonicalTraitIdentity, CanonicalTypeIdentity, ExternalOpaqueTypeIdentity,
    ModulePrivateGenericInstanceTypeIdentity, ModulePrivateNominalIdentity,
    StableTraitRequirementIdentity,
};
use crate::compiler_frontend::external_packages::{
    CanonicalBindingSymbolIdentity, ExternalAbiType, ExternalPackageRegistry,
    ExternalSymbolCategory, ExternalSymbolPath, ExternalTypeDef, ExternalTypeId,
};
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallMutationEffect, PublicCallParameterAccess,
    PublicCallParameterSummary, PublicCallReactiveEffect, PublicCallSummary,
    PublicCallTransferEffect, PublicCallTransferEligibility,
};
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, ModuleRootRole, OriginDeclarationId, OriginFunctionId, OriginTraitId,
    OriginTypeCategory, OriginTypeId, StableModuleOriginIdentity, StablePackageIdentity,
};

fn empty_summary() -> PublicCallSummary {
    PublicCallSummary {
        parameters: Vec::new(),
        return_alias: FunctionReturnAliasSummary::Fresh,
    }
}

fn shared_summary(parameter_count: usize) -> PublicCallSummary {
    PublicCallSummary {
        parameters: (0..parameter_count)
            .map(|_| PublicCallParameterSummary {
                access: PublicCallParameterAccess::Shared,
                mutation: PublicCallMutationEffect::NoWrite,
                transfer_eligibility: PublicCallTransferEligibility::Eligible,
                transfer_effect: PublicCallTransferEffect::MayConsume,
                reactive_effect: PublicCallReactiveEffect::None,
            })
            .collect(),
        return_alias: FunctionReturnAliasSummary::Fresh,
    }
}

fn source_trait_origin(name: &str) -> OriginTraitId {
    OriginTraitId::new(module_origin(), name.to_owned())
}

fn evidence_interface(requirement_names: &[&str]) -> PublicSemanticInterface {
    let target_origin = struct_origin("Widget");
    let trait_origin = source_trait_origin("DISPLAY_TEXT");
    let trait_identity = CanonicalTraitIdentity::Source(trait_origin.clone());

    let mut methods = Vec::new();
    let mut mappings = Vec::new();
    let mut summaries = Vec::new();
    for requirement_name in requirement_names {
        let method_origin = OriginFunctionId::new_receiver(
            module_origin(),
            (*requirement_name).to_owned(),
            target_origin.clone(),
        );
        methods.push(PublicReceiverMethodSemantics {
            method_origin: method_origin.clone(),
            category: PublicReceiverMethodCategory::ConcreteLocal,
            parameters: vec![PublicParameterTypeSlot {
                name: Some("this".to_owned()),
                type_identity: CanonicalTypeIdentity::SourceNominal(target_origin.clone()),
                access: PublicCallParameterAccess::Shared,
                folded_default: None,
            }],
            returns: Vec::new(),
            error_return: None,
        });
        mappings.push(PublicEvidenceRequirementMapping {
            requirement_identity: StableTraitRequirementIdentity::new(
                trait_identity.clone(),
                (*requirement_name).to_owned(),
            ),
            method_origin: method_origin.clone(),
        });
        summaries.push(ConcreteCallSummaryRecord {
            origin: method_origin,
            summary: shared_summary(1),
        });
    }

    PublicSemanticInterface {
        module_origin: module_origin(),
        export_bindings: Vec::new(),
        binding_exports: Vec::new(),
        declarations: vec![
            PublicDeclarationRecord {
                origin: OriginDeclarationId::Type(target_origin.clone()),
                semantics: PublicDeclarationSemantics::Struct(PublicStructSemantics {
                    generic_parameters: Vec::new(),
                    fields: Vec::new(),
                    receiver_methods: methods,
                }),
            },
            PublicDeclarationRecord {
                origin: OriginDeclarationId::Trait(trait_origin),
                semantics: PublicDeclarationSemantics::Trait(PublicTraitSemantics {
                    requirements: requirement_names
                        .iter()
                        .map(|name| PublicTraitRequirementSurface {
                            name: (*name).to_owned(),
                            receiver_access: PublicTraitReceiverAccess::Immutable,
                            parameters: Vec::new(),
                            returns: Vec::new(),
                        })
                        .collect(),
                    incompatibilities: Vec::new(),
                }),
            },
        ],
        reusable_evidence: vec![PublicEvidenceRecord {
            identity: CanonicalEvidenceIdentity::new(
                CanonicalTypeIdentity::SourceNominal(target_origin),
                trait_identity,
            ),
            ownership: PublicEvidenceOwnership::SourceCanonical,
            requirement_mappings: mappings,
        }],
        concrete_call_summaries: summaries,
    }
}

fn function_record(
    origin: OriginFunctionId,
    category: PublicFunctionCategory,
) -> PublicDeclarationRecord {
    PublicDeclarationRecord {
        origin: OriginDeclarationId::Function(origin),
        semantics: PublicDeclarationSemantics::Function(PublicFunctionSemantics {
            category,
            parameters: Vec::new(),
            returns: Vec::new(),
            error_return: None,
        }),
    }
}

fn local_interface(
    bindings: Vec<ExportBinding>,
    declarations: Vec<PublicDeclarationRecord>,
    concrete_call_summaries: Vec<ConcreteCallSummaryRecord>,
) -> LocalPublicInterface {
    LocalPublicInterface {
        draft: PublicInterfaceDraft {
            module_origin: module_origin(),
            export_bindings: bindings,
            binding_exports: Vec::new(),
            declarations,
            reusable_evidence: Vec::new(),
        },
        concrete_call_summaries,
    }
}

fn provider_module_origin() -> StableModuleOriginIdentity {
    StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "provider".to_owned(),
        ModuleRootRole::Normal,
    )
}

fn binding_interface(target: CanonicalBindingSymbolIdentity) -> PublicSemanticInterface {
    PublicSemanticInterface {
        module_origin: module_origin(),
        export_bindings: Vec::new(),
        binding_exports: vec![PublicBindingExport {
            exporting_module: module_origin(),
            public_name: "binding".to_owned(),
            target,
        }],
        declarations: Vec::new(),
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    }
}

#[test]
fn rejects_export_binding_without_declaration() {
    let origin = OriginFunctionId::new_free(module_origin(), "render".to_owned());
    let binding = ExportBinding::new(
        module_origin(),
        "render".to_owned(),
        OriginDeclarationId::Function(origin),
    );

    let error = PublicSemanticInterface::close_from_local(
        local_interface(vec![binding], Vec::new(), Vec::new()),
        &SourceProviderImportSet::default(),
        &ExternalPackageRegistry::default(),
    )
    .expect_err("a successful interface cannot publish a dangling binding");

    assert!(error.msg.contains("reachable declaration origin"));
}

#[test]
fn rejects_binding_export_owned_by_another_module() {
    let interface = PublicSemanticInterface {
        module_origin: module_origin(),
        export_bindings: Vec::new(),
        binding_exports: vec![PublicBindingExport {
            exporting_module: provider_module_origin(),
            public_name: "sine".to_owned(),
            target: CanonicalBindingSymbolIdentity {
                package: StablePackageIdentity::binding(
                    crate::builder_surface::PackageOrigin::Core,
                    "@core/math",
                ),
                symbol_path: ExternalSymbolPath::from_single("sin"),
                category: ExternalSymbolCategory::Function,
            },
        }],
        declarations: Vec::new(),
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    };

    let error = interface
        .validate_for_publication()
        .expect_err("a binding export must be owned by the publishing module");

    assert!(
        error
            .msg
            .contains("binding export 'sine' belongs to module"),
        "unexpected publication error: {:?}",
        error
    );
}

#[test]
fn rejects_nonempty_unresolved_binding_target_before_publication() {
    let interface = binding_interface(CanonicalBindingSymbolIdentity {
        package: StablePackageIdentity::binding(
            crate::builder_surface::PackageOrigin::Core,
            "@core/missing",
        ),
        symbol_path: ExternalSymbolPath::from_single("present_but_unregistered"),
        category: ExternalSymbolCategory::Function,
    });

    let error = interface
        .validate_binding_targets(&ExternalPackageRegistry::new())
        .expect_err("a nonempty but unresolved binding identity must not publish");

    assert!(error.msg.contains("unresolved canonical target"));
}

#[test]
fn rejects_binding_target_with_wrong_symbol_category_on_consumer_admission() {
    let interface = binding_interface(CanonicalBindingSymbolIdentity {
        package: StablePackageIdentity::binding(
            crate::builder_surface::PackageOrigin::Core,
            "@core/io",
        ),
        symbol_path: ExternalSymbolPath::from_single("line"),
        category: ExternalSymbolCategory::Type,
    });
    let provider_imports = SourceProviderImportSet::new(vec![SourceProviderImport {
        importer_source: vec!["consumer".to_owned()],
        imported_path: vec!["provider".to_owned()],
        from_grouped: true,
        implicit_template_scope: false,
        interface: &interface,
    }]);

    let error = provider_imports
        .validate_binding_targets(&ExternalPackageRegistry::new())
        .expect_err("a category mismatch in a trusted provider interface must fail admission");

    assert!(error.msg.contains("unresolved canonical target"));
}

#[test]
fn rejects_nested_external_type_from_same_path_but_different_package_origin() {
    let mut registry = ExternalPackageRegistry::default();
    let package_id = registry
        .register_package(
            "@shared/api",
            crate::builder_surface::PackageOrigin::ProjectLocal,
        )
        .expect("consumer package should register");
    registry
        .register_type_in_package(
            package_id,
            ExternalTypeId(31),
            ExternalTypeDef {
                name: "Handle".to_owned(),
                package_id,
                abi_type: ExternalAbiType::Handle,
            },
        )
        .expect("consumer type should register");

    let external_identity = CanonicalTypeIdentity::ExternalOpaque(ExternalOpaqueTypeIdentity::new(
        StablePackageIdentity::binding(
            crate::builder_surface::PackageOrigin::Builder,
            "@shared/api",
        ),
        ExternalSymbolPath::from_single("Handle"),
    ));
    let interface = PublicSemanticInterface {
        module_origin: module_origin(),
        export_bindings: Vec::new(),
        binding_exports: Vec::new(),
        declarations: vec![PublicDeclarationRecord {
            origin: OriginDeclarationId::Type(struct_origin("Wrapper")),
            semantics: PublicDeclarationSemantics::Struct(PublicStructSemantics {
                generic_parameters: Vec::new(),
                fields: vec![PublicFieldTypeSlot {
                    name: "handle".to_owned(),
                    type_identity: CanonicalTypeIdentity::Option(Box::new(external_identity)),
                    folded_default: None,
                }],
                receiver_methods: Vec::new(),
            }),
        }],
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    };

    let error = interface
        .validate_binding_targets(&registry)
        .expect_err("package origin must participate in nested opaque type identity");

    assert!(error.msg.contains("unresolved canonical external type"));
}

#[test]
fn accepts_external_type_with_same_stable_identity_and_different_local_ids() {
    let mut registry = ExternalPackageRegistry::default();
    registry
        .register_package(
            "@dummy/first",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("dummy package should shift local package allocation");
    let package_id = registry
        .register_package(
            "@shared/api",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("consumer package should register");
    registry
        .register_type_in_package(
            package_id,
            ExternalTypeId(901),
            ExternalTypeDef {
                name: "Handle".to_owned(),
                package_id,
                abi_type: ExternalAbiType::Handle,
            },
        )
        .expect("consumer type should register with an unrelated local ID");

    let interface = PublicSemanticInterface {
        module_origin: module_origin(),
        export_bindings: Vec::new(),
        binding_exports: Vec::new(),
        declarations: vec![PublicDeclarationRecord {
            origin: OriginDeclarationId::Function(OriginFunctionId::new_free(
                module_origin(),
                "open".to_owned(),
            )),
            semantics: PublicDeclarationSemantics::Function(PublicFunctionSemantics {
                category: PublicFunctionCategory::GenericTemplate(
                    PublicGenericTemplateDescriptor {
                        generic_parameters: Vec::new(),
                    },
                ),
                parameters: Vec::new(),
                returns: vec![PublicReturnTypeSlot {
                    type_identity: CanonicalTypeIdentity::ExternalOpaque(
                        ExternalOpaqueTypeIdentity::new(
                            StablePackageIdentity::binding(
                                crate::builder_surface::PackageOrigin::Builder,
                                "@shared/api",
                            ),
                            ExternalSymbolPath::from_single("Handle"),
                        ),
                    ),
                }],
                error_return: None,
            }),
        }],
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    };

    interface
        .validate_binding_targets(&registry)
        .expect("stable package/type identity must not depend on consumer-local IDs");
}

#[test]
fn rejects_declaration_category_mismatch() {
    let origin = OriginFunctionId::new_free(module_origin(), "render".to_owned());
    let declaration = PublicDeclarationRecord {
        origin: OriginDeclarationId::Function(origin),
        semantics: PublicDeclarationSemantics::Trait(PublicTraitSemantics {
            requirements: Vec::new(),
            incompatibilities: Vec::new(),
        }),
    };

    let error = PublicSemanticInterface::close_from_local(
        local_interface(Vec::new(), vec![declaration], Vec::new()),
        &SourceProviderImportSet::default(),
        &ExternalPackageRegistry::default(),
    )
    .expect_err("semantic category drift must fail publication");

    assert!(error.msg.contains("disagrees with semantic category"));
}

#[test]
fn rejects_direct_private_nominal_before_interface_closure() {
    let origin = OriginFunctionId::new_free(module_origin(), "expose_private".to_owned());
    let declaration = PublicDeclarationRecord {
        origin: OriginDeclarationId::Function(origin),
        semantics: PublicDeclarationSemantics::Function(PublicFunctionSemantics {
            category: PublicFunctionCategory::GenericTemplate(PublicGenericTemplateDescriptor {
                generic_parameters: Vec::new(),
            }),
            parameters: Vec::new(),
            returns: vec![PublicReturnTypeSlot {
                type_identity: CanonicalTypeIdentity::ModulePrivateNominal(
                    ModulePrivateNominalIdentity::new(
                        module_origin(),
                        "Hidden".to_owned(),
                        OriginTypeCategory::Struct,
                    ),
                ),
            }],
            error_return: None,
        }),
    };

    let error = PublicSemanticInterface::close_from_local(
        local_interface(Vec::new(), vec![declaration], Vec::new()),
        &SourceProviderImportSet::default(),
        &ExternalPackageRegistry::default(),
    )
    .expect_err("artefact-private nominals cannot enter provider closure");

    assert!(error.msg.contains("artefact-private type identity"));
}

#[test]
fn rejects_nested_private_generic_instance_before_publication() {
    let private_instance = CanonicalTypeIdentity::ModulePrivateGenericInstance(
        ModulePrivateGenericInstanceTypeIdentity::new(
            ModulePrivateNominalIdentity::new(
                module_origin(),
                "HiddenBox".to_owned(),
                OriginTypeCategory::Struct,
            ),
            vec![CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int)].into_boxed_slice(),
        ),
    );
    let origin = OriginFunctionId::new_free(module_origin(), "expose_nested_private".to_owned());
    let interface = PublicSemanticInterface {
        module_origin: module_origin(),
        export_bindings: Vec::new(),
        binding_exports: Vec::new(),
        declarations: vec![PublicDeclarationRecord {
            origin: OriginDeclarationId::Function(origin),
            semantics: PublicDeclarationSemantics::Function(PublicFunctionSemantics {
                category: PublicFunctionCategory::GenericTemplate(
                    PublicGenericTemplateDescriptor {
                        generic_parameters: Vec::new(),
                    },
                ),
                parameters: Vec::new(),
                returns: vec![PublicReturnTypeSlot {
                    type_identity: CanonicalTypeIdentity::Option(Box::new(private_instance)),
                }],
                error_return: None,
            }),
        }],
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    };

    let error = interface
        .validate_for_publication()
        .expect_err("nested artefact-private instances cannot publish");

    assert!(error.msg.contains("artefact-private type identity"));
}

#[test]
fn rejects_missing_concrete_free_function_summary() {
    let origin = OriginFunctionId::new_free(module_origin(), "render".to_owned());
    let declaration = function_record(origin, PublicFunctionCategory::ConcreteLocal);

    let error = PublicSemanticInterface::close_from_local(
        local_interface(Vec::new(), vec![declaration], Vec::new()),
        &SourceProviderImportSet::default(),
        &ExternalPackageRegistry::default(),
    )
    .expect_err("each concrete callable needs exactly one summary");

    assert!(error.msg.contains("missing concrete call summary"));
}

#[test]
fn rejects_concrete_summary_for_generic_template() {
    let origin = OriginFunctionId::new_free(module_origin(), "render".to_owned());
    let declaration = function_record(
        origin.clone(),
        PublicFunctionCategory::GenericTemplate(PublicGenericTemplateDescriptor {
            generic_parameters: Vec::new(),
        }),
    );
    let summary = ConcreteCallSummaryRecord {
        origin,
        summary: empty_summary(),
    };

    let error = PublicSemanticInterface::close_from_local(
        local_interface(Vec::new(), vec![declaration], vec![summary]),
        &SourceProviderImportSet::default(),
        &ExternalPackageRegistry::default(),
    )
    .expect_err("generic base declarations cannot carry concrete summaries");

    assert!(error.msg.contains("unexpected concrete call summary"));
}

#[test]
fn rejects_missing_concrete_receiver_summary() {
    let receiver_origin = struct_origin("Widget");
    let method_origin = OriginFunctionId::new_receiver(
        module_origin(),
        "render".to_owned(),
        receiver_origin.clone(),
    );
    let declaration = PublicDeclarationRecord {
        origin: OriginDeclarationId::Type(receiver_origin),
        semantics: PublicDeclarationSemantics::Struct(PublicStructSemantics {
            generic_parameters: Vec::new(),
            fields: Vec::new(),
            receiver_methods: vec![PublicReceiverMethodSemantics {
                method_origin,
                category: PublicReceiverMethodCategory::ConcreteLocal,
                parameters: Vec::new(),
                returns: Vec::new(),
                error_return: None,
            }],
        }),
    };

    let error = PublicSemanticInterface::close_from_local(
        local_interface(Vec::new(), vec![declaration], Vec::new()),
        &SourceProviderImportSet::default(),
        &ExternalPackageRegistry::default(),
    )
    .expect_err("receiver surfaces require their concrete summary closure");

    assert!(error.msg.contains("missing concrete call summary"));
}

#[test]
fn closes_provider_reexport_over_nested_nominal_without_adding_a_public_binding() {
    let provider_module = provider_module_origin();
    let hidden_origin = OriginTypeId::new(
        provider_module.clone(),
        "HiddenLabel".to_owned(),
        OriginTypeCategory::Struct,
    );
    let card_origin = OriginTypeId::new(
        provider_module.clone(),
        "PublicCard".to_owned(),
        OriginTypeCategory::Struct,
    );
    let make_origin = OriginFunctionId::new_free(provider_module.clone(), "make_card".to_owned());

    let hidden_record = PublicDeclarationRecord {
        origin: OriginDeclarationId::Type(hidden_origin.clone()),
        semantics: PublicDeclarationSemantics::Struct(PublicStructSemantics {
            generic_parameters: Vec::new(),
            fields: vec![PublicFieldTypeSlot {
                name: "text".to_owned(),
                type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
                folded_default: None,
            }],
            receiver_methods: Vec::new(),
        }),
    };
    let card_record = PublicDeclarationRecord {
        origin: OriginDeclarationId::Type(card_origin.clone()),
        semantics: PublicDeclarationSemantics::Struct(PublicStructSemantics {
            generic_parameters: Vec::new(),
            fields: vec![PublicFieldTypeSlot {
                name: "label".to_owned(),
                type_identity: CanonicalTypeIdentity::SourceNominal(hidden_origin.clone()),
                folded_default: None,
            }],
            receiver_methods: Vec::new(),
        }),
    };
    let make_record = PublicDeclarationRecord {
        origin: OriginDeclarationId::Function(make_origin.clone()),
        semantics: PublicDeclarationSemantics::Function(PublicFunctionSemantics {
            category: PublicFunctionCategory::ConcreteLocal,
            parameters: Vec::new(),
            returns: vec![PublicReturnTypeSlot {
                type_identity: CanonicalTypeIdentity::SourceNominal(card_origin.clone()),
            }],
            error_return: None,
        }),
    };
    let provider = PublicSemanticInterface {
        module_origin: provider_module.clone(),
        export_bindings: vec![
            ExportBinding::new(
                provider_module.clone(),
                "HiddenLabel".to_owned(),
                OriginDeclarationId::Type(hidden_origin.clone()),
            ),
            ExportBinding::new(
                provider_module.clone(),
                "PublicCard".to_owned(),
                OriginDeclarationId::Type(card_origin.clone()),
            ),
            ExportBinding::new(
                provider_module,
                "make_card".to_owned(),
                OriginDeclarationId::Function(make_origin.clone()),
            ),
        ],
        binding_exports: Vec::new(),
        declarations: vec![hidden_record, card_record, make_record],
        reusable_evidence: Vec::new(),
        concrete_call_summaries: vec![ConcreteCallSummaryRecord {
            origin: make_origin.clone(),
            summary: empty_summary(),
        }],
    };

    let facade_bindings = vec![
        ExportBinding::new(
            module_origin(),
            "Card".to_owned(),
            OriginDeclarationId::Type(card_origin.clone()),
        ),
        ExportBinding::new(
            module_origin(),
            "make_card".to_owned(),
            OriginDeclarationId::Function(make_origin.clone()),
        ),
    ];
    let provider_imports = SourceProviderImportSet::new(vec![SourceProviderImport {
        importer_source: Vec::new(),
        imported_path: Vec::new(),
        from_grouped: true,
        implicit_template_scope: false,
        interface: &provider,
    }]);

    let facade = PublicSemanticInterface::close_from_local(
        local_interface(facade_bindings, Vec::new(), Vec::new()),
        &provider_imports,
        &ExternalPackageRegistry::default(),
    )
    .expect("the facade should close over provider-owned nested nominal facts");

    assert!(
        facade
            .declaration(&OriginDeclarationId::Type(hidden_origin))
            .is_some()
    );
    assert!(facade.exported_origin("HiddenLabel").is_none());
    assert_eq!(
        facade.concrete_call_summary(&make_origin),
        Some(&empty_summary())
    );
}

#[test]
fn accepts_marker_and_multi_requirement_reusable_evidence() {
    evidence_interface(&[])
        .validate_for_publication()
        .expect("marker evidence has exact empty requirement coverage");
    evidence_interface(&["display", "identifier"])
        .validate_for_publication()
        .expect("each authored requirement maps once to an exact target method");
}

#[test]
fn accepts_source_owned_evidence_for_canonical_core_trait() {
    let mut interface = evidence_interface(&["display"]);
    interface
        .declarations
        .retain(|declaration| !matches!(declaration.origin, OriginDeclarationId::Trait(_)));

    let core_trait = CanonicalTraitIdentity::Core(CanonicalCoreTraitIdentity::Displayable);
    interface.reusable_evidence[0].identity = CanonicalEvidenceIdentity::new(
        CanonicalTypeIdentity::SourceNominal(struct_origin("Widget")),
        core_trait.clone(),
    );
    interface.reusable_evidence[0].requirement_mappings[0].requirement_identity =
        StableTraitRequirementIdentity::new(core_trait, "display".to_owned());

    let target_declaration = interface
        .declarations
        .iter_mut()
        .find(|declaration| {
            declaration.origin == OriginDeclarationId::Type(struct_origin("Widget"))
        })
        .expect("target declaration should exist");
    let PublicDeclarationSemantics::Struct(target) = &mut target_declaration.semantics else {
        unreachable!("target fixture is a struct");
    };
    target.receiver_methods[0].returns = vec![PublicReturnTypeSlot {
        type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
    }];

    interface
        .validate_for_publication()
        .expect("source-owned core-trait evidence should validate against core metadata");
}

#[test]
fn rejects_reusable_evidence_with_unknown_target() {
    let mut interface = evidence_interface(&[]);
    let trait_identity = interface.reusable_evidence[0]
        .identity
        .trait_identity()
        .clone();
    interface.reusable_evidence[0].identity = CanonicalEvidenceIdentity::new(
        CanonicalTypeIdentity::SourceNominal(struct_origin("MissingWidget")),
        trait_identity,
    );

    let error = interface
        .validate_for_publication()
        .expect_err("evidence cannot name a target absent from the interface closure");

    assert!(error.msg.contains("missing target declaration"));
}

#[test]
fn rejects_reusable_evidence_with_unknown_source_trait() {
    let mut interface = evidence_interface(&[]);
    interface.reusable_evidence[0].identity = CanonicalEvidenceIdentity::new(
        CanonicalTypeIdentity::SourceNominal(struct_origin("Widget")),
        CanonicalTraitIdentity::Source(source_trait_origin("MISSING_TRAIT")),
    );

    let error = interface
        .validate_for_publication()
        .expect_err("source evidence cannot name a trait absent from the closure");

    assert!(error.msg.contains("missing source trait declaration"));
}

#[test]
fn rejects_reusable_evidence_mapping_owned_by_another_trait() {
    let mut interface = evidence_interface(&["display"]);
    interface.reusable_evidence[0].requirement_mappings[0].requirement_identity =
        StableTraitRequirementIdentity::new(
            CanonicalTraitIdentity::Source(source_trait_origin("OTHER_TRAIT")),
            "display".to_owned(),
        );

    let error = interface
        .validate_for_publication()
        .expect_err("a requirement mapping must belong to the evidence trait");

    assert!(error.msg.contains("owned by another trait"));
}

#[test]
fn rejects_duplicate_and_missing_reusable_evidence_requirements() {
    let mut duplicate = evidence_interface(&["display", "identifier"]);
    duplicate.reusable_evidence[0].requirement_mappings[1].requirement_identity =
        duplicate.reusable_evidence[0].requirement_mappings[0]
            .requirement_identity
            .clone();
    let error = duplicate
        .validate_for_publication()
        .expect_err("one requirement cannot be mapped twice");
    assert!(error.msg.contains("more than once"));

    let mut missing = evidence_interface(&["display", "identifier"]);
    missing.reusable_evidence[0].requirement_mappings.pop();
    let error = missing
        .validate_for_publication()
        .expect_err("each authored requirement needs one mapping");
    assert!(error.msg.contains("maps 1 requirement"));
}

#[test]
fn rejects_reusable_evidence_method_on_another_receiver() {
    let mut interface = evidence_interface(&["display"]);
    let target_declaration = interface
        .declarations
        .iter_mut()
        .find(|declaration| {
            declaration.origin == OriginDeclarationId::Type(struct_origin("Widget"))
        })
        .expect("target declaration should exist");
    let PublicDeclarationSemantics::Struct(target) = &mut target_declaration.semantics else {
        unreachable!("target fixture is a struct");
    };
    target.receiver_methods.clear();

    let other_origin = struct_origin("OtherWidget");
    let other_method_origin =
        OriginFunctionId::new_receiver(module_origin(), "display".to_owned(), other_origin.clone());
    interface.declarations.push(PublicDeclarationRecord {
        origin: OriginDeclarationId::Type(other_origin.clone()),
        semantics: PublicDeclarationSemantics::Struct(PublicStructSemantics {
            generic_parameters: Vec::new(),
            fields: Vec::new(),
            receiver_methods: vec![PublicReceiverMethodSemantics {
                method_origin: other_method_origin.clone(),
                category: PublicReceiverMethodCategory::ConcreteLocal,
                parameters: vec![PublicParameterTypeSlot {
                    name: Some("this".to_owned()),
                    type_identity: CanonicalTypeIdentity::SourceNominal(other_origin),
                    access: PublicCallParameterAccess::Shared,
                    folded_default: None,
                }],
                returns: Vec::new(),
                error_return: None,
            }],
        }),
    });
    interface.reusable_evidence[0].requirement_mappings[0].method_origin =
        other_method_origin.clone();
    interface.concrete_call_summaries[0].origin = other_method_origin;

    let error = interface
        .validate_for_publication()
        .expect_err("a method attached to another receiver cannot satisfy evidence");

    assert!(error.msg.contains("not attached to target"));
}

#[test]
fn rejects_reusable_evidence_mapped_to_differently_named_method() {
    let mut interface = evidence_interface(&["display"]);
    let wrong_method_origin = OriginFunctionId::new_receiver(
        module_origin(),
        "identifier".to_owned(),
        struct_origin("Widget"),
    );

    let target_declaration = interface
        .declarations
        .iter_mut()
        .find(|declaration| {
            declaration.origin == OriginDeclarationId::Type(struct_origin("Widget"))
        })
        .expect("target declaration should exist");
    let PublicDeclarationSemantics::Struct(target) = &mut target_declaration.semantics else {
        unreachable!("target fixture is a struct");
    };
    target.receiver_methods[0].method_origin = wrong_method_origin.clone();
    interface.reusable_evidence[0].requirement_mappings[0].method_origin =
        wrong_method_origin.clone();
    interface.concrete_call_summaries[0].origin = wrong_method_origin;

    let error = interface
        .validate_for_publication()
        .expect_err("a differently named method cannot satisfy a requirement by shape alone");

    assert!(error.msg.contains("differently named method"));
}

#[test]
fn rejects_reusable_evidence_with_incompatible_method_shape() {
    let mut interface = evidence_interface(&["display"]);
    let target_declaration = interface
        .declarations
        .iter_mut()
        .find(|declaration| {
            declaration.origin == OriginDeclarationId::Type(struct_origin("Widget"))
        })
        .expect("target declaration should exist");
    let PublicDeclarationSemantics::Struct(target) = &mut target_declaration.semantics else {
        unreachable!("target fixture is a struct");
    };
    target.receiver_methods[0].parameters[0].type_identity =
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int);

    let error = interface
        .validate_for_publication()
        .expect_err("the mapped method receiver shape must match the evidence target");

    assert!(error.msg.contains("incompatible receiver method"));
}
