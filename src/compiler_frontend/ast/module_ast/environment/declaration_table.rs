//! Stable top-level declaration table for AST environment construction.
//!
//! WHAT: stores one slot per top-level declaration discovered by the header/dependency stages.
//! WHY: AST environment construction updates placeholders in place as declarations are resolved,
//! so body emission and type resolution can share one indexed declaration source without
//! reconstructing lookup indexes.
//!
//! Owned by the AST environment builder and consumed by AST emission, `ScopeContext`, and
//! finalization. Each table owns flat semantic slots and construction indexes. Generated
//! environments restore their declarations from stable canonical inputs rather than inheriting
//! requester tables.

use crate::compiler_frontend::ast::ast_nodes::{Declaration, DeclarationValue};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::headers::module_symbols::{
    DeclarationId, OrderedSemanticDeclaration, OrderedSemanticDeclarationKind,
};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringId;
use rustc_hash::{FxHashMap, FxHashSet};

/// Module constants that have completed dependency-ordered semantic resolution.
///
/// Top-level declarations use dense table identity here. Body-local `#` declarations remain in
/// lexical scope frames because they have no top-level `DeclarationId`.
#[derive(Clone, Debug, Default)]
pub(crate) struct ResolvedConstantSet {
    declarations: Vec<bool>,
}

impl ResolvedConstantSet {
    pub(in crate::compiler_frontend::ast) fn insert(&mut self, declaration_id: DeclarationId) {
        if self.declarations.len() <= declaration_id.index() {
            self.declarations.resize(declaration_id.index() + 1, false);
        }
        self.declarations[declaration_id.index()] = true;
    }

    pub(in crate::compiler_frontend::ast) fn contains(
        &self,
        declaration_id: DeclarationId,
    ) -> bool {
        self.declarations
            .get(declaration_id.index())
            .copied()
            .unwrap_or(false)
    }

    pub(in crate::compiler_frontend::ast) fn iter(
        &self,
    ) -> impl Iterator<Item = DeclarationId> + '_ {
        self.declarations
            .iter()
            .enumerate()
            .filter_map(|(index, resolved)| resolved.then_some(DeclarationId::from_index(index)))
    }
}

/// Indexed table of all top-level declarations in a module.
///
/// Provides fast path-based and name-based lookups with optional visibility filtering.
/// Declarations are stored in dependency-sorted order and indexed by `DeclarationId`.
#[derive(Debug)]
pub(crate) struct TopLevelDeclarationTable {
    /// Dense semantic slots followed by compiler/generated declarations.
    ///
    /// Compile-time-only aliases and traits own stable IDs but no value declaration row.
    declarations: Vec<Option<Declaration>>,
    /// Semantic paths, including metadata-only rows, plus compiler/generated declaration paths.
    by_path: FxHashMap<PathId, DeclarationId>,
    /// Name-to-IDs map for declarations that carry a simple name.
    ///
    /// Multiple declarations may share a name (overloads or different paths).
    by_name: FxHashMap<StringId, Vec<DeclarationId>>,
    /// Top-level constant headers indexed by their Stage 3 semantic slot.
    ///
    /// WHAT: records the header role joined with `DeclarationValue::PendingConstant`.
    /// WHY: declared type identity cannot distinguish a pending constant from a nominal shell.
    /// Dense identity matches `ResolvedConstantSet`; ids beyond the vector are not constants.
    constant_headers: Vec<bool>,
}

impl TopLevelDeclarationTable {
    pub(crate) fn empty() -> Self {
        Self {
            declarations: Vec::new(),
            by_path: FxHashMap::default(),
            by_name: FxHashMap::default(),
            constant_headers: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn new(declarations: Vec<Declaration>, path_fork: &PathInternerFork) -> Self {
        let ordered_declarations: Vec<_> = declarations
            .into_iter()
            .enumerate()
            .map(|(header_index, declaration)| OrderedSemanticDeclaration {
                declaration_id: DeclarationId::from_index(header_index),
                header_index,
                path: declaration.id,
                kind: OrderedSemanticDeclarationKind::Function,
                declaration: Some(declaration),
            })
            .collect();
        Self::from_stage3_order(ordered_declarations, Vec::new(), path_fork)
            .expect("direct declaration tables require unique paths")
    }

    /// Build the table from Stage 3's final declaration-like header order.
    pub(crate) fn from_stage3_order(
        ordered_declarations: Vec<OrderedSemanticDeclaration>,
        compiler_owned_declarations: Vec<Declaration>,
        path_fork: &PathInternerFork,
    ) -> Result<Self, InfrastructureFailure> {
        let mut by_path = FxHashMap::default();
        let mut by_name: FxHashMap<StringId, Vec<DeclarationId>> = FxHashMap::default();
        let mut constant_headers: Vec<bool> = Vec::new();
        let mut declaration_slots =
            Vec::with_capacity(ordered_declarations.len() + compiler_owned_declarations.len());

        for ordered in ordered_declarations {
            if ordered.declaration_id.index() != declaration_slots.len() {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "Stage 3 declaration IDs were not dense and ordered.",
                ));
            }
            let declaration_id = ordered.declaration_id;
            let path = ordered.path;
            if ordered.kind == OrderedSemanticDeclarationKind::Constant {
                if constant_headers.len() <= declaration_id.index() {
                    constant_headers.resize(declaration_id.index() + 1, false);
                }
                constant_headers[declaration_id.index()] = true;
            }
            if let Some(name) = path_fork.component(path) {
                by_name.entry(name).or_default().push(declaration_id);
            }
            if ordered.kind.owns_value_row() != ordered.declaration.is_some() {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "Stage 3 declaration storage did not match its semantic kind.",
                ));
            }
            if ordered
                .declaration
                .as_ref()
                .is_some_and(|declaration| declaration.id != path)
            {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "Stage 3 declaration row did not match its semantic path.",
                ));
            }
            if by_path.insert(path, declaration_id).is_some() {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "Stage 3 produced duplicate semantic declaration paths.",
                ));
            }
            declaration_slots.push(ordered.declaration);
        }

        for declaration in compiler_owned_declarations {
            let declaration_id = DeclarationId::from_index(declaration_slots.len());
            let existing = by_path.insert(declaration.id, declaration_id);

            if existing.is_some() {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "Compiler-owned declaration path collided with a Stage 3 declaration.",
                ));
            }
            if let Some(name) = path_fork.component(declaration.id) {
                by_name.entry(name).or_default().push(declaration_id);
            }
            declaration_slots.push(Some(declaration));
        }

        Ok(Self {
            declarations: declaration_slots,
            by_path,
            by_name,
            constant_headers,
        })
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &Declaration> {
        self.declarations.iter().filter_map(Option::as_ref)
    }

    // Path-based lookups

    pub(crate) fn get_by_path(&self, path: &PathId) -> Option<&Declaration> {
        let declaration_id = self.declaration_id_by_path(path)?;
        self.get_by_id(declaration_id)
    }

    /// Append one declaration while a generated environment is being assembled.
    ///
    /// WHAT: extends the declaration vector and both construction indexes in one operation.
    /// WHY: generated materialisation adds a small number of declarations to an existing
    ///     environment; rebuilding the complete table for each addition makes that hot path
    ///     quadratic and needlessly clones unrelated declarations.
    ///
    /// This is intentionally construction-only. Callers must not use it to mutate a completed
    /// AST environment, and duplicate paths are rejected rather than replacing an existing row.
    pub(in crate::compiler_frontend::ast) fn append_for_construction(
        &mut self,
        declaration: Declaration,
        path_fork: &PathInternerFork,
    ) -> Option<DeclarationId> {
        if self.declaration_id_by_path(&declaration.id).is_some() {
            return None;
        }

        let declaration_id = DeclarationId::from_index(self.len());
        let path = declaration.id;
        let name = path_fork.component(path);
        self.declarations.push(Some(declaration));
        self.by_path.insert(path, declaration_id);
        if let Some(name) = name {
            self.by_name.entry(name).or_default().push(declaration_id);
        }
        Some(declaration_id)
    }

    // Name-based lookups

    /// Look up a visible declaration by name, excluding receiver-method declarations.
    ///
    /// Receiver methods are filtered out because they should only be reachable through
    /// receiver-call syntax, not through ordinary name resolution.
    pub(in crate::compiler_frontend::ast) fn get_visible_non_receiver_by_name(
        &self,
        name: StringId,
        visible: Option<&FxHashSet<PathId>>,
    ) -> Option<&Declaration> {
        self.find_visible_by_name(name, visible, |declaration| {
            !is_receiver_method_declaration(declaration)
        })
    }

    // Visibility-filtered lookups

    /// Look up a resolved declaration by path, checking both resolution state and visibility.
    ///
    /// Unresolved constant placeholders are treated as absent so that callers do not
    /// accidentally consume a declaration whose type or value has not been determined yet.
    pub(crate) fn get_visible_resolved_by_path(
        &self,
        path: &PathId,
        visible: Option<&FxHashSet<PathId>>,
    ) -> Option<&Declaration> {
        let declaration = self.get_by_path(path)?;
        if self.is_unresolved_constant_placeholder(declaration) {
            return None;
        }
        if let Some(visible) = visible
            && !visible.contains(&declaration.id)
        {
            return None;
        }
        Some(declaration)
    }

    /// Look up a resolved declaration by name, checking both resolution state and visibility.
    pub(crate) fn get_visible_resolved_by_name(
        &self,
        name: StringId,
        visible: Option<&FxHashSet<PathId>>,
    ) -> Option<&Declaration> {
        self.find_visible_by_name(name, visible, |declaration| {
            !self.is_unresolved_constant_placeholder(declaration)
        })
    }

    // Internal helpers
    pub(in crate::compiler_frontend::ast) fn get_by_id(
        &self,
        declaration_id: DeclarationId,
    ) -> Option<&Declaration> {
        self.declarations
            .get(declaration_id.index())
            .and_then(Option::as_ref)
    }

    pub(in crate::compiler_frontend::ast) fn get_mut_by_id(
        &mut self,
        declaration_id: DeclarationId,
    ) -> Option<&mut Declaration> {
        self.declarations
            .get_mut(declaration_id.index())
            .and_then(Option::as_mut)
    }

    /// Replace one known declaration row without repeating a source-path lookup.
    ///
    /// A replacement must preserve the row's path so the construction-time path and name indexes
    /// remain valid.
    pub(in crate::compiler_frontend::ast) fn replace_by_id(
        &mut self,
        declaration_id: DeclarationId,
        declaration: Declaration,
    ) -> bool {
        let Some(current) = self.get_by_id(declaration_id) else {
            return false;
        };
        if current.id != declaration.id {
            return false;
        }

        self.declarations[declaration_id.index()] = Some(declaration);
        true
    }

    pub(in crate::compiler_frontend::ast) fn declaration_id_by_path(
        &self,
        path: &PathId,
    ) -> Option<DeclarationId> {
        self.by_path.get(path).copied()
    }

    /// Whether a declaration ID names a top-level constant header.
    fn is_constant_header(&self, declaration_id: DeclarationId) -> bool {
        self.constant_headers
            .get(declaration_id.index())
            .copied()
            .unwrap_or(false)
    }

    /// Whether a top-level placeholder is a constant header whose value has not been folded yet.
    ///
    /// The explicit pending state and retained header role jointly hide unresolved constants.
    /// A typed constant may share its type with a nominal shell, so type identity cannot
    /// determine whether a compile-time initializer is ready.
    pub(crate) fn is_unresolved_constant_placeholder(&self, declaration: &Declaration) -> bool {
        matches!(declaration.value, DeclarationValue::PendingConstant)
            && self
                .declaration_id_by_path(&declaration.id)
                .is_some_and(|declaration_id| self.is_constant_header(declaration_id))
    }

    fn declaration_ids_by_name(&self, name: StringId) -> impl Iterator<Item = DeclarationId> + '_ {
        self.by_name
            .get(&name)
            .into_iter()
            .flat_map(|ids| ids.iter().copied())
    }

    pub(crate) fn len(&self) -> usize {
        self.declarations.len()
    }

    /// Find the first declaration matching `name` that satisfies `predicate` and is visible.
    ///
    /// Visibility is checked only when a `visible` set is provided; otherwise every
    /// declaration in the name group is considered visible.
    fn find_visible_by_name(
        &self,
        name: StringId,
        visible: Option<&FxHashSet<PathId>>,
        predicate: impl Fn(&Declaration) -> bool,
    ) -> Option<&Declaration> {
        self.declaration_ids_by_name(name)
            .filter_map(|declaration_id| self.get_by_id(declaration_id))
            .find(|declaration| {
                if !predicate(declaration) {
                    return false;
                }
                match visible {
                    Some(visible_set) => visible_set.contains(&declaration.id),
                    None => true,
                }
            })
    }
}

/// Predicate helper used only by `get_visible_non_receiver_by_name`.
///
/// Extracted to keep the call site readable and to give the exclusion rule a name.
fn is_receiver_method_declaration(declaration: &Declaration) -> bool {
    declaration.is_receiver_function()
}
