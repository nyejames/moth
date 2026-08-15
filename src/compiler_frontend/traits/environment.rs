//! Resolved trait environment.
//!
//! WHAT: owns resolved trait definitions, stable trait IDs, lookup by canonical path,
//!      and a unified core-trait registry for compiler-owned metadata.
//! WHY: trait declarations are compile-time contracts. They must not be registered as
//!      ordinary `DataType`s or value declarations, but later AST phases still need
//!      deterministic identity and a single path for resolving both `DISPLAYABLE`
//!      and the core cast trait names.

use crate::compiler_frontend::ast::statements::functions::ReturnChannel;
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastTarget,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalCoreTraitIdentity, CanonicalTraitIdentity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::traits::definitions::{
    ResolvedTraitDefinition, ResolvedTraitParameter, ResolvedTraitRequirement, ResolvedTraitReturn,
    TraitReceiverRequirement, TraitVisibility,
};
use crate::compiler_frontend::traits::ids::{TraitId, TraitRequirementId};
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::FxHashMap;

pub(crate) const DISPLAYABLE_TRAIT_NAME: &str = "DISPLAYABLE";
pub(crate) const DISPLAYABLE_REQUIREMENT_NAME: &str = "display";
const TRAIT_THIS_NAME: &str = "This";

/// Optional per-core-trait classifier recorded beside compiler-owned trait
/// definitions.
///
/// WHAT: maps a registered `TraitId` back to the small amount of compiler-owned
/// metadata later cast phases need.
/// WHY: most of the compiler should treat core traits like ordinary static
/// trait metadata, but cast resolution needs to know which builtin target and
/// fallibility a core cast trait represents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CoreTraitKind {
    Displayable,
    Castable {
        target: BuiltinCastTarget,
        fallibility: BuiltinCastFallibility,
    },
}

/// Trait metadata lookup table for one compiled module.
///
/// WHAT: stores resolved trait definitions indexed by canonical path, a
///      `core_traits_by_name` table that resolves compiler-owned trait
///      names (such as `DISPLAYABLE`, `CASTABLE_TO_INT`, ...) without
///      touching the user-visible `visible_trait_names` binding map, a
///      `core_trait_kinds` side table that classifies core traits so the
///      AST environment builder can wire builtin cast evidence rows, an
///      `incompatible_traits` symmetric store for trait-pair metadata, and a
///      `public_incompatible_traits` symmetric store that retains only the
///      relations authored by an explicitly public `TRAIT must not TRAIT`
///      header in an export-capable root.
/// WHY: AST traits and core cast traits must both be reachable from source
///      spellings, but core traits must resolve without dependency clauses and must
///      not share a code path with user declarations. Sharing one registry
///      also keeps the trait environment from growing a parallel field
///      for every new core trait. Mutual-incompatibility metadata is owned
///      by the trait subsystem so conformance validation can reject types
///      that claim both sides of a `must not` relation. The public-only store
///      lets the public-interface draft retain explicitly public
///      incompatibility facts for direct public traits without reparsing
///      headers or reconstructing visibility later: a private `must not`
///      relation never enters the draft even when both traits are public.
#[derive(Clone, Debug, Default)]
pub(crate) struct TraitEnvironment {
    definitions: Vec<ResolvedTraitDefinition>,
    ids_by_path: FxHashMap<InternedPath, TraitId>,
    paths_by_id: FxHashMap<TraitId, Vec<InternedPath>>,
    ids_by_canonical_identity: FxHashMap<CanonicalTraitIdentity, TraitId>,
    core_traits_by_name: FxHashMap<&'static str, TraitId>,
    core_trait_kinds: FxHashMap<TraitId, CoreTraitKind>,
    incompatible_traits: FxHashMap<TraitId, Vec<TraitId>>,
    /// Symmetric store of incompatibility relations authored by an explicitly
    /// public `TRAIT must not TRAIT` header in an export-capable root.
    ///
    /// WHAT: only relations whose header has an export-capable file role and a
    ///      public export mode are recorded here. Private relations and
    ///      compiler-owned core cast trait pairs stay out, so this store is the
    ///      single source a direct public trait record consults for the
    ///      incompatibilities it carries into the public-interface draft.
    /// WHY: retaining the resolved public visibility once, at the same time
    ///      the symmetric relation is resolved, avoids reparsing headers or
    ///      reconstructing public/private visibility later. The order of each
    ///      vector is the deterministic authored source order in which the
    ///      relations were registered.
    public_incompatible_traits: FxHashMap<TraitId, Vec<TraitId>>,
}

impl TraitEnvironment {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Registers a single compiler-owned core trait definition using the
    /// shared metadata path.
    ///
    /// WHAT: builds a `ResolvedTraitDefinition` whose canonical path and
    ///      displayable name both come from the supplied static source
    ///      names, with the requirement signature shaped by the trait's
    ///      success return type and optional `Error!` channel. Idempotent:
    ///      re-registration of the same trait name returns the original
    ///      `TraitId`.
    /// WHY: `DISPLAYABLE` and every core cast trait must share one
    ///      registration path so the trait environment never grows a
    ///      parallel field per core trait.
    pub(crate) fn register_core_trait(
        &mut self,
        type_environment: &mut TypeEnvironment,
        string_table: &mut StringTable,
        trait_name: &'static str,
        requirement_name: &'static str,
        success_type: TypeId,
        error_return_type: Option<TypeId>,
    ) -> TraitId {
        if let Some(existing_id) = self.core_traits_by_name.get(trait_name) {
            return *existing_id;
        }

        let name = string_table.intern(trait_name);
        let requirement_name = string_table.intern(requirement_name);
        let this_name = string_table.intern(TRAIT_THIS_NAME);
        let path = InternedPath::from_single_str(trait_name, string_table);
        let source_file = InternedPath::new();
        let location = SourceLocation::default();
        let this_type = type_environment.register_synthetic_generic_parameter(this_name);

        let id = self.allocate_trait_id();
        let requirement_id = self.allocate_requirement_id();

        // Core cast traits never use mutable source access; `cast` must not
        // require mutable access or consume the source. `DISPLAYABLE` follows
        // the same convention because user receiver methods implement
        // `display |This| -> String` immutably today.
        let receiver = TraitReceiverRequirement::Immutable { this_type };
        let mut returns = vec![ResolvedTraitReturn {
            type_id: success_type,
            channel: ReturnChannel::Success,
            location: location.clone(),
        }];
        if let Some(error_type) = error_return_type {
            // Fallible core cast traits propagate the cast failure through a
            // dedicated `Error!` return channel so the resolved expression
            // keeps the user-authored source's existing `Error!` shape.
            returns.push(ResolvedTraitReturn {
                type_id: error_type,
                channel: ReturnChannel::Error,
                location: location.clone(),
            });
        }

        let requirement = ResolvedTraitRequirement {
            id: requirement_id,
            name: requirement_name,
            name_location: location.clone(),
            receiver,
            parameters: Vec::new(),
            returns,
            location: location.clone(),
        };

        let definition = ResolvedTraitDefinition {
            id,
            name,
            canonical_path: path.clone(),
            source_file,
            this_type,
            requirements: vec![requirement],
            declaration_location: location,
            visibility: TraitVisibility::Core,
        };

        self.ids_by_path.insert(path, id);
        self.paths_by_id
            .entry(id)
            .or_default()
            .push(definition.canonical_path.clone());
        self.core_traits_by_name.insert(trait_name, id);
        self.definitions.push(definition);
        id
    }

    /// Records the kind classifier for a previously registered core trait.
    ///
    /// WHAT: stores a `CoreTraitKind` entry so the AST environment builder
    ///      can later ask the trait environment for the `BuiltinCastTarget`
    ///      and fallibility of a core cast trait `TraitId` without
    ///      re-resolving the static metadata table.
    /// WHY: keeping the classification in the trait environment instead of
    ///      threading `CoreCastTrait` everywhere avoids a backwards
    ///      dependency from the trait subsystem into the cast subsystem and
    ///      keeps `TraitId` as the only public handle.
    pub(crate) fn record_core_trait_kind(&mut self, trait_id: TraitId, kind: CoreTraitKind) {
        self.core_trait_kinds.insert(trait_id, kind);
        let canonical_identity = match kind {
            CoreTraitKind::Displayable => CanonicalCoreTraitIdentity::Displayable,
            CoreTraitKind::Castable {
                target,
                fallibility,
            } => CanonicalCoreTraitIdentity::Castable {
                target,
                fallibility,
            },
        };
        self.ids_by_canonical_identity
            .insert(CanonicalTraitIdentity::Core(canonical_identity), trait_id);
    }

    /// Records a symmetric incompatibility relation between two traits.
    ///
    /// WHAT: adds `right` to the incompatibility list for `left` and vice
    ///      versa. Self-pairs are ignored because a trait is never recorded
    ///      as incompatible with itself.
    /// WHY: validation must be able to ask "are these two traits mutually
    ///      incompatible?" in either direction without the caller needing
    ///      to know which side was registered first. The store is owned by
    ///      the trait environment so the relation stays with trait metadata.
    pub(crate) fn record_incompatible_traits(&mut self, left: TraitId, right: TraitId) {
        if left == right {
            return;
        }

        Self::record_incompatible_trait_direction(&mut self.incompatible_traits, left, right);
        Self::record_incompatible_trait_direction(&mut self.incompatible_traits, right, left);
    }

    /// Returns `true` when `left` and `right` are recorded as incompatible.
    pub(crate) fn traits_are_incompatible(&self, left: TraitId, right: TraitId) -> bool {
        if left == right {
            return false;
        }

        self.incompatible_traits
            .get(&left)
            .is_some_and(|incompatible| incompatible.contains(&right))
    }

    fn record_incompatible_trait_direction(
        incompatible_traits: &mut FxHashMap<TraitId, Vec<TraitId>>,
        source: TraitId,
        target: TraitId,
    ) {
        let incompatible = incompatible_traits.entry(source).or_default();
        if !incompatible.contains(&target) {
            incompatible.push(target);
        }
    }

    /// Records a symmetric public incompatibility relation between two traits.
    ///
    /// WHAT: adds `right` to the public incompatibility list for `left` and vice
    ///      versa, preserving the authored source order. Self-pairs are ignored
    ///      because a trait is never recorded as incompatible with itself. Only
    ///      call this for relations authored by an explicitly public
    ///      `TRAIT must not TRAIT` header in an export-capable root.
    /// WHY: the public-interface draft consults only this store when attaching
    ///      incompatibilities to a direct public trait record, so a private
    ///      relation never leaks even when both traits are public. Reusing the
    ///      shared direction helper keeps one symmetric-insertion owner.
    pub(crate) fn record_public_incompatible_traits(&mut self, left: TraitId, right: TraitId) {
        if left == right {
            return;
        }

        Self::record_incompatible_trait_direction(
            &mut self.public_incompatible_traits,
            left,
            right,
        );
        Self::record_incompatible_trait_direction(
            &mut self.public_incompatible_traits,
            right,
            left,
        );
    }

    /// Returns the publicly-authored incompatible trait ids for `trait_id`, in
    /// authored source order, or an empty slice when no public relation
    /// involves it.
    ///
    /// WHAT: a direct public trait record consults this to carry symmetric
    ///      incompatibilities into the public-interface draft. The returned
    ///      order is the deterministic authored source order, independent of
    ///      hash-map iteration, so the draft never sorts by rendered text.
    pub(crate) fn public_incompatibilities_for(&self, trait_id: TraitId) -> &[TraitId] {
        self.public_incompatible_traits
            .get(&trait_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Returns every trait id appearing in any public incompatibility relation.
    ///
    /// WHAT: iterates both sides of the public store so the resolved
    ///      trait-source-fact table can retain the source/core classification
    ///      for every incompatibility trait id before the `TraitEnvironment`
    ///      is dropped.
    pub(crate) fn public_incompatible_trait_ids(&self) -> impl Iterator<Item = TraitId> + '_ {
        self.public_incompatible_traits
            .keys()
            .copied()
            .chain(self.public_incompatible_traits.values().flatten().copied())
    }

    /// Registers the compiler-owned `DISPLAYABLE` scaffold.
    ///
    /// WHAT: thin wrapper that forwards to the unified `register_core_trait`
    ///      path with the `DISPLAYABLE` source spelling and a `String`
    ///      return, then records its `CoreTraitKind::Displayable`
    ///      classifier.
    /// WHY: the historical one-off helper is preserved at the call site so
    ///      the AST environment builder does not need to know the
    ///      displayable internals, while the underlying implementation
    ///      shares the same generalized path as every core cast trait.
    pub(crate) fn register_core_displayable(
        &mut self,
        type_environment: &mut TypeEnvironment,
        string_table: &mut StringTable,
    ) -> TraitId {
        let string_type = type_environment.builtins().string;
        let trait_id = self.register_core_trait(
            type_environment,
            string_table,
            DISPLAYABLE_TRAIT_NAME,
            DISPLAYABLE_REQUIREMENT_NAME,
            string_type,
            None,
        );
        self.record_core_trait_kind(trait_id, CoreTraitKind::Displayable);
        trait_id
    }

    pub(crate) fn insert(&mut self, definition: ResolvedTraitDefinition) -> Option<TraitId> {
        if let Some(existing_id) = self.ids_by_path.get(&definition.canonical_path).copied() {
            return Some(existing_id);
        }

        let id = definition.id;
        self.ids_by_path
            .insert(definition.canonical_path.clone(), id);
        self.paths_by_id
            .entry(id)
            .or_default()
            .push(definition.canonical_path.clone());
        self.definitions.push(definition);
        None
    }

    /// Registers another consumer-local spelling for an existing trait.
    ///
    /// Imported aliases share one canonical trait identity and one dense `TraitId`. Retaining
    /// every bound path lets visibility and public-surface checks compare the exact local dependency
    /// target without treating an alias as another trait definition.
    pub(crate) fn register_path(
        &mut self,
        path: InternedPath,
        trait_id: TraitId,
    ) -> Result<(), CompilerError> {
        if let Some(existing_id) = self.ids_by_path.get(&path) {
            if *existing_id == trait_id {
                return Ok(());
            }

            return Err(CompilerError::compiler_error(
                "A trait path resolved to two different trait identities.",
            ));
        }

        self.ids_by_path.insert(path.clone(), trait_id);
        self.paths_by_id.entry(trait_id).or_default().push(path);
        Ok(())
    }

    pub(crate) fn has_path(&self, trait_id: TraitId, path: &InternedPath) -> bool {
        self.paths_by_id
            .get(&trait_id)
            .is_some_and(|paths| paths.contains(path))
    }

    pub(crate) fn paths_for(&self, trait_id: TraitId) -> &[InternedPath] {
        self.paths_by_id
            .get(&trait_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Joins one stable cross-module trait identity to its consumer-local dense handle.
    pub(crate) fn register_canonical_identity(
        &mut self,
        identity: CanonicalTraitIdentity,
        trait_id: TraitId,
    ) -> Result<(), CompilerError> {
        if let Some(existing_id) = self.ids_by_canonical_identity.get(&identity) {
            if *existing_id == trait_id {
                return Ok(());
            }

            return Err(CompilerError::compiler_error(
                "A canonical trait identity resolved to two consumer-local trait definitions.",
            ));
        }

        self.ids_by_canonical_identity.insert(identity, trait_id);
        Ok(())
    }

    pub(crate) fn id_for_canonical_identity(
        &self,
        identity: &CanonicalTraitIdentity,
    ) -> Option<TraitId> {
        self.ids_by_canonical_identity.get(identity).copied()
    }

    pub(crate) fn canonical_identity_for_id(
        &self,
        trait_id: TraitId,
    ) -> Option<&CanonicalTraitIdentity> {
        self.ids_by_canonical_identity
            .iter()
            .find_map(|(identity, candidate)| (*candidate == trait_id).then_some(identity))
    }

    pub(crate) fn get(&self, id: TraitId) -> Option<&ResolvedTraitDefinition> {
        self.definitions.get(id.0 as usize)
    }

    pub(crate) fn definitions(&self) -> impl Iterator<Item = &ResolvedTraitDefinition> {
        self.definitions.iter()
    }

    pub(crate) fn id_for_path(&self, path: &InternedPath) -> Option<TraitId> {
        self.ids_by_path.get(path).copied()
    }

    /// Resolves a compiler-owned core trait by source spelling.
    ///
    /// WHAT: maps a `StringId` to the canonical `TraitId` for any registered
    ///      core trait (currently `DISPLAYABLE` and the twelve core cast
    ///      traits). Returns `None` for user-authored trait names.
    /// WHY: core trait metadata is not registered through normal file
    ///      visibility, but user source still refers to it with the ordinary
    ///      trait name in conformances and static bounds. Centralising the
    ///      lookup here removes the previous one-off `DISPLAYABLE` helper
    ///      and prevents future one-off helpers for every new core cast
    ///      trait.
    pub(crate) fn core_trait_id_for_name(
        &self,
        trait_name: StringId,
        string_table: &StringTable,
    ) -> Option<TraitId> {
        self.core_traits_by_name
            .get(string_table.resolve(trait_name))
            .copied()
    }

    /// Returns the recorded `CoreTraitKind` for a `TraitId`, or `None` when
    /// the trait is not compiler-owned.
    #[allow(dead_code)] // Used by the cast surface tests and downstream phase 4 callers.
    pub(crate) fn core_trait_kind(&self, trait_id: TraitId) -> Option<CoreTraitKind> {
        self.core_trait_kinds.get(&trait_id).copied()
    }

    /// Returns the registered `TraitId` for a static core trait name.
    ///
    /// WHAT: bypasses the `StringTable`-bound `core_trait_id_for_name` helper
    ///      for the AST environment builder, which always knows the static
    ///      source name from the cast catalogue.
    /// WHY: registration code iterates the static core cast trait table and
    ///      needs the trait id without re-interning the source name through
    ///      the `StringTable`.
    pub(crate) fn core_trait_id_for_static_name(
        &self,
        trait_name: &'static str,
    ) -> Option<TraitId> {
        self.core_traits_by_name.get(trait_name).copied()
    }

    pub(crate) fn next_trait_id(&self) -> TraitId {
        TraitId(self.definitions.len() as u32)
    }

    pub(crate) fn next_requirement_id(&self) -> TraitRequirementId {
        let requirement_count = self
            .definitions
            .iter()
            .map(|definition| definition.requirements.len())
            .sum::<usize>();
        TraitRequirementId(requirement_count as u32)
    }

    fn allocate_trait_id(&self) -> TraitId {
        self.next_trait_id()
    }

    fn allocate_requirement_id(&self) -> TraitRequirementId {
        self.next_requirement_id()
    }
}

pub(crate) fn trait_this_name(string_table: &mut StringTable) -> StringId {
    string_table.intern(TRAIT_THIS_NAME)
}

pub(crate) fn requirement_parameter_from_type(
    name: InternedPath,
    value_mode: ValueMode,
    type_id: TypeId,
    location: SourceLocation,
) -> ResolvedTraitParameter {
    ResolvedTraitParameter {
        name,
        value_mode,
        type_id,
        location,
    }
}
