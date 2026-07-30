//! Stable top-level declaration table for AST environment construction.
//!
//! WHAT: stores one slot per top-level declaration discovered by the header/dependency stages.
//! WHY: AST environment construction updates placeholders in place as declarations are resolved,
//! so body emission and type resolution can share one indexed declaration source without
//! reconstructing lookup indexes.
//!
//! Owned by the AST environment builder and consumed by AST emission, `ScopeContext`, and
//! finalization. The table is immutable after construction except for in-place replacements
//! during environment building.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringId;
use rustc_hash::{FxHashMap, FxHashSet};

/// Opaque index into `TopLevelDeclarationTable::declarations`.
///
/// IDs are created only by `TopLevelDeclarationTable::new` and are valid only within the
/// table that produced them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::compiler_frontend::ast) struct DeclarationId(u32);

/// Indexed table of all top-level declarations in a module.
///
/// Provides fast path-based and name-based lookups with optional visibility filtering.
/// Declarations are stored in dependency-sorted order and indexed by `DeclarationId`.
#[derive(Clone, Debug)]
pub(crate) struct TopLevelDeclarationTable {
    declarations: Vec<Declaration>,
    /// Path-to-ID map built from `Declaration::id` at construction time.
    by_path: FxHashMap<InternedPath, DeclarationId>,
    /// Name-to-IDs map for declarations that carry a simple name.
    ///
    /// Multiple declarations may share a name (overloads or different paths).
    by_name: FxHashMap<StringId, Vec<DeclarationId>>,
}

impl TopLevelDeclarationTable {
    pub(crate) fn new(declarations: Vec<Declaration>) -> Self {
        let mut by_path = FxHashMap::default();
        let mut by_name: FxHashMap<StringId, Vec<DeclarationId>> = FxHashMap::default();

        for (index, declaration) in declarations.iter().enumerate() {
            let declaration_id = DeclarationId(index as u32);
            // InternedPath is cheap to clone; we need an owned key for the map.
            by_path.insert(declaration.id.to_owned(), declaration_id);
            if let Some(name) = declaration.id.name() {
                by_name.entry(name).or_default().push(declaration_id);
            }
        }

        Self {
            declarations,
            by_path,
            by_name,
        }
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &Declaration> {
        self.declarations.iter()
    }

    // Path-based lookups

    pub(crate) fn get_by_path(&self, path: &InternedPath) -> Option<&Declaration> {
        self.by_path
            .get(path)
            .and_then(|declaration_id| self.get_by_id(*declaration_id))
    }

    pub(in crate::compiler_frontend::ast) fn get_mut_by_path(
        &mut self,
        path: &InternedPath,
    ) -> Option<&mut Declaration> {
        let declaration_id = *self.by_path.get(path)?;
        self.declarations.get_mut(declaration_id.index())
    }

    /// Replace the declaration stored at the given path, returning its ID on success.
    ///
    /// Returns `None` if the path is not present in the table. The caller is responsible
    /// for ensuring the replacement declaration uses the same path and name as the original
    /// so that the indexes remain consistent.
    pub(in crate::compiler_frontend::ast) fn replace_by_path(
        &mut self,
        declaration: Declaration,
    ) -> Option<DeclarationId> {
        let declaration_id = *self.by_path.get(&declaration.id)?;
        self.declarations[declaration_id.index()] = declaration;
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
    fn get_by_id(&self, declaration_id: DeclarationId) -> Option<&Declaration> {
        self.declarations.get(declaration_id.index())
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
        let declaration_ids = self.by_name.get(&name)?;

        declaration_ids
            .iter()
            .filter_map(|declaration_id| self.get_by_id(*declaration_id))
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

impl DeclarationId {
    fn index(self) -> usize {
        self.0 as usize
    }
}

/// Predicate helper used only by `get_visible_non_receiver_by_name`.
///
/// Extracted to keep the call site readable and to give the exclusion rule a name.
fn is_receiver_method_declaration(declaration: &Declaration) -> bool {
    declaration.value.is_receiver_function()
}
