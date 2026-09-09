//! HIR source-mapping and human-readable-name side tables.
//!
//! This module owns the reversible AST/HIR location mapping and the canonical path identity used
//! by diagnostics, borrow checking, and debug rendering.
//!
//! The HIR side table is a central repository for metadata that doesn't belong directly in the
//! HIR tree (which is meant to be lean and easy to transform). It allows us to:
//! - Map HIR nodes back to their original AST locations for diagnostics.
//! - Track human-readable names for locals, functions, and types.
//! - Intern source locations to save memory and allow O(1) identity checks.

#[cfg(any(test, feature = "show_hir"))]
use crate::compiler_frontend::datatypes::generic_identity_bridge::display_generic_instantiation_key;
use crate::compiler_frontend::datatypes::generic_identity_bridge::{
    GenericInstantiationKey, TypeIdentityKey,
};
use crate::compiler_frontend::hir::blocks::{HirBlock, HirLocal};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{
    BlockId, ChoiceId, FieldId, FunctionId, HirNodeId, HirValueId, LocalId, StructId,
};
use crate::compiler_frontend::hir::reactivity::{
    HirReactiveSource, HirReactiveTemplate, ReactiveSourceId, ReactiveTemplateId,
};
use crate::compiler_frontend::hir::statements::HirStatement;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringIdRemap, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use rustc_hash::FxHashMap;
use std::fmt::{Display, Formatter, Result as FmtResult};

#[cfg(test)]
const EMPTY_HIR_LOCATIONS: [HirLocation; 0] = [];

// -------------------------
//  ID & Metadata Types
// -------------------------

/// Stable identifier for an interned source location.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SourceLocationId(pub u32);

/// Defines the origin of a local variable, helping diagnostics distinguish between
/// user-defined variables and compiler-generated temporaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum HirLocalOriginKind {
    /// A variable explicitly declared by the user in the source code.
    User,

    /// A temporary variable created by the compiler during lowering (e.g., for complex expressions).
    CompilerTemp,

    /// A special mutable argument created for internal bookkeeping.
    CompilerFreshMutableArg,
}

/// Metadata about where a local variable came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct HirLocalOrigin {
    pub kind: HirLocalOriginKind,

    /// If this is a compiler temporary, this may point to the call site or expression that
    /// triggered its creation.
    pub call_location: Option<SourceLocationId>,

    /// If this is a function argument, its 0-based index.
    pub argument_index: Option<usize>,
}

// -------------------------
//  HIR Location Mapping
// -------------------------

/// Canonical references into the HIR graph for source mapping.
///
/// This enum allows us to use a single side-table key for any kind of HIR construct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum HirLocation {
    Block(BlockId),
    Function(FunctionId),
    Struct(StructId),
    Field(FieldId),
    Local(LocalId),
    Statement(HirNodeId),
    Value(HirValueId),
    Terminator(BlockId),
    Choice(ChoiceId),
}

impl Display for HirLocation {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            HirLocation::Block(id) => write!(f, "block({id})"),
            HirLocation::Function(id) => write!(f, "function({id})"),
            HirLocation::Struct(id) => write!(f, "struct({id})"),
            HirLocation::Field(id) => write!(f, "field({id})"),
            HirLocation::Local(id) => write!(f, "local({id})"),
            HirLocation::Statement(id) => write!(f, "statement({id})"),
            HirLocation::Value(id) => write!(f, "value({id})"),
            HirLocation::Terminator(block) => write!(f, "terminator({block})"),
            HirLocation::Choice(id) => write!(f, "choice({})", id.0),
        }
    }
}

impl From<BlockId> for HirLocation {
    fn from(value: BlockId) -> Self {
        HirLocation::Block(value)
    }
}

impl From<FunctionId> for HirLocation {
    fn from(value: FunctionId) -> Self {
        HirLocation::Function(value)
    }
}

impl From<StructId> for HirLocation {
    fn from(value: StructId) -> Self {
        HirLocation::Struct(value)
    }
}

impl From<FieldId> for HirLocation {
    fn from(value: FieldId) -> Self {
        HirLocation::Field(value)
    }
}

impl From<LocalId> for HirLocation {
    fn from(value: LocalId) -> Self {
        HirLocation::Local(value)
    }
}

impl From<HirNodeId> for HirLocation {
    fn from(value: HirNodeId) -> Self {
        HirLocation::Statement(value)
    }
}

impl From<ChoiceId> for HirLocation {
    fn from(value: ChoiceId) -> Self {
        HirLocation::Choice(value)
    }
}

impl From<HirValueId> for HirLocation {
    fn from(value: HirValueId) -> Self {
        HirLocation::Value(value)
    }
}

/// A simplified, hashable version of `SourceLocation` used for interning.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SourceLocationKey {
    scope: InternedPath,
    start_line: i32,
    start_column: i32,
    end_line: i32,
    end_column: i32,
}

impl From<&SourceLocation> for SourceLocationKey {
    fn from(value: &SourceLocation) -> Self {
        Self {
            scope: value.scope.clone(),
            start_line: value.start_pos.line_number,
            start_column: value.start_pos.char_column,
            end_line: value.end_pos.line_number,
            end_column: value.end_pos.char_column,
        }
    }
}

// -------------------------
//  HIR Side Table
// -------------------------

/// Side-table for reversible AST <-> HIR source mapping plus human-readable names for HIR IDs.
///
/// Design goals:
/// - O(1) average lookups for all forward/backward mappings.
/// - Location interning to avoid repeated `SourceLocation` cloning.
/// - Zero string formatting work during mapping writes.
#[derive(Debug, Clone, Default)]
pub(crate) struct HirSideTable {
    /// Interned source locations.
    source_locations: Vec<SourceLocation>,

    /// Index for O(1) location interning.
    source_location_index: FxHashMap<SourceLocationKey, SourceLocationId>,

    /// Maps an AST location to one or more HIR locations that were lowered from it.
    ast_to_hir: FxHashMap<SourceLocationId, Vec<HirLocation>>,

    /// Maps a HIR location back to the primary AST location it was lowered from.
    hir_to_ast: FxHashMap<HirLocation, SourceLocationId>,

    /// Maps a HIR location to the source location that should be used for diagnostics.
    /// This may differ from `hir_to_ast` for compiler-generated code that should point to a
    /// specific expression or statement.
    hir_to_source: FxHashMap<HirLocation, SourceLocationId>,

    /// Exact spans for authored block terminators. Generated terminators have no entry.
    terminator_spans: FxHashMap<BlockId, SourceSpan>,

    // -------------------------------------------------------------------------
    //  Name side-tables. Store canonical path identity.
    //  Rendering and diagnostics derive leaf names from these.
    // -------------------------------------------------------------------------
    local_names: FxHashMap<LocalId, InternedPath>,
    local_origins: FxHashMap<LocalId, HirLocalOrigin>,
    function_names: FxHashMap<FunctionId, InternedPath>,
    struct_names: FxHashMap<StructId, InternedPath>,
    generic_struct_instances: FxHashMap<StructId, GenericInstantiationKey>,
    field_names: FxHashMap<FieldId, InternedPath>,
    choice_names: FxHashMap<ChoiceId, InternedPath>,
    generic_choice_instances: FxHashMap<ChoiceId, GenericInstantiationKey>,

    // -------------------------------------------------------------------------
    //  Reactivity side-tables. Store source/template metadata outside the core IR.
    // -------------------------------------------------------------------------
    next_reactive_source_id: u32,
    reactive_sources: FxHashMap<ReactiveSourceId, HirReactiveSource>,
    reactive_source_by_local: FxHashMap<LocalId, ReactiveSourceId>,
    reactive_source_by_path: FxHashMap<InternedPath, ReactiveSourceId>,
    next_reactive_template_id: u32,
    reactive_templates: FxHashMap<ReactiveTemplateId, HirReactiveTemplate>,
    reactive_template_by_value: FxHashMap<HirValueId, ReactiveTemplateId>,
}

impl HirSideTable {
    // -------------------------
    //  Table Management
    // -------------------------

    /// Clears all stored mappings. Used primarily in tests to ensure a clean state.
    #[cfg(test)]
    pub(crate) fn clear(&mut self) {
        self.source_locations.clear();
        self.source_location_index.clear();
        self.ast_to_hir.clear();
        self.hir_to_ast.clear();
        self.hir_to_source.clear();
        self.local_names.clear();
        self.terminator_spans.clear();
        self.local_origins.clear();
        self.function_names.clear();
        self.struct_names.clear();
        self.generic_struct_instances.clear();
        self.field_names.clear();
        self.choice_names.clear();
        self.generic_choice_instances.clear();
        self.next_reactive_source_id = 0;
        self.reactive_sources.clear();
        self.reactive_source_by_local.clear();
        self.reactive_source_by_path.clear();
        self.next_reactive_template_id = 0;
        self.reactive_templates.clear();
        self.reactive_template_by_value.clear();
    }

    /// Remaps all `StringId`s within the side-table using the provided remap table.
    /// This is necessary during incremental compilation or when merging string tables.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        // Remap all stored source locations.
        for location in &mut self.source_locations {
            location.remap_string_ids(remap);
        }

        // Rebuild the index because SourceLocationKey contains InternedPath, which may have changed.
        self.source_location_index.clear();
        for (i, location) in self.source_locations.iter().enumerate() {
            let key = SourceLocationKey::from(location);
            self.source_location_index
                .insert(key, SourceLocationId(i as u32));
        }

        // Remap all name side-tables.
        for path in self.local_names.values_mut() {
            path.remap_string_ids(remap);
        }

        for path in self.function_names.values_mut() {
            path.remap_string_ids(remap);
        }

        for path in self.struct_names.values_mut() {
            path.remap_string_ids(remap);
        }

        for key in self.generic_struct_instances.values_mut() {
            remap_generic_instantiation_key(key, remap);
        }

        for path in self.field_names.values_mut() {
            path.remap_string_ids(remap);
        }

        for path in self.choice_names.values_mut() {
            path.remap_string_ids(remap);
        }

        for key in self.generic_choice_instances.values_mut() {
            remap_generic_instantiation_key(key, remap);
        }

        for source in self.reactive_sources.values_mut() {
            source.remap_string_ids(remap);
        }

        self.reactive_source_by_path.clear();
        for (source_id, source) in &self.reactive_sources {
            self.reactive_source_by_path
                .insert(source.path.clone(), *source_id);
        }

        for template in self.reactive_templates.values_mut() {
            template.remap_string_ids(remap);
        }
    }

    // -------------------------
    //  Location Interning
    // -------------------------

    /// Interns a `SourceLocation` and returns a stable `SourceLocationId`.
    /// If the location has already been interned, its existing ID is returned.
    #[inline]
    pub(crate) fn intern_source_location(&mut self, location: &SourceLocation) -> SourceLocationId {
        let key = SourceLocationKey::from(location);

        if let Some(existing_id) = self.source_location_index.get(&key) {
            return *existing_id;
        }

        let new_id = SourceLocationId(self.source_locations.len() as u32);
        self.source_locations.push(location.clone());
        self.source_location_index.insert(key, new_id);

        new_id
    }

    /// Returns the `SourceLocation` associated with the given ID, if it exists.
    #[inline]
    pub(crate) fn source_location(&self, id: SourceLocationId) -> Option<&SourceLocation> {
        self.source_locations.get(id.0 as usize)
    }

    /// Returns the `SourceLocationId` for the given `SourceLocation` if it has been interned.
    #[cfg(test)]
    #[inline]
    pub(crate) fn source_id_for_location(
        &self,
        location: &SourceLocation,
    ) -> Option<SourceLocationId> {
        let key = SourceLocationKey::from(location);
        self.source_location_index.get(&key).copied()
    }

    // -------------------------
    //  Structural Mappings
    // -------------------------

    /// Registers a reversible mapping between an AST location and a HIR location.
    #[inline]
    pub(crate) fn map_ast_to_hir(
        &mut self,
        ast_location: &SourceLocation,
        hir_location: HirLocation,
    ) {
        let ast_id = self.intern_source_location(ast_location);

        // Forward mapping: one AST location can map to multiple HIR nodes.
        let entry = self.ast_to_hir.entry(ast_id).or_default();
        if !entry.contains(&hir_location) {
            entry.push(hir_location);
        }

        // Backward mapping: each HIR node has one primary AST source.
        self.hir_to_ast.insert(hir_location, ast_id);
    }

    /// Registers a diagnostic source location for a HIR location.
    #[inline]
    pub(crate) fn map_hir_source_location(
        &mut self,
        hir_location: HirLocation,
        hir_source: &SourceLocation,
    ) {
        let source_id = self.intern_source_location(hir_source);
        self.hir_to_source.insert(hir_location, source_id);
    }

    /// Helper to map a statement's AST and HIR locations.
    pub(crate) fn map_statement(
        &mut self,
        ast_location: &SourceLocation,
        statement: &HirStatement,
    ) {
        let hir_location = HirLocation::Statement(statement.id);

        self.map_ast_to_hir(ast_location, hir_location);
        self.map_hir_source_location(hir_location, &statement.location);
    }

    /// Helper to map a value's AST and HIR locations.
    pub(crate) fn map_value(
        &mut self,
        ast_location: &SourceLocation,
        value_id: HirValueId,
        source_location: &SourceLocation,
    ) {
        let hir_location = HirLocation::Value(value_id);

        self.map_ast_to_hir(ast_location, hir_location);
        self.map_hir_source_location(hir_location, source_location);
    }

    /// Helper to map a function's AST and HIR locations.
    pub(crate) fn map_function(&mut self, ast_location: &SourceLocation, function: &HirFunction) {
        let hir_location = HirLocation::Function(function.id);

        self.map_ast_to_hir(ast_location, hir_location);
        self.map_hir_source_location(hir_location, ast_location);
    }

    /// Helper to map a block's AST and HIR locations.
    pub(crate) fn map_block(&mut self, ast_location: &SourceLocation, block: &HirBlock) {
        let hir_location = HirLocation::Block(block.id);

        self.map_ast_to_hir(ast_location, hir_location);
        self.map_hir_source_location(hir_location, ast_location);
    }

    /// Helper to map a block terminator's AST and HIR locations.
    pub(crate) fn map_terminator(&mut self, ast_location: &SourceLocation, block_id: BlockId) {
        let hir_location = HirLocation::Terminator(block_id);

        self.map_ast_to_hir(ast_location, hir_location);
        self.map_hir_source_location(hir_location, ast_location);
    }

    /// Stores the exact authored span for a block terminator.
    pub(crate) fn map_terminator_span(&mut self, block_id: BlockId, span: SourceSpan) {
        self.terminator_spans.insert(block_id, span);
    }

    /// Registers the source location for a local variable if provided.
    pub(crate) fn map_local_source(&mut self, local: &HirLocal) {
        if let Some(location) = &local.source_info {
            self.map_hir_source_location(HirLocation::Local(local.id), location);
        }
    }

    // -------------------------
    //  Name & Origin Bindings
    // -------------------------

    /// Binds a human-readable name to a local variable.
    #[inline]
    pub(crate) fn bind_local_name(&mut self, local_id: LocalId, name: InternedPath) {
        self.local_names.insert(local_id, name);
    }

    /// Binds metadata about the origin of a local variable.
    pub(crate) fn bind_local_origin(
        &mut self,
        local_id: LocalId,
        kind: HirLocalOriginKind,
        call_location: Option<&SourceLocation>,
        argument_index: Option<usize>,
    ) {
        let call_location = call_location.map(|location| self.intern_source_location(location));

        self.local_origins.insert(
            local_id,
            HirLocalOrigin {
                kind,
                call_location,
                argument_index,
            },
        );
    }

    /// Binds a human-readable name to a function.
    #[inline]
    pub(crate) fn bind_function_name(&mut self, function_id: FunctionId, name: InternedPath) {
        self.function_names.insert(function_id, name);
    }

    /// Binds a human-readable name to a struct.
    #[inline]
    pub(crate) fn bind_struct_name(&mut self, struct_id: StructId, name: InternedPath) {
        self.struct_names.insert(struct_id, name);
    }

    /// Binds a generic instantiation key to a struct ID.
    #[inline]
    pub(crate) fn bind_generic_struct_instance(
        &mut self,
        struct_id: StructId,
        key: GenericInstantiationKey,
    ) {
        self.generic_struct_instances.insert(struct_id, key);
    }

    /// Binds a human-readable name to a field.
    #[inline]
    pub(crate) fn bind_field_name(&mut self, field_id: FieldId, name: InternedPath) {
        self.field_names.insert(field_id, name);
    }

    /// Binds a human-readable name to a choice.
    #[inline]
    pub(crate) fn bind_choice_name(&mut self, choice_id: ChoiceId, name: InternedPath) {
        self.choice_names.insert(choice_id, name);
    }

    /// Binds a generic instantiation key to a choice ID.
    #[inline]
    pub(crate) fn bind_generic_choice_instance(
        &mut self,
        choice_id: ChoiceId,
        key: GenericInstantiationKey,
    ) {
        self.generic_choice_instances.insert(choice_id, key);
    }

    /// Binds a local as a stable reactive source.
    ///
    /// WHAT: allocates a HIR `ReactiveSourceId` and indexes it by both local and AST path.
    /// WHY: template dependencies name AST-resolved sources by path, while later HIR and backend
    /// consumers need stable local/source IDs.
    pub(crate) fn bind_reactive_source(
        &mut self,
        mut source: HirReactiveSource,
    ) -> ReactiveSourceId {
        if let Some(existing) = self.reactive_source_by_local.get(&source.local_id).copied() {
            if let Some(previous_source) = self.reactive_sources.get(&existing) {
                self.reactive_source_by_path.remove(&previous_source.path);
            }

            source.id = existing;
            self.reactive_source_by_path
                .insert(source.path.clone(), existing);
            self.reactive_sources.insert(existing, source);
            return existing;
        }

        let id = ReactiveSourceId(self.next_reactive_source_id);
        self.next_reactive_source_id += 1;
        source.id = id;

        self.reactive_source_by_local.insert(source.local_id, id);
        self.reactive_source_by_path.insert(source.path.clone(), id);
        self.reactive_sources.insert(id, source);

        id
    }

    /// Binds reactive template metadata to one lowered HIR value.
    pub(crate) fn bind_reactive_template(
        &mut self,
        mut template: HirReactiveTemplate,
    ) -> ReactiveTemplateId {
        if let Some(existing) = self
            .reactive_template_by_value
            .get(&template.value_id)
            .copied()
        {
            template.id = existing;
            self.reactive_templates.insert(existing, template);
            return existing;
        }

        let id = ReactiveTemplateId(self.next_reactive_template_id);
        self.next_reactive_template_id += 1;
        template.id = id;

        self.reactive_template_by_value
            .insert(template.value_id, id);
        self.reactive_templates.insert(id, template);

        id
    }

    // -------------------------
    //  Metadata Lookups
    // -------------------------

    /// Returns the diagnostic source location for the given value, if available.
    #[inline]
    pub(crate) fn value_source_location(&self, value_id: HirValueId) -> Option<&SourceLocation> {
        self.hir_source_location_for_hir(HirLocation::Value(value_id))
    }

    /// Returns the original AST source location for the given value, if available.
    #[inline]
    pub(crate) fn value_ast_location(&self, value_id: HirValueId) -> Option<&SourceLocation> {
        self.ast_location_for_hir(HirLocation::Value(value_id))
    }

    /// Returns all HIR locations associated with the given AST location.
    #[cfg(test)]
    pub(crate) fn hir_locations_for_ast(&self, ast_location: &SourceLocation) -> &[HirLocation] {
        let Some(ast_id) = self.source_id_for_location(ast_location) else {
            return &EMPTY_HIR_LOCATIONS;
        };

        self.ast_to_hir
            .get(&ast_id)
            .map(Vec::as_slice)
            .unwrap_or(&EMPTY_HIR_LOCATIONS)
    }

    /// Returns the AST `SourceLocationId` for the given HIR location.
    #[inline]
    pub(crate) fn ast_source_id_for_hir(
        &self,
        hir_location: HirLocation,
    ) -> Option<SourceLocationId> {
        self.hir_to_ast.get(&hir_location).copied()
    }

    /// Returns the original AST `SourceLocation` for the given HIR location.
    #[inline]
    pub(crate) fn ast_location_for_hir(
        &self,
        hir_location: HirLocation,
    ) -> Option<&SourceLocation> {
        let source_id = self.ast_source_id_for_hir(hir_location)?;
        self.source_location(source_id)
    }

    /// Returns the diagnostic `SourceLocationId` for the given HIR location.
    #[inline]
    pub(crate) fn hir_source_id_for_hir(
        &self,
        hir_location: HirLocation,
    ) -> Option<SourceLocationId> {
        self.hir_to_source.get(&hir_location).copied()
    }

    /// Returns the diagnostic `SourceLocation` for the given HIR location.
    #[inline]
    pub(crate) fn hir_source_location_for_hir(
        &self,
        hir_location: HirLocation,
    ) -> Option<&SourceLocation> {
        let source_id = self.hir_source_id_for_hir(hir_location)?;
        self.source_location(source_id)
    }

    /// Returns the exact authored span for a block terminator, if one was recorded.
    #[inline]
    pub(crate) fn terminator_span(&self, block_id: BlockId) -> Option<&SourceSpan> {
        self.terminator_spans.get(&block_id)
    }

    /// Returns the interned path for a local variable.
    #[inline]
    pub(crate) fn local_name_path(&self, local_id: LocalId) -> Option<&InternedPath> {
        self.local_names.get(&local_id)
    }

    /// Returns origin metadata for a local variable.
    #[inline]
    pub(crate) fn local_origin(&self, local_id: LocalId) -> Option<HirLocalOrigin> {
        self.local_origins.get(&local_id).copied()
    }

    /// Returns the origin kind for a local variable.
    #[inline]
    pub(crate) fn local_origin_kind(&self, local_id: LocalId) -> Option<HirLocalOriginKind> {
        self.local_origin(local_id).map(|origin| origin.kind)
    }

    /// Returns the interned path for a function.
    #[inline]
    pub(crate) fn function_name_path(&self, function_id: FunctionId) -> Option<&InternedPath> {
        self.function_names.get(&function_id)
    }

    #[inline]
    pub(crate) fn reactive_source_id_for_local(
        &self,
        local_id: LocalId,
    ) -> Option<ReactiveSourceId> {
        self.reactive_source_by_local.get(&local_id).copied()
    }

    #[inline]
    pub(crate) fn reactive_source_id_for_path(
        &self,
        path: &InternedPath,
    ) -> Option<ReactiveSourceId> {
        self.reactive_source_by_path.get(path).copied()
    }

    #[inline]
    pub(crate) fn reactive_source(
        &self,
        source_id: ReactiveSourceId,
    ) -> Option<&HirReactiveSource> {
        self.reactive_sources.get(&source_id)
    }

    #[inline]
    pub(crate) fn reactive_sources(&self) -> impl Iterator<Item = &HirReactiveSource> {
        self.reactive_sources.values()
    }

    #[inline]
    pub(crate) fn reactive_template_id_for_value(
        &self,
        value_id: HirValueId,
    ) -> Option<ReactiveTemplateId> {
        self.reactive_template_by_value.get(&value_id).copied()
    }

    #[inline]
    pub(crate) fn reactive_template_for_value(
        &self,
        value_id: HirValueId,
    ) -> Option<&HirReactiveTemplate> {
        let template_id = self.reactive_template_id_for_value(value_id)?;
        self.reactive_templates.get(&template_id)
    }

    #[inline]
    pub(crate) fn reactive_templates(&self) -> impl Iterator<Item = &HirReactiveTemplate> {
        self.reactive_templates.values()
    }

    /// Returns the interned path for a struct.
    #[inline]
    pub(crate) fn struct_name_path(&self, struct_id: StructId) -> Option<&InternedPath> {
        self.struct_names.get(&struct_id)
    }

    /// Returns the interned path for a field.
    #[inline]
    pub(crate) fn field_name_path(&self, field_id: FieldId) -> Option<&InternedPath> {
        self.field_names.get(&field_id)
    }

    /// Returns the interned path for a choice.
    #[inline]
    #[cfg(any(test, feature = "show_hir"))]
    pub(crate) fn choice_name_path(&self, choice_id: ChoiceId) -> Option<&InternedPath> {
        self.choice_names.get(&choice_id)
    }

    /// Resolves a choice name to a string.
    #[inline]
    #[cfg(any(test, feature = "show_hir"))]
    pub(crate) fn resolve_choice_name<'a>(
        &self,
        choice_id: ChoiceId,
        string_table: &'a StringTable,
    ) -> Option<&'a str> {
        self.choice_name_path(choice_id)
            .and_then(|path| path.name_str(string_table))
    }

    /// Returns a human-readable display name for a choice, including generic arguments if applicable.
    #[cfg(any(test, feature = "show_hir"))]
    pub(crate) fn display_choice_name(
        &self,
        choice_id: ChoiceId,
        string_table: &StringTable,
    ) -> Option<String> {
        if let Some(key) = self.generic_choice_instances.get(&choice_id) {
            return Some(display_generic_instantiation_key(key, string_table));
        }

        self.resolve_choice_name(choice_id, string_table)
            .map(str::to_owned)
    }

    /// Resolves a local variable name to a string.
    #[inline]
    pub(crate) fn resolve_local_name<'a>(
        &self,
        local_id: LocalId,
        string_table: &'a StringTable,
    ) -> Option<&'a str> {
        self.local_name_path(local_id)
            .and_then(|path| path.name_str(string_table))
    }

    /// Resolves a function name to a string.
    #[inline]
    pub(crate) fn resolve_function_name<'a>(
        &self,
        function_id: FunctionId,
        string_table: &'a StringTable,
    ) -> Option<&'a str> {
        self.function_name_path(function_id)
            .and_then(|path| path.name_str(string_table))
    }

    /// Resolves a struct name to a string.
    #[inline]
    #[cfg(any(test, feature = "show_hir"))]
    pub(crate) fn resolve_struct_name<'a>(
        &self,
        struct_id: StructId,
        string_table: &'a StringTable,
    ) -> Option<&'a str> {
        self.struct_name_path(struct_id)
            .and_then(|path| path.name_str(string_table))
    }

    /// Returns a human-readable display name for a struct, including generic arguments if applicable.
    #[cfg(any(test, feature = "show_hir"))]
    pub(crate) fn display_struct_name(
        &self,
        struct_id: StructId,
        string_table: &StringTable,
    ) -> Option<String> {
        if let Some(key) = self.generic_struct_instances.get(&struct_id) {
            return Some(display_generic_instantiation_key(key, string_table));
        }

        self.resolve_struct_name(struct_id, string_table)
            .map(str::to_owned)
    }

    /// Resolves a field name to a string.
    #[inline]
    #[cfg(any(test, feature = "show_hir"))]
    pub(crate) fn resolve_field_name<'a>(
        &self,
        field_id: FieldId,
        string_table: &'a StringTable,
    ) -> Option<&'a str> {
        self.field_name_path(field_id)
            .and_then(|path| path.name_str(string_table))
    }
}
// -------------------------
//  Helper Functions
// -------------------------

/// Remaps all `StringId`s within a `GenericInstantiationKey`.
fn remap_generic_instantiation_key(key: &mut GenericInstantiationKey, remap: &StringIdRemap) {
    key.base_path.remap_string_ids(remap);

    for argument in &mut key.arguments {
        remap_type_identity_key(argument, remap);
    }
}

/// Remaps all `StringId`s within a `TypeIdentityKey`.
fn remap_type_identity_key(key: &mut TypeIdentityKey, remap: &StringIdRemap) {
    match key {
        TypeIdentityKey::Nominal(path) => path.remap_string_ids(remap),

        TypeIdentityKey::Collection { element: inner, .. } | TypeIdentityKey::Option(inner) => {
            remap_type_identity_key(inner, remap)
        }
        TypeIdentityKey::Map { key, value } => {
            remap_type_identity_key(key, remap);
            remap_type_identity_key(value, remap);
        }

        TypeIdentityKey::FallibleCarrier { success, error } => {
            remap_type_identity_key(success, remap);
            remap_type_identity_key(error, remap);
        }

        TypeIdentityKey::GenericInstance(instance) => {
            remap_generic_instantiation_key(instance, remap);
        }

        TypeIdentityKey::Builtin(_) | TypeIdentityKey::External(_) => {}
    }
}
