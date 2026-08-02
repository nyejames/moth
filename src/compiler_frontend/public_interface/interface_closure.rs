//! Recursive semantic closure for completed public interfaces.
//!
//! WHAT: starts from one module's export bindings, joins provider-owned re-export origins to
//! immutable completed interfaces and retains every declaration, callable summary, canonical
//! type, trait and reusable evidence fact reachable from that surface.
//! WHY: consumers bind from one provider interface only. A facade must carry the closed semantic
//! facts behind its aliases instead of forcing consumers to reopen transitive providers.

use super::SourceProviderImportSet;
use super::model::{
    ConcreteCallSummaryRecord, LocalPublicInterface, PublicChoiceSemantics,
    PublicDeclarationRecord, PublicDeclarationSemantics, PublicEvidenceRecord,
    PublicFunctionCategory, PublicFunctionSemantics, PublicGenericParameterSurface,
    PublicInterfaceDraft, PublicReceiverMethodCategory, PublicReceiverMethodSemantics,
    PublicSemanticInterface, PublicStructSemantics, PublicTraitSemantics, TraitSurfaceTypeIdentity,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalTraitIdentity, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::PublicFoldedValue;
use crate::compiler_frontend::semantic_identity::{OriginDeclarationId, OriginFunctionId};
use rustc_hash::FxHashSet;
use std::collections::VecDeque;

impl PublicSemanticInterface {
    /// Close one completed local surface over the immutable interfaces selected by Stage 0.
    pub(crate) fn close_from_local(
        local: LocalPublicInterface,
        provider_imports: &SourceProviderImportSet<'_>,
        external_registry: &ExternalPackageRegistry,
    ) -> Result<Self, CompilerError> {
        let LocalPublicInterface {
            draft,
            concrete_call_summaries,
        } = local;
        let PublicInterfaceDraft {
            module_origin,
            export_bindings,
            export_diagnostic_provenance,
            binding_exports,
            declarations,
            reusable_evidence,
        } = draft;

        let direct = Self {
            module_origin: module_origin.clone(),
            export_bindings: export_bindings.clone(),
            export_diagnostic_provenance: export_diagnostic_provenance.clone(),
            binding_exports: binding_exports.clone(),
            declarations,
            reusable_evidence,
            concrete_call_summaries,
        };
        direct.validate_closure_input()?;

        let mut providers: Vec<&PublicSemanticInterface> = provider_imports.interfaces().collect();
        providers.sort_by(|left, right| left.module_origin.cmp(&right.module_origin));
        providers.dedup_by(|left, right| left.module_origin == right.module_origin);

        let mut closure = InterfaceClosure::new(&direct, providers);
        for binding in &export_bindings {
            closure.enqueue_declaration(binding.origin().clone());
        }
        closure.compute()?;

        let interface = Self {
            module_origin,
            export_bindings,
            export_diagnostic_provenance,
            binding_exports,
            declarations: closure.declarations,
            reusable_evidence: closure.reusable_evidence,
            concrete_call_summaries: closure.concrete_call_summaries,
        };
        interface.validate_for_publication()?;
        interface.validate_binding_targets(external_registry)?;
        Ok(interface)
    }
}

struct InterfaceClosure<'a> {
    direct: &'a PublicSemanticInterface,
    providers: Vec<&'a PublicSemanticInterface>,
    pending_declarations: VecDeque<OriginDeclarationId>,
    selected_declarations: FxHashSet<OriginDeclarationId>,
    selected_summaries: FxHashSet<OriginFunctionId>,
    selected_evidence:
        FxHashSet<crate::compiler_frontend::canonical_type_identity::CanonicalEvidenceIdentity>,
    declarations: Vec<PublicDeclarationRecord>,
    reusable_evidence: Vec<PublicEvidenceRecord>,
    concrete_call_summaries: Vec<ConcreteCallSummaryRecord>,
}

impl<'a> InterfaceClosure<'a> {
    fn new(
        direct: &'a PublicSemanticInterface,
        providers: Vec<&'a PublicSemanticInterface>,
    ) -> Self {
        Self {
            direct,
            providers,
            pending_declarations: VecDeque::new(),
            selected_declarations: FxHashSet::default(),
            selected_summaries: FxHashSet::default(),
            selected_evidence: FxHashSet::default(),
            declarations: Vec::new(),
            reusable_evidence: Vec::new(),
            concrete_call_summaries: Vec::new(),
        }
    }

    fn enqueue_declaration(&mut self, origin: OriginDeclarationId) {
        if !self.selected_declarations.contains(&origin) {
            self.pending_declarations.push_back(origin);
        }
    }

    fn compute(&mut self) -> Result<(), CompilerError> {
        loop {
            while let Some(origin) = self.pending_declarations.pop_front() {
                if !self.selected_declarations.insert(origin.clone()) {
                    continue;
                }

                let declaration = self.find_declaration(&origin)?;
                self.enqueue_declaration_dependencies(&declaration);
                self.copy_callable_summaries(&declaration)?;
                self.declarations.push(declaration);
            }

            if !self.select_reachable_evidence()? {
                break;
            }
        }

        Ok(())
    }

    fn find_declaration(
        &self,
        origin: &OriginDeclarationId,
    ) -> Result<PublicDeclarationRecord, CompilerError> {
        let mut found = self.direct.declaration(origin);
        for provider in &self.providers {
            let Some(candidate) = provider.declaration(origin) else {
                continue;
            };
            if let Some(existing) = found
                && existing != candidate
            {
                return Err(closure_error(format!(
                    "provider interfaces disagree on declaration origin {:?}",
                    origin
                )));
            }
            found = Some(candidate);
        }

        found.cloned().ok_or_else(|| {
            closure_error(format!(
                "reachable declaration origin {:?} is absent from the local and completed provider interfaces",
                origin
            ))
        })
    }

    fn copy_callable_summaries(
        &mut self,
        declaration: &PublicDeclarationRecord,
    ) -> Result<(), CompilerError> {
        let mut callable_origins = Vec::new();
        collect_concrete_callable_origins(declaration, &mut callable_origins);

        for origin in callable_origins {
            if !self.selected_summaries.insert(origin.clone()) {
                continue;
            }
            let summary = self.find_summary(&origin)?;
            self.concrete_call_summaries
                .push(ConcreteCallSummaryRecord { origin, summary });
        }

        Ok(())
    }

    fn find_summary(
        &self,
        origin: &OriginFunctionId,
    ) -> Result<crate::compiler_frontend::public_call_summary::PublicCallSummary, CompilerError>
    {
        let mut found = self.direct.concrete_call_summary(origin);
        for provider in &self.providers {
            let Some(candidate) = provider.concrete_call_summary(origin) else {
                continue;
            };
            if let Some(existing) = found
                && existing != candidate
            {
                return Err(closure_error(format!(
                    "provider interfaces disagree on concrete call summary {:?}",
                    origin
                )));
            }
            found = Some(candidate);
        }

        found.cloned().ok_or_else(|| {
            closure_error(format!(
                "reachable concrete callable {:?} has no completed call summary",
                origin
            ))
        })
    }

    fn enqueue_declaration_dependencies(&mut self, declaration: &PublicDeclarationRecord) {
        match &declaration.semantics {
            PublicDeclarationSemantics::Function(function) => {
                self.enqueue_function_dependencies(function)
            }
            PublicDeclarationSemantics::Struct(structure) => {
                self.enqueue_struct_dependencies(structure)
            }
            PublicDeclarationSemantics::Choice(choice) => self.enqueue_choice_dependencies(choice),
            PublicDeclarationSemantics::TransparentAlias(alias) => {
                self.enqueue_type(&alias.target_type_identity)
            }
            PublicDeclarationSemantics::Constant(constant) => {
                self.enqueue_type(&constant.type_identity);
                self.enqueue_folded_value(&constant.folded_value);
            }
            PublicDeclarationSemantics::Trait(trait_semantics) => {
                self.enqueue_trait_dependencies(trait_semantics)
            }
        }
    }

    fn enqueue_function_dependencies(&mut self, function: &PublicFunctionSemantics) {
        for parameter in &function.parameters {
            self.enqueue_type(&parameter.type_identity);
            if let Some(default) = &parameter.folded_default {
                self.enqueue_folded_value(default);
            }
        }
        for return_slot in &function.returns {
            self.enqueue_type(&return_slot.type_identity);
        }
        if let Some(error_return) = &function.error_return {
            self.enqueue_type(error_return);
        }
        if let PublicFunctionCategory::GenericTemplate(template) = &function.category {
            self.enqueue_generic_parameters(&template.generic_parameters);
        }
    }

    fn enqueue_struct_dependencies(&mut self, structure: &PublicStructSemantics) {
        self.enqueue_generic_parameters(&structure.generic_parameters);
        for field in &structure.fields {
            self.enqueue_type(&field.type_identity);
            if let Some(default) = &field.folded_default {
                self.enqueue_folded_value(default);
            }
        }
        for method in &structure.receiver_methods {
            self.enqueue_receiver_method_dependencies(method);
        }
    }

    fn enqueue_choice_dependencies(&mut self, choice: &PublicChoiceSemantics) {
        self.enqueue_generic_parameters(&choice.generic_parameters);
        for variant in &choice.variants {
            for field in &variant.payload_fields {
                self.enqueue_type(&field.type_identity);
            }
        }
        for method in &choice.receiver_methods {
            self.enqueue_receiver_method_dependencies(method);
        }
    }

    fn enqueue_receiver_method_dependencies(&mut self, method: &PublicReceiverMethodSemantics) {
        for parameter in &method.parameters {
            self.enqueue_type(&parameter.type_identity);
            if let Some(default) = &parameter.folded_default {
                self.enqueue_folded_value(default);
            }
        }
        for return_slot in &method.returns {
            self.enqueue_type(&return_slot.type_identity);
        }
        if let Some(error_return) = &method.error_return {
            self.enqueue_type(error_return);
        }
    }

    fn enqueue_trait_dependencies(&mut self, trait_semantics: &PublicTraitSemantics) {
        for requirement in &trait_semantics.requirements {
            for parameter in &requirement.parameters {
                if let TraitSurfaceTypeIdentity::Concrete(identity) = &parameter.type_identity {
                    self.enqueue_type(identity);
                }
            }
            for return_slot in &requirement.returns {
                if let TraitSurfaceTypeIdentity::Concrete(identity) = &return_slot.type_identity {
                    self.enqueue_type(identity);
                }
            }
        }
        for incompatibility in &trait_semantics.incompatibilities {
            self.enqueue_trait(incompatibility);
        }
    }

    fn enqueue_generic_parameters(&mut self, parameters: &[PublicGenericParameterSurface]) {
        for parameter in parameters {
            for bound in &parameter.bounds {
                self.enqueue_trait(bound);
            }
        }
    }

    fn enqueue_type(&mut self, identity: &CanonicalTypeIdentity) {
        let mut dependencies = Vec::new();
        identity.visit(&mut |nested| match nested {
            CanonicalTypeIdentity::SourceNominal(origin) => {
                dependencies.push(OriginDeclarationId::Type(origin.clone()));
            }
            CanonicalTypeIdentity::GenericInstance(instance) => {
                dependencies.push(OriginDeclarationId::Type(instance.base().clone()));
            }
            CanonicalTypeIdentity::Builtin(_)
            | CanonicalTypeIdentity::ModulePrivateNominal(_)
            | CanonicalTypeIdentity::ModulePrivateGenericInstance(_)
            | CanonicalTypeIdentity::ExternalOpaque(_)
            | CanonicalTypeIdentity::Collection(_)
            | CanonicalTypeIdentity::OrderedMap(_)
            | CanonicalTypeIdentity::Option(_)
            | CanonicalTypeIdentity::FallibleCarrier(_)
            | CanonicalTypeIdentity::GenericParameter(_) => {}
        });

        for dependency in dependencies {
            self.enqueue_declaration(dependency);
        }
    }

    fn enqueue_trait(&mut self, identity: &CanonicalTraitIdentity) {
        if let CanonicalTraitIdentity::Source(origin) = identity {
            self.enqueue_declaration(OriginDeclarationId::Trait(origin.clone()));
        }
    }

    fn enqueue_folded_value(&mut self, value: &PublicFoldedValue) {
        let mut type_identities = Vec::new();
        value.visit_type_identities(&mut |identity| type_identities.push(identity.clone()));
        for identity in type_identities {
            self.enqueue_type(&identity);
        }
    }

    fn select_reachable_evidence(&mut self) -> Result<bool, CompilerError> {
        let mut added = false;
        let candidates: Vec<PublicEvidenceRecord> = self
            .evidence_sources()
            .flat_map(|interface| interface.reusable_evidence.iter().cloned())
            .collect();

        for evidence in candidates {
            if self.selected_evidence.contains(&evidence.identity)
                || !type_references_selected_declaration(
                    evidence.identity.target_type_identity(),
                    &self.selected_declarations,
                )
            {
                continue;
            }

            self.enqueue_type(evidence.identity.target_type_identity());
            self.enqueue_trait(evidence.identity.trait_identity());
            for mapping in &evidence.requirement_mappings {
                self.enqueue_trait(mapping.requirement_identity.trait_identity());
            }
            self.selected_evidence.insert(evidence.identity.clone());
            self.reusable_evidence.push(evidence);
            added = true;
        }

        Ok(added)
    }

    fn evidence_sources(&self) -> impl Iterator<Item = &PublicSemanticInterface> {
        std::iter::once(self.direct).chain(self.providers.iter().copied())
    }
}

fn collect_concrete_callable_origins(
    declaration: &PublicDeclarationRecord,
    origins: &mut Vec<OriginFunctionId>,
) {
    match &declaration.semantics {
        PublicDeclarationSemantics::Function(function) => {
            if matches!(function.category, PublicFunctionCategory::ConcreteLocal)
                && let OriginDeclarationId::Function(origin) = &declaration.origin
            {
                origins.push(origin.clone());
            }
        }
        PublicDeclarationSemantics::Struct(structure) => {
            collect_concrete_receiver_origins(&structure.receiver_methods, origins)
        }
        PublicDeclarationSemantics::Choice(choice) => {
            collect_concrete_receiver_origins(&choice.receiver_methods, origins)
        }
        PublicDeclarationSemantics::TransparentAlias(_)
        | PublicDeclarationSemantics::Constant(_)
        | PublicDeclarationSemantics::Trait(_) => {}
    }
}

fn collect_concrete_receiver_origins(
    methods: &[PublicReceiverMethodSemantics],
    origins: &mut Vec<OriginFunctionId>,
) {
    for method in methods {
        if matches!(method.category, PublicReceiverMethodCategory::ConcreteLocal) {
            origins.push(method.method_origin.clone());
        }
    }
}

fn type_references_selected_declaration(
    identity: &CanonicalTypeIdentity,
    selected: &FxHashSet<OriginDeclarationId>,
) -> bool {
    match identity {
        CanonicalTypeIdentity::SourceNominal(origin) => {
            selected.contains(&OriginDeclarationId::Type(origin.clone()))
        }
        CanonicalTypeIdentity::Collection(collection) => {
            type_references_selected_declaration(collection.element(), selected)
        }
        CanonicalTypeIdentity::OrderedMap(map) => {
            type_references_selected_declaration(map.key(), selected)
                || type_references_selected_declaration(map.value(), selected)
        }
        CanonicalTypeIdentity::Option(inner) => {
            type_references_selected_declaration(inner, selected)
        }
        CanonicalTypeIdentity::FallibleCarrier(carrier) => {
            type_references_selected_declaration(carrier.success(), selected)
                || type_references_selected_declaration(carrier.error(), selected)
        }
        CanonicalTypeIdentity::GenericInstance(instance) => {
            selected.contains(&OriginDeclarationId::Type(instance.base().clone()))
                || instance
                    .arguments()
                    .iter()
                    .any(|argument| type_references_selected_declaration(argument, selected))
        }
        CanonicalTypeIdentity::ModulePrivateGenericInstance(instance) => instance
            .arguments()
            .iter()
            .any(|argument| type_references_selected_declaration(argument, selected)),
        CanonicalTypeIdentity::Builtin(_)
        | CanonicalTypeIdentity::ModulePrivateNominal(_)
        | CanonicalTypeIdentity::ExternalOpaque(_)
        | CanonicalTypeIdentity::GenericParameter(_) => false,
    }
}

fn closure_error(detail: impl Into<String>) -> CompilerError {
    CompilerError::compiler_error(format!(
        "public semantic interface closure: {}",
        detail.into()
    ))
}
