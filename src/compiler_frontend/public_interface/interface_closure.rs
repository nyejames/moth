//! Recursive semantic closure for completed public interfaces.
//!
//! WHAT: starts from one module's export bindings, joins provider-owned re-export origins to
//! immutable completed interfaces and retains every declaration, callable summary, canonical
//! type, trait and reusable evidence fact reachable from that surface.
//! WHY: consumers bind from one provider interface only. A facade must carry the closed semantic
//! facts behind its aliases instead of forcing consumers to reopen transitive providers. Closure
//! runs against one transient indexed view per completed interface and a declaration/evidence
//! work queue, so it never scans every provider for each selected fact.

use super::SourceProviderImportSet;
use super::interface_view::InterfaceView;
use super::model::{
    ConcreteCallSummaryRecord, LocalPublicInterface, PublicChoiceSemantics,
    PublicDeclarationRecord, PublicDeclarationSemantics, PublicEvidenceRecord,
    PublicFunctionCategory, PublicFunctionSemantics, PublicGenericParameterSurface,
    PublicInterfaceDraft, PublicReceiverMethodCategory, PublicReceiverMethodSemantics,
    PublicSemanticInterface, PublicStructSemantics, PublicTraitSemantics, TraitSurfaceTypeIdentity,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalEvidenceIdentity, CanonicalTraitIdentity, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::PublicFoldedValue;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::semantic_identity::{OriginDeclarationId, OriginFunctionId};
use rustc_hash::{FxHashMap, FxHashSet};
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

        let mut direct = Self {
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
        // Collapse repeated references to one completed interface, but keep two distinct
        // interfaces that claim the same module origin so the closure agreement check rejects
        // their disagreement deterministically instead of silently preferring one.
        providers.dedup_by(|left, right| std::ptr::eq(*left, *right));

        let mut closure = InterfaceClosure::new(&direct, providers)?;
        for binding in &export_bindings {
            closure.work.enqueue_declaration(binding.origin().clone());
        }
        closure.compute()?;
        let (declarations, reusable_evidence, concrete_call_summaries) =
            closure.into_selection().materialize(&mut direct)?;

        let interface = Self {
            module_origin,
            export_bindings,
            export_diagnostic_provenance,
            binding_exports,
            declarations,
            reusable_evidence,
            concrete_call_summaries,
        };
        interface.validate_for_publication()?;
        interface.validate_binding_targets(external_registry)?;
        Ok(interface)
    }
}

/// The three closed fact vectors produced by one interface closure.
type ClosedInterfaceRecords = (
    Vec<PublicDeclarationRecord>,
    Vec<PublicEvidenceRecord>,
    Vec<ConcreteCallSummaryRecord>,
);

/// Index position of one record inside one source interface.
///
/// `source` is 0 for the direct interface and `provider_index + 1` for each provider.
#[derive(Clone, Copy, Debug)]
struct RecordRef {
    source: usize,
    record: usize,
}

/// Immutable indexed lookup state shared by the closure work queue.
///
/// Combined origin maps are built once. Agreement checks stay O(k) for the few interfaces that
/// publish the same origin, and every lookup is direct instead of a per-provider linear scan.
/// Keeping the state separate from the mutable queues lets closure steps borrow records from the
/// state while enqueuing new work.
struct ClosureState<'direct, 'provider> {
    direct_view: InterfaceView<'direct>,
    provider_views: Vec<InterfaceView<'provider>>,
    declarations_by_origin: FxHashMap<OriginDeclarationId, Vec<RecordRef>>,
    summaries_by_origin: FxHashMap<OriginFunctionId, Vec<RecordRef>>,
    evidence_by_origin: FxHashMap<OriginDeclarationId, Vec<RecordRef>>,
}

impl<'direct, 'provider> ClosureState<'direct, 'provider> {
    fn new(
        direct: &'direct PublicSemanticInterface,
        providers: &[&'provider PublicSemanticInterface],
    ) -> Result<Self, CompilerError> {
        let direct_view = InterfaceView::build(direct)?;
        let mut provider_views = Vec::with_capacity(providers.len());
        for provider in providers {
            provider_views.push(InterfaceView::build(provider)?);
        }

        let mut declarations_by_origin: FxHashMap<OriginDeclarationId, Vec<RecordRef>> =
            FxHashMap::default();
        let mut summaries_by_origin: FxHashMap<OriginFunctionId, Vec<RecordRef>> =
            FxHashMap::default();
        let mut evidence_by_origin: FxHashMap<OriginDeclarationId, Vec<RecordRef>> =
            FxHashMap::default();

        for (record, declaration) in direct.declarations.iter().enumerate() {
            declarations_by_origin
                .entry(declaration.origin.clone())
                .or_default()
                .push(RecordRef { source: 0, record });
        }
        for (record, summary) in direct.concrete_call_summaries.iter().enumerate() {
            summaries_by_origin
                .entry(summary.origin.clone())
                .or_default()
                .push(RecordRef { source: 0, record });
        }
        for (record, evidence) in direct.reusable_evidence.iter().enumerate() {
            index_evidence(evidence, 0, record, &mut evidence_by_origin);
        }

        for (provider_index, view) in provider_views.iter().enumerate() {
            let source = provider_index + 1;
            for (record, declaration) in view.interface().declarations.iter().enumerate() {
                declarations_by_origin
                    .entry(declaration.origin.clone())
                    .or_default()
                    .push(RecordRef { source, record });
            }
            for (record, summary) in view.interface().concrete_call_summaries.iter().enumerate() {
                summaries_by_origin
                    .entry(summary.origin.clone())
                    .or_default()
                    .push(RecordRef { source, record });
            }
            for (record, evidence) in view.interface().reusable_evidence.iter().enumerate() {
                index_evidence(evidence, source, record, &mut evidence_by_origin);
            }
        }

        Ok(Self {
            direct_view,
            provider_views,
            declarations_by_origin,
            summaries_by_origin,
            evidence_by_origin,
        })
    }

    fn declaration(
        &self,
        source: usize,
        origin: &OriginDeclarationId,
    ) -> Option<&PublicDeclarationRecord> {
        if source == 0 {
            self.direct_view.declaration(origin)
        } else {
            self.provider_views[source - 1].declaration(origin)
        }
    }

    fn summary(&self, source: usize, origin: &OriginFunctionId) -> Option<&PublicCallSummary> {
        if source == 0 {
            self.direct_view.concrete_call_summary(origin)
        } else {
            self.provider_views[source - 1].concrete_call_summary(origin)
        }
    }

    fn evidence(
        &self,
        source: usize,
        identity: &CanonicalEvidenceIdentity,
    ) -> Option<&PublicEvidenceRecord> {
        if source == 0 {
            self.direct_view.evidence(identity)
        } else {
            self.provider_views[source - 1].evidence(identity)
        }
    }

    fn evidence_identity(&self, reference: RecordRef) -> &CanonicalEvidenceIdentity {
        if reference.source == 0 {
            &self.direct_view.interface().reusable_evidence[reference.record].identity
        } else {
            &self.provider_views[reference.source - 1]
                .interface()
                .reusable_evidence[reference.record]
                .identity
        }
    }

    /// Find one declaration record across the direct interface and providers.
    ///
    /// All publishers of one origin must agree; the record is borrowed through the indexed view
    /// and moved or cloned once at finalization.
    fn find_declaration(
        &self,
        origin: &OriginDeclarationId,
    ) -> Result<&PublicDeclarationRecord, CompilerError> {
        let references = self.declarations_by_origin.get(origin).ok_or_else(|| {
            closure_error(format!(
                "reachable declaration origin {:?} is absent from the local and completed provider interfaces",
                origin
            ))
        })?;
        let first_record = self
            .declaration(references[0].source, origin)
            .ok_or_else(|| {
                closure_error(format!(
                    "declaration view lost origin {:?} during index construction",
                    origin
                ))
            })?;

        for reference in &references[1..] {
            let candidate = self.declaration(reference.source, origin).ok_or_else(|| {
                closure_error(format!(
                    "declaration view lost origin {:?} during index construction",
                    origin
                ))
            })?;
            if first_record != candidate {
                return Err(closure_error(format!(
                    "provider interfaces disagree on declaration origin {:?}",
                    origin
                )));
            }
        }

        Ok(first_record)
    }

    /// Validate that every interface publishing one concrete summary agrees on it.
    fn validate_summary_agreement(&self, origin: &OriginFunctionId) -> Result<(), CompilerError> {
        let references = self.summaries_by_origin.get(origin).ok_or_else(|| {
            closure_error(format!(
                "reachable concrete callable {:?} has no completed call summary",
                origin
            ))
        })?;
        let first = self.summary(references[0].source, origin).ok_or_else(|| {
            closure_error(format!(
                "summary view lost origin {:?} during index construction",
                origin
            ))
        })?;

        for reference in &references[1..] {
            let candidate = self.summary(reference.source, origin).ok_or_else(|| {
                closure_error(format!(
                    "summary view lost origin {:?} during index construction",
                    origin
                ))
            })?;
            if first != candidate {
                return Err(closure_error(format!(
                    "provider interfaces disagree on concrete call summary {:?}",
                    origin
                )));
            }
        }

        Ok(())
    }
}

/// Mutable closure queues and selection sets.
///
/// Kept separate from [`ClosureState`] so dependency walking can borrow records from the state
/// while enqueueing new declarations and evidence.
struct ClosureWork {
    pending_declarations: VecDeque<OriginDeclarationId>,
    pending_evidence: VecDeque<RecordRef>,
    selected_declarations: FxHashSet<OriginDeclarationId>,
    selected_summaries: FxHashSet<OriginFunctionId>,
    selected_evidence: FxHashSet<CanonicalEvidenceIdentity>,
}

impl ClosureWork {
    fn new() -> Self {
        Self {
            pending_declarations: VecDeque::new(),
            pending_evidence: VecDeque::new(),
            selected_declarations: FxHashSet::default(),
            selected_summaries: FxHashSet::default(),
            selected_evidence: FxHashSet::default(),
        }
    }

    fn enqueue_declaration(&mut self, origin: OriginDeclarationId) {
        if !self.selected_declarations.contains(&origin) {
            self.pending_declarations.push_back(origin);
        }
    }

    fn copy_callable_summaries(
        &mut self,
        state: &ClosureState<'_, '_>,
        declaration: &PublicDeclarationRecord,
    ) -> Result<(), CompilerError> {
        let mut callable_origins = Vec::new();
        collect_concrete_callable_origins(declaration, &mut callable_origins);

        for origin in callable_origins {
            if self.selected_summaries.insert(origin.clone()) {
                state.validate_summary_agreement(&origin)?;
            }
        }

        Ok(())
    }

    fn process_pending_evidence(
        &mut self,
        state: &ClosureState<'_, '_>,
    ) -> Result<bool, CompilerError> {
        let mut added = false;
        while let Some(reference) = self.pending_evidence.pop_front() {
            let identity = state.evidence_identity(reference).clone();
            if !self.selected_evidence.insert(identity.clone()) {
                continue;
            }

            let evidence = state.evidence(reference.source, &identity).ok_or_else(|| {
                closure_error(format!(
                    "evidence view lost identity {:?} during index construction",
                    identity
                ))
            })?;
            self.enqueue_type(evidence.identity.target_type_identity());
            self.enqueue_trait(evidence.identity.trait_identity());
            for mapping in &evidence.requirement_mappings {
                self.enqueue_trait(mapping.requirement_identity.trait_identity());
            }
            added = true;
        }

        Ok(added)
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
}

/// Work-queue closure over indexed completed interfaces.
struct InterfaceClosure<'direct, 'provider> {
    state: ClosureState<'direct, 'provider>,
    work: ClosureWork,
}

impl<'direct, 'provider> InterfaceClosure<'direct, 'provider> {
    fn new(
        direct: &'direct PublicSemanticInterface,
        providers: Vec<&'provider PublicSemanticInterface>,
    ) -> Result<Self, CompilerError> {
        Ok(Self {
            state: ClosureState::new(direct, &providers)?,
            work: ClosureWork::new(),
        })
    }

    fn compute(&mut self) -> Result<(), CompilerError> {
        loop {
            while let Some(origin) = self.work.pending_declarations.pop_front() {
                if !self.work.selected_declarations.insert(origin.clone()) {
                    continue;
                }

                let declaration = self.state.find_declaration(&origin)?;
                self.work.enqueue_declaration_dependencies(declaration);
                self.work
                    .copy_callable_summaries(&self.state, declaration)?;
                if let Some(evidence_refs) = self.state.evidence_by_origin.get(&origin) {
                    self.work
                        .pending_evidence
                        .extend(evidence_refs.iter().copied());
                }
            }

            if !self.work.process_pending_evidence(&self.state)? {
                break;
            }
        }

        Ok(())
    }

    /// Consume the closure into its selection facts for final materialization.
    fn into_selection(self) -> ClosureSelection<'provider> {
        let InterfaceClosure { state, work } = self;
        let ClosureState {
            provider_views,
            declarations_by_origin,
            summaries_by_origin,
            ..
        } = state;
        let ClosureWork {
            selected_declarations,
            selected_summaries,
            selected_evidence,
            ..
        } = work;

        ClosureSelection {
            provider_views,
            declarations_by_origin,
            summaries_by_origin,
            selected_declarations,
            selected_summaries,
            selected_evidence,
        }
    }
}

/// Owned selection facts used to materialize the final closed interface.
///
/// The direct view is dropped with the closure so the owned direct interface can be moved from;
/// provider records stay reachable through these borrowed views.
struct ClosureSelection<'provider> {
    provider_views: Vec<InterfaceView<'provider>>,
    declarations_by_origin: FxHashMap<OriginDeclarationId, Vec<RecordRef>>,
    summaries_by_origin: FxHashMap<OriginFunctionId, Vec<RecordRef>>,
    selected_declarations: FxHashSet<OriginDeclarationId>,
    selected_summaries: FxHashSet<OriginFunctionId>,
    selected_evidence: FxHashSet<CanonicalEvidenceIdentity>,
}

impl<'provider> ClosureSelection<'provider> {
    /// Move the selected direct records out once and clone the selected provider records.
    ///
    /// Final vectors are sorted by stable semantic identity so publication order never depends
    /// on provider import order.
    fn materialize(
        self,
        direct: &mut PublicSemanticInterface,
    ) -> Result<ClosedInterfaceRecords, CompilerError> {
        let mut declarations = Vec::with_capacity(self.selected_declarations.len());
        let mut moved_declarations = FxHashSet::default();
        move_selected_direct_records(
            &mut direct.declarations,
            &self.selected_declarations,
            |declaration| declaration.origin.clone(),
            &mut moved_declarations,
            &mut declarations,
        );
        for origin in self
            .selected_declarations
            .iter()
            .filter(|origin| !moved_declarations.contains(*origin))
        {
            let reference = self
                .declarations_by_origin
                .get(origin)
                .and_then(|references| references.iter().find(|reference| reference.source != 0))
                .ok_or_else(|| {
                    closure_error(format!(
                        "selected declaration origin {:?} lost its provider record",
                        origin
                    ))
                })?;
            declarations.push(
                self.provider_views[reference.source - 1]
                    .interface()
                    .declarations[reference.record]
                    .clone(),
            );
        }
        declarations.sort_by(|left, right| left.origin.cmp(&right.origin));

        let mut reusable_evidence = Vec::with_capacity(self.selected_evidence.len());
        let mut moved_evidence = FxHashSet::default();
        move_selected_direct_records(
            &mut direct.reusable_evidence,
            &self.selected_evidence,
            |evidence| evidence.identity.clone(),
            &mut moved_evidence,
            &mut reusable_evidence,
        );
        for identity in self
            .selected_evidence
            .iter()
            .filter(|identity| !moved_evidence.contains(*identity))
        {
            let reference = self
                .provider_views
                .iter()
                .enumerate()
                .find_map(|(provider_index, view)| {
                    view.evidence(identity).map(|_| RecordRef {
                        source: provider_index + 1,
                        record: view
                            .interface()
                            .reusable_evidence
                            .iter()
                            .position(|record| &record.identity == identity)
                            .expect("evidence view index must match its vector"),
                    })
                })
                .ok_or_else(|| {
                    closure_error(format!(
                        "selected evidence identity {:?} lost its provider record",
                        identity
                    ))
                })?;
            reusable_evidence.push(
                self.provider_views[reference.source - 1]
                    .interface()
                    .reusable_evidence[reference.record]
                    .clone(),
            );
        }
        reusable_evidence.sort_by(|left, right| left.identity.cmp(&right.identity));

        let mut concrete_call_summaries = Vec::with_capacity(self.selected_summaries.len());
        let mut moved_summaries = FxHashSet::default();
        move_selected_direct_records(
            &mut direct.concrete_call_summaries,
            &self.selected_summaries,
            |summary| summary.origin.clone(),
            &mut moved_summaries,
            &mut concrete_call_summaries,
        );
        for origin in self
            .selected_summaries
            .iter()
            .filter(|origin| !moved_summaries.contains(*origin))
        {
            let reference = self
                .summaries_by_origin
                .get(origin)
                .and_then(|references| references.iter().find(|reference| reference.source != 0))
                .ok_or_else(|| {
                    closure_error(format!(
                        "selected summary origin {:?} lost its provider record",
                        origin
                    ))
                })?;
            let summary = self.provider_views[reference.source - 1]
                .interface()
                .concrete_call_summaries[reference.record]
                .summary
                .clone();
            concrete_call_summaries.push(ConcreteCallSummaryRecord {
                origin: origin.clone(),
                summary,
            });
        }
        concrete_call_summaries.sort_by(|left, right| left.origin.cmp(&right.origin));

        Ok((declarations, reusable_evidence, concrete_call_summaries))
    }
}

/// Index one evidence record by every declaration origin its target type references.
fn index_evidence(
    evidence: &PublicEvidenceRecord,
    source: usize,
    record: usize,
    evidence_by_origin: &mut FxHashMap<OriginDeclarationId, Vec<RecordRef>>,
) {
    let mut referenced_origins = Vec::new();
    collect_type_origins(
        evidence.identity.target_type_identity(),
        &mut referenced_origins,
    );
    for origin in referenced_origins {
        evidence_by_origin
            .entry(origin)
            .or_default()
            .push(RecordRef { source, record });
    }
}

/// Move every selected direct record out exactly once, in descending index order.
///
/// `swap_remove` is safe in strictly descending order because lower indices never move.
fn move_selected_direct_records<K, T>(
    records: &mut Vec<T>,
    selected: &FxHashSet<K>,
    key_of: impl Fn(&T) -> K,
    moved: &mut FxHashSet<K>,
    output: &mut Vec<T>,
) where
    K: std::hash::Hash + Eq + Clone,
{
    let mut selected_positions = Vec::new();
    for (index, record) in records.iter().enumerate() {
        let key = key_of(record);
        if selected.contains(&key) {
            selected_positions.push((key, index));
        }
    }
    selected_positions.sort_by_key(|(_, index)| std::cmp::Reverse(*index));
    for (key, index) in selected_positions {
        moved.insert(key);
        output.push(records.swap_remove(index));
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

/// Collect every declaration origin referenced by one canonical type identity.
///
/// This precomputes the evidence eligibility predicate once per evidence record at closure
/// setup, so the fixed point never re-scans all candidates or re-walks type shapes.
fn collect_type_origins(identity: &CanonicalTypeIdentity, origins: &mut Vec<OriginDeclarationId>) {
    match identity {
        CanonicalTypeIdentity::SourceNominal(origin) => {
            origins.push(OriginDeclarationId::Type(origin.clone()));
        }
        CanonicalTypeIdentity::Collection(collection) => {
            collect_type_origins(collection.element(), origins);
        }
        CanonicalTypeIdentity::OrderedMap(map) => {
            collect_type_origins(map.key(), origins);
            collect_type_origins(map.value(), origins);
        }
        CanonicalTypeIdentity::Option(inner) => collect_type_origins(inner, origins),
        CanonicalTypeIdentity::FallibleCarrier(carrier) => {
            collect_type_origins(carrier.success(), origins);
            collect_type_origins(carrier.error(), origins);
        }
        CanonicalTypeIdentity::GenericInstance(instance) => {
            origins.push(OriginDeclarationId::Type(instance.base().clone()));
            for argument in instance.arguments() {
                collect_type_origins(argument, origins);
            }
        }
        CanonicalTypeIdentity::ModulePrivateGenericInstance(instance) => {
            for argument in instance.arguments() {
                collect_type_origins(argument, origins);
            }
        }
        CanonicalTypeIdentity::Builtin(_)
        | CanonicalTypeIdentity::ModulePrivateNominal(_)
        | CanonicalTypeIdentity::ExternalOpaque(_)
        | CanonicalTypeIdentity::GenericParameter(_) => {}
    }
}

fn closure_error(detail: impl Into<String>) -> CompilerError {
    CompilerError::compiler_error(format!(
        "public semantic interface closure: {}",
        detail.into()
    ))
}
