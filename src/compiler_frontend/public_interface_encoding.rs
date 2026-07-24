//! Deterministic canonical bytes for the direct public-interface draft.
//!
//! WHAT: owns the frontend-only byte encoding consumed later as public-interface hash input.
//! Every retained [`PublicInterfaceDraft`] fact receives an explicit domain or variant tag,
//! fixed-width scalar representation and length-prefixed string or sequence boundary.
//!
//! WHY: the draft must have one deterministic representation before provider interfaces and the
//! five fingerprint policy exist. This module intentionally does not choose a digest algorithm,
//! persistent format, cache schema or provenance association. HIR-owned synthetic-interface
//! provenance never enters the draft and cannot enter these bytes.

use crate::builder_surface::PackageOrigin;
use crate::compiler_frontend::ast::statements::functions::ReturnChannel;
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastTarget,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalCoreTraitIdentity, CanonicalEvidenceIdentity,
    CanonicalTraitIdentity, CanonicalTypeIdentity, ExportedGenericParameterIdentity,
    GenericDeclarationOrigin, GenericDeclarationOwner, StableTraitRequirementIdentity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::defined_public_type_surface::{
    PublicChoiceVariantSurface, PublicFieldTypeSlot, PublicGenericParameterSurface,
    PublicParameterTypeSlot, PublicReturnTypeSlot,
};
use crate::compiler_frontend::external_packages::ExternalSymbolPath;
use crate::compiler_frontend::folded_value::{FiniteFloat, PublicFoldedField, PublicFoldedValue};
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallMutationEffect, PublicCallParameterAccess,
    PublicCallParameterSummary, PublicCallReactiveEffect, PublicCallSummary,
    PublicCallSummaryState, PublicCallTransferEffect, PublicCallTransferEligibility,
};
use crate::compiler_frontend::public_interface_draft::{
    PublicChoiceSemantics, PublicDeclarationRecord, PublicDeclarationSemantics,
    PublicEvidenceRecord, PublicEvidenceRequirementMapping, PublicFunctionSemantics,
    PublicGenericTemplateDescriptor, PublicInterfaceDraft, PublicReceiverMethodSemantics,
    PublicStructSemantics, PublicTraitReceiverAccess, PublicTraitRequirementParameter,
    PublicTraitRequirementReturn, PublicTraitRequirementSurface, PublicTraitSemantics,
    TraitSurfaceTypeIdentity,
};
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, FunctionOriginKind, ModuleRootRole, OriginConstantId, OriginDeclarationId,
    OriginFunctionId, OriginTraitId, OriginTypeCategory, OriginTypeId, StableModuleOriginIdentity,
    StablePackageIdentity,
};
use crate::compiler_frontend::value_mode::ValueMode;

/// Encode one completed direct public-interface draft as deterministic hash input.
pub(crate) fn encode_public_interface_draft(
    draft: &PublicInterfaceDraft,
) -> Result<Vec<u8>, CompilerError> {
    let mut encoder = CanonicalEncoder::new();
    encoder.write_bytes(b"moth.public-interface-draft");
    encoder.write_u16(1);

    encode_module_origin(&mut encoder, &draft.module_origin);

    let export_bindings = sorted_encoded_items(
        &draft.export_bindings,
        encode_export_binding,
        encode_export_binding,
    )?;
    encoder.write_encoded_sequence(&export_bindings)?;

    let declarations = sorted_encoded_items(
        &draft.declarations,
        |record| encode_origin_declaration_key(&record.origin),
        encode_declaration_record,
    )?;
    encoder.write_encoded_sequence(&declarations)?;

    let reusable_evidence = sorted_encoded_items(
        &draft.reusable_evidence,
        |record| encode_evidence_identity(&record.identity),
        encode_evidence_record,
    )?;
    encoder.write_encoded_sequence(&reusable_evidence)?;

    Ok(encoder.finish())
}

struct CanonicalEncoder {
    bytes: Vec<u8>,
}

impl CanonicalEncoder {
    fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }

    fn write_u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn write_u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn write_u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn write_u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn write_i32(&mut self, value: i32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn write_bool(&mut self, value: bool) {
        self.write_u8(u8::from(value));
    }

    fn write_count(&mut self, count: usize) -> Result<(), CompilerError> {
        let count = u64::try_from(count).map_err(|_| {
            CompilerError::compiler_error(
                "public-interface canonical encoding encountered a sequence too large for its fixed-width length field",
            )
        })?;
        self.write_u64(count);
        Ok(())
    }

    fn write_index(&mut self, value: usize) -> Result<(), CompilerError> {
        let value = u64::try_from(value).map_err(|_| {
            CompilerError::compiler_error(
                "public-interface canonical encoding encountered a semantic index too large for its fixed-width representation",
            )
        })?;
        self.write_u64(value);
        Ok(())
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        self.write_u64(bytes.len() as u64);
        self.bytes.extend_from_slice(bytes);
    }

    fn write_string(&mut self, value: &str) {
        self.write_bytes(value.as_bytes());
    }

    fn write_encoded_sequence(&mut self, items: &[Vec<u8>]) -> Result<(), CompilerError> {
        self.write_count(items.len())?;
        for item in items {
            self.write_bytes(item);
        }
        Ok(())
    }
}

fn sorted_encoded_items<T>(
    items: &[T],
    mut key_encoder: impl FnMut(&T) -> Result<Vec<u8>, CompilerError>,
    mut item_encoder: impl FnMut(&T) -> Result<Vec<u8>, CompilerError>,
) -> Result<Vec<Vec<u8>>, CompilerError> {
    let mut encoded = Vec::with_capacity(items.len());
    for item in items {
        encoded.push((key_encoder(item)?, item_encoder(item)?));
    }

    encoded.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    Ok(encoded.into_iter().map(|(_, item)| item).collect())
}

fn ordered_encoded_items<T>(
    items: &[T],
    mut item_encoder: impl FnMut(&T) -> Result<Vec<u8>, CompilerError>,
) -> Result<Vec<Vec<u8>>, CompilerError> {
    items.iter().map(&mut item_encoder).collect()
}

fn encode_module_origin(encoder: &mut CanonicalEncoder, origin: &StableModuleOriginIdentity) {
    encode_package_identity(encoder, origin.package());
    encoder.write_string(origin.logical_module_path());
    encode_module_root_role(encoder, origin.role());
}

fn encode_package_identity(encoder: &mut CanonicalEncoder, package: &StablePackageIdentity) {
    encode_package_origin(encoder, package.origin());
    encoder.write_string(package.name());
}

fn encode_package_origin(encoder: &mut CanonicalEncoder, origin: PackageOrigin) {
    encoder.write_u8(match origin {
        PackageOrigin::Core => 0,
        PackageOrigin::Standard => 1,
        PackageOrigin::Builder => 2,
        PackageOrigin::ProjectLocal => 3,
        PackageOrigin::Dependency => 4,
    });
}

fn encode_module_root_role(encoder: &mut CanonicalEncoder, role: ModuleRootRole) {
    encoder.write_u8(match role {
        ModuleRootRole::Normal => 0,
        ModuleRootRole::Support => 1,
        ModuleRootRole::ProjectPackageFacade => 2,
    });
}

fn encode_export_binding(binding: &ExportBinding) -> Result<Vec<u8>, CompilerError> {
    let mut encoder = CanonicalEncoder::new();
    encode_module_origin(&mut encoder, binding.exporting_module());
    encoder.write_string(binding.public_name());
    encode_origin_declaration(&mut encoder, binding.origin())?;
    Ok(encoder.finish())
}

fn encode_origin_declaration_key(origin: &OriginDeclarationId) -> Result<Vec<u8>, CompilerError> {
    let mut encoder = CanonicalEncoder::new();
    encode_origin_declaration(&mut encoder, origin)?;
    Ok(encoder.finish())
}

fn encode_origin_declaration(
    encoder: &mut CanonicalEncoder,
    origin: &OriginDeclarationId,
) -> Result<(), CompilerError> {
    match origin {
        OriginDeclarationId::Function(origin) => {
            encoder.write_u8(0);
            encode_origin_function(encoder, origin)?;
        }
        OriginDeclarationId::Type(origin) => {
            encoder.write_u8(1);
            encode_origin_type(encoder, origin)?;
        }
        OriginDeclarationId::Constant(origin) => {
            encoder.write_u8(2);
            encode_origin_constant(encoder, origin);
        }
        OriginDeclarationId::Trait(origin) => {
            encoder.write_u8(3);
            encode_origin_trait(encoder, origin);
        }
    }
    Ok(())
}

fn encode_origin_type(
    encoder: &mut CanonicalEncoder,
    origin: &OriginTypeId,
) -> Result<(), CompilerError> {
    encode_module_origin(encoder, origin.module_origin());
    encoder.write_string(origin.defining_name());
    encoder.write_u8(match origin.category() {
        OriginTypeCategory::Struct => 0,
        OriginTypeCategory::Choice => 1,
        OriginTypeCategory::TransparentAlias => 2,
    });
    Ok(())
}

fn encode_origin_function(
    encoder: &mut CanonicalEncoder,
    origin: &OriginFunctionId,
) -> Result<(), CompilerError> {
    encode_module_origin(encoder, origin.module_origin());
    encoder.write_string(origin.defining_name());
    match origin.kind() {
        FunctionOriginKind::Free => encoder.write_u8(0),
        FunctionOriginKind::Receiver(receiver) => {
            encoder.write_u8(1);
            encode_origin_type(encoder, receiver)?;
        }
    }
    Ok(())
}

fn encode_origin_constant(encoder: &mut CanonicalEncoder, origin: &OriginConstantId) {
    encode_module_origin(encoder, origin.module_origin());
    encoder.write_string(origin.defining_name());
}

fn encode_origin_trait(encoder: &mut CanonicalEncoder, origin: &OriginTraitId) {
    encode_module_origin(encoder, origin.module_origin());
    encoder.write_string(origin.defining_name());
}

fn encode_declaration_record(record: &PublicDeclarationRecord) -> Result<Vec<u8>, CompilerError> {
    let mut encoder = CanonicalEncoder::new();
    encode_origin_declaration(&mut encoder, &record.origin)?;
    encode_declaration_semantics(&mut encoder, &record.semantics)?;
    Ok(encoder.finish())
}

fn encode_declaration_semantics(
    encoder: &mut CanonicalEncoder,
    semantics: &PublicDeclarationSemantics,
) -> Result<(), CompilerError> {
    match semantics {
        PublicDeclarationSemantics::Function(function) => {
            encoder.write_u8(0);
            encode_function_semantics(encoder, function)?;
        }
        PublicDeclarationSemantics::Struct(structure) => {
            encoder.write_u8(1);
            encode_struct_semantics(encoder, structure)?;
        }
        PublicDeclarationSemantics::Choice(choice) => {
            encoder.write_u8(2);
            encode_choice_semantics(encoder, choice)?;
        }
        PublicDeclarationSemantics::TransparentAlias(alias) => {
            encoder.write_u8(3);
            encode_canonical_type(encoder, &alias.target_type_identity)?;
        }
        PublicDeclarationSemantics::Constant(constant) => {
            encoder.write_u8(4);
            encode_canonical_type(encoder, &constant.type_identity)?;
            encode_folded_value(encoder, &constant.folded_value)?;
        }
        PublicDeclarationSemantics::Trait(trait_semantics) => {
            encoder.write_u8(5);
            encode_trait_semantics(encoder, trait_semantics)?;
        }
    }
    Ok(())
}

fn encode_function_semantics(
    encoder: &mut CanonicalEncoder,
    function: &PublicFunctionSemantics,
) -> Result<(), CompilerError> {
    encode_optional(
        encoder,
        function.generic_template.as_ref(),
        encode_generic_template_descriptor,
    )?;
    encode_parameter_slots(encoder, &function.parameters)?;
    encode_return_slots(encoder, &function.returns)?;
    encode_optional(
        encoder,
        function.error_return.as_ref(),
        encode_canonical_type,
    )?;
    encode_callable_summary_state(
        encoder,
        function.generic_template.is_some(),
        function.parameters.len(),
        &function.call_summary,
        "free function",
    )
}

fn encode_generic_template_descriptor(
    encoder: &mut CanonicalEncoder,
    template: &PublicGenericTemplateDescriptor,
) -> Result<(), CompilerError> {
    encode_generic_parameter_surfaces(encoder, &template.generic_parameters)
}

fn encode_struct_semantics(
    encoder: &mut CanonicalEncoder,
    structure: &PublicStructSemantics,
) -> Result<(), CompilerError> {
    encode_generic_parameter_surfaces(encoder, &structure.generic_parameters)?;
    encode_field_slots(encoder, &structure.fields)?;
    encode_receiver_methods(encoder, &structure.receiver_methods)
}

fn encode_choice_semantics(
    encoder: &mut CanonicalEncoder,
    choice: &PublicChoiceSemantics,
) -> Result<(), CompilerError> {
    encode_generic_parameter_surfaces(encoder, &choice.generic_parameters)?;
    let variants = ordered_encoded_items(&choice.variants, encode_choice_variant)?;
    encoder.write_encoded_sequence(&variants)?;
    encode_receiver_methods(encoder, &choice.receiver_methods)
}

fn encode_trait_semantics(
    encoder: &mut CanonicalEncoder,
    trait_semantics: &PublicTraitSemantics,
) -> Result<(), CompilerError> {
    let requirements =
        ordered_encoded_items(&trait_semantics.requirements, encode_trait_requirement)?;
    encoder.write_encoded_sequence(&requirements)?;

    let incompatibilities = sorted_encoded_items(
        &trait_semantics.incompatibilities,
        encode_canonical_trait_identity,
        encode_canonical_trait_identity,
    )?;
    encoder.write_encoded_sequence(&incompatibilities)
}

fn encode_generic_parameter_surfaces(
    encoder: &mut CanonicalEncoder,
    parameters: &[PublicGenericParameterSurface],
) -> Result<(), CompilerError> {
    let parameters = ordered_encoded_items(parameters, encode_generic_parameter_surface)?;
    encoder.write_encoded_sequence(&parameters)
}

fn encode_generic_parameter_surface(
    parameter: &PublicGenericParameterSurface,
) -> Result<Vec<u8>, CompilerError> {
    let mut encoder = CanonicalEncoder::new();
    encode_exported_generic_parameter_identity(&mut encoder, &parameter.identity)?;
    let bounds = ordered_encoded_items(&parameter.bounds, |bound| {
        encode_canonical_trait_identity(bound)
    })?;
    encoder.write_encoded_sequence(&bounds)?;
    Ok(encoder.finish())
}

fn encode_parameter_slots(
    encoder: &mut CanonicalEncoder,
    parameters: &[PublicParameterTypeSlot],
) -> Result<(), CompilerError> {
    let parameters = ordered_encoded_items(parameters, encode_parameter_slot)?;
    encoder.write_encoded_sequence(&parameters)
}

fn encode_parameter_slot(parameter: &PublicParameterTypeSlot) -> Result<Vec<u8>, CompilerError> {
    let mut encoder = CanonicalEncoder::new();
    encode_optional_string(&mut encoder, parameter.name.as_deref());
    encode_canonical_type(&mut encoder, &parameter.type_identity)?;
    encode_optional(
        &mut encoder,
        parameter.folded_default.as_ref(),
        encode_folded_value,
    )?;
    Ok(encoder.finish())
}

fn encode_return_slots(
    encoder: &mut CanonicalEncoder,
    returns: &[PublicReturnTypeSlot],
) -> Result<(), CompilerError> {
    let returns = ordered_encoded_items(returns, |return_slot| {
        let mut encoder = CanonicalEncoder::new();
        encode_canonical_type(&mut encoder, &return_slot.type_identity)?;
        Ok(encoder.finish())
    })?;
    encoder.write_encoded_sequence(&returns)
}

fn encode_field_slots(
    encoder: &mut CanonicalEncoder,
    fields: &[PublicFieldTypeSlot],
) -> Result<(), CompilerError> {
    let fields = ordered_encoded_items(fields, encode_field_slot)?;
    encoder.write_encoded_sequence(&fields)
}

fn encode_field_slot(field: &PublicFieldTypeSlot) -> Result<Vec<u8>, CompilerError> {
    let mut encoder = CanonicalEncoder::new();
    encoder.write_string(&field.name);
    encode_canonical_type(&mut encoder, &field.type_identity)?;
    encode_optional(
        &mut encoder,
        field.folded_default.as_ref(),
        encode_folded_value,
    )?;
    Ok(encoder.finish())
}

fn encode_choice_variant(variant: &PublicChoiceVariantSurface) -> Result<Vec<u8>, CompilerError> {
    let mut encoder = CanonicalEncoder::new();
    encoder.write_string(&variant.name);
    encode_field_slots(&mut encoder, &variant.payload_fields)?;
    Ok(encoder.finish())
}

fn encode_receiver_methods(
    encoder: &mut CanonicalEncoder,
    methods: &[PublicReceiverMethodSemantics],
) -> Result<(), CompilerError> {
    let methods = sorted_encoded_items(
        methods,
        |method| {
            let mut encoder = CanonicalEncoder::new();
            encode_origin_function(&mut encoder, &method.method_origin)?;
            Ok(encoder.finish())
        },
        encode_receiver_method,
    )?;
    encoder.write_encoded_sequence(&methods)
}

fn encode_receiver_method(
    method: &PublicReceiverMethodSemantics,
) -> Result<Vec<u8>, CompilerError> {
    let mut encoder = CanonicalEncoder::new();
    encode_origin_function(&mut encoder, &method.method_origin)?;
    encoder.write_bool(method.generic_template);
    encode_parameter_slots(&mut encoder, &method.parameters)?;
    encode_return_slots(&mut encoder, &method.returns)?;
    encode_optional(
        &mut encoder,
        method.error_return.as_ref(),
        encode_canonical_type,
    )?;
    encode_callable_summary_state(
        &mut encoder,
        method.generic_template,
        method.parameters.len(),
        &method.call_summary,
        "receiver method",
    )?;
    Ok(encoder.finish())
}

fn encode_trait_requirement(
    requirement: &PublicTraitRequirementSurface,
) -> Result<Vec<u8>, CompilerError> {
    let mut encoder = CanonicalEncoder::new();
    encoder.write_string(requirement.canonical_encoding_name());
    encode_trait_receiver_access(
        &mut encoder,
        requirement.canonical_encoding_receiver_access(),
    );

    let parameters = ordered_encoded_items(
        requirement.canonical_encoding_parameters(),
        encode_trait_requirement_parameter,
    )?;
    encoder.write_encoded_sequence(&parameters)?;

    let returns = ordered_encoded_items(
        requirement.canonical_encoding_returns(),
        encode_trait_requirement_return,
    )?;
    encoder.write_encoded_sequence(&returns)?;
    Ok(encoder.finish())
}

fn encode_trait_requirement_parameter(
    parameter: &PublicTraitRequirementParameter,
) -> Result<Vec<u8>, CompilerError> {
    let mut encoder = CanonicalEncoder::new();
    encode_optional_string(&mut encoder, parameter.canonical_encoding_name());
    encode_value_mode(&mut encoder, parameter.canonical_encoding_value_mode());
    encode_trait_surface_type_identity(&mut encoder, parameter.canonical_encoding_type_identity())?;
    Ok(encoder.finish())
}

fn encode_trait_requirement_return(
    return_slot: &PublicTraitRequirementReturn,
) -> Result<Vec<u8>, CompilerError> {
    let mut encoder = CanonicalEncoder::new();
    encode_return_channel(&mut encoder, return_slot.canonical_encoding_channel());
    encode_trait_surface_type_identity(
        &mut encoder,
        return_slot.canonical_encoding_type_identity(),
    )?;
    Ok(encoder.finish())
}

fn encode_trait_surface_type_identity(
    encoder: &mut CanonicalEncoder,
    identity: &TraitSurfaceTypeIdentity,
) -> Result<(), CompilerError> {
    match identity {
        TraitSurfaceTypeIdentity::SelfType => encoder.write_u8(0),
        TraitSurfaceTypeIdentity::Concrete(identity) => {
            encoder.write_u8(1);
            encode_canonical_type(encoder, identity)?;
        }
    }
    Ok(())
}

fn encode_trait_receiver_access(encoder: &mut CanonicalEncoder, access: PublicTraitReceiverAccess) {
    encoder.write_u8(match access {
        PublicTraitReceiverAccess::Immutable => 0,
        PublicTraitReceiverAccess::Mutable => 1,
    });
}

fn encode_callable_summary_state(
    encoder: &mut CanonicalEncoder,
    generic_template: bool,
    signature_parameter_count: usize,
    state: &PublicCallSummaryState,
    callable_kind: &str,
) -> Result<(), CompilerError> {
    match (generic_template, state) {
        (true, PublicCallSummaryState::PendingGenerated) => {
            encoder.write_u8(0);
            Ok(())
        }
        (false, PublicCallSummaryState::Finalized(summary)) => {
            if summary.parameters.len() != signature_parameter_count {
                return Err(CompilerError::compiler_error(format!(
                    "public-interface canonical encoding found {} summary parameter(s) for a non-generic {}; the public signature has {} parameter(s)",
                    summary.parameters.len(),
                    callable_kind,
                    signature_parameter_count
                )));
            }
            encoder.write_u8(1);
            encode_call_summary(encoder, summary)
        }
        (true, PublicCallSummaryState::Finalized(_)) => {
            Err(CompilerError::compiler_error(format!(
                "public-interface canonical encoding found Finalized call-summary state for a generic {}; generated summaries must remain PendingGenerated",
                callable_kind
            )))
        }
        (false, PublicCallSummaryState::PendingGenerated) => {
            Err(CompilerError::compiler_error(format!(
                "public-interface canonical encoding found PendingGenerated call-summary state for a non-generic {}; local summaries must be Finalized",
                callable_kind
            )))
        }
        (_, PublicCallSummaryState::PendingLocal) => Err(CompilerError::compiler_error(format!(
            "public-interface canonical encoding cannot encode PendingLocal call-summary state for {}; borrow validation must finalize the local callable before fingerprint input is produced",
            callable_kind
        ))),
    }
}

fn encode_call_summary(
    encoder: &mut CanonicalEncoder,
    summary: &PublicCallSummary,
) -> Result<(), CompilerError> {
    let parameters = ordered_encoded_items(&summary.parameters, encode_call_parameter_summary)?;
    encoder.write_encoded_sequence(&parameters)?;
    encode_return_alias_summary(encoder, &summary.return_alias)
}

fn encode_call_parameter_summary(
    parameter: &PublicCallParameterSummary,
) -> Result<Vec<u8>, CompilerError> {
    let mut encoder = CanonicalEncoder::new();
    encode_call_parameter_access(&mut encoder, parameter.access);
    encode_call_mutation_effect(&mut encoder, parameter.mutation);
    encode_call_transfer_eligibility(&mut encoder, parameter.transfer_eligibility);
    encode_call_transfer_effect(&mut encoder, parameter.transfer_effect);
    encode_call_reactive_effect(&mut encoder, parameter.reactive_effect);
    Ok(encoder.finish())
}

fn encode_call_parameter_access(encoder: &mut CanonicalEncoder, access: PublicCallParameterAccess) {
    encoder.write_u8(match access {
        PublicCallParameterAccess::Shared => 0,
        PublicCallParameterAccess::Mutable => 1,
        PublicCallParameterAccess::Reactive => 2,
    });
}

fn encode_call_mutation_effect(encoder: &mut CanonicalEncoder, effect: PublicCallMutationEffect) {
    encoder.write_u8(match effect {
        PublicCallMutationEffect::NoWrite => 0,
        PublicCallMutationEffect::Writes => 1,
    });
}

fn encode_call_transfer_eligibility(
    encoder: &mut CanonicalEncoder,
    eligibility: PublicCallTransferEligibility,
) {
    encoder.write_u8(match eligibility {
        PublicCallTransferEligibility::Ineligible => 0,
        PublicCallTransferEligibility::Eligible => 1,
    });
}

fn encode_call_transfer_effect(encoder: &mut CanonicalEncoder, effect: PublicCallTransferEffect) {
    encoder.write_u8(match effect {
        PublicCallTransferEffect::NeverConsumes => 0,
        PublicCallTransferEffect::MayConsume => 1,
        PublicCallTransferEffect::AlwaysConsumes => 2,
    });
}

fn encode_call_reactive_effect(encoder: &mut CanonicalEncoder, effect: PublicCallReactiveEffect) {
    encoder.write_u8(match effect {
        PublicCallReactiveEffect::None => 0,
        PublicCallReactiveEffect::Subscribes => 1,
        PublicCallReactiveEffect::Invalidates => 2,
        PublicCallReactiveEffect::SubscribesAndInvalidates => 3,
    });
}

fn encode_return_alias_summary(
    encoder: &mut CanonicalEncoder,
    summary: &FunctionReturnAliasSummary,
) -> Result<(), CompilerError> {
    match summary {
        FunctionReturnAliasSummary::Fresh => encoder.write_u8(0),
        FunctionReturnAliasSummary::AliasParams(parameter_indices) => {
            encoder.write_u8(1);
            encoder.write_count(parameter_indices.len())?;
            for parameter_index in parameter_indices {
                encoder.write_index(*parameter_index)?;
            }
        }
        FunctionReturnAliasSummary::Unknown => encoder.write_u8(2),
    }
    Ok(())
}

fn encode_value_mode(encoder: &mut CanonicalEncoder, value_mode: &ValueMode) {
    encoder.write_u8(match value_mode {
        ValueMode::MutableOwned => 0,
        ValueMode::MutableReference => 1,
        ValueMode::ImmutableOwned => 2,
        ValueMode::ImmutableReference => 3,
    });
}

fn encode_return_channel(encoder: &mut CanonicalEncoder, channel: ReturnChannel) {
    encoder.write_u8(match channel {
        ReturnChannel::Success => 0,
        ReturnChannel::Error => 1,
    });
}

fn encode_evidence_identity(
    identity: &CanonicalEvidenceIdentity,
) -> Result<Vec<u8>, CompilerError> {
    let mut encoder = CanonicalEncoder::new();
    encode_canonical_type(&mut encoder, identity.target_type_identity())?;
    let trait_identity = encode_canonical_trait_identity(identity.trait_identity())?;
    encoder.write_bytes(&trait_identity);
    Ok(encoder.finish())
}

fn encode_evidence_record(record: &PublicEvidenceRecord) -> Result<Vec<u8>, CompilerError> {
    let mut encoder = CanonicalEncoder::new();
    let identity = encode_evidence_identity(&record.identity)?;
    encoder.write_bytes(&identity);
    encoder.write_u8(match record.ownership {
        crate::compiler_frontend::public_interface_draft::PublicEvidenceOwnership::SourceCanonical => 0,
        crate::compiler_frontend::public_interface_draft::PublicEvidenceOwnership::Builtin => {
            return Err(CompilerError::compiler_error(
                "public-interface canonical encoding found builtin evidence in a direct module draft; builtin evidence belongs to the separate compiler-global path",
            ));
        }
    });

    let mappings = ordered_encoded_items(
        &record.requirement_mappings,
        encode_evidence_requirement_mapping,
    )?;
    encoder.write_encoded_sequence(&mappings)?;
    Ok(encoder.finish())
}

fn encode_evidence_requirement_mapping(
    mapping: &PublicEvidenceRequirementMapping,
) -> Result<Vec<u8>, CompilerError> {
    let mut encoder = CanonicalEncoder::new();
    encode_stable_trait_requirement_identity(&mut encoder, &mapping.requirement_identity)?;
    encode_origin_function(&mut encoder, &mapping.method_origin)?;
    Ok(encoder.finish())
}

fn encode_stable_trait_requirement_identity(
    encoder: &mut CanonicalEncoder,
    identity: &StableTraitRequirementIdentity,
) -> Result<(), CompilerError> {
    let trait_identity = encode_canonical_trait_identity(identity.trait_identity())?;
    encoder.write_bytes(&trait_identity);
    encoder.write_string(identity.requirement_name());
    Ok(())
}

fn encode_exported_generic_parameter_identity(
    encoder: &mut CanonicalEncoder,
    identity: &ExportedGenericParameterIdentity,
) -> Result<(), CompilerError> {
    encode_generic_declaration_origin(encoder, identity.declaration_origin())?;
    encoder.write_u32(identity.position());
    encoder.write_string(identity.authored_name());
    Ok(())
}

fn encode_generic_declaration_origin(
    encoder: &mut CanonicalEncoder,
    origin: &GenericDeclarationOrigin,
) -> Result<(), CompilerError> {
    match origin.owner() {
        GenericDeclarationOwner::FreeFunction(function) => {
            encoder.write_u8(0);
            encode_origin_function(encoder, function)?;
        }
        GenericDeclarationOwner::NominalType(nominal) => {
            encoder.write_u8(1);
            encode_origin_type(encoder, nominal)?;
        }
    }
    Ok(())
}

fn encode_canonical_trait_identity(
    identity: &CanonicalTraitIdentity,
) -> Result<Vec<u8>, CompilerError> {
    let mut encoder = CanonicalEncoder::new();
    match identity {
        CanonicalTraitIdentity::Source(origin) => {
            encoder.write_u8(0);
            encode_origin_trait(&mut encoder, origin);
        }
        CanonicalTraitIdentity::Core(core) => {
            encoder.write_u8(1);
            encode_canonical_core_trait_identity(&mut encoder, *core);
        }
    }
    Ok(encoder.finish())
}

fn encode_canonical_core_trait_identity(
    encoder: &mut CanonicalEncoder,
    identity: CanonicalCoreTraitIdentity,
) {
    match identity {
        CanonicalCoreTraitIdentity::Displayable => encoder.write_u8(0),
        CanonicalCoreTraitIdentity::Castable {
            target,
            fallibility,
        } => {
            encoder.write_u8(1);
            encode_builtin_cast_target(encoder, target);
            encode_builtin_cast_fallibility(encoder, fallibility);
        }
    }
}

fn encode_builtin_cast_target(encoder: &mut CanonicalEncoder, target: BuiltinCastTarget) {
    encoder.write_u8(match target {
        BuiltinCastTarget::Bool => 0,
        BuiltinCastTarget::Int => 1,
        BuiltinCastTarget::String => 2,
        BuiltinCastTarget::Char => 3,
        BuiltinCastTarget::Float => 4,
        BuiltinCastTarget::Error => 5,
    });
}

fn encode_builtin_cast_fallibility(
    encoder: &mut CanonicalEncoder,
    fallibility: BuiltinCastFallibility,
) {
    encoder.write_u8(match fallibility {
        BuiltinCastFallibility::Infallible => 0,
        BuiltinCastFallibility::Fallible => 1,
    });
}

fn encode_canonical_type(
    encoder: &mut CanonicalEncoder,
    identity: &CanonicalTypeIdentity,
) -> Result<(), CompilerError> {
    match identity {
        CanonicalTypeIdentity::Builtin(builtin) => {
            encoder.write_u8(0);
            encode_canonical_builtin_type(encoder, *builtin);
        }
        CanonicalTypeIdentity::SourceNominal(origin) => {
            encoder.write_u8(1);
            encode_origin_type(encoder, origin)?;
        }
        CanonicalTypeIdentity::ExternalOpaque(external) => {
            encoder.write_u8(2);
            encoder.write_string(external.package_path());
            encode_external_symbol_path(encoder, external.symbol_path())?;
        }
        CanonicalTypeIdentity::Collection(collection) => {
            encoder.write_u8(3);
            encode_canonical_type(encoder, collection.element())?;
            match collection.fixed_capacity() {
                Some(capacity) => {
                    encoder.write_u8(1);
                    encoder.write_index(capacity)?;
                }
                None => encoder.write_u8(0),
            }
        }
        CanonicalTypeIdentity::OrderedMap(map) => {
            encoder.write_u8(4);
            encode_canonical_type(encoder, map.key())?;
            encode_canonical_type(encoder, map.value())?;
        }
        CanonicalTypeIdentity::Option(inner) => {
            encoder.write_u8(5);
            encode_canonical_type(encoder, inner)?;
        }
        CanonicalTypeIdentity::FallibleCarrier(carrier) => {
            encoder.write_u8(6);
            encode_canonical_type(encoder, carrier.success())?;
            encode_canonical_type(encoder, carrier.error())?;
        }
        CanonicalTypeIdentity::GenericInstance(instance) => {
            encoder.write_u8(7);
            encode_origin_type(encoder, instance.base())?;
            let arguments = ordered_encoded_items(instance.arguments(), |argument| {
                let mut encoder = CanonicalEncoder::new();
                encode_canonical_type(&mut encoder, argument)?;
                Ok(encoder.finish())
            })?;
            encoder.write_encoded_sequence(&arguments)?;
        }
        CanonicalTypeIdentity::GenericParameter(parameter) => {
            encoder.write_u8(8);
            encode_exported_generic_parameter_identity(encoder, parameter)?;
        }
    }
    Ok(())
}

fn encode_canonical_builtin_type(encoder: &mut CanonicalEncoder, builtin: CanonicalBuiltinType) {
    encoder.write_u8(match builtin {
        CanonicalBuiltinType::Bool => 0,
        CanonicalBuiltinType::Int => 1,
        CanonicalBuiltinType::Float => 2,
        CanonicalBuiltinType::Decimal => 3,
        CanonicalBuiltinType::String => 4,
        CanonicalBuiltinType::Char => 5,
        CanonicalBuiltinType::Range => 6,
        CanonicalBuiltinType::None => 7,
    });
}

fn encode_external_symbol_path(
    encoder: &mut CanonicalEncoder,
    path: &ExternalSymbolPath,
) -> Result<(), CompilerError> {
    let components = path.components();
    encoder.write_count(components.len())?;
    for component in components {
        encoder.write_string(component);
    }
    Ok(())
}

fn encode_folded_value(
    encoder: &mut CanonicalEncoder,
    value: &PublicFoldedValue,
) -> Result<(), CompilerError> {
    match value {
        PublicFoldedValue::Int(value) => {
            encoder.write_u8(0);
            encoder.write_i32(*value);
        }
        PublicFoldedValue::Float(value) => {
            encoder.write_u8(1);
            encode_finite_float(encoder, value);
        }
        PublicFoldedValue::Bool(value) => {
            encoder.write_u8(2);
            encoder.write_bool(*value);
        }
        PublicFoldedValue::Char(value) => {
            encoder.write_u8(3);
            encoder.write_u32(*value as u32);
        }
        PublicFoldedValue::String(value) => {
            encoder.write_u8(4);
            encoder.write_string(value);
        }
        PublicFoldedValue::Collection(values) => {
            encoder.write_u8(5);
            let values = ordered_encoded_items(values, |value| {
                let mut encoder = CanonicalEncoder::new();
                encode_folded_value(&mut encoder, value)?;
                Ok(encoder.finish())
            })?;
            encoder.write_encoded_sequence(&values)?;
        }
        PublicFoldedValue::Record(fields) => {
            encoder.write_u8(6);
            encode_folded_fields(encoder, fields)?;
        }
        PublicFoldedValue::Choice {
            type_identity,
            variant_name,
            fields,
        } => {
            encoder.write_u8(7);
            encode_canonical_type(encoder, type_identity)?;
            encoder.write_string(variant_name);
            encode_folded_fields(encoder, fields)?;
        }
        PublicFoldedValue::Range { start, end } => {
            encoder.write_u8(8);
            encode_folded_value(encoder, start)?;
            encode_folded_value(encoder, end)?;
        }
        PublicFoldedValue::OptionSome(value) => {
            encoder.write_u8(9);
            encode_folded_value(encoder, value)?;
        }
        PublicFoldedValue::OptionNone => encoder.write_u8(10),
    }
    Ok(())
}

fn encode_folded_fields(
    encoder: &mut CanonicalEncoder,
    fields: &[PublicFoldedField],
) -> Result<(), CompilerError> {
    let fields = ordered_encoded_items(fields, |field| {
        let mut encoder = CanonicalEncoder::new();
        encoder.write_string(&field.name);
        encode_folded_value(&mut encoder, &field.value)?;
        Ok(encoder.finish())
    })?;
    encoder.write_encoded_sequence(&fields)
}

fn encode_finite_float(encoder: &mut CanonicalEncoder, value: &FiniteFloat) {
    encoder.write_u64(value.normalized_bits());
}

fn encode_optional<T>(
    encoder: &mut CanonicalEncoder,
    value: Option<&T>,
    mut encode_value: impl FnMut(&mut CanonicalEncoder, &T) -> Result<(), CompilerError>,
) -> Result<(), CompilerError> {
    match value {
        Some(value) => {
            encoder.write_u8(1);
            encode_value(encoder, value)?;
        }
        None => encoder.write_u8(0),
    }
    Ok(())
}

fn encode_optional_string(encoder: &mut CanonicalEncoder, value: Option<&str>) {
    match value {
        Some(value) => {
            encoder.write_u8(1);
            encoder.write_string(value);
        }
        None => encoder.write_u8(0),
    }
}
