//! Focused contracts for the direct public-interface canonical byte owner.

use super::{
    PublicChoiceSemantics, PublicConstantSemantics, PublicDeclarationRecord,
    PublicDeclarationSemantics, PublicEvidenceOwnership, PublicEvidenceRecord,
    PublicEvidenceRequirementMapping, PublicFunctionSemantics, PublicGenericTemplateDescriptor,
    PublicInterfaceDraft, PublicReceiverMethodSemantics, PublicStructSemantics,
    PublicTraitReceiverAccess, PublicTraitRequirementParameter, PublicTraitRequirementReturn,
    PublicTraitRequirementSurface, PublicTraitSemantics, TraitSurfaceTypeIdentity,
};
use crate::compiler_frontend::ast::statements::functions::ReturnChannel;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalCoreTraitIdentity, CanonicalEvidenceIdentity,
    CanonicalTraitIdentity, CanonicalTypeIdentity, ExportedGenericParameterIdentity,
    GenericDeclarationOrigin, StableTraitRequirementIdentity,
};
use crate::compiler_frontend::defined_public_type_surface::{
    PublicChoiceVariantSurface, PublicFieldTypeSlot, PublicGenericParameterSurface,
    PublicParameterTypeSlot, PublicReturnTypeSlot,
};
use crate::compiler_frontend::folded_value::{FiniteFloat, PublicFoldedValue};
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallMutationEffect, PublicCallParameterAccess,
    PublicCallParameterSummary, PublicCallReactiveEffect, PublicCallSummary,
    PublicCallSummaryState, PublicCallTransferEffect, PublicCallTransferEligibility,
};
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, ModuleRootRole, OriginDeclarationId, OriginFunctionId, OriginTraitId,
    OriginTypeCategory, OriginTypeId, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::value_mode::ValueMode;

fn module_origin(path: &str) -> StableModuleOriginIdentity {
    StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("encoding-tests"),
        path.to_owned(),
        ModuleRootRole::Normal,
    )
}

fn type_origin(name: &str, category: OriginTypeCategory) -> OriginTypeId {
    OriginTypeId::new(module_origin("surface"), name.to_owned(), category)
}

fn function_origin(name: &str) -> OriginFunctionId {
    OriginFunctionId::new_free(module_origin("surface"), name.to_owned())
}

fn receiver_method_origin(receiver: &OriginTypeId, name: &str) -> OriginFunctionId {
    OriginFunctionId::new_receiver(module_origin("surface"), name.to_owned(), receiver.clone())
}

fn trait_origin(name: &str) -> OriginTraitId {
    OriginTraitId::new(module_origin("surface"), name.to_owned())
}

fn builtin(kind: CanonicalBuiltinType) -> CanonicalTypeIdentity {
    CanonicalTypeIdentity::Builtin(kind)
}

fn generic_parameter(
    declaration_origin: GenericDeclarationOrigin,
    position: u32,
    name: &str,
) -> PublicGenericParameterSurface {
    PublicGenericParameterSurface {
        identity: ExportedGenericParameterIdentity::new(
            declaration_origin,
            position,
            name.to_owned(),
        ),
        bounds: vec![CanonicalTraitIdentity::Core(
            CanonicalCoreTraitIdentity::Displayable,
        )],
    }
}

fn finalized_summary(parameters: Vec<PublicCallParameterSummary>) -> PublicCallSummaryState {
    PublicCallSummaryState::Finalized(PublicCallSummary {
        parameters,
        return_alias: FunctionReturnAliasSummary::AliasParams(vec![0]),
    })
}

fn empty_summary() -> PublicCallSummaryState {
    PublicCallSummaryState::Finalized(PublicCallSummary {
        parameters: Vec::new(),
        return_alias: FunctionReturnAliasSummary::Fresh,
    })
}

fn fixture_draft() -> PublicInterfaceDraft {
    let function = function_origin("run");
    let structure = type_origin("Widget", OriginTypeCategory::Struct);
    let choice = type_origin("Outcome", OriginTypeCategory::Choice);
    let alias = type_origin("Label", OriginTypeCategory::TransparentAlias);
    let constant = crate::compiler_frontend::semantic_identity::OriginConstantId::new(
        module_origin("surface"),
        "PI".to_owned(),
    );
    let trait_id = trait_origin("Renderable");
    let other_trait = trait_origin("Incompatible");
    let render_method = receiver_method_origin(&structure, "render");
    let reset_method = receiver_method_origin(&structure, "reset");
    let generic_method = receiver_method_origin(&structure, "map");

    let function_generic = generic_parameter(
        GenericDeclarationOrigin::free_function(function.clone())
            .expect("free function generic owner should be valid"),
        0,
        "T",
    );
    let structure_generic = generic_parameter(
        GenericDeclarationOrigin::nominal_type(structure.clone())
            .expect("struct generic owner should be valid"),
        0,
        "U",
    );

    let render_requirement = PublicTraitRequirementSurface {
        name: "render".to_owned(),
        receiver_access: PublicTraitReceiverAccess::Immutable,
        parameters: vec![PublicTraitRequirementParameter {
            name: Some("format".to_owned()),
            value_mode: ValueMode::ImmutableOwned,
            type_identity: TraitSurfaceTypeIdentity::Concrete(Box::new(builtin(
                CanonicalBuiltinType::String,
            ))),
        }],
        returns: vec![PublicTraitRequirementReturn {
            channel: ReturnChannel::Success,
            type_identity: TraitSurfaceTypeIdentity::SelfType,
        }],
    };

    let function_semantics = PublicFunctionSemantics {
        generic_template: Some(PublicGenericTemplateDescriptor {
            generic_parameters: vec![function_generic],
        }),
        parameters: vec![
            PublicParameterTypeSlot {
                name: Some("value".to_owned()),
                type_identity: builtin(CanonicalBuiltinType::Int),
                folded_default: Some(PublicFoldedValue::Int(7)),
            },
            PublicParameterTypeSlot {
                name: Some("label".to_owned()),
                type_identity: builtin(CanonicalBuiltinType::String),
                folded_default: None,
            },
        ],
        returns: vec![PublicReturnTypeSlot {
            type_identity: builtin(CanonicalBuiltinType::String),
        }],
        error_return: Some(builtin(CanonicalBuiltinType::Int)),
        call_summary: PublicCallSummaryState::PendingGenerated,
    };

    let receiver_methods = vec![
        PublicReceiverMethodSemantics {
            method_origin: render_method.clone(),
            generic_template: false,
            parameters: Vec::new(),
            returns: vec![PublicReturnTypeSlot {
                type_identity: builtin(CanonicalBuiltinType::String),
            }],
            error_return: None,
            call_summary: empty_summary(),
        },
        PublicReceiverMethodSemantics {
            method_origin: reset_method.clone(),
            generic_template: false,
            parameters: vec![PublicParameterTypeSlot {
                name: Some("value".to_owned()),
                type_identity: builtin(CanonicalBuiltinType::Bool),
                folded_default: None,
            }],
            returns: Vec::new(),
            error_return: None,
            call_summary: finalized_summary(vec![PublicCallParameterSummary {
                access: PublicCallParameterAccess::Reactive,
                mutation: PublicCallMutationEffect::Writes,
                transfer_eligibility: PublicCallTransferEligibility::Eligible,
                transfer_effect: PublicCallTransferEffect::MayConsume,
                reactive_effect: PublicCallReactiveEffect::SubscribesAndInvalidates,
            }]),
        },
        PublicReceiverMethodSemantics {
            method_origin: generic_method,
            generic_template: true,
            parameters: Vec::new(),
            returns: Vec::new(),
            error_return: None,
            call_summary: PublicCallSummaryState::PendingGenerated,
        },
    ];

    let declarations = vec![
        PublicDeclarationRecord {
            origin: OriginDeclarationId::Function(function.clone()),
            semantics: PublicDeclarationSemantics::Function(function_semantics),
        },
        PublicDeclarationRecord {
            origin: OriginDeclarationId::Type(structure.clone()),
            semantics: PublicDeclarationSemantics::Struct(PublicStructSemantics {
                generic_parameters: vec![structure_generic],
                fields: vec![PublicFieldTypeSlot {
                    name: "name".to_owned(),
                    type_identity: builtin(CanonicalBuiltinType::String),
                    folded_default: Some(PublicFoldedValue::String("moth".to_owned())),
                }],
                receiver_methods,
            }),
        },
        PublicDeclarationRecord {
            origin: OriginDeclarationId::Type(choice.clone()),
            semantics: PublicDeclarationSemantics::Choice(PublicChoiceSemantics {
                generic_parameters: Vec::new(),
                variants: vec![
                    PublicChoiceVariantSurface {
                        name: "Some".to_owned(),
                        payload_fields: vec![PublicFieldTypeSlot {
                            name: "value".to_owned(),
                            type_identity: builtin(CanonicalBuiltinType::Int),
                            folded_default: None,
                        }],
                    },
                    PublicChoiceVariantSurface {
                        name: "None".to_owned(),
                        payload_fields: Vec::new(),
                    },
                ],
                receiver_methods: Vec::new(),
            }),
        },
        PublicDeclarationRecord {
            origin: OriginDeclarationId::Type(alias),
            semantics: PublicDeclarationSemantics::TransparentAlias(super::PublicAliasSemantics {
                target_type_identity: builtin(CanonicalBuiltinType::String),
            }),
        },
        PublicDeclarationRecord {
            origin: OriginDeclarationId::Constant(constant),
            semantics: PublicDeclarationSemantics::Constant(PublicConstantSemantics {
                type_identity: builtin(CanonicalBuiltinType::Float),
                folded_value: PublicFoldedValue::Float(
                    FiniteFloat::new(1.5).expect("fixture float should be finite"),
                ),
            }),
        },
        PublicDeclarationRecord {
            origin: OriginDeclarationId::Trait(trait_id.clone()),
            semantics: PublicDeclarationSemantics::Trait(PublicTraitSemantics {
                requirements: vec![render_requirement],
                incompatibilities: vec![
                    CanonicalTraitIdentity::Source(other_trait.clone()),
                    CanonicalTraitIdentity::Core(CanonicalCoreTraitIdentity::Displayable),
                ],
            }),
        },
        PublicDeclarationRecord {
            origin: OriginDeclarationId::Trait(other_trait.clone()),
            semantics: PublicDeclarationSemantics::Trait(PublicTraitSemantics {
                requirements: Vec::new(),
                incompatibilities: Vec::new(),
            }),
        },
    ];

    let export_bindings = declarations
        .iter()
        .map(|record| {
            let name = match &record.origin {
                OriginDeclarationId::Function(origin) => origin.defining_name(),
                OriginDeclarationId::Type(origin) => origin.defining_name(),
                OriginDeclarationId::Constant(origin) => origin.defining_name(),
                OriginDeclarationId::Trait(origin) => origin.defining_name(),
            };
            ExportBinding::new(
                module_origin("surface"),
                name.to_owned(),
                record.origin.clone(),
            )
        })
        .collect();

    PublicInterfaceDraft {
        module_origin: module_origin("surface"),
        export_bindings,
        declarations,
        reusable_evidence: vec![
            PublicEvidenceRecord {
                identity: CanonicalEvidenceIdentity::new(
                    CanonicalTypeIdentity::SourceNominal(structure),
                    CanonicalTraitIdentity::Source(trait_id),
                ),
                ownership: PublicEvidenceOwnership::SourceCanonical,
                requirement_mappings: vec![PublicEvidenceRequirementMapping {
                    requirement_identity: StableTraitRequirementIdentity::new(
                        CanonicalTraitIdentity::Source(trait_origin("Renderable")),
                        "render".to_owned(),
                    ),
                    method_origin: render_method,
                }],
            },
            PublicEvidenceRecord {
                identity: CanonicalEvidenceIdentity::new(
                    CanonicalTypeIdentity::SourceNominal(choice),
                    CanonicalTraitIdentity::Source(other_trait),
                ),
                ownership: PublicEvidenceOwnership::SourceCanonical,
                requirement_mappings: Vec::new(),
            },
        ],
    }
}

fn bytes(draft: &PublicInterfaceDraft) -> Vec<u8> {
    draft
        .canonical_bytes()
        .expect("fixture draft should encode successfully")
}

fn first_function_mut(draft: &mut PublicInterfaceDraft) -> &mut PublicFunctionSemantics {
    draft
        .declarations
        .iter_mut()
        .find_map(|record| match &mut record.semantics {
            PublicDeclarationSemantics::Function(function) => Some(function),
            _ => None,
        })
        .expect("fixture should contain one function")
}

fn receiver_method_mut<'a>(
    draft: &'a mut PublicInterfaceDraft,
    name: &str,
) -> &'a mut PublicReceiverMethodSemantics {
    draft
        .declarations
        .iter_mut()
        .find_map(|record| match &mut record.semantics {
            PublicDeclarationSemantics::Struct(structure) => structure
                .receiver_methods
                .iter_mut()
                .find(|method| method.method_origin.defining_name() == name),
            PublicDeclarationSemantics::Choice(choice) => choice
                .receiver_methods
                .iter_mut()
                .find(|method| method.method_origin.defining_name() == name),
            _ => None,
        })
        .expect("fixture should contain the requested receiver method")
}

#[test]
fn canonical_bytes_repeat_and_ignore_unordered_fact_order() {
    let draft = fixture_draft();
    let repeated = bytes(&draft);
    assert_eq!(repeated, bytes(&draft));

    let mut reordered = draft.clone();
    reordered.export_bindings.reverse();
    reordered.declarations.reverse();
    reordered.reusable_evidence.reverse();
    for record in &mut reordered.declarations {
        match &mut record.semantics {
            PublicDeclarationSemantics::Struct(structure) => {
                structure.receiver_methods.reverse();
            }
            PublicDeclarationSemantics::Trait(trait_semantics) => {
                trait_semantics.incompatibilities.reverse();
            }
            _ => {}
        }
    }

    assert_eq!(repeated, bytes(&reordered));
}

#[test]
fn authored_order_changes_remain_observable() {
    let original = fixture_draft();
    let mut reordered = original.clone();
    first_function_mut(&mut reordered).parameters.swap(0, 1);
    assert_ne!(bytes(&original), bytes(&reordered));
}

#[test]
fn representative_public_fact_changes_alter_bytes() {
    let original = fixture_draft();

    let mut identity_changed = original.clone();
    identity_changed.module_origin = module_origin("other-surface");
    assert_ne!(bytes(&original), bytes(&identity_changed));

    let mut variant_changed = original.clone();
    let type_record_index = variant_changed
        .declarations
        .iter()
        .enumerate()
        .find(|(_, record)| matches!(record.origin, OriginDeclarationId::Type(_)))
        .map(|(index, _)| index)
        .expect("fixture should contain a type declaration");
    let replacement_constant = crate::compiler_frontend::semantic_identity::OriginConstantId::new(
        module_origin("surface"),
        "P2".to_owned(),
    );
    let old_type_origin = variant_changed.declarations[type_record_index]
        .origin
        .clone();
    variant_changed.declarations[type_record_index] = PublicDeclarationRecord {
        origin: OriginDeclarationId::Constant(replacement_constant.clone()),
        semantics: PublicDeclarationSemantics::Constant(PublicConstantSemantics {
            type_identity: builtin(CanonicalBuiltinType::Int),
            folded_value: PublicFoldedValue::Int(1),
        }),
    };
    let binding_index = variant_changed
        .export_bindings
        .iter()
        .position(|binding| binding.origin() == &old_type_origin)
        .expect("fixture should bind the replaced type declaration");
    let exporting_module = variant_changed.export_bindings[binding_index]
        .exporting_module()
        .clone();
    variant_changed.export_bindings[binding_index] = ExportBinding::new(
        exporting_module,
        "P2".to_owned(),
        OriginDeclarationId::Constant(replacement_constant),
    );
    assert_ne!(bytes(&original), bytes(&variant_changed));

    let mut type_changed = original.clone();
    first_function_mut(&mut type_changed).returns[0].type_identity =
        builtin(CanonicalBuiltinType::Int);
    assert_ne!(bytes(&original), bytes(&type_changed));

    let mut folded_changed = original.clone();
    let constant_record = folded_changed
        .declarations
        .iter_mut()
        .find_map(|record| match &mut record.semantics {
            PublicDeclarationSemantics::Constant(constant) => Some(constant),
            _ => None,
        })
        .expect("fixture should contain a constant");
    constant_record.folded_value = PublicFoldedValue::Int(2);
    assert_ne!(bytes(&original), bytes(&folded_changed));

    let mut call_summary_changed = original.clone();
    receiver_method_mut(&mut call_summary_changed, "render").call_summary =
        PublicCallSummaryState::Finalized(PublicCallSummary {
            parameters: Vec::new(),
            return_alias: FunctionReturnAliasSummary::Unknown,
        });
    assert_ne!(bytes(&original), bytes(&call_summary_changed));

    let mut evidence_changed = original.clone();
    evidence_changed.reusable_evidence[0].identity = CanonicalEvidenceIdentity::new(
        builtin(CanonicalBuiltinType::Int),
        CanonicalTraitIdentity::Core(CanonicalCoreTraitIdentity::Displayable),
    );
    assert_ne!(bytes(&original), bytes(&evidence_changed));
}

#[test]
fn tags_and_sequence_lengths_separate_values() {
    let original = fixture_draft();

    let mut variant_changed = original.clone();
    let constant = variant_changed
        .declarations
        .iter_mut()
        .find_map(|record| match &mut record.semantics {
            PublicDeclarationSemantics::Constant(constant) => Some(constant),
            _ => None,
        })
        .expect("fixture should contain a constant");
    constant.type_identity = builtin(CanonicalBuiltinType::Bool);
    constant.folded_value = PublicFoldedValue::Bool(true);
    assert_ne!(bytes(&original), bytes(&variant_changed));

    let mut sequence_changed = original.clone();
    let reset_method = receiver_method_mut(&mut sequence_changed, "reset");
    reset_method.parameters.pop();
    let PublicCallSummaryState::Finalized(summary) = &mut reset_method.call_summary else {
        panic!("non-generic fixture receiver should have a finalized call summary");
    };
    summary.parameters.pop();
    assert_ne!(bytes(&original), bytes(&sequence_changed));
}

#[test]
fn finite_float_encoding_uses_normalized_exact_bits() {
    let mut positive_zero = fixture_draft();
    let mut negative_zero = positive_zero.clone();
    let positive = positive_zero
        .declarations
        .iter_mut()
        .find_map(|record| match &mut record.semantics {
            PublicDeclarationSemantics::Constant(constant) => Some(constant),
            _ => None,
        })
        .expect("fixture should contain a constant");
    positive.folded_value =
        PublicFoldedValue::Float(FiniteFloat::new(0.0).expect("positive zero should be finite"));
    let negative = negative_zero
        .declarations
        .iter_mut()
        .find_map(|record| match &mut record.semantics {
            PublicDeclarationSemantics::Constant(constant) => Some(constant),
            _ => None,
        })
        .expect("fixture should contain a constant");
    negative.folded_value =
        PublicFoldedValue::Float(FiniteFloat::new(-0.0).expect("negative zero should be finite"));
    assert_eq!(bytes(&positive_zero), bytes(&negative_zero));

    let mut next_float = positive_zero.clone();
    let constant = next_float
        .declarations
        .iter_mut()
        .find_map(|record| match &mut record.semantics {
            PublicDeclarationSemantics::Constant(constant) => Some(constant),
            _ => None,
        })
        .expect("fixture should contain a constant");
    constant.folded_value = PublicFoldedValue::Float(
        FiniteFloat::new(1.0 + f64::EPSILON).expect("next finite float should be finite"),
    );
    let mut one_float = positive_zero;
    let constant = one_float
        .declarations
        .iter_mut()
        .find_map(|record| match &mut record.semantics {
            PublicDeclarationSemantics::Constant(constant) => Some(constant),
            _ => None,
        })
        .expect("fixture should contain a constant");
    constant.folded_value =
        PublicFoldedValue::Float(FiniteFloat::new(1.0).expect("one should be finite"));
    assert_ne!(bytes(&one_float), bytes(&next_float));
}

#[test]
fn pending_local_is_rejected_and_pending_generated_is_accepted() {
    let mut pending_local = fixture_draft();
    first_function_mut(&mut pending_local).call_summary = PublicCallSummaryState::PendingLocal;
    let error = pending_local
        .canonical_bytes()
        .expect_err("PendingLocal must not reach fingerprint input");
    assert!(error.msg.contains("PendingLocal"));

    let mut pending_generated = fixture_draft();
    first_function_mut(&mut pending_generated).call_summary =
        PublicCallSummaryState::PendingGenerated;
    assert!(!bytes(&pending_generated).is_empty());
}

#[test]
fn direct_drafts_reject_builtin_and_inconsistent_callable_states() {
    let mut builtin_evidence = fixture_draft();
    builtin_evidence.reusable_evidence[0].ownership = PublicEvidenceOwnership::Builtin;
    let error = builtin_evidence
        .canonical_bytes()
        .expect_err("builtin evidence belongs to the compiler-global path");
    assert!(error.msg.contains("builtin evidence"));

    let mut generic_finalized = fixture_draft();
    first_function_mut(&mut generic_finalized).call_summary = empty_summary();
    let error = generic_finalized
        .canonical_bytes()
        .expect_err("generic templates must retain PendingGenerated state");
    assert!(error.msg.contains("generic free function"));

    let mut non_generic_pending = fixture_draft();
    receiver_method_mut(&mut non_generic_pending, "render").call_summary =
        PublicCallSummaryState::PendingGenerated;
    let error = non_generic_pending
        .canonical_bytes()
        .expect_err("non-generic receiver methods must be finalized");
    assert!(error.msg.contains("non-generic receiver method"));
}
