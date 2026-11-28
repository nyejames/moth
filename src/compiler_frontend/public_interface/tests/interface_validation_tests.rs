//! Focused publication-invariant tests for completed public semantic interfaces.
//!
//! WHAT: verifies that malformed successful surfaces fail before provider publication when a
//! binding lacks its declaration, declaration categories disagree, concrete free or receiver
//! summaries are missing, or a generic declaration carries an impossible concrete summary. Local
//! reusable evidence is judged before closure; evidence that joins a provider trait waits for it.
//! WHY: consumers treat a successful provider interface as trusted compiler-owned input, so
//! these failures belong at the publication boundary rather than in source dependency diagnostics.

use super::super::model::{
    ConcreteCallSummaryRecord, PublicBindingExport, PublicCallableReturnContract,
    PublicEvidenceOwnership, PublicEvidenceRecord, PublicEvidenceRequirementMapping,
    PublicFieldTypeSlot, PublicFunctionSemantics, PublicParameterTypeSlot, PublicReturnTypeSlot,
    PublicTraitReceiverAccess, PublicTraitRequirementSurface, PublicTraitReturnContract,
};
use super::super::{
    LocalPublicInterface, PublicDeclarationRecord, PublicDeclarationSemantics,
    PublicFunctionCategory, PublicGenericTemplateDescriptor, PublicInterfaceDraft,
    PublicReceiverMethodCategory, PublicReceiverMethodSemantics, PublicSemanticInterface,
    PublicStructSemantics, PublicTraitSemantics, SourceProviderDependency,
    SourceProviderDependencySet,
};
use super::test_support::{module_origin, struct_origin};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalCoreTraitIdentity, CanonicalEvidenceIdentity,
    CanonicalTraitIdentity, CanonicalTypeIdentity, ExportedGenericParameterIdentity,
    ExternalOpaqueTypeIdentity, GenericDeclarationOrigin, ModulePrivateNominalIdentity,
    StableTraitRequirementIdentity,
};
use crate::compiler_frontend::datatypes::CallableEvaluation;
use crate::compiler_frontend::external_packages::{
    CanonicalBindingSymbolIdentity, ExternalAbiType, ExternalPackageRegistry,
    ExternalSymbolCategory, ExternalSymbolPath, ExternalTypeDef, ExternalTypeId,
};
use crate::compiler_frontend::public_call_summary::typed::{
    CellPort, CellPost, Component, Coverage, EntryPort, ExportPort, Footprint, Lane, ObjectRef,
    OutputPort, PathAccess, PortId, Post, Product, ScalarSource, StorageKind, SupportId,
    TypedCallAllocationFacts, ValueRef, ValueSource,
};
use crate::compiler_frontend::public_call_summary::{
    CallField, CallResultPath, FunctionReturnAliasSummary, PublicCallMutationEffect,
    PublicCallParameterAccess, PublicCallParameterSummary, PublicCallSummary,
    PublicCallTransferEffect,
};
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, ModuleRootRole, OriginDeclarationId, OriginFunctionId, OriginTraitId,
    OriginTypeCategory, OriginTypeId, PublicExportRoute, StableModuleOriginIdentity,
    StablePackageIdentity,
};
use crate::compiler_frontend::source::SourceId;
use crate::compiler_frontend::symbols::identity::DependencyShellId;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

fn empty_summary() -> PublicCallSummary {
    PublicCallSummary {
        parameters: Vec::new(),
        return_alias: FunctionReturnAliasSummary::Known(Box::new(TypedCallAllocationFacts {
            success: Lane::Returning(Product::default()),
            ..Default::default()
        })),
        escapes_builtin_failure: false,
        uses_fast_semantics: false,
        may_trap: false,
        may_fail: false,
    }
}

fn returning_post(post: Post) -> Lane {
    Lane::Returning(
        Product::new(vec![Component {
            support: SupportId(0),
            alternatives: vec![post],
        }])
        .unwrap(),
    )
}

fn value(source: ValueSource) -> ValueRef {
    ValueRef {
        source,
        access: PathAccess {
            restriction: None,
            derived_from: None,
        },
        dependencies: Vec::new(),
    }
}

fn result_object() -> ObjectRef {
    ObjectRef::Exact(ExportPort::Result(CallResultPath {
        slot: 0,
        path: Vec::new(),
    }))
}

fn shared_summary(parameter_count: usize) -> PublicCallSummary {
    PublicCallSummary {
        parameters: (0..parameter_count)
            .map(|_| PublicCallParameterSummary {
                access: PublicCallParameterAccess::Shared,
                mutation: PublicCallMutationEffect::NoWrite,
                transfer_effect: PublicCallTransferEffect::MayConsume,
            })
            .collect(),
        ..empty_summary()
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
                contract:
                    crate::compiler_frontend::public_interface::PublicParameterContract::Value,
            }],
            return_contract: PublicCallableReturnContract::Returns {
                success: Vec::new(),
                error: None,
            },
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
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: vec![
            PublicDeclarationRecord {
                origin: OriginDeclarationId::Type(target_origin.clone()),
                synthetic_interface_provenance: Default::default(),
                semantics: PublicDeclarationSemantics::Struct(PublicStructSemantics {
                    generic_parameters: Vec::new(),
                    fields: Vec::new(),
                    receiver_methods: methods,
                }),
            },
            PublicDeclarationRecord {
                origin: OriginDeclarationId::Trait(trait_origin),
                synthetic_interface_provenance: Default::default(),
                semantics: PublicDeclarationSemantics::Trait(PublicTraitSemantics {
                    requirements: requirement_names
                        .iter()
                        .map(|name| PublicTraitRequirementSurface {
                            name: (*name).to_owned(),
                            receiver_access: PublicTraitReceiverAccess::Immutable,
                            parameters: Vec::new(),
                            return_contract: PublicTraitReturnContract::Returns {
                                success: Vec::new(),
                                error: None,
                            },
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
        synthetic_interface_provenance: Default::default(),
        semantics: PublicDeclarationSemantics::Function(PublicFunctionSemantics {
            evaluation: CallableEvaluation::Runtime,
            category,
            parameters: Vec::new(),
            return_contract: PublicCallableReturnContract::Returns {
                success: Vec::new(),
                error: None,
            },
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
            export_diagnostic_provenance: Vec::new(),
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
        export_diagnostic_provenance: Vec::new(),
        binding_exports: vec![PublicBindingExport {
            exporting_module: module_origin(),
            route: PublicExportRoute::ROOT,
            public_name: "binding".to_owned(),
            target,
        }],
        declarations: Vec::new(),
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    }
}

#[test]
fn binding_export_duplicates_are_scoped_to_the_route() {
    let target = CanonicalBindingSymbolIdentity {
        package: StablePackageIdentity::binding(
            crate::builder_surface::PackageOrigin::Core,
            "@core/math",
        ),
        symbol_path: ExternalSymbolPath::from_single("sin"),
        category: ExternalSymbolCategory::Function,
    };
    let mut interface = binding_interface(target);
    let forms = PublicExportRoute::from_canonical_spelling("forms".to_owned());
    interface.binding_exports.push(PublicBindingExport {
        exporting_module: module_origin(),
        route: forms.clone(),
        public_name: "binding".to_owned(),
        target: CanonicalBindingSymbolIdentity {
            package: StablePackageIdentity::binding(
                crate::builder_surface::PackageOrigin::Core,
                "@core/math",
            ),
            symbol_path: ExternalSymbolPath::from_single("cos"),
            category: ExternalSymbolCategory::Function,
        },
    });
    interface
        .validate_for_publication()
        .expect("the same leaf in different routes is not a duplicate");

    interface.binding_exports.push(PublicBindingExport {
        exporting_module: module_origin(),
        route: forms,
        public_name: "binding".to_owned(),
        target: CanonicalBindingSymbolIdentity {
            package: StablePackageIdentity::binding(
                crate::builder_surface::PackageOrigin::Core,
                "@core/math",
            ),
            symbol_path: ExternalSymbolPath::from_single("tan"),
            category: ExternalSymbolCategory::Function,
        },
    });
    let error = interface
        .validate_for_publication()
        .expect_err("one leaf cannot be published twice in the same route");
    assert!(
        error
            .message
            .contains("duplicate public export 'binding' in route 'forms'"),
        "unexpected publication error: {:?}",
        error
    );
}

#[test]
fn rejects_export_binding_without_declaration() {
    let origin = OriginFunctionId::new_free(module_origin(), "render".to_owned());
    let binding = ExportBinding::new(
        module_origin(),
        PublicExportRoute::ROOT,
        "render".to_owned(),
        OriginDeclarationId::Function(origin),
    );

    let error = PublicSemanticInterface::close_from_local(
        local_interface(vec![binding], Vec::new(), Vec::new()),
        &SourceProviderDependencySet::default(),
        &ExternalPackageRegistry::default(),
    )
    .expect_err("a successful interface cannot publish a dangling binding");

    assert!(error.message.contains("reachable declaration origin"));
}

#[test]
fn rejects_binding_export_owned_by_another_module() {
    let interface = PublicSemanticInterface {
        module_origin: module_origin(),
        export_bindings: Vec::new(),
        export_diagnostic_provenance: Vec::new(),
        binding_exports: vec![PublicBindingExport {
            exporting_module: provider_module_origin(),
            route: PublicExportRoute::ROOT,
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
            .message
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

    assert!(error.message.contains("unresolved canonical target"));
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
    let provider_dependencies = SourceProviderDependencySet::new(vec![SourceProviderDependency {
        kind: crate::compiler_frontend::public_interface::ProviderDependencyKind::Authored {
            shell: DependencyShellId::new(SourceId::from_index(0), 0),
            route: crate::compiler_frontend::semantic_identity::PublicExportRoute::ROOT,
        },
        interface: &interface,
    }])
    .expect("one authored provider should register");

    let error = provider_dependencies
        .validate_binding_targets(&ExternalPackageRegistry::new())
        .expect_err("a category mismatch in a trusted provider interface must fail admission");

    assert!(error.message.contains("unresolved canonical target"));
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
            crate::builder_surface::PackageOrigin::Dependency,
            "@shared/api",
        ),
        ExternalSymbolPath::from_single("Handle"),
    ));
    let interface = PublicSemanticInterface {
        module_origin: module_origin(),
        export_bindings: Vec::new(),
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: vec![PublicDeclarationRecord {
            origin: OriginDeclarationId::Type(struct_origin("Wrapper")),
            synthetic_interface_provenance: Default::default(),
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

    assert!(error.message.contains("unresolved canonical external type"));
}

#[test]
fn accepts_external_type_with_same_stable_identity_and_different_local_ids() {
    let mut registry = ExternalPackageRegistry::default();
    registry
        .register_package(
            "@builder/test/dummy/first",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("dummy package should shift local package allocation");
    let package_id = registry
        .register_package(
            "@builder/test/shared/api",
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
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: vec![PublicDeclarationRecord {
            origin: OriginDeclarationId::Function(OriginFunctionId::new_free(
                module_origin(),
                "open".to_owned(),
            )),
            synthetic_interface_provenance: Default::default(),
            semantics: PublicDeclarationSemantics::Function(PublicFunctionSemantics {
                evaluation: CallableEvaluation::Runtime,
                category: PublicFunctionCategory::GenericTemplate(
                    PublicGenericTemplateDescriptor {
                        generic_parameters: Vec::new(),
                    },
                ),
                parameters: Vec::new(),
                return_contract: PublicCallableReturnContract::Returns {
                    success: vec![PublicReturnTypeSlot {
                        type_identity: CanonicalTypeIdentity::ExternalOpaque(
                            ExternalOpaqueTypeIdentity::new(
                                StablePackageIdentity::binding(
                                    crate::builder_surface::PackageOrigin::Builder,
                                    "@builder/test/shared/api",
                                ),
                                ExternalSymbolPath::from_single("Handle"),
                            ),
                        ),
                    }],
                    error: None,
                },
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
        synthetic_interface_provenance: Default::default(),
        semantics: PublicDeclarationSemantics::Trait(PublicTraitSemantics {
            requirements: Vec::new(),
            incompatibilities: Vec::new(),
        }),
    };

    let error = PublicSemanticInterface::close_from_local(
        local_interface(Vec::new(), vec![declaration], Vec::new()),
        &SourceProviderDependencySet::default(),
        &ExternalPackageRegistry::default(),
    )
    .expect_err("semantic category drift must fail publication");

    assert!(error.message.contains("disagrees with semantic category"));
}

#[test]
fn rejects_direct_private_nominal_before_interface_closure() {
    let origin = OriginFunctionId::new_free(module_origin(), "expose_private".to_owned());
    let declaration = PublicDeclarationRecord {
        origin: OriginDeclarationId::Function(origin),
        synthetic_interface_provenance: Default::default(),
        semantics: PublicDeclarationSemantics::Function(PublicFunctionSemantics {
            evaluation: CallableEvaluation::Runtime,
            category: PublicFunctionCategory::GenericTemplate(PublicGenericTemplateDescriptor {
                generic_parameters: Vec::new(),
            }),
            parameters: Vec::new(),
            return_contract: PublicCallableReturnContract::Returns {
                success: vec![PublicReturnTypeSlot {
                    type_identity: CanonicalTypeIdentity::ModulePrivateNominal(
                        ModulePrivateNominalIdentity::new(
                            module_origin(),
                            "Hidden".to_owned(),
                            OriginTypeCategory::Struct,
                        ),
                    ),
                }],
                error: None,
            },
        }),
    };

    let error = PublicSemanticInterface::close_from_local(
        local_interface(Vec::new(), vec![declaration], Vec::new()),
        &SourceProviderDependencySet::default(),
        &ExternalPackageRegistry::default(),
    )
    .expect_err("artefact-private nominals cannot enter provider closure");

    assert!(error.message.contains("artefact-private type identity"));
}

#[test]
fn rejects_nested_private_generic_instance_before_publication() {
    let private_instance = CanonicalTypeIdentity::ModulePrivateGenericInstance {
        base: ModulePrivateNominalIdentity::new(
            module_origin(),
            "HiddenBox".to_owned(),
            OriginTypeCategory::Struct,
        ),
        arguments: vec![CanonicalTypeIdentity::Builtin(
            CanonicalBuiltinType::FixedScalar(FixedScalar::I32),
        )]
        .into_boxed_slice(),
    };
    let origin = OriginFunctionId::new_free(module_origin(), "expose_nested_private".to_owned());
    let interface = PublicSemanticInterface {
        module_origin: module_origin(),
        export_bindings: Vec::new(),
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: vec![PublicDeclarationRecord {
            origin: OriginDeclarationId::Function(origin),
            synthetic_interface_provenance: Default::default(),
            semantics: PublicDeclarationSemantics::Function(PublicFunctionSemantics {
                evaluation: CallableEvaluation::Runtime,
                category: PublicFunctionCategory::GenericTemplate(
                    PublicGenericTemplateDescriptor {
                        generic_parameters: Vec::new(),
                    },
                ),
                parameters: Vec::new(),
                return_contract: PublicCallableReturnContract::Returns {
                    success: vec![PublicReturnTypeSlot {
                        type_identity: CanonicalTypeIdentity::Option(Box::new(private_instance)),
                    }],
                    error: None,
                },
            }),
        }],
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    };

    let error = interface
        .validate_for_publication()
        .expect_err("nested artefact-private instances cannot publish");

    assert!(error.message.contains("artefact-private type identity"));
}

#[test]
fn rejects_missing_concrete_free_function_summary() {
    let origin = OriginFunctionId::new_free(module_origin(), "render".to_owned());
    let declaration = function_record(origin, PublicFunctionCategory::ConcreteLocal);

    let error = PublicSemanticInterface::close_from_local(
        local_interface(Vec::new(), vec![declaration], Vec::new()),
        &SourceProviderDependencySet::default(),
        &ExternalPackageRegistry::default(),
    )
    .expect_err("each concrete callable needs exactly one summary");

    assert!(error.message.contains("missing concrete call summary"));
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
        &SourceProviderDependencySet::default(),
        &ExternalPackageRegistry::default(),
    )
    .expect_err("generic base declarations cannot carry concrete summaries");

    assert!(error.message.contains("unexpected concrete call summary"));
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
        synthetic_interface_provenance: Default::default(),
        semantics: PublicDeclarationSemantics::Struct(PublicStructSemantics {
            generic_parameters: Vec::new(),
            fields: Vec::new(),
            receiver_methods: vec![PublicReceiverMethodSemantics {
                method_origin,
                category: PublicReceiverMethodCategory::ConcreteLocal,
                parameters: Vec::new(),
                return_contract: PublicCallableReturnContract::Returns {
                    success: Vec::new(),
                    error: None,
                },
            }],
        }),
    };

    let error = PublicSemanticInterface::close_from_local(
        local_interface(Vec::new(), vec![declaration], Vec::new()),
        &SourceProviderDependencySet::default(),
        &ExternalPackageRegistry::default(),
    )
    .expect_err("receiver surfaces require their concrete summary closure");

    assert!(error.message.contains("missing concrete call summary"));
}

#[test]
fn validates_nested_generic_call_path_ordinals_against_the_closed_layout() {
    let envelope = struct_origin("Envelope");
    let parameter = ExportedGenericParameterIdentity::new(
        GenericDeclarationOrigin::nominal_type(envelope.clone()).unwrap(),
        0,
        "T".to_owned(),
    );
    let layout = PublicDeclarationRecord {
        origin: OriginDeclarationId::Type(envelope.clone()),
        synthetic_interface_provenance: Default::default(),
        semantics: PublicDeclarationSemantics::Struct(PublicStructSemantics {
            generic_parameters: vec![super::super::model::PublicGenericParameterSurface {
                identity: parameter.clone(),
                bounds: Vec::new(),
            }],
            fields: vec![PublicFieldTypeSlot {
                name: "payload".to_owned(),
                type_identity: CanonicalTypeIdentity::Option(Box::new(
                    CanonicalTypeIdentity::GenericParameter(parameter),
                )),
                folded_default: None,
            }],
            receiver_methods: Vec::new(),
        }),
    };
    let origin = OriginFunctionId::new_free(module_origin(), "unwrap_first".to_owned());
    let mut declaration = function_record(origin.clone(), PublicFunctionCategory::ConcreteLocal);
    let PublicDeclarationSemantics::Function(function) = &mut declaration.semantics else {
        unreachable!("function declaration fixture");
    };
    let string = CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String);
    function.parameters.push(PublicParameterTypeSlot {
        name: None,
        type_identity: CanonicalTypeIdentity::GenericInstance {
            base: envelope,
            arguments: Box::new([CanonicalTypeIdentity::Collection {
                element: Box::new(string.clone()),
                fixed_capacity: None,
            }]),
        },
        access: PublicCallParameterAccess::Shared,
        folded_default: None,
        contract: super::super::model::PublicParameterContract::Value,
    });
    function.return_contract = PublicCallableReturnContract::Returns {
        success: vec![PublicReturnTypeSlot {
            type_identity: string,
        }],
        error: None,
    };
    let make_summary = |path: Vec<_>| PublicCallSummary {
        return_alias: FunctionReturnAliasSummary::Known(Box::new(TypedCallAllocationFacts {
            ports: vec![EntryPort::Value {
                parameter: 0,
                path: path.into_boxed_slice(),
            }],
            support_count: 1,
            success: returning_post(Post {
                results: vec![(
                    0,
                    value(ValueSource::Allocation(ObjectRef::Entry(PortId(0)))),
                )],
                ..Default::default()
            }),
            ..Default::default()
        })),
        ..shared_summary(1)
    };
    let mut interface = PublicSemanticInterface {
        module_origin: module_origin(),
        export_bindings: Vec::new(),
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: vec![declaration, layout],
        reusable_evidence: Vec::new(),
        concrete_call_summaries: vec![ConcreteCallSummaryRecord {
            origin,
            summary: make_summary(vec![
                CallField::Field(0),
                CallField::Payload {
                    variant: 1,
                    field: 0,
                },
                CallField::Elem,
            ]),
        }],
    };
    interface
        .validate_for_publication()
        .expect("instantiated payload matches String");
    for path in [
        vec![CallField::Field(1)],
        vec![CallField::Field(0)],
        vec![
            CallField::Field(0),
            CallField::Payload {
                variant: 0,
                field: 0,
            },
        ],
        vec![
            CallField::Field(0),
            CallField::Payload {
                variant: 1,
                field: 0,
            },
            CallField::MapKey,
        ],
        vec![
            CallField::Field(0),
            CallField::Payload {
                variant: 1,
                field: 0,
            },
        ],
    ] {
        interface.concrete_call_summaries[0].summary = make_summary(path);
        assert!(interface.validate_for_publication().is_err());
    }

    interface.concrete_call_summaries[0].summary = make_summary(vec![CallField::Field(0)]);
    let PublicDeclarationSemantics::Function(function) = &mut interface.declarations[0].semantics
    else {
        unreachable!("function declaration fixture");
    };
    let CanonicalTypeIdentity::GenericInstance { arguments, .. } =
        &function.parameters[0].type_identity
    else {
        unreachable!("generic parameter fixture");
    };
    function.return_contract = PublicCallableReturnContract::Returns {
        success: vec![PublicReturnTypeSlot {
            type_identity: CanonicalTypeIdentity::Option(Box::new(arguments[0].clone())),
        }],
        error: None,
    };
    interface
        .validate_for_publication()
        .expect("compound generic leaf is exactly substituted");

    interface.declarations.pop();
    interface
        .validate_closure_input()
        .expect("a provider may still supply the projected layout");
    assert!(interface.validate_for_publication().is_err());
}

#[test]
fn scalar_result_cannot_publish_an_allocation_source() {
    let origin = OriginFunctionId::new_free(module_origin(), "flag".to_owned());
    let mut declaration = function_record(origin.clone(), PublicFunctionCategory::ConcreteLocal);
    let PublicDeclarationSemantics::Function(function) = &mut declaration.semantics else {
        unreachable!("function declaration fixture");
    };
    function.return_contract = PublicCallableReturnContract::Returns {
        success: vec![PublicReturnTypeSlot {
            type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Bool),
        }],
        error: None,
    };
    let mut interface = evidence_interface(&[]);
    interface.declarations.push(declaration);
    let string = CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String);
    let allocation = result_object();
    interface
        .concrete_call_summaries
        .push(ConcreteCallSummaryRecord {
            origin,
            summary: PublicCallSummary {
                return_alias: FunctionReturnAliasSummary::Known(Box::new(
                    TypedCallAllocationFacts {
                        support_count: 1,
                        success: returning_post(Post {
                            results: vec![(0, value(ValueSource::Allocation(allocation.clone())))],
                            retained: vec![(allocation, StorageKind::Allocation, string)],
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                )),
                ..empty_summary()
            },
        });
    assert!(interface.validate_for_publication().is_err());

    interface.concrete_call_summaries[0].summary.return_alias = empty_summary().return_alias;
    assert!(interface.validate_for_publication().is_err());

    interface.concrete_call_summaries[0].summary.return_alias =
        FunctionReturnAliasSummary::Known(Box::new(TypedCallAllocationFacts {
            support_count: 1,
            success: returning_post(Post {
                results: vec![(0, value(ValueSource::Scalar(ScalarSource::Independent)))],
                ..Default::default()
            }),
            ..Default::default()
        }));
    interface
        .validate_for_publication()
        .expect("a scalar postcondition matches the declaration");
}

#[test]
fn unpublished_error_channel_requires_the_private_builtin_failure_reservation() {
    let origin = OriginFunctionId::new_free(module_origin(), "helper".to_owned());
    let mut declaration = function_record(origin.clone(), PublicFunctionCategory::ConcreteLocal);
    let PublicDeclarationSemantics::Function(function) = &mut declaration.semantics else {
        unreachable!("function declaration fixture");
    };
    function.parameters.push(PublicParameterTypeSlot {
        name: None,
        type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
        access: PublicCallParameterAccess::Shared,
        folded_default: None,
        contract: super::super::model::PublicParameterContract::Value,
    });
    let mut interface = evidence_interface(&[]);
    interface.declarations.push(declaration);
    let error_facts = |field| {
        let error = result_object();
        FunctionReturnAliasSummary::Known(Box::new(TypedCallAllocationFacts {
            ports: vec![EntryPort::Value {
                parameter: 0,
                path: Box::new([]),
            }],
            support_count: 1,
            error: returning_post(Post {
                cells: vec![(
                    CellPort::Edge(error.clone(), field),
                    CellPost::Replace(vec![Some(value(ValueSource::Allocation(
                        ObjectRef::Entry(PortId(0)),
                    )))]),
                )],
                results: vec![(0, value(ValueSource::Allocation(error.clone())))],
                retained: vec![(
                    error,
                    StorageKind::Allocation,
                    CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error),
                )],
                ..Default::default()
            }),
            ..Default::default()
        }))
    };
    interface
        .concrete_call_summaries
        .push(ConcreteCallSummaryRecord {
            origin,
            summary: PublicCallSummary {
                return_alias: error_facts(CallField::Field(0)),
                ..shared_summary(1)
            },
        });
    assert!(interface.validate_for_publication().is_err());

    interface.concrete_call_summaries[0]
        .summary
        .escapes_builtin_failure = true;
    interface
        .validate_for_publication()
        .expect("a private builtin Error lane need not alter the canonical authored signature");

    interface.concrete_call_summaries[0].summary.return_alias = error_facts(CallField::Field(1));
    assert!(interface.validate_for_publication().is_err());
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
        synthetic_interface_provenance: Default::default(),
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
        synthetic_interface_provenance: Default::default(),
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
        synthetic_interface_provenance: Default::default(),
        semantics: PublicDeclarationSemantics::Function(PublicFunctionSemantics {
            evaluation: CallableEvaluation::Runtime,
            category: PublicFunctionCategory::ConcreteLocal,
            parameters: Vec::new(),
            return_contract: PublicCallableReturnContract::Returns {
                success: vec![PublicReturnTypeSlot {
                    type_identity: CanonicalTypeIdentity::SourceNominal(card_origin.clone()),
                }],
                error: None,
            },
        }),
    };
    let make_summary = PublicCallSummary {
        return_alias: FunctionReturnAliasSummary::Known(Box::new(TypedCallAllocationFacts {
            support_count: 1,
            success: returning_post(Post {
                coverage: Coverage::Unresolved(Footprint::Locations(vec![OutputPort::Result(0)])),
                ..Default::default()
            }),
            ..Default::default()
        })),
        ..empty_summary()
    };
    let provider = PublicSemanticInterface {
        module_origin: provider_module.clone(),
        export_bindings: vec![
            ExportBinding::new(
                provider_module.clone(),
                PublicExportRoute::ROOT,
                "HiddenLabel".to_owned(),
                OriginDeclarationId::Type(hidden_origin.clone()),
            ),
            ExportBinding::new(
                provider_module.clone(),
                PublicExportRoute::ROOT,
                "PublicCard".to_owned(),
                OriginDeclarationId::Type(card_origin.clone()),
            ),
            ExportBinding::new(
                provider_module,
                PublicExportRoute::ROOT,
                "make_card".to_owned(),
                OriginDeclarationId::Function(make_origin.clone()),
            ),
        ],
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: vec![hidden_record, card_record, make_record],
        reusable_evidence: Vec::new(),
        concrete_call_summaries: vec![ConcreteCallSummaryRecord {
            origin: make_origin.clone(),
            summary: make_summary.clone(),
        }],
    };

    let facade_bindings = vec![
        ExportBinding::new(
            module_origin(),
            PublicExportRoute::ROOT,
            "Card".to_owned(),
            OriginDeclarationId::Type(card_origin.clone()),
        ),
        ExportBinding::new(
            module_origin(),
            PublicExportRoute::ROOT,
            "make_card".to_owned(),
            OriginDeclarationId::Function(make_origin.clone()),
        ),
    ];
    let provider_dependencies = SourceProviderDependencySet::new(vec![SourceProviderDependency {
        kind: crate::compiler_frontend::public_interface::ProviderDependencyKind::Authored {
            shell: DependencyShellId::new(SourceId::from_index(0), 0),
            route: crate::compiler_frontend::semantic_identity::PublicExportRoute::ROOT,
        },
        interface: &provider,
    }])
    .expect("one authored provider should register");

    let facade = PublicSemanticInterface::close_from_local(
        local_interface(facade_bindings, Vec::new(), Vec::new()),
        &provider_dependencies,
        &ExternalPackageRegistry::default(),
    )
    .expect("the facade should close over provider-owned nested nominal facts");

    assert!(
        facade
            .declaration(&OriginDeclarationId::Type(hidden_origin))
            .is_some()
    );
    assert!(
        !facade
            .export_bindings
            .iter()
            .any(|binding| binding.public_name() == "HiddenLabel")
    );
    assert_eq!(
        facade
            .concrete_call_summaries
            .iter()
            .find(|record| record.origin == make_origin)
            .map(|record| &record.summary),
        Some(&make_summary)
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
    target.receiver_methods[0].return_contract = PublicCallableReturnContract::Returns {
        success: vec![PublicReturnTypeSlot {
            type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
        }],
        error: None,
    };
    let result = result_object();
    interface.concrete_call_summaries[0].summary.return_alias =
        FunctionReturnAliasSummary::Known(Box::new(TypedCallAllocationFacts {
            success: returning_post(Post {
                results: vec![(0, value(ValueSource::Allocation(result.clone())))],
                retained: vec![(
                    result,
                    StorageKind::Allocation,
                    CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
                )],
                ..Default::default()
            }),
            support_count: 1,
            ..Default::default()
        }));

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

    assert!(error.message.contains("missing target declaration"));
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

    assert!(error.message.contains("missing source trait declaration"));
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

    assert!(error.message.contains("owned by another trait"));
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
    assert!(error.message.contains("more than once"));

    let mut missing = evidence_interface(&["display", "identifier"]);
    missing.reusable_evidence[0].requirement_mappings.pop();
    let error = missing
        .validate_for_publication()
        .expect_err("each authored requirement needs one mapping");
    assert!(error.message.contains("maps 1 requirement"));
}

#[test]
fn rejects_malformed_local_evidence_before_closure() {
    evidence_interface(&["display", "identifier"])
        .validate_closure_input()
        .expect("well-formed local evidence passes before closure");

    let mut interface = evidence_interface(&["display", "identifier"]);
    interface.reusable_evidence[0].requirement_mappings.pop();
    let error = interface
        .validate_closure_input()
        .expect_err("local evidence joins only local declarations, so it is judged now");
    assert!(error.message.contains("maps 1 requirement"));
}

#[test]
fn rejects_malformed_core_trait_evidence_before_closure() {
    let mut interface = evidence_interface(&["display"]);
    interface
        .declarations
        .retain(|declaration| !matches!(declaration.origin, OriginDeclarationId::Trait(_)));
    interface.reusable_evidence[0].identity = CanonicalEvidenceIdentity::new(
        CanonicalTypeIdentity::SourceNominal(struct_origin("Widget")),
        CanonicalTraitIdentity::Core(CanonicalCoreTraitIdentity::Displayable),
    );
    interface.reusable_evidence[0].requirement_mappings.clear();

    let error = interface
        .validate_closure_input()
        .expect_err("core-trait evidence has no provider declaration to wait for");
    assert!(error.message.contains("maps 0 requirement"));
}

#[test]
fn defers_evidence_joining_provider_trait_until_closure_supplies_it() {
    let mut interface = evidence_interface(&["display"]);
    let trait_position = interface
        .declarations
        .iter()
        .position(|declaration| matches!(declaration.origin, OriginDeclarationId::Trait(_)))
        .expect("trait declaration should exist");
    let provider_trait = interface.declarations.remove(trait_position);
    interface.reusable_evidence[0].requirement_mappings.clear();

    interface
        .validate_closure_input()
        .expect("evidence whose trait closure has not supplied waits for the closed pass");

    interface.declarations.push(provider_trait);
    let error = interface
        .validate_for_publication()
        .expect_err("closure supplies the trait, so the empty mapping is judged then");
    assert!(error.message.contains("maps 0 requirement"));
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
        synthetic_interface_provenance: Default::default(),
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
                    contract:
                        crate::compiler_frontend::public_interface::PublicParameterContract::Value,
                }],
                return_contract: PublicCallableReturnContract::Returns {
                    success: Vec::new(),
                    error: None,
                },
            }],
        }),
    });
    interface.reusable_evidence[0].requirement_mappings[0].method_origin =
        other_method_origin.clone();
    interface.concrete_call_summaries[0].origin = other_method_origin;

    let error = interface
        .validate_for_publication()
        .expect_err("a method attached to another receiver cannot satisfy evidence");

    assert!(error.message.contains("not attached to target"));
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

    assert!(error.message.contains("differently named method"));
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
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::FixedScalar(FixedScalar::I32));

    let error = interface
        .validate_for_publication()
        .expect_err("the mapped method receiver shape must match the evidence target");

    assert!(error.message.contains("incompatible receiver method"));
}
