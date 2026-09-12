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
use crate::compiler_frontend::datatypes::generic_identity_bridge::display_generic_instantiation_key_with_fork;
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
use crate::compiler_frontend::symbols::path_interner::{
    PathId, PathIdRemap, PathInternerFork,
};
use crate::compiler_frontend::symbols::string_interning::{StringIdRemap, StringTable};
use rustc_hash::FxHashMap;
use std::fmt::{Display, Formatter, Result as FmtResult};

#[cfg(test)]
const EMPTY_HIR_LOCATIONS: [HirLocation; 0] = [];

// -------------------------
//  ID & Metadata Types
// -------------------------

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

    /// Exact source span of the call/expression that triggered a compiler temporary.
    pub call_span: Option<SourceSpan>,

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

// -------------------------
//  HIR Side Table
// -------------------------

/// Side-table for reversible AST <-> HIR source mapping plus human-readable names for HIR IDs.
///
/// Source mapping stores exact `SourceSpan` values directly. HIR IDs are process-local semantic
/// keys; no line/column or path-derived location representation is retained here.
#[derive(Debug, Clone, Default)]
pub(crate) struct HirSideTable {
    /// Maps an AST span to one or more HIR locations that were lowered from it.
    ast_to_hir: FxHashMap<SourceSpan, Vec<HirLocation>>,

    /// Maps a HIR location back to the primary AST span it was lowered from.
    hir_to_ast: FxHashMap<HirLocation, SourceSpan>,

    /// Maps a HIR location to the exact source span used for diagnostics.
    hir_to_source: FxHashMap<HirLocation, SourceSpan>,

    /// Exact spans for authored block terminators. Generated terminators have no entry.
    terminator_spans: FxHashMap<BlockId, SourceSpan>,

    // -------------------------------------------------------------------------
    //  Name side-tables. Store canonical path identity.
    //  Rendering and diagnostics derive leaf names from these.
    // -------------------------------------------------------------------------
    local_names: FxHashMap<LocalId, PathId>,
    local_origins: FxHashMap<LocalId, HirLocalOrigin>,
    function_names: FxHashMap<FunctionId, PathId>,
    struct_names: FxHashMap<StructId, PathId>,
    generic_struct_instances: FxHashMap<StructId, GenericInstantiationKey>,
    field_names: FxHashMap<FieldId, PathId>,
    choice_names: FxHashMap<ChoiceId, PathId>,
    generic_choice_instances: FxHashMap<ChoiceId, GenericInstantiationKey>,

    // -------------------------------------------------------------------------
    //  Reactivity side-tables. Store source/template metadata outside the core IR.
    // -------------------------------------------------------------------------
    next_reactive_source_id: u32,
    reactive_sources: FxHashMap<ReactiveSourceId, HirReactiveSource>,
    reactive_source_by_local: FxHashMap<LocalId, ReactiveSourceId>,
    reactive_source_by_path: FxHashMap<PathId, ReactiveSourceId>,
    next_reactive_template_id: u32,
    reactive_templates: FxHashMap<ReactiveTemplateId, HirReactiveTemplate>,
    reactive_template_by_value: FxHashMap<HirValueId, ReactiveTemplateId>,
}

impl HirSideTable {
    // -------------------------
    //  Table Management
    // -------------------------

    #[cfg(test)]
    pub(crate) fn clear(&mut self) {
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

    /// Remap every path identity retained by HIR side metadata after a module-local path merge.
    pub(crate) fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        if remap.is_identity() {
            return;
        }

        for path in self.local_names.values_mut() {
            *path = remap.get(*path);
        }
        for path in self.function_names.values_mut() {
            *path = remap.get(*path);
        }
        for path in self.struct_names.values_mut() {
            *path = remap.get(*path);
        }
        for path in self.field_names.values_mut() {
            *path = remap.get(*path);
        }
        for path in self.choice_names.values_mut() {
            *path = remap.get(*path);
        }
        for key in self.generic_struct_instances.values_mut() {
            key.remap_path_ids(remap);
        }
        for key in self.generic_choice_instances.values_mut() {
            key.remap_path_ids(remap);
        }
        for source in self.reactive_sources.values_mut() {
            source.path = remap.get(source.path);
        }

        let mut reactive_source_by_path = FxHashMap::default();
        for (source_id, source) in &self.reactive_sources {
            reactive_source_by_path.insert(source.path, *source_id);
        }
        self.reactive_source_by_path = reactive_source_by_path;
    }

    /// Remaps string IDs retained by side-table metadata.
    ///
    /// Generic identity keys own both string-backed and path-backed components; only the string
    /// components change during a string-table merge.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        for key in self.generic_struct_instances.values_mut() {
            remap_generic_instantiation_key(key, remap);
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
                .insert(source.path, *source_id);
        }

        for template in self.reactive_templates.values_mut() {
            template.remap_string_ids(remap);
        }
    }

    // -------------------------
    //  Structural Mappings
    // -------------------------

    /// Registers a reversible mapping between an AST span and a HIR location.
    #[inline]
    pub(crate) fn map_ast_to_hir(
        &mut self,
        ast_span: Option<SourceSpan>,
        hir_location: HirLocation,
    ) {
        let Some(ast_span) = ast_span else {
            return;
        };

        let entry = self.ast_to_hir.entry(ast_span).or_default();
        if !entry.contains(&hir_location) {
            entry.push(hir_location);
        }

        self.hir_to_ast.insert(hir_location, ast_span);
    }

    /// Registers an exact source span for a HIR location when one exists.
    #[inline]
    pub(crate) fn map_hir_source_span(
        &mut self,
        hir_location: HirLocation,
        hir_source: Option<SourceSpan>,
    ) {
        if let Some(span) = hir_source {
            self.hir_to_source.insert(hir_location, span);
        }
    }

    /// Helper to map a statement's AST and HIR spans.
    pub(crate) fn map_statement(&mut self, ast_span: Option<SourceSpan>, statement: &HirStatement) {
        let hir_location = HirLocation::Statement(statement.id);

        self.map_ast_to_hir(ast_span, hir_location);
        self.map_hir_source_span(hir_location, statement.span);
    }

    /// Helper to map a value's AST and HIR spans.
    pub(crate) fn map_value(
        &mut self,
        ast_span: Option<SourceSpan>,
        value_id: HirValueId,
        source_span: Option<SourceSpan>,
    ) {
        let hir_location = HirLocation::Value(value_id);

        self.map_ast_to_hir(ast_span, hir_location);
        self.map_hir_source_span(hir_location, source_span);
    }

    /// Helper to map a function's AST and HIR spans.
    pub(crate) fn map_function(&mut self, ast_span: Option<SourceSpan>, function: &HirFunction) {
        let hir_location = HirLocation::Function(function.id);

        self.map_ast_to_hir(ast_span, hir_location);
        self.map_hir_source_span(hir_location, ast_span);
    }

    /// Helper to map a block's AST and HIR spans.
    pub(crate) fn map_block(&mut self, ast_span: Option<SourceSpan>, block: &HirBlock) {
        let hir_location = HirLocation::Block(block.id);

        self.map_ast_to_hir(ast_span, hir_location);
        self.map_hir_source_span(hir_location, ast_span);
    }

    /// Helper to map a block terminator's AST and HIR spans.
    pub(crate) fn map_terminator(&mut self, ast_span: Option<SourceSpan>, block_id: BlockId) {
        let hir_location = HirLocation::Terminator(block_id);

        self.map_ast_to_hir(ast_span, hir_location);
        self.map_hir_source_span(hir_location, ast_span);
    }

    /// Stores the exact authored span for a block terminator.
    pub(crate) fn map_terminator_span(&mut self, block_id: BlockId, span: SourceSpan) {
        self.terminator_spans.insert(block_id, span);
    }

    /// Registers the exact source span for a local variable if provided.
    pub(crate) fn map_local_source(&mut self, local: &HirLocal) {
        self.map_hir_source_span(HirLocation::Local(local.id), local.span);
    }

    // -------------------------
    //  Name & Origin Bindings
    // -------------------------

    /// Binds a human-readable name to a local variable.
    #[inline]
    pub(crate) fn bind_local_name(&mut self, local_id: LocalId, name: PathId) {
        self.local_names.insert(local_id, name);
    }

    /// Binds metadata about the origin of a local variable.
    pub(crate) fn bind_local_origin(
        &mut self,
        local_id: LocalId,
        kind: HirLocalOriginKind,
        call_span: Option<SourceSpan>,
        argument_index: Option<usize>,
    ) {
        self.local_origins.insert(
            local_id,
            HirLocalOrigin {
                kind,
                call_span,
                argument_index,
            },
        );
    }

    /// Binds a human-readable name to a function.
    #[inline]
    pub(crate) fn bind_function_name(&mut self, function_id: FunctionId, name: PathId) {
        self.function_names.insert(function_id, name);
    }

    /// Binds a human-readable name to a struct.
    #[inline]
    pub(crate) fn bind_struct_name(&mut self, struct_id: StructId, name: PathId) {
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
    pub(crate) fn bind_field_name(&mut self, field_id: FieldId, name: PathId) {
        self.field_names.insert(field_id, name);
    }

    /// Binds a human-readable name to a choice.
    #[inline]
    pub(crate) fn bind_choice_name(&mut self, choice_id: ChoiceId, name: PathId) {
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
            self.reactive_source_by_path.insert(source.path, existing);
            self.reactive_sources.insert(existing, source);
            return existing;
        }

        let id = ReactiveSourceId(self.next_reactive_source_id);
        self.next_reactive_source_id += 1;
        source.id = id;

        self.reactive_source_by_local.insert(source.local_id, id);
        self.reactive_source_by_path.insert(source.path, id);
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

    /// Returns the diagnostic source span for the given value, if available.
    #[inline]
    pub(crate) fn value_source_span(&self, value_id: HirValueId) -> Option<SourceSpan> {
        self.hir_source_span_for_hir(HirLocation::Value(value_id))
    }

    /// Returns the original AST source span for the given value, if available.
    #[inline]
    pub(crate) fn value_ast_span(&self, value_id: HirValueId) -> Option<SourceSpan> {
        self.ast_span_for_hir(HirLocation::Value(value_id))
    }

    /// Returns all HIR locations associated with the given AST span.
    #[cfg(test)]
    pub(crate) fn hir_locations_for_ast(&self, ast_span: SourceSpan) -> &[HirLocation] {
        self.ast_to_hir
            .get(&ast_span)
            .map(Vec::as_slice)
            .unwrap_or(&EMPTY_HIR_LOCATIONS)
    }

    /// Returns the original AST source span for the given HIR location.
    #[inline]
    pub(crate) fn ast_span_for_hir(&self, hir_location: HirLocation) -> Option<SourceSpan> {
        self.hir_to_ast.get(&hir_location).copied()
    }

    /// Returns the diagnostic source span for the given HIR location.
    #[inline]
    pub(crate) fn hir_source_span_for_hir(&self, hir_location: HirLocation) -> Option<SourceSpan> {
        self.hir_to_source.get(&hir_location).copied()
    }

    /// Returns the exact authored span for a block terminator, if one was recorded.
    #[inline]
    pub(crate) fn terminator_span(&self, block_id: BlockId) -> Option<&SourceSpan> {
        self.terminator_spans.get(&block_id)
    }

    /// Returns the interned path for a local variable.
    #[inline]
    pub(crate) fn local_name_path(&self, local_id: LocalId) -> Option<PathId> {
        self.local_names.get(&local_id).copied()
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
    pub(crate) fn function_name_path(&self, function_id: FunctionId) -> Option<PathId> {
        self.function_names.get(&function_id).copied()
    }

    #[inline]
    pub(crate) fn reactive_source_id_for_local(
        &self,
        local_id: LocalId,
    ) -> Option<ReactiveSourceId> {
        self.reactive_source_by_local.get(&local_id).copied()
    }

    #[inline]
    pub(crate) fn reactive_source_id_for_path(&self, path: PathId) -> Option<ReactiveSourceId> {
        self.reactive_source_by_path.get(&path).copied()
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

    #[inline]
    pub(crate) fn struct_name_path(&self, struct_id: StructId) -> Option<PathId> {
        self.struct_names.get(&struct_id).copied()
    }
    #[inline]
    pub(crate) fn field_name_path(&self, field_id: FieldId) -> Option<PathId> {
        self.field_names.get(&field_id).copied()
    }

    #[inline]
    pub(crate) fn choice_name_path(&self, choice_id: ChoiceId) -> Option<PathId> {
        self.choice_names.get(&choice_id).copied()
    }

    /// Returns the interned path for a field.
    /// Resolves a choice name to its leaf component.
    #[inline]
    #[cfg(any(test, feature = "show_hir"))]
    pub(crate) fn resolve_choice_name<'a>(
        &self,
        choice_id: ChoiceId,
        path_fork: &PathInternerFork,
        string_table: &'a StringTable,
    ) -> Option<&'a str> {
        self.choice_name_path(choice_id)
            .and_then(|path| path_fork.component(path))
            .map(|component| string_table.resolve(component))
    }

    /// Returns a human-readable display name for a choice, including generic arguments if applicable.
    #[cfg(any(test, feature = "show_hir"))]
    pub(crate) fn display_choice_name(
        &self,
        choice_id: ChoiceId,
        path_fork: &PathInternerFork,
        string_table: &StringTable,
    ) -> Option<String> {
        if let Some(key) = self.generic_choice_instances.get(&choice_id) {
            return Some(display_generic_instantiation_key_with_fork(
                key,
                path_fork,
                string_table,
            ));
        }

        self.resolve_choice_name(choice_id, path_fork, string_table)
            .map(str::to_owned)
    }

    /// Resolves a local variable name to its leaf component.
    #[inline]
    pub(crate) fn resolve_local_name<'a>(
        &self,
        local_id: LocalId,
        path_fork: &PathInternerFork,
        string_table: &'a StringTable,
    ) -> Option<&'a str> {
        self.local_name_path(local_id)
            .and_then(|path| path_fork.component(path))
            .map(|component| string_table.resolve(component))
    }

    /// Resolves a function name to its leaf component.
    #[inline]
    pub(crate) fn resolve_function_name<'a>(
        &self,
        function_id: FunctionId,
        path_fork: &PathInternerFork,
        string_table: &'a StringTable,
    ) -> Option<&'a str> {
        self.function_name_path(function_id)
            .and_then(|path| path_fork.component(path))
            .map(|component| string_table.resolve(component))
    }

    /// Resolves a struct name to its leaf component.
    #[inline]
    #[cfg(any(test, feature = "show_hir"))]
    pub(crate) fn resolve_struct_name<'a>(
        &self,
        struct_id: StructId,
        path_fork: &PathInternerFork,
        string_table: &'a StringTable,
    ) -> Option<&'a str> {
        self.struct_name_path(struct_id)
            .and_then(|path| path_fork.component(path))
            .map(|component| string_table.resolve(component))
    }

    /// Returns a human-readable display name for a struct, including generic arguments if applicable.
    #[cfg(any(test, feature = "show_hir"))]
    pub(crate) fn display_struct_name(
        &self,
        struct_id: StructId,
        path_fork: &PathInternerFork,
        string_table: &StringTable,
    ) -> Option<String> {
        if let Some(key) = self.generic_struct_instances.get(&struct_id) {
            return Some(display_generic_instantiation_key_with_fork(
                key,
                path_fork,
                string_table,
            ));
        }

        self.resolve_struct_name(struct_id, path_fork, string_table)
            .map(str::to_owned)
    }

    /// Resolves a field name to its leaf component.
    #[inline]
    #[cfg(any(test, feature = "show_hir"))]
    pub(crate) fn resolve_field_name<'a>(
        &self,
        field_id: FieldId,
        path_fork: &PathInternerFork,
        string_table: &'a StringTable,
    ) -> Option<&'a str> {
        self.field_name_path(field_id)
            .and_then(|path| path_fork.component(path))
            .map(|component| string_table.resolve(component))
    }
}

/// Recursively visits string-backed generic identity payloads.
///
/// Path identities themselves are stable `PathId`s and therefore do not participate in string
/// table remapping.
fn remap_generic_instantiation_key(key: &mut GenericInstantiationKey, remap: &StringIdRemap) {
    for argument in &mut key.arguments {
        remap_type_identity_key(argument, remap);
    }
}

/// Recursively visits string-backed generic identity payloads.
fn remap_type_identity_key(key: &mut TypeIdentityKey, remap: &StringIdRemap) {
    match key {
        TypeIdentityKey::Nominal(_) => {}
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
