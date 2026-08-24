//! Stable top-level declaration table for AST environment construction.
//!
//! WHAT: stores one slot per top-level declaration discovered by the header/dependency stages.
//! WHY: AST environment construction updates placeholders in place as declarations are resolved,
//! so body emission and type resolution can share one indexed declaration source without
//! reconstructing lookup indexes.
//!
//! Owned by the AST environment builder and consumed by AST emission, `ScopeContext`, and
//! finalization. Root tables update their own rows during environment building. Generated tables
//! inherit an immutable completed table and keep only local replacements and appends, so nested
//! materialisation never copies or mutates requester rows.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::compiler_errors::CompilerError;
#[cfg(test)]
use crate::compiler_frontend::headers::module_symbols::OrderedSemanticDeclarationKind;
use crate::compiler_frontend::headers::module_symbols::{
    CompilerOwnedDeclaration, CompilerOwnedDeclarationKind, DeclarationId,
    OrderedSemanticDeclaration,
};
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringId;
use rustc_hash::{FxHashMap, FxHashSet};
use std::rc::Rc;

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
    /// Immutable declarations inherited by a generated-local table.
    base: Option<Rc<TopLevelDeclarationTable>>,
    inherited_len: usize,
    /// Root semantic slots or slots appended only by this generated layer.
    ///
    /// Compile-time-only aliases and traits own stable IDs but no value declaration row.
    declarations: Vec<Option<Declaration>>,
    /// Generated-local replacements for inherited declaration slots.
    replacements: FxHashMap<DeclarationId, Declaration>,
    /// Root semantic paths, including metadata-only rows, plus compiler/generated declaration paths.
    by_path: FxHashMap<InternedPath, DeclarationId>,
    /// Name-to-IDs map for declarations that carry a simple name.
    ///
    /// Multiple declarations may share a name (overloads or different paths).
    by_name: FxHashMap<StringId, Vec<DeclarationId>>,
}

impl Clone for TopLevelDeclarationTable {
    fn clone(&self) -> Self {
        // A generated layer owns only its local delta, so cloning it shares every inherited row.
        // A flat root clone is the only table operation that copies rows which could otherwise
        // have formed an inherited prefix, and the benchmark counter keeps that boundary visible.
        if self.base.is_none() {
            add_frontend_counter(
                FrontendCounter::GeneratedDeclarationInheritedRowCopies,
                self.declarations.len(),
            );
        }

        Self {
            base: self.base.clone(),
            inherited_len: self.inherited_len,
            declarations: self.declarations.clone(),
            replacements: self.replacements.clone(),
            by_path: self.by_path.clone(),
            by_name: self.by_name.clone(),
        }
    }
}

impl TopLevelDeclarationTable {
    pub(crate) fn empty() -> Self {
        Self {
            base: None,
            inherited_len: 0,
            declarations: Vec::new(),
            replacements: FxHashMap::default(),
            by_path: FxHashMap::default(),
            by_name: FxHashMap::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn new(declarations: Vec<Declaration>) -> Self {
        let ordered_declarations: Vec<_> = declarations
            .into_iter()
            .enumerate()
            .map(|(header_index, declaration)| OrderedSemanticDeclaration {
                declaration_id: DeclarationId::from_index(header_index),
                header_index,
                path: declaration.id.clone(),
                kind: OrderedSemanticDeclarationKind::Function,
                declaration: Some(declaration),
            })
            .collect();
        Self::from_stage3_order(ordered_declarations, Vec::new())
            .expect("direct declaration tables require unique paths")
    }

    /// Build the table from Stage 3's final declaration-like header order.
    pub(crate) fn from_stage3_order(
        ordered_declarations: Vec<OrderedSemanticDeclaration>,
        compiler_owned_declarations: Vec<CompilerOwnedDeclaration>,
    ) -> Result<Self, CompilerError> {
        let mut by_path = FxHashMap::default();
        let mut by_name: FxHashMap<StringId, Vec<DeclarationId>> = FxHashMap::default();
        let mut declaration_slots =
            Vec::with_capacity(ordered_declarations.len() + compiler_owned_declarations.len());

        for ordered in ordered_declarations {
            if ordered.declaration_id.index() != declaration_slots.len() {
                return Err(CompilerError::compiler_error(
                    "Stage 3 declaration IDs were not dense and ordered.",
                ));
            }
            let declaration_id = ordered.declaration_id;
            let path = ordered.path.clone();
            if let Some(name) = path.name() {
                by_name.entry(name).or_default().push(declaration_id);
            }
            if ordered.kind.owns_value_row() != ordered.declaration.is_some() {
                return Err(CompilerError::compiler_error(
                    "Stage 3 declaration storage did not match its semantic kind.",
                ));
            }
            if ordered
                .declaration
                .as_ref()
                .is_some_and(|declaration| declaration.id != path)
            {
                return Err(CompilerError::compiler_error(
                    "Stage 3 declaration row did not match its semantic path.",
                ));
            }
            if by_path.insert(path, declaration_id).is_some() {
                return Err(CompilerError::compiler_error(
                    "Stage 3 produced duplicate semantic declaration paths.",
                ));
            }
            declaration_slots.push(ordered.declaration);
        }

        let semantic_len = declaration_slots.len();
        for compiler_owned in compiler_owned_declarations {
            let declaration = compiler_owned.declaration;
            let declaration_id = DeclarationId::from_index(declaration_slots.len());
            let existing = by_path.insert(declaration.id.clone(), declaration_id);

            // An authored function named `start` shares the implicit start path. Preserve the
            // trailing-path shadow so body validation can emit the authored source diagnostic.
            let shadows_authored_start = compiler_owned.kind == CompilerOwnedDeclarationKind::Start
                && existing.is_some_and(|existing| existing.index() < semantic_len);
            if existing.is_some() && !shadows_authored_start {
                return Err(CompilerError::compiler_error(
                    "Compiler-owned declaration path collided with a Stage 3 declaration.",
                ));
            }
            if let Some(name) = declaration.id.name() {
                by_name.entry(name).or_default().push(declaration_id);
            }
            declaration_slots.push(Some(declaration));
        }

        Ok(Self {
            base: None,
            inherited_len: 0,
            declarations: declaration_slots,
            replacements: FxHashMap::default(),
            by_path,
            by_name,
        })
    }

    /// Start a generated-local layer over one immutable completed declaration table.
    pub(crate) fn fork_for_generated(base: Rc<Self>) -> Self {
        let inherited_len = base.len();
        Self {
            base: Some(base),
            inherited_len,
            declarations: Vec::new(),
            replacements: FxHashMap::default(),
            by_path: FxHashMap::default(),
            by_name: FxHashMap::default(),
        }
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &Declaration> {
        (0..self.len()).filter_map(|index| self.get_by_id(DeclarationId::from_index(index)))
    }

    // Path-based lookups

    pub(crate) fn get_by_path(&self, path: &InternedPath) -> Option<&Declaration> {
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
    ) -> Option<DeclarationId> {
        if self.declaration_id_by_path(&declaration.id).is_some() {
            return None;
        }

        let declaration_id = DeclarationId::from_index(self.len());
        let path = declaration.id.to_owned();
        let name = declaration.id.name();
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
        visible: Option<&FxHashSet<InternedPath>>,
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
        path: &InternedPath,
        visible: Option<&FxHashSet<InternedPath>>,
    ) -> Option<&Declaration> {
        let declaration = self.get_by_path(path)?;
        if declaration.is_unresolved_constant_placeholder() {
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
        visible: Option<&FxHashSet<InternedPath>>,
    ) -> Option<&Declaration> {
        self.find_visible_by_name(name, visible, |declaration| {
            !declaration.is_unresolved_constant_placeholder()
        })
    }

    // Internal helpers
    pub(in crate::compiler_frontend::ast) fn get_by_id(
        &self,
        declaration_id: DeclarationId,
    ) -> Option<&Declaration> {
        if let Some(replacement) = self.replacements.get(&declaration_id) {
            return Some(replacement);
        }
        if declaration_id.index() < self.inherited_len {
            return self.base.as_ref()?.get_by_id(declaration_id);
        }
        self.declarations
            .get(declaration_id.index() - self.inherited_len)
            .and_then(Option::as_ref)
    }

    pub(in crate::compiler_frontend::ast) fn get_mut_by_id(
        &mut self,
        declaration_id: DeclarationId,
    ) -> Option<&mut Declaration> {
        if declaration_id.index() >= self.len() {
            return None;
        }
        if declaration_id.index() >= self.inherited_len {
            return self
                .declarations
                .get_mut(declaration_id.index() - self.inherited_len)?
                .as_mut();
        }

        if !self.replacements.contains_key(&declaration_id) {
            let inherited = self.base.as_ref()?.get_by_id(declaration_id)?.clone();
            self.replacements.insert(declaration_id, inherited);
        }
        self.replacements.get_mut(&declaration_id)
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

        if declaration_id.index() < self.inherited_len {
            self.replacements.insert(declaration_id, declaration);
        } else {
            self.declarations[declaration_id.index() - self.inherited_len] = Some(declaration);
        }
        true
    }

    pub(in crate::compiler_frontend::ast) fn declaration_id_by_path(
        &self,
        path: &InternedPath,
    ) -> Option<DeclarationId> {
        self.by_path.get(path).copied().or_else(|| {
            self.base
                .as_ref()
                .and_then(|base| base.declaration_id_by_path(path))
        })
    }

    fn declaration_ids_by_name(
        &self,
        name: StringId,
    ) -> Box<dyn Iterator<Item = DeclarationId> + '_> {
        let inherited = self
            .base
            .iter()
            .flat_map(move |base| base.declaration_ids_by_name(name));
        let local = self
            .by_name
            .get(&name)
            .into_iter()
            .flat_map(|ids| ids.iter().copied());
        Box::new(inherited.chain(local))
    }

    pub(crate) fn len(&self) -> usize {
        self.inherited_len + self.declarations.len()
    }

    /// Find the first declaration matching `name` that satisfies `predicate` and is visible.
    ///
    /// Visibility is checked only when a `visible` set is provided; otherwise every
    /// declaration in the name group is considered visible.
    fn find_visible_by_name(
        &self,
        name: StringId,
        visible: Option<&FxHashSet<InternedPath>>,
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
    declaration.value.is_receiver_function()
}
