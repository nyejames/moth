//! Publication-time validation for completed public semantic interfaces.
//!
//! WHAT: closes a post-borrow [`LocalPublicInterface`] into one immutable
//! [`PublicSemanticInterface`] and validates the declaration, binding, receiver, evidence and
//! concrete-call-summary joins before the build system publishes the interface to another wave.
//! WHY: a successful provider slot is compiler-owned trusted input. Missing or contradictory
//! facts must fail at publication as [`InfrastructureFailure`] rather than becoming consumer source
//! diagnostics or silently disappearing during dependency projection.

use super::model::{
    PublicCallableReturnContract, PublicDeclarationSemantics, PublicEvidenceRecord,
    PublicFunctionCategory, PublicGenericParameterSurface, PublicParameterContract,
    PublicParameterTypeSlot, PublicReceiverMethodCategory, PublicReceiverMethodSemantics,
    PublicSemanticInterface, PublicTraitParameterContract, PublicTraitReceiverAccess,
    PublicTraitRequirementSurface, PublicTraitReturnContract, TraitSurfaceTypeIdentity,
};
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastTarget,
};
use crate::compiler_frontend::builtins::casts::traits::BUILTIN_CAST_TRAIT_ROWS;
use crate::compiler_frontend::builtins::error_type::builtin_error_field_type;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalCoreTraitIdentity, CanonicalEvidenceIdentity,
    CanonicalTraitIdentity, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::expressions::OPTION_SOME_VARIANT_INDEX;
use crate::compiler_frontend::public_call_summary::typed::{
    Layout, Shape, TypedCallAllocationFacts,
};
use crate::compiler_frontend::public_call_summary::{
    CallEndpoint, CallField, CallParameterPath, CallPathType, PublicCallParameterAccess,
    PublicCallSummary, validate_public_call_allocation_layout,
    validate_public_call_parameter_contract,
};
use crate::compiler_frontend::semantic_identity::{
    FunctionOriginKind, OriginDeclarationId, OriginFunctionId, OriginTypeCategory, OriginTypeId,
};
use crate::compiler_frontend::traits::environment::DISPLAYABLE_REQUIREMENT_NAME;
use rustc_hash::{FxHashMap, FxHashSet};
use std::borrow::Cow;

impl PublicSemanticInterface {
    /// Validate locally produced declaration, summary and evidence facts before provider closure.
    /// Cross-provider bindings are intentionally validated only after closure supplies them.
    /// Evidence is judged here only where its join is local. Rows that need a provider-owned
    /// declaration wait for the closed pass, because closure has not supplied it yet.
    pub(super) fn validate_closure_input(&self) -> Result<(), InfrastructureFailure> {
        self.validate_no_private_type_identities()?;

        let mut declarations_by_origin = FxHashMap::default();
        for declaration in &self.declarations {
            if declarations_by_origin
                .insert(&declaration.origin, &declaration.semantics)
                .is_some()
            {
                return Err(publication_error(format!(
                    "duplicate declaration origin {:?}",
                    declaration.origin
                )));
            }
            validate_declaration_category(&declaration.origin, &declaration.semantics)?;
        }
        self.validate_concrete_call_summaries(&declarations_by_origin, true)?;
        self.validate_reusable_evidence(&declarations_by_origin, EvidenceJoinScope::LocalOnly)
    }

    /// Validate the total closed semantic surface owned by one successful provider interface.
    pub(crate) fn validate_for_publication(&self) -> Result<(), InfrastructureFailure> {
        self.validate_no_private_type_identities()?;

        let declarations_by_origin = self.validate_declarations_and_bindings()?;
        self.validate_concrete_call_summaries(&declarations_by_origin, false)?;
        self.validate_reusable_evidence(&declarations_by_origin, EvidenceJoinScope::Closed)
    }

    /// Validate every stable binding target against the registry for this compilation boundary.
    ///
    /// Provider interfaces may cross boundaries whose registries assign different local IDs, so
    /// validation resolves the canonical package, path, origin and category instead of comparing
    /// donor-local handles. A missing target makes the successful interface untrustworthy.
    pub(crate) fn validate_binding_targets(
        &self,
        external_registry: &ExternalPackageRegistry,
    ) -> Result<(), InfrastructureFailure> {
        for binding in &self.binding_exports {
            if external_registry
                .resolve_canonical_symbol(&binding.target)
                .is_none()
            {
                return Err(publication_error(format!(
                    "binding export '{}' references unresolved canonical target {:?}",
                    binding.public_name, binding.target
                )));
            }
        }

        for declaration in &self.declarations {
            let mut invalid_identity = None;
            visit_declaration_type_identities(declaration, &mut |identity| {
                if invalid_identity.is_none()
                    && let CanonicalTypeIdentity::ExternalOpaque(external) = identity
                    && external_registry
                        .resolve_canonical_package_type_by_path(
                            external.package(),
                            external.symbol_path(),
                        )
                        .is_none()
                {
                    invalid_identity = Some(external.clone());
                }
            });
            if let Some(identity) = invalid_identity {
                return Err(publication_error(format!(
                    "public semantic surface references unresolved canonical external type {:?}",
                    identity
                )));
            }
        }
        for evidence in &self.reusable_evidence {
            let mut invalid_identity = None;
            evidence
                .identity
                .target_type_identity()
                .visit(&mut |identity| {
                    if invalid_identity.is_none()
                        && let CanonicalTypeIdentity::ExternalOpaque(external) = identity
                        && external_registry
                            .resolve_canonical_package_type_by_path(
                                external.package(),
                                external.symbol_path(),
                            )
                            .is_none()
                    {
                        invalid_identity = Some(external.clone());
                    }
                });
            if let Some(identity) = invalid_identity {
                return Err(publication_error(format!(
                    "reusable evidence references unresolved canonical external type {:?}",
                    identity
                )));
            }
        }

        Ok(())
    }

    /// Reject artefact-private nominal identities in the declaration surface.
    ///
    /// WHY: reusable evidence is not walked here. Its target is always a public `SourceNominal`:
    /// projection drops private targets, and `validate_reusable_evidence` requires that shape
    /// before closure and at publication. A pre-closure walk therefore could never find a
    /// private identity in evidence.
    fn validate_no_private_type_identities(&self) -> Result<(), InfrastructureFailure> {
        let mut invalid_identity = None;
        let mut inspect = |identity: &CanonicalTypeIdentity| {
            if invalid_identity.is_none()
                && matches!(
                    identity,
                    CanonicalTypeIdentity::ModulePrivateNominal(_)
                        | CanonicalTypeIdentity::ModulePrivateGenericInstance { .. }
                )
            {
                invalid_identity = Some(identity.clone());
            }
        };

        for declaration in &self.declarations {
            visit_declaration_type_identities(declaration, &mut inspect);
        }

        if let Some(identity) = invalid_identity {
            return Err(publication_error(format!(
                "public semantic surface references artefact-private type identity {:?}",
                identity
            )));
        }

        Ok(())
    }

    fn validate_declarations_and_bindings(
        &self,
    ) -> Result<FxHashMap<&OriginDeclarationId, &PublicDeclarationSemantics>, InfrastructureFailure>
    {
        let mut declarations_by_origin = FxHashMap::default();
        for declaration in &self.declarations {
            if declarations_by_origin
                .insert(&declaration.origin, &declaration.semantics)
                .is_some()
            {
                return Err(publication_error(format!(
                    "duplicate declaration origin {:?}",
                    declaration.origin
                )));
            }
            validate_declaration_category(&declaration.origin, &declaration.semantics)?;
        }

        let mut public_names = FxHashSet::default();
        for binding in &self.export_bindings {
            if binding.exporting_module() != &self.module_origin {
                return Err(publication_error(format!(
                    "export binding '{}' belongs to module {:?}, not publishing module {:?}",
                    binding.public_name(),
                    binding.exporting_module(),
                    self.module_origin
                )));
            }
            if !public_names.insert((binding.route(), binding.public_name())) {
                return Err(publication_error(format!(
                    "duplicate public export '{}' in route '{}'",
                    binding.public_name(),
                    binding.route().as_str()
                )));
            }
            if !declarations_by_origin.contains_key(binding.origin()) {
                return Err(publication_error(format!(
                    "export binding '{}' references missing declaration origin {:?}",
                    binding.public_name(),
                    binding.origin()
                )));
            }
        }

        for binding in &self.binding_exports {
            if binding.exporting_module != self.module_origin {
                return Err(publication_error(format!(
                    "binding export '{}' belongs to module {:?}, not publishing module {:?}",
                    binding.public_name, binding.exporting_module, self.module_origin
                )));
            }
            if !public_names.insert((&binding.route, binding.public_name.as_str())) {
                return Err(publication_error(format!(
                    "duplicate public export '{}' in route '{}'",
                    binding.public_name,
                    binding.route.as_str()
                )));
            }
            if binding.target.package.name().is_empty()
                || binding.target.symbol_path.components().is_empty()
            {
                return Err(publication_error(format!(
                    "binding export '{}' has an incomplete canonical identity",
                    binding.public_name
                )));
            }
        }

        Ok(declarations_by_origin)
    }

    fn validate_concrete_call_summaries(
        &self,
        declarations_by_origin: &FxHashMap<&OriginDeclarationId, &PublicDeclarationSemantics>,
        allow_unavailable_layouts: bool,
    ) -> Result<(), InfrastructureFailure> {
        let mut expected = FxHashMap::default();
        for (origin, semantics) in declarations_by_origin {
            collect_expected_callable_summaries(origin, semantics, &mut expected)?;
        }
        let layouts = call_path_layouts(&self.declarations);

        let mut seen = FxHashSet::default();
        for record in &self.concrete_call_summaries {
            if !seen.insert(&record.origin) {
                return Err(publication_error(format!(
                    "duplicate concrete call summary for {:?}",
                    record.origin
                )));
            }
            let Some(signature) = expected.remove(&record.origin) else {
                return Err(publication_error(format!(
                    "unexpected concrete call summary for {:?}",
                    record.origin
                )));
            };
            signature
                .validate_summary(&layouts, &record.summary, allow_unavailable_layouts)
                .map_err(|error| {
                    publication_error(format!(
                        "invalid concrete call summary for {:?}: {}",
                        record.origin, error.message
                    ))
                })?;
        }

        if let Some((origin, _)) = expected
            .into_iter()
            .min_by(|left, right| left.0.cmp(right.0))
        {
            return Err(publication_error(format!(
                "missing concrete call summary for {:?}",
                origin
            )));
        }

        Ok(())
    }

    fn validate_reusable_evidence(
        &self,
        declarations_by_origin: &FxHashMap<&OriginDeclarationId, &PublicDeclarationSemantics>,
        scope: EvidenceJoinScope,
    ) -> Result<(), InfrastructureFailure> {
        let mut identities = FxHashSet::default();

        for evidence in &self.reusable_evidence {
            if !identities.insert(&evidence.identity) {
                return Err(publication_error(format!(
                    "duplicate reusable evidence identity {:?}",
                    evidence.identity
                )));
            }

            let CanonicalTypeIdentity::SourceNominal(target_origin) =
                evidence.identity.target_type_identity()
            else {
                return Err(publication_error(format!(
                    "reusable source evidence {:?} has a non-source-nominal target",
                    evidence.identity
                )));
            };
            let target_declaration = OriginDeclarationId::Type(target_origin.clone());
            // Rows that join a provider-owned declaration wait for the closed pass.
            if scope == EvidenceJoinScope::LocalOnly
                && !evidence_joins_are_local(
                    &target_declaration,
                    evidence.identity.trait_identity(),
                    declarations_by_origin,
                )
            {
                continue;
            }
            let target_semantics = declarations_by_origin
                .get(&target_declaration)
                .copied()
                .ok_or_else(|| {
                    publication_error(format!(
                        "reusable evidence {:?} names missing target declaration {:?}",
                        evidence.identity, target_declaration
                    ))
                })?;
            let target_methods = receiver_methods(target_semantics);

            match evidence.identity.trait_identity() {
                CanonicalTraitIdentity::Source(trait_origin) => {
                    let trait_declaration = OriginDeclarationId::Trait(trait_origin.clone());
                    let trait_semantics = declarations_by_origin
                        .get(&trait_declaration)
                        .copied()
                        .and_then(|semantics| match semantics {
                            PublicDeclarationSemantics::Trait(trait_semantics) => {
                                Some(trait_semantics)
                            }
                            _ => None,
                        })
                        .ok_or_else(|| {
                            publication_error(format!(
                                "reusable evidence {:?} names missing source trait declaration {:?}",
                                evidence.identity, trait_declaration
                            ))
                        })?;

                    validate_evidence_requirement_mappings(
                        evidence,
                        &trait_semantics.requirements,
                        target_origin,
                        target_methods,
                    )?;
                }
                CanonicalTraitIdentity::Core(core_identity) => {
                    let requirement = core_trait_requirement(*core_identity)?;
                    validate_evidence_requirement_mappings(
                        evidence,
                        std::slice::from_ref(&requirement),
                        target_origin,
                        target_methods,
                    )?;
                }
                CanonicalTraitIdentity::ModulePrivate(identity) => {
                    return Err(publication_error(format!(
                        "reusable evidence exposed module-private trait identity {identity:?}"
                    )));
                }
            }
        }

        Ok(())
    }
}

/// Which reusable-evidence rows a check may judge.
///
/// Before closure only the local declarations exist. A row that joins a provider-owned
/// declaration cannot be judged yet, so the pre-closure pass leaves it to the closed pass.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EvidenceJoinScope {
    /// Judge only rows that join nothing a provider must supply.
    LocalOnly,
    /// Judge every row after closure supplies each declaration it joins.
    Closed,
}

/// Whether one evidence row joins nothing that only a provider can supply.
///
/// A row needs its target declaration and, for a source trait, the trait declaration. Closure
/// supplies provider-owned declarations, so a missing one defers the row to the closed pass.
/// Core and module-private traits carry no declaration closure could supply.
fn evidence_joins_are_local(
    target_declaration: &OriginDeclarationId,
    trait_identity: &CanonicalTraitIdentity,
    declarations_by_origin: &FxHashMap<&OriginDeclarationId, &PublicDeclarationSemantics>,
) -> bool {
    if !declarations_by_origin.contains_key(target_declaration) {
        return false;
    }
    match trait_identity {
        CanonicalTraitIdentity::Source(trait_origin) => {
            declarations_by_origin.contains_key(&OriginDeclarationId::Trait(trait_origin.clone()))
        }
        CanonicalTraitIdentity::Core(_) | CanonicalTraitIdentity::ModulePrivate(_) => true,
    }
}

fn validate_evidence_requirement_mappings(
    evidence: &PublicEvidenceRecord,
    requirements: &[PublicTraitRequirementSurface],
    target_origin: &OriginTypeId,
    target_methods: &[PublicReceiverMethodSemantics],
) -> Result<(), InfrastructureFailure> {
    if evidence.requirement_mappings.len() != requirements.len() {
        return Err(publication_error(format!(
            "reusable evidence {:?} maps {} requirement(s), but its trait declares {}",
            evidence.identity,
            evidence.requirement_mappings.len(),
            requirements.len()
        )));
    }

    let mut mapped_requirements = FxHashSet::default();
    for mapping in &evidence.requirement_mappings {
        if mapping.requirement_identity.trait_identity() != evidence.identity.trait_identity() {
            return Err(publication_error(format!(
                "reusable evidence {:?} maps requirement {:?} owned by another trait",
                evidence.identity, mapping.requirement_identity
            )));
        }

        let requirement_name = mapping.requirement_identity.requirement_name();
        if !mapped_requirements.insert(requirement_name) {
            return Err(publication_error(format!(
                "reusable evidence {:?} maps trait requirement '{}' more than once",
                evidence.identity, requirement_name
            )));
        }
        let requirement = requirements
            .iter()
            .find(|requirement| requirement.name == requirement_name)
            .ok_or_else(|| {
                publication_error(format!(
                    "reusable evidence {:?} maps unknown trait requirement '{}'",
                    evidence.identity, requirement_name
                ))
            })?;
        let method = target_methods
            .iter()
            .find(|method| method.method_origin == mapping.method_origin)
            .ok_or_else(|| {
                publication_error(format!(
                    "reusable evidence {:?} maps requirement {:?} to a method not attached to target {:?}",
                    evidence.identity, mapping.requirement_identity, target_origin
                ))
            })?;
        if method.method_origin.defining_name() != requirement_name {
            return Err(publication_error(format!(
                "reusable evidence {:?} maps requirement '{}' to differently named method '{}'",
                evidence.identity,
                requirement_name,
                method.method_origin.defining_name()
            )));
        }

        validate_evidence_method_shape(&evidence.identity, requirement, method)?;
    }

    Ok(())
}

fn core_trait_requirement(
    identity: CanonicalCoreTraitIdentity,
) -> Result<PublicTraitRequirementSurface, InfrastructureFailure> {
    let (requirement_name, success_type, error_type) = match identity {
        CanonicalCoreTraitIdentity::Displayable => (
            DISPLAYABLE_REQUIREMENT_NAME,
            CanonicalBuiltinType::String,
            None,
        ),
        CanonicalCoreTraitIdentity::Castable {
            target,
            fallibility,
        } => {
            let metadata = BUILTIN_CAST_TRAIT_ROWS
                .iter()
                .find(|metadata| metadata.target == target && metadata.fallibility == fallibility)
                .ok_or_else(|| {
                    publication_error(format!(
                        "canonical core cast trait {:?} has no registered metadata row",
                        identity
                    ))
                })?;
            let error_type = (fallibility == BuiltinCastFallibility::Fallible)
                .then_some(CanonicalBuiltinType::Error);

            (
                metadata.requirement_name,
                canonical_builtin_for_cast_target(target),
                error_type,
            )
        }
    };

    let return_contract = PublicTraitReturnContract::Returns {
        success: vec![TraitSurfaceTypeIdentity::Concrete(Box::new(
            CanonicalTypeIdentity::Builtin(success_type),
        ))],
        error: error_type.map(|error_type| {
            TraitSurfaceTypeIdentity::Concrete(Box::new(CanonicalTypeIdentity::Builtin(error_type)))
        }),
    };

    Ok(PublicTraitRequirementSurface {
        name: requirement_name.to_owned(),
        receiver_access: PublicTraitReceiverAccess::Immutable,
        parameters: Vec::new(),
        return_contract,
    })
}

fn canonical_builtin_for_cast_target(target: BuiltinCastTarget) -> CanonicalBuiltinType {
    match target {
        BuiltinCastTarget::Bool => CanonicalBuiltinType::Bool,
        BuiltinCastTarget::String => CanonicalBuiltinType::String,
        BuiltinCastTarget::Char => CanonicalBuiltinType::Char,
        BuiltinCastTarget::Error => CanonicalBuiltinType::Error,
        BuiltinCastTarget::Fixed(scalar) => CanonicalBuiltinType::FixedScalar(scalar),
        BuiltinCastTarget::Number(scale) => CanonicalBuiltinType::Number(scale),
    }
}

fn validate_evidence_method_shape(
    evidence_identity: &CanonicalEvidenceIdentity,
    requirement: &PublicTraitRequirementSurface,
    method: &PublicReceiverMethodSemantics,
) -> Result<(), InfrastructureFailure> {
    let Some(receiver) = method.parameters.first() else {
        return Err(publication_error(format!(
            "reusable evidence {:?} maps requirement '{}' to method {:?} without a receiver parameter",
            evidence_identity, requirement.name, method.method_origin
        )));
    };
    if !receiver_type_matches_evidence_target(
        &receiver.type_identity,
        evidence_identity.target_type_identity(),
    ) {
        return Err(evidence_shape_error(evidence_identity, requirement, method));
    }

    let receiver_access_matches = matches!(
        (requirement.receiver_access, receiver.access),
        (
            PublicTraitReceiverAccess::Immutable,
            PublicCallParameterAccess::Shared
        ) | (
            PublicTraitReceiverAccess::Mutable,
            PublicCallParameterAccess::Mutable
        )
    );
    if !receiver_access_matches {
        return Err(evidence_shape_error(evidence_identity, requirement, method));
    }

    let method_parameters = &method.parameters[1..];
    if method_parameters.len() != requirement.parameters.len() {
        return Err(evidence_shape_error(evidence_identity, requirement, method));
    }
    for (required, actual) in requirement.parameters.iter().zip(method_parameters) {
        let access_matches = match actual.access {
            PublicCallParameterAccess::Shared => !required.value_mode.is_mutable(),
            PublicCallParameterAccess::Mutable => required.value_mode.is_mutable(),
        };
        if !access_matches
            || !trait_parameter_contract_matches(
                &required.contract,
                &receiver.type_identity,
                &actual.contract,
            )
            || !trait_surface_type_matches(
                &required.type_identity,
                &receiver.type_identity,
                &actual.type_identity,
            )
        {
            return Err(evidence_shape_error(evidence_identity, requirement, method));
        }
    }

    let contracts_match = match (&requirement.return_contract, &method.return_contract) {
        (PublicTraitReturnContract::Never, PublicCallableReturnContract::Never) => true,
        (
            PublicTraitReturnContract::Returns {
                success: required_success,
                error: required_error,
            },
            PublicCallableReturnContract::Returns {
                success: method_success,
                error: method_error,
            },
        ) => {
            if required_success.len() != method_success.len() {
                return Err(evidence_shape_error(evidence_identity, requirement, method));
            }
            for (required, actual) in required_success.iter().zip(method_success) {
                if !trait_surface_type_matches(
                    required,
                    &receiver.type_identity,
                    &actual.type_identity,
                ) {
                    return Err(evidence_shape_error(evidence_identity, requirement, method));
                }
            }

            match (required_error, method_error) {
                (None, None) => true,
                (Some(required), Some(actual)) => {
                    trait_surface_type_matches(required, &receiver.type_identity, actual)
                }
                _ => false,
            }
        }
        (PublicTraitReturnContract::Returns { .. }, PublicCallableReturnContract::Never)
        | (PublicTraitReturnContract::Never, PublicCallableReturnContract::Returns { .. }) => false,
    };
    if !contracts_match {
        return Err(evidence_shape_error(evidence_identity, requirement, method));
    }

    Ok(())
}

fn receiver_type_matches_evidence_target(
    receiver: &CanonicalTypeIdentity,
    target: &CanonicalTypeIdentity,
) -> bool {
    if receiver == target {
        return true;
    }

    matches!(
        (receiver, target),
        (
            CanonicalTypeIdentity::GenericInstance { base, .. },
            CanonicalTypeIdentity::SourceNominal(target_origin),
        ) if base == target_origin
    )
}

fn trait_surface_type_matches(
    required: &TraitSurfaceTypeIdentity,
    receiver: &CanonicalTypeIdentity,
    actual: &CanonicalTypeIdentity,
) -> bool {
    match required {
        TraitSurfaceTypeIdentity::SelfType => actual == receiver,
        TraitSurfaceTypeIdentity::Concrete(required) => actual == required.as_ref(),
    }
}
fn trait_parameter_contract_matches(
    required: &PublicTraitParameterContract,
    receiver: &CanonicalTypeIdentity,
    actual: &PublicParameterContract,
) -> bool {
    match (required, actual) {
        (PublicTraitParameterContract::Value, PublicParameterContract::Value)
        | (PublicTraitParameterContract::Wire, PublicParameterContract::Wire) => true,
        (
            PublicTraitParameterContract::Route { input: required },
            PublicParameterContract::Route { input: actual },
        ) => match (required, actual) {
            (None, None) => true,
            (Some(TraitSurfaceTypeIdentity::SelfType), Some(actual)) => actual == receiver,
            (Some(TraitSurfaceTypeIdentity::Concrete(required)), Some(actual)) => {
                required.as_ref() == actual
            }
            _ => false,
        },
        _ => false,
    }
}

fn evidence_shape_error(
    evidence_identity: &CanonicalEvidenceIdentity,
    requirement: &PublicTraitRequirementSurface,
    method: &PublicReceiverMethodSemantics,
) -> InfrastructureFailure {
    publication_error(format!(
        "reusable evidence {:?} maps requirement '{}' to incompatible receiver method {:?}",
        evidence_identity, requirement.name, method.method_origin
    ))
}

fn validate_declaration_category(
    origin: &OriginDeclarationId,
    semantics: &PublicDeclarationSemantics,
) -> Result<(), InfrastructureFailure> {
    let matches = matches!(
        (origin, semantics),
        (
            OriginDeclarationId::Function(_),
            PublicDeclarationSemantics::Function(_)
        ) | (
            OriginDeclarationId::Type(_),
            PublicDeclarationSemantics::Struct(_)
                | PublicDeclarationSemantics::Choice(_)
                | PublicDeclarationSemantics::TransparentAlias(_)
        ) | (
            OriginDeclarationId::Constant(_),
            PublicDeclarationSemantics::Constant(_)
        ) | (
            OriginDeclarationId::Trait(_),
            PublicDeclarationSemantics::Trait(_)
        )
    );
    if !matches {
        return Err(publication_error(format!(
            "declaration origin {:?} disagrees with semantic category {:?}",
            origin, semantics
        )));
    }

    if let (OriginDeclarationId::Type(origin), semantics) = (origin, semantics) {
        let category_matches = matches!(
            (origin.category(), semantics),
            (
                OriginTypeCategory::Struct,
                PublicDeclarationSemantics::Struct(_)
            ) | (
                OriginTypeCategory::Choice,
                PublicDeclarationSemantics::Choice(_)
            ) | (
                OriginTypeCategory::TransparentAlias,
                PublicDeclarationSemantics::TransparentAlias(_)
            )
        );
        if !category_matches {
            return Err(publication_error(format!(
                "type origin {:?} disagrees with semantic category {:?}",
                origin, semantics
            )));
        }
    }

    Ok(())
}

fn collect_expected_callable_summaries<'a>(
    origin: &'a OriginDeclarationId,
    semantics: &'a PublicDeclarationSemantics,
    expected: &mut FxHashMap<&'a OriginFunctionId, CallSignature<'a>>,
) -> Result<(), InfrastructureFailure> {
    match semantics {
        PublicDeclarationSemantics::Function(function) => {
            let OriginDeclarationId::Function(origin) = origin else {
                crate::compiler_bug!(
                    CompilerStage::AstConstruction,
                    "declaration category was validated before callable collection"
                );
            };
            if !matches!(origin.kind(), FunctionOriginKind::Free) {
                return Err(publication_error(format!(
                    "free-function record uses receiver origin {:?}",
                    origin
                )));
            }
            // Const callables carry their evaluation facts in the const executable contract.
            if !function.evaluation.has_runtime_body() {
                return Ok(());
            }
            if matches!(function.category, PublicFunctionCategory::ConcreteLocal) {
                insert_expected_summary(
                    origin,
                    CallSignature {
                        parameters: &function.parameters,
                        returns: &function.return_contract,
                    },
                    expected,
                )?;
            }
        }
        PublicDeclarationSemantics::Struct(structure) => {
            let OriginDeclarationId::Type(receiver_origin) = origin else {
                crate::compiler_bug!(
                    CompilerStage::AstConstruction,
                    "declaration category was validated before callable collection"
                );
            };
            collect_expected_receiver_summaries(
                receiver_origin,
                &structure.receiver_methods,
                expected,
            )?;
        }
        PublicDeclarationSemantics::Choice(choice) => {
            let OriginDeclarationId::Type(receiver_origin) = origin else {
                crate::compiler_bug!(
                    CompilerStage::AstConstruction,
                    "declaration category was validated before callable collection"
                );
            };
            collect_expected_receiver_summaries(
                receiver_origin,
                &choice.receiver_methods,
                expected,
            )?;
        }
        PublicDeclarationSemantics::TransparentAlias(_)
        | PublicDeclarationSemantics::Constant(_)
        | PublicDeclarationSemantics::Trait(_) => {}
    }
    Ok(())
}

fn collect_expected_receiver_summaries<'a>(
    receiver_origin: &OriginTypeId,
    methods: &'a [PublicReceiverMethodSemantics],
    expected: &mut FxHashMap<&'a OriginFunctionId, CallSignature<'a>>,
) -> Result<(), InfrastructureFailure> {
    for method in methods {
        if method.method_origin.receiver() != Some(receiver_origin) {
            return Err(publication_error(format!(
                "receiver method {:?} is attached to incompatible receiver {:?}",
                method.method_origin, receiver_origin
            )));
        }
        if matches!(method.category, PublicReceiverMethodCategory::ConcreteLocal) {
            insert_expected_summary(
                &method.method_origin,
                CallSignature {
                    parameters: &method.parameters,
                    returns: &method.return_contract,
                },
                expected,
            )?;
        }
    }
    Ok(())
}

fn insert_expected_summary<'a>(
    origin: &'a OriginFunctionId,
    signature: CallSignature<'a>,
    expected: &mut FxHashMap<&'a OriginFunctionId, CallSignature<'a>>,
) -> Result<(), InfrastructureFailure> {
    if expected.insert(origin, signature).is_some() {
        return Err(publication_error(format!(
            "duplicate concrete callable origin {:?}",
            origin
        )));
    }
    Ok(())
}

type CallPathLayouts<'a> = FxHashMap<&'a OriginTypeId, &'a PublicDeclarationSemantics>;

fn call_path_layouts(
    declarations: &[super::model::PublicDeclarationRecord],
) -> CallPathLayouts<'_> {
    declarations
        .iter()
        .filter_map(|declaration| match &declaration.origin {
            OriginDeclarationId::Type(origin) => Some((origin, &declaration.semantics)),
            _ => None,
        })
        .collect()
}

struct CallSignature<'a> {
    parameters: &'a [PublicParameterTypeSlot],
    returns: &'a PublicCallableReturnContract,
}

#[allow(dead_code)]
struct TypedSignatureLayout<'a, 'decl, F> {
    signature: &'a CallSignature<'decl>,
    layouts: &'a CallPathLayouts<'decl>,
    builtin_error: bool,
    private_layout: F,
}

type StableChildren<'a> = &'a [(CallField, CanonicalTypeIdentity)];

impl<'a, 'decl, F> Layout for TypedSignatureLayout<'a, 'decl, F>
where
    F: Fn(&CanonicalTypeIdentity) -> Option<StableChildren<'decl>>,
{
    type Type = CanonicalTypeIdentity;

    fn parameter(&self, index: usize) -> Option<CallEndpoint<Self::Type>> {
        self.stable(&self.signature.parameters.get(index)?.type_identity)
    }

    fn result(&self, error_lane: bool, slot: usize) -> Option<CallEndpoint<Self::Type>> {
        let PublicCallableReturnContract::Returns { success, error } = self.signature.returns
        else {
            return None;
        };
        if error_lane {
            (slot == 0).then_some(())?;
            if let Some(error) = error {
                self.stable(error)
            } else {
                self.builtin_error.then(|| {
                    known_endpoint(CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error))
                })
            }
        } else {
            self.stable(&success.get(slot)?.type_identity)
        }
    }

    fn child(&self, ty: &Self::Type, selector: CallField) -> Option<CallEndpoint<Self::Type>> {
        if matches!(
            ty,
            CanonicalTypeIdentity::ModulePrivateNominal(_)
                | CanonicalTypeIdentity::ModulePrivateGenericInstance { .. }
        ) {
            let rows = (self.private_layout)(ty)?;
            return self.stable(&rows.iter().find(|(field, _)| *field == selector)?.1);
        }
        match canonical_path_type(ty, &mut std::iter::once(&selector), self.layouts, None)? {
            CallPathType::Known(child) => self.stable(&child),
            CallPathType::Unavailable => None,
        }
    }

    fn stable(&self, ty: &Self::Type) -> Option<CallEndpoint<Self::Type>> {
        if matches!(
            ty,
            CanonicalTypeIdentity::ModulePrivateNominal(_)
                | CanonicalTypeIdentity::ModulePrivateGenericInstance { .. }
        ) {
            (self.private_layout)(ty)?;
        }
        Some(known_endpoint(ty.clone()))
    }

    fn children(&self, ty: &Self::Type, visit: &mut dyn FnMut(CallEndpoint<Self::Type>)) {
        let mut emit = |selector| {
            if let Some(child) = self.child(ty, selector) {
                visit(child);
            }
        };
        match ty {
            CanonicalTypeIdentity::Collection { .. } => emit(CallField::Elem),
            CanonicalTypeIdentity::OrderedMap { .. } => {
                emit(CallField::MapKey);
                emit(CallField::Elem);
            }
            CanonicalTypeIdentity::Option(_) => emit(CallField::Payload {
                variant: OPTION_SOME_VARIANT_INDEX as u32,
                field: 0,
            }),
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error) => {
                for ordinal in 0.. {
                    if builtin_error_field_type(ordinal).is_none() {
                        break;
                    }
                    emit(CallField::Field(ordinal));
                }
            }
            CanonicalTypeIdentity::SourceNominal(origin)
            | CanonicalTypeIdentity::GenericInstance { base: origin, .. } => {
                match self.layouts.get(origin) {
                    Some(PublicDeclarationSemantics::Struct(structure)) => {
                        for ordinal in 0..structure.fields.len() {
                            emit(CallField::Field(ordinal as u32));
                        }
                    }
                    Some(PublicDeclarationSemantics::Choice(choice)) => {
                        for (variant, layout) in choice.variants.iter().enumerate() {
                            for field in 0..layout.payload_fields.len() {
                                emit(CallField::Payload {
                                    variant: variant as u32,
                                    field: field as u32,
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
            CanonicalTypeIdentity::ModulePrivateNominal(_)
            | CanonicalTypeIdentity::ModulePrivateGenericInstance { .. } => {
                if let Some(rows) = (self.private_layout)(ty) {
                    for (selector, _) in rows {
                        emit(*selector);
                    }
                }
            }
            _ => {}
        }
    }
}

fn known_endpoint(ty: CanonicalTypeIdentity) -> CallEndpoint<CanonicalTypeIdentity> {
    CallEndpoint {
        allocation: has_canonical_allocation_surface(&ty),
        ty,
    }
}

#[cfg(test)]
#[test]
fn typed_signature_resolves_a_private_field_only_with_its_stable_layout() {
    use crate::compiler_frontend::canonical_type_identity::ModulePrivateNominalIdentity;
    use crate::compiler_frontend::semantic_identity::{
        ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
    };
    let private = CanonicalTypeIdentity::ModulePrivateNominal(ModulePrivateNominalIdentity::new(
        StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("typed-layout"),
            "entry".to_owned(),
            ModuleRootRole::Normal,
        ),
        "InternalTerminal".to_owned(),
        OriginTypeCategory::Struct,
    ));
    let signature = CallSignature {
        parameters: &[],
        returns: &PublicCallableReturnContract::Never,
    };
    let layouts = CallPathLayouts::default();
    let ty = CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Bool);
    let rows = [(CallField::Field(0), ty.clone())];
    let with_private = TypedSignatureLayout {
        signature: &signature,
        layouts: &layouts,
        builtin_error: false,
        private_layout: |identity: &CanonicalTypeIdentity| {
            (identity == &private).then_some(rows.as_slice())
        },
    };
    let without_private = TypedSignatureLayout {
        signature: &signature,
        layouts: &layouts,
        builtin_error: false,
        private_layout: |_: &CanonicalTypeIdentity| None,
    };
    assert!(
        without_private
            .child(&private, CallField::Field(0))
            .is_none()
    );
    let field = with_private.child(&private, CallField::Field(0)).unwrap();
    assert_eq!(field.ty, ty);
    assert!(!field.allocation);
    assert!(with_private.child(&private, CallField::Elem).is_none());
}

impl CallSignature<'_> {
    #[allow(dead_code)]
    fn validate_typed_allocation<'decl>(
        &'decl self,
        layouts: &CallPathLayouts<'decl>,
        facts: &TypedCallAllocationFacts,
        shape: &Shape<'_>,
        builtin_error: bool,
        private_layout: impl Fn(&CanonicalTypeIdentity) -> Option<StableChildren<'decl>>,
    ) -> Result<(), InfrastructureFailure> {
        let (success_slots, error_slot) = match self.returns {
            PublicCallableReturnContract::Never => (None, false),
            PublicCallableReturnContract::Returns { success, error } => {
                (Some(success.len()), error.is_some() || builtin_error)
            }
        };
        if !shape
            .parameters
            .iter()
            .copied()
            .eq(self.parameters.iter().map(|slot| slot.access))
            || shape.success_slots != success_slots
            || shape.error_slot != error_slot
        {
            return Err(publication_error(
                "typed call shape disagrees with its declaration".to_owned(),
            ));
        }
        facts.validate_layout(
            shape,
            &TypedSignatureLayout {
                signature: self,
                layouts,
                builtin_error,
                private_layout,
            },
        )
    }

    fn validate_summary(
        &self,
        layouts: &CallPathLayouts<'_>,
        summary: &PublicCallSummary,
        allow_unavailable_layouts: bool,
    ) -> Result<(), InfrastructureFailure> {
        validate_public_call_parameter_contract(
            self.parameters.iter().map(|parameter| parameter.access),
            summary,
        )?;
        let builtin_error = CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error);
        let (success, error) = match self.returns {
            PublicCallableReturnContract::Never => (None, None),
            PublicCallableReturnContract::Returns { success, error } => (
                Some(success.as_slice()),
                error
                    .as_ref()
                    .or(summary.escapes_builtin_failure.then_some(&builtin_error)),
            ),
        };
        let parameter_type = |path: &CallParameterPath| {
            self.parameters
                .get(path.index)
                .map(|slot| &slot.type_identity)
        };
        validate_public_call_allocation_layout(
            summary,
            success.map(|slots| slots.len()),
            error.is_some(),
            |path| {
                signature_path_type(
                    parameter_type(path)?,
                    &mut path.path.iter(),
                    layouts,
                    allow_unavailable_layouts,
                )
            },
            |error_lane, path| {
                let root = if error_lane {
                    error?
                } else {
                    &success?.get(path.slot)?.type_identity
                };
                signature_path_type(
                    root,
                    &mut path.path.iter(),
                    layouts,
                    allow_unavailable_layouts,
                )
            },
            |target, path| {
                signature_path_type(
                    parameter_type(target)?,
                    &mut target.path.iter().chain(path),
                    layouts,
                    allow_unavailable_layouts,
                )
            },
        )
    }
}

fn signature_path_type<'a, 'path>(
    root: &'a CanonicalTypeIdentity,
    path: &mut impl Iterator<Item = &'path CallField>,
    layouts: &CallPathLayouts<'a>,
    allow_unavailable_layouts: bool,
) -> Option<CallPathType<Cow<'a, CanonicalTypeIdentity>>> {
    canonical_path_type(root, path, layouts, None).filter(|path| match path {
        CallPathType::Known(ty) => has_canonical_allocation_surface(ty),
        CallPathType::Unavailable => allow_unavailable_layouts,
    })
}

/// A generic instance's arguments were authored in the enclosing layout's parameter scope.
/// Keep that scope when following a selected field's parameter back to its argument.
struct LayoutArguments<'types, 'frame> {
    parameters: &'types [PublicGenericParameterSurface],
    arguments: &'types [CanonicalTypeIdentity],
    outer: Option<&'frame LayoutArguments<'types, 'frame>>,
}

fn canonical_path_type<'a, 'path>(
    ty: &'a CanonicalTypeIdentity,
    path: &mut impl Iterator<Item = &'path CallField>,
    layouts: &CallPathLayouts<'a>,
    arguments: Option<&LayoutArguments<'a, '_>>,
) -> Option<CallPathType<Cow<'a, CanonicalTypeIdentity>>> {
    let (ty, arguments) = resolve_layout_argument(ty, arguments)?;
    let Some(&selector) = path.next() else {
        return instantiate_path_type(ty, arguments).map(CallPathType::Known);
    };

    match (ty, selector) {
        (CanonicalTypeIdentity::Option(inner), CallField::Payload { variant, field: 0 })
            if variant as usize == OPTION_SOME_VARIANT_INDEX =>
        {
            canonical_path_type(inner, path, layouts, arguments)
        }
        (CanonicalTypeIdentity::Collection { element, .. }, CallField::Elem) => {
            canonical_path_type(element, path, layouts, arguments)
        }
        (CanonicalTypeIdentity::OrderedMap { value, .. }, CallField::Elem)
        | (CanonicalTypeIdentity::OrderedMap { key: value, .. }, CallField::MapKey) => {
            canonical_path_type(value, path, layouts, arguments)
        }
        (
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error),
            CallField::Field(ordinal),
        ) => {
            let field = builtin_error_field_type(ordinal)?;
            // The manifest's field types are builtins with no allocation-bearing children.
            path.next()
                .is_none()
                .then_some(CallPathType::Known(Cow::Owned(field)))
        }
        (CanonicalTypeIdentity::SourceNominal(origin), _) => {
            nominal_path_type(origin, &[], selector, path, layouts, arguments)
        }
        (
            CanonicalTypeIdentity::GenericInstance {
                base,
                arguments: instance,
            },
            _,
        ) => nominal_path_type(base, instance, selector, path, layouts, arguments),
        _ => None,
    }
}

fn nominal_path_type<'a, 'path>(
    origin: &OriginTypeId,
    instance: &'a [CanonicalTypeIdentity],
    selector: CallField,
    path: &mut impl Iterator<Item = &'path CallField>,
    layouts: &CallPathLayouts<'a>,
    arguments: Option<&LayoutArguments<'a, '_>>,
) -> Option<CallPathType<Cow<'a, CanonicalTypeIdentity>>> {
    let kind_matches = matches!(
        (origin.category(), selector),
        (OriginTypeCategory::Struct, CallField::Field(_))
            | (OriginTypeCategory::Choice, CallField::Payload { .. })
    );
    if !kind_matches {
        return None;
    }
    let Some(declaration) = layouts.get(origin).copied() else {
        return Some(CallPathType::Unavailable);
    };
    let (child, parameters) = match (declaration, selector) {
        (PublicDeclarationSemantics::Struct(structure), CallField::Field(ordinal)) => (
            &structure.fields.get(ordinal as usize)?.type_identity,
            structure.generic_parameters.as_slice(),
        ),
        (PublicDeclarationSemantics::Choice(choice), CallField::Payload { variant, field }) => (
            &choice
                .variants
                .get(variant as usize)?
                .payload_fields
                .get(field as usize)?
                .type_identity,
            choice.generic_parameters.as_slice(),
        ),
        _ => return None,
    };
    if parameters.len() != instance.len() {
        return None;
    }
    if parameters.is_empty() {
        return canonical_path_type(child, path, layouts, arguments);
    }
    let arguments = LayoutArguments {
        parameters,
        arguments: instance,
        outer: arguments,
    };
    canonical_path_type(child, path, layouts, Some(&arguments))
}

fn resolve_layout_argument<'types, 'frame>(
    mut ty: &'types CanonicalTypeIdentity,
    mut arguments: Option<&'frame LayoutArguments<'types, 'frame>>,
) -> Option<(
    &'types CanonicalTypeIdentity,
    Option<&'frame LayoutArguments<'types, 'frame>>,
)> {
    while let CanonicalTypeIdentity::GenericParameter(identity) = ty {
        let mut frame = arguments;
        loop {
            let bindings = frame?;
            if let Some(index) = bindings
                .parameters
                .iter()
                .position(|parameter| &parameter.identity == identity)
            {
                ty = bindings.arguments.get(index)?;
                arguments = bindings.outer;
                break;
            }
            frame = bindings.outer;
        }
    }
    Some((ty, arguments))
}

/// Materialize only the compound portions that actually contain a substituted parameter.
fn instantiate_path_type<'a>(
    ty: &'a CanonicalTypeIdentity,
    arguments: Option<&LayoutArguments<'a, '_>>,
) -> Option<Cow<'a, CanonicalTypeIdentity>> {
    use CanonicalTypeIdentity::{Collection, GenericInstance, Option as OptionType, OrderedMap};

    let (ty, arguments) = resolve_layout_argument(ty, arguments)?;
    let result = match ty {
        OptionType(inner) => {
            let resolved = instantiate_path_type(inner, arguments)?;
            if std::ptr::eq(resolved.as_ref(), inner.as_ref()) {
                return Some(Cow::Borrowed(ty));
            }
            OptionType(Box::new(resolved.into_owned()))
        }
        Collection {
            element,
            fixed_capacity,
        } => {
            let resolved = instantiate_path_type(element, arguments)?;
            if std::ptr::eq(resolved.as_ref(), element.as_ref()) {
                return Some(Cow::Borrowed(ty));
            }
            Collection {
                element: Box::new(resolved.into_owned()),
                fixed_capacity: *fixed_capacity,
            }
        }
        OrderedMap { key, value } => {
            let resolved_key = instantiate_path_type(key, arguments)?;
            let resolved_value = instantiate_path_type(value, arguments)?;
            if std::ptr::eq(resolved_key.as_ref(), key.as_ref())
                && std::ptr::eq(resolved_value.as_ref(), value.as_ref())
            {
                return Some(Cow::Borrowed(ty));
            }
            OrderedMap {
                key: Box::new(resolved_key.into_owned()),
                value: Box::new(resolved_value.into_owned()),
            }
        }
        GenericInstance {
            base,
            arguments: instance,
        } => {
            let mut resolved: Option<Vec<CanonicalTypeIdentity>> = None;
            for (index, original) in instance.iter().enumerate() {
                let argument = instantiate_path_type(original, arguments)?;
                if let Some(output) = &mut resolved {
                    output.push(argument.into_owned());
                } else if !std::ptr::eq(argument.as_ref(), original) {
                    let mut output = Vec::with_capacity(instance.len());
                    output.extend_from_slice(&instance[..index]);
                    output.push(argument.into_owned());
                    resolved = Some(output);
                }
            }
            let Some(resolved) = resolved else {
                return Some(Cow::Borrowed(ty));
            };
            GenericInstance {
                base: base.clone(),
                arguments: resolved.into_boxed_slice(),
            }
        }
        _ => return Some(Cow::Borrowed(ty)),
    };
    Some(Cow::Owned(result))
}

/// Canonical counterpart of HIR's `has_in_place_mutation_surface`. The builtin Error
/// identity denotes the compiler-owned choice, not a scalar.
fn has_canonical_allocation_surface(ty: &CanonicalTypeIdentity) -> bool {
    match ty {
        CanonicalTypeIdentity::Builtin(builtin) => match builtin {
            CanonicalBuiltinType::Bool
            | CanonicalBuiltinType::Char
            | CanonicalBuiltinType::None
            | CanonicalBuiltinType::FixedScalar(_)
            | CanonicalBuiltinType::Number(_) => false,
            CanonicalBuiltinType::String
            | CanonicalBuiltinType::Range
            | CanonicalBuiltinType::Error => true,
        },
        CanonicalTypeIdentity::AnonymousConstRecord
        | CanonicalTypeIdentity::GenericParameter(_) => false,
        CanonicalTypeIdentity::SourceNominal(_)
        | CanonicalTypeIdentity::ModulePrivateNominal(_)
        | CanonicalTypeIdentity::ExternalOpaque(_)
        | CanonicalTypeIdentity::Collection { .. }
        | CanonicalTypeIdentity::OrderedMap { .. }
        | CanonicalTypeIdentity::Option(_)
        | CanonicalTypeIdentity::GenericInstance { .. }
        | CanonicalTypeIdentity::ModulePrivateGenericInstance { .. } => true,
    }
}

fn receiver_methods(
    semantics: &PublicDeclarationSemantics,
) -> &[super::model::PublicReceiverMethodSemantics] {
    match semantics {
        PublicDeclarationSemantics::Struct(structure) => &structure.receiver_methods,
        PublicDeclarationSemantics::Choice(choice) => &choice.receiver_methods,
        _ => &[],
    }
}

fn visit_declaration_type_identities(
    declaration: &super::model::PublicDeclarationRecord,
    visitor: &mut impl FnMut(&CanonicalTypeIdentity),
) {
    match &declaration.semantics {
        PublicDeclarationSemantics::Function(function) => {
            visit_callable_type_identities(
                &function.parameters,
                &function.return_contract,
                visitor,
            );
        }
        PublicDeclarationSemantics::Struct(structure) => {
            for field in &structure.fields {
                field.type_identity.visit(visitor);
                if let Some(default) = &field.folded_default {
                    default.visit_type_identities(visitor);
                }
            }
            for method in &structure.receiver_methods {
                visit_callable_type_identities(
                    &method.parameters,
                    &method.return_contract,
                    visitor,
                );
            }
        }
        PublicDeclarationSemantics::Choice(choice) => {
            for variant in &choice.variants {
                for field in &variant.payload_fields {
                    field.type_identity.visit(visitor);
                }
            }
            for method in &choice.receiver_methods {
                visit_callable_type_identities(
                    &method.parameters,
                    &method.return_contract,
                    visitor,
                );
            }
        }
        PublicDeclarationSemantics::TransparentAlias(alias) => {
            alias.target_type_identity.visit(visitor);
        }
        PublicDeclarationSemantics::Constant(constant) => {
            constant.type_identity.visit(visitor);
            constant.folded_value.visit_type_identities(visitor);
        }
        PublicDeclarationSemantics::Trait(trait_semantics) => {
            for identity in trait_semantics.concrete_type_identities() {
                identity.visit(visitor);
            }
        }
    }
}

fn visit_callable_type_identities(
    parameters: &[super::model::PublicParameterTypeSlot],
    return_contract: &super::model::PublicCallableReturnContract,
    visitor: &mut impl FnMut(&CanonicalTypeIdentity),
) {
    for parameter in parameters {
        for identity in parameter.semantic_type_identities() {
            identity.visit(visitor);
        }
        if let Some(default) = &parameter.folded_default {
            default.visit_type_identities(visitor);
        }
    }
    let PublicCallableReturnContract::Returns { success, error } = return_contract else {
        return;
    };
    for returned in success {
        returned.type_identity.visit(visitor);
    }
    if let Some(error_return) = error {
        error_return.visit(visitor);
    }
}

fn publication_error(message: String) -> InfrastructureFailure {
    InfrastructureFailure::internal(
        CompilerStage::AstConstruction,
        format!("public semantic interface publication invariant failed: {message}"),
    )
}
