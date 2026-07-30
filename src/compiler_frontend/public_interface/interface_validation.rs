//! Publication-time validation for completed public semantic interfaces.
//!
//! WHAT: closes a post-borrow [`LocalPublicInterface`] into one immutable
//! [`PublicSemanticInterface`] and validates the declaration, binding, receiver, evidence and
//! concrete-call-summary joins before the build system publishes the interface to another wave.
//! WHY: a successful provider slot is compiler-owned trusted input. Missing or contradictory
//! facts must fail at publication as [`CompilerError`] rather than becoming consumer source
//! diagnostics or silently disappearing during import projection.

use super::model::{
    PublicDeclarationSemantics, PublicEvidenceRecord, PublicFunctionCategory,
    PublicReceiverMethodCategory, PublicReceiverMethodSemantics, PublicSemanticInterface,
    PublicTraitReceiverAccess, PublicTraitRequirementReturn, PublicTraitRequirementSurface,
    TraitSurfaceTypeIdentity,
};
use crate::compiler_frontend::ast::statements::functions::ReturnChannel;
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastTarget,
};
use crate::compiler_frontend::builtins::casts::traits::BUILTIN_CAST_TRAIT_ROWS;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalCoreTraitIdentity, CanonicalTraitIdentity, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::public_call_summary::validate_public_call_summary;
use crate::compiler_frontend::semantic_identity::{
    FunctionOriginKind, OriginDeclarationId, OriginFunctionId, OriginTypeCategory, OriginTypeId,
};
use crate::compiler_frontend::traits::environment::DISPLAYABLE_REQUIREMENT_NAME;
use rustc_hash::{FxHashMap, FxHashSet};

impl PublicSemanticInterface {
    /// Validate locally produced declaration, summary and evidence facts before provider closure.
    /// Cross-provider bindings are intentionally validated only after closure supplies them.
    pub(super) fn validate_closure_input(&self) -> Result<(), CompilerError> {
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
        self.validate_concrete_call_summaries(&declarations_by_origin)?;
        self.validate_reusable_evidence(&declarations_by_origin)
    }

    /// Validate the total closed semantic surface owned by one successful provider interface.
    pub(crate) fn validate_for_publication(&self) -> Result<(), CompilerError> {
        self.validate_no_private_type_identities()?;

        let declarations_by_origin = self.validate_declarations_and_bindings()?;
        self.validate_concrete_call_summaries(&declarations_by_origin)?;
        self.validate_reusable_evidence(&declarations_by_origin)
    }

    /// Validate every stable binding target against the registry for this compilation boundary.
    ///
    /// Provider interfaces may cross boundaries whose registries assign different local IDs, so
    /// validation resolves the canonical package, path, origin and category instead of comparing
    /// donor-local handles. A missing target makes the successful interface untrustworthy.
    pub(crate) fn validate_binding_targets(
        &self,
        external_registry: &ExternalPackageRegistry,
    ) -> Result<(), CompilerError> {
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

    fn validate_no_private_type_identities(&self) -> Result<(), CompilerError> {
        let mut invalid_identity = None;
        let mut inspect = |identity: &CanonicalTypeIdentity| {
            if invalid_identity.is_none()
                && matches!(
                    identity,
                    CanonicalTypeIdentity::ModulePrivateNominal(_)
                        | CanonicalTypeIdentity::ModulePrivateGenericInstance(_)
                )
            {
                invalid_identity = Some(identity.clone());
            }
        };

        for declaration in &self.declarations {
            visit_declaration_type_identities(declaration, &mut inspect);
        }
        for evidence in &self.reusable_evidence {
            evidence.identity.target_type_identity().visit(&mut inspect);
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
    ) -> Result<FxHashMap<&OriginDeclarationId, &PublicDeclarationSemantics>, CompilerError> {
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
            if !public_names.insert(binding.public_name()) {
                return Err(publication_error(format!(
                    "duplicate public export name '{}'",
                    binding.public_name()
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
            if !public_names.insert(binding.public_name.as_str()) {
                return Err(publication_error(format!(
                    "duplicate public export name '{}'",
                    binding.public_name
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
    ) -> Result<(), CompilerError> {
        let mut expected = FxHashMap::default();
        for (origin, semantics) in declarations_by_origin {
            collect_expected_callable_summaries(origin, semantics, &mut expected)?;
        }

        let mut seen = FxHashSet::default();
        for record in &self.concrete_call_summaries {
            if !seen.insert(&record.origin) {
                return Err(publication_error(format!(
                    "duplicate concrete call summary for {:?}",
                    record.origin
                )));
            }
            let Some(parameter_access) = expected.remove(&record.origin) else {
                return Err(publication_error(format!(
                    "unexpected concrete call summary for {:?}",
                    record.origin
                )));
            };
            validate_public_call_summary(&parameter_access, &record.summary).map_err(|error| {
                publication_error(format!(
                    "invalid concrete call summary for {:?}: {}",
                    record.origin, error.msg
                ))
            })?;
        }

        if let Some((origin, _)) = expected
            .into_iter()
            .min_by(|left, right| left.0.cmp(&right.0))
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
    ) -> Result<(), CompilerError> {
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

fn validate_evidence_requirement_mappings(
    evidence: &PublicEvidenceRecord,
    requirements: &[PublicTraitRequirementSurface],
    target_origin: &OriginTypeId,
    target_methods: &[PublicReceiverMethodSemantics],
) -> Result<(), CompilerError> {
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
) -> Result<PublicTraitRequirementSurface, CompilerError> {
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

    let mut returns = vec![PublicTraitRequirementReturn {
        channel: ReturnChannel::Success,
        type_identity: TraitSurfaceTypeIdentity::Concrete(Box::new(
            CanonicalTypeIdentity::Builtin(success_type),
        )),
    }];
    if let Some(error_type) = error_type {
        returns.push(PublicTraitRequirementReturn {
            channel: ReturnChannel::Error,
            type_identity: TraitSurfaceTypeIdentity::Concrete(Box::new(
                CanonicalTypeIdentity::Builtin(error_type),
            )),
        });
    }

    Ok(PublicTraitRequirementSurface {
        name: requirement_name.to_owned(),
        receiver_access: PublicTraitReceiverAccess::Immutable,
        parameters: Vec::new(),
        returns,
    })
}

fn canonical_builtin_for_cast_target(target: BuiltinCastTarget) -> CanonicalBuiltinType {
    match target {
        BuiltinCastTarget::Bool => CanonicalBuiltinType::Bool,
        BuiltinCastTarget::Int => CanonicalBuiltinType::Int,
        BuiltinCastTarget::String => CanonicalBuiltinType::String,
        BuiltinCastTarget::Char => CanonicalBuiltinType::Char,
        BuiltinCastTarget::Float => CanonicalBuiltinType::Float,
        BuiltinCastTarget::Error => CanonicalBuiltinType::Error,
    }
}

fn validate_evidence_method_shape(
    evidence_identity: &crate::compiler_frontend::canonical_type_identity::CanonicalEvidenceIdentity,
    requirement: &PublicTraitRequirementSurface,
    method: &PublicReceiverMethodSemantics,
) -> Result<(), CompilerError> {
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
            crate::compiler_frontend::public_call_summary::PublicCallParameterAccess::Shared
        ) | (
            PublicTraitReceiverAccess::Mutable,
            crate::compiler_frontend::public_call_summary::PublicCallParameterAccess::Mutable
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
            crate::compiler_frontend::public_call_summary::PublicCallParameterAccess::Shared => {
                !required.value_mode.is_mutable()
            }
            crate::compiler_frontend::public_call_summary::PublicCallParameterAccess::Mutable => {
                required.value_mode.is_mutable()
            }
            crate::compiler_frontend::public_call_summary::PublicCallParameterAccess::Reactive => {
                false
            }
        };
        if !access_matches
            || !trait_surface_type_matches(
                &required.type_identity,
                &receiver.type_identity,
                &actual.type_identity,
            )
        {
            return Err(evidence_shape_error(evidence_identity, requirement, method));
        }
    }

    let required_success = requirement
        .returns
        .iter()
        .filter(|returned| returned.channel == ReturnChannel::Success)
        .collect::<Vec<_>>();
    if required_success.len() != method.returns.len() {
        return Err(evidence_shape_error(evidence_identity, requirement, method));
    }
    for (required, actual) in required_success.iter().zip(&method.returns) {
        if !trait_surface_type_matches(
            &required.type_identity,
            &receiver.type_identity,
            &actual.type_identity,
        ) {
            return Err(evidence_shape_error(evidence_identity, requirement, method));
        }
    }

    let required_errors = requirement
        .returns
        .iter()
        .filter(|returned| returned.channel == ReturnChannel::Error)
        .collect::<Vec<_>>();
    let error_matches = match (required_errors.as_slice(), &method.error_return) {
        ([], None) => true,
        ([required], Some(actual)) => {
            trait_surface_type_matches(&required.type_identity, &receiver.type_identity, actual)
        }
        _ => false,
    };
    if !error_matches {
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
            CanonicalTypeIdentity::GenericInstance(instance),
            CanonicalTypeIdentity::SourceNominal(target_origin),
        ) if instance.base() == target_origin
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

fn evidence_shape_error(
    evidence_identity: &crate::compiler_frontend::canonical_type_identity::CanonicalEvidenceIdentity,
    requirement: &PublicTraitRequirementSurface,
    method: &PublicReceiverMethodSemantics,
) -> CompilerError {
    publication_error(format!(
        "reusable evidence {:?} maps requirement '{}' to incompatible receiver method {:?}",
        evidence_identity, requirement.name, method.method_origin
    ))
}

fn validate_declaration_category(
    origin: &OriginDeclarationId,
    semantics: &PublicDeclarationSemantics,
) -> Result<(), CompilerError> {
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

fn collect_expected_callable_summaries(
    origin: &OriginDeclarationId,
    semantics: &PublicDeclarationSemantics,
    expected: &mut FxHashMap<
        OriginFunctionId,
        Vec<crate::compiler_frontend::public_call_summary::PublicCallParameterAccess>,
    >,
) -> Result<(), CompilerError> {
    match semantics {
        PublicDeclarationSemantics::Function(function) => {
            let OriginDeclarationId::Function(origin) = origin else {
                unreachable!("declaration category was validated before callable collection");
            };
            if !matches!(origin.kind(), FunctionOriginKind::Free) {
                return Err(publication_error(format!(
                    "free-function record uses receiver origin {:?}",
                    origin
                )));
            }
            if matches!(function.category, PublicFunctionCategory::ConcreteLocal) {
                insert_expected_summary(origin, &function.parameters, expected)?;
            }
        }
        PublicDeclarationSemantics::Struct(structure) => {
            let OriginDeclarationId::Type(receiver_origin) = origin else {
                unreachable!("declaration category was validated before callable collection");
            };
            collect_expected_receiver_summaries(
                receiver_origin,
                &structure.receiver_methods,
                expected,
            )?;
        }
        PublicDeclarationSemantics::Choice(choice) => {
            let OriginDeclarationId::Type(receiver_origin) = origin else {
                unreachable!("declaration category was validated before callable collection");
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

fn collect_expected_receiver_summaries(
    receiver_origin: &OriginTypeId,
    methods: &[super::model::PublicReceiverMethodSemantics],
    expected: &mut FxHashMap<
        OriginFunctionId,
        Vec<crate::compiler_frontend::public_call_summary::PublicCallParameterAccess>,
    >,
) -> Result<(), CompilerError> {
    for method in methods {
        if method.method_origin.receiver() != Some(receiver_origin) {
            return Err(publication_error(format!(
                "receiver method {:?} is attached to incompatible receiver {:?}",
                method.method_origin, receiver_origin
            )));
        }
        if matches!(method.category, PublicReceiverMethodCategory::ConcreteLocal) {
            insert_expected_summary(&method.method_origin, &method.parameters, expected)?;
        }
    }
    Ok(())
}

fn insert_expected_summary(
    origin: &OriginFunctionId,
    parameters: &[super::model::PublicParameterTypeSlot],
    expected: &mut FxHashMap<
        OriginFunctionId,
        Vec<crate::compiler_frontend::public_call_summary::PublicCallParameterAccess>,
    >,
) -> Result<(), CompilerError> {
    let parameter_access = parameters
        .iter()
        .map(|parameter| parameter.access)
        .collect();
    if expected.insert(origin.clone(), parameter_access).is_some() {
        return Err(publication_error(format!(
            "duplicate concrete callable origin {:?}",
            origin
        )));
    }
    Ok(())
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
                &function.returns,
                function.error_return.as_ref(),
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
                    &method.returns,
                    method.error_return.as_ref(),
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
                    &method.returns,
                    method.error_return.as_ref(),
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
            for requirement in &trait_semantics.requirements {
                for parameter in &requirement.parameters {
                    if let TraitSurfaceTypeIdentity::Concrete(identity) = &parameter.type_identity {
                        identity.visit(visitor);
                    }
                }
                for returned in &requirement.returns {
                    if let TraitSurfaceTypeIdentity::Concrete(identity) = &returned.type_identity {
                        identity.visit(visitor);
                    }
                }
            }
        }
    }
}

fn visit_callable_type_identities(
    parameters: &[super::model::PublicParameterTypeSlot],
    returns: &[super::model::PublicReturnTypeSlot],
    error_return: Option<&CanonicalTypeIdentity>,
    visitor: &mut impl FnMut(&CanonicalTypeIdentity),
) {
    for parameter in parameters {
        parameter.type_identity.visit(visitor);
        if let Some(default) = &parameter.folded_default {
            default.visit_type_identities(visitor);
        }
    }
    for returned in returns {
        returned.type_identity.visit(visitor);
    }
    if let Some(error_return) = error_return {
        error_return.visit(visitor);
    }
}

fn publication_error(message: String) -> CompilerError {
    CompilerError::compiler_error(format!(
        "public semantic interface publication invariant failed: {message}"
    ))
}
