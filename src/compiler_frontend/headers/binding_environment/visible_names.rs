//! Header-stage visible-name registry.
//!
//! WHAT: tracks which names are visible in one source file, classifies their binding kind,
//! and detects collisions.
//! WHY: same-file declarations, dependency clauses, aliases, builtins, and prelude symbols share one
//! namespace; silent shadowing must be rejected before AST body parsing.
//! MUST NOT: resolve dependency paths to files or external packages (that belongs in target
//! and public export resolution).

use crate::compiler_frontend::builtins::casts::traits::for_each_core_cast_trait_name;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, ReservedNameOwner};
use crate::compiler_frontend::external_packages::ExternalSymbolId;
use crate::compiler_frontend::headers::binding_environment::NamespaceRecordSource;
use crate::compiler_frontend::headers::binding_environment::diagnostics;
use crate::compiler_frontend::headers::dependency_clause_syntax::DependencyAlias;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use rustc_hash::FxHashMap;

/// Classification of a visible name binding.
///
/// WHY: collision logic depends on whether two bindings refer to the same underlying target.
/// Enums make the resolution path explicit in type names and match arms.
pub(crate) enum VisibleNameBinding {
    SameFileDeclaration {
        declaration_path: InternedPath,
    },
    SourceDependency {
        canonical_path: InternedPath,
    },
    TypeAlias {
        canonical_path: InternedPath,
    },
    Trait {
        canonical_path: InternedPath,
    },
    ExternalImport {
        symbol_id: ExternalSymbolId,
    },
    Builtin,
    Prelude {
        symbol_id: ExternalSymbolId,
    },
    NamespaceRecord {
        record_source: NamespaceRecordSource,
    },
    /// Compiler-owned core cast trait name reserved before any source bindings.
    ///
    /// WHY: core cast trait names such as `CASTABLE_TO_INT` are globally visible
    ///      without dependencies and must not be shadowed by declarations, aliases,
    ///      dependencies, or namespace records.
    ReservedCoreCastTraitName,
}

/// Stored entry for a registered visible name, including the binding and its authored span.
///
/// WHY: preserving the original span lets collision diagnostics emit secondary labels pointing
/// to the first declaration or dependency.
struct VisibleNameEntry {
    binding: VisibleNameBinding,
    span: Option<SourceSpan>,
}

/// Per-file registry of visible names.
///
/// WHAT: maintains a map from local spelling to its binding classification and authored span.
/// WHY: centralizing collision checks prevents drift between same-file declarations, dependencies,
/// builtins, and prelude symbols.
pub(crate) struct VisibleNameRegistry {
    names: FxHashMap<StringId, VisibleNameEntry>,
}

impl VisibleNameRegistry {
    pub(crate) fn new() -> Self {
        Self {
            names: FxHashMap::default(),
        }
    }

    /// Reserve the compiler-owned core cast trait names so they cannot be
    /// shadowed by later declarations, dependencies, aliases, or namespace records.
    ///
    /// WHY: core cast trait names are globally visible without dependencies; treating
    /// them as pre-existing bindings lets the normal collision path reject any
    /// user-visible spelling that would claim one of those names.
    pub(crate) fn reserve_core_cast_trait_names(&mut self, string_table: &mut StringTable) {
        for_each_core_cast_trait_name(|trait_name| {
            let name_id = string_table.intern(trait_name);
            self.names.insert(
                name_id,
                VisibleNameEntry {
                    binding: VisibleNameBinding::ReservedCoreCastTraitName,
                    span: None,
                },
            );
        });
    }

    /// Attempt to register a visible name while retaining its exact authored span.
    ///
    /// WHY: same target from two sources (e.g., re-binding the same symbol) is harmless. Different
    /// targets with the same local spelling are a collision.
    ///
    /// Returns `Ok(())` when the name is registered or already present with the same target.
    /// Returns `Err` with a structured diagnostic when the name collides with a different target.
    pub(crate) fn register(
        &mut self,
        local_name: StringId,
        binding: VisibleNameBinding,
        span: Option<SourceSpan>,
    ) -> Result<(), CompilerDiagnostic> {
        if let Some(entry) = self.names.get(&local_name) {
            if can_coexist(&entry.binding, &binding) {
                return Ok(());
            }
            if matches!(entry.binding, VisibleNameBinding::ReservedCoreCastTraitName) {
                let mut diagnostic = CompilerDiagnostic::reserved_name_collision(
                    local_name,
                    ReservedNameOwner::CoreTrait,
                    span,
                );
                diagnostic.primary_span = span;
                return Err(diagnostic);
            }
            let mut diagnostic =
                diagnostics::dependency_name_collision(local_name, span, entry.span);
            diagnostic.primary_span = span;
            return Err(diagnostic);
        }
        self.names
            .insert(local_name, VisibleNameEntry { binding, span });
        Ok(())
    }

    /// Retrieve the binding for a name, if any.
    pub(crate) fn get(&self, name: StringId) -> Option<&VisibleNameBinding> {
        self.names.get(&name).map(|entry| &entry.binding)
    }

    /// Remove a generated same-file declaration that is not part of the source-visible surface.
    ///
    /// WHAT: keeps the registry aligned with the visibility maps when synthetic source files
    ///       hide their generated self constants before implicit providers are registered.
    /// WHY: the registry is the collision authority; leaving the removed declaration behind
    ///      would make a hidden implementation detail collide with a valid implicit export.
    pub(crate) fn remove_same_file_declaration(
        &mut self,
        local_name: StringId,
        declaration_path: &InternedPath,
    ) {
        let should_remove = self.names.get(&local_name).is_some_and(|entry| {
            matches!(
                &entry.binding,
                VisibleNameBinding::SameFileDeclaration {
                    declaration_path: registered_path
                } if registered_path == declaration_path
            )
        });
        if should_remove {
            self.names.remove(&local_name);
        }
    }
}

/// Determine whether two bindings refer to the same underlying target.
///
/// WHY: binding the same symbol twice (without alias, or with the same alias) is not an error.
fn is_same_target(a: &VisibleNameBinding, b: &VisibleNameBinding) -> bool {
    if matches!(
        (a, b),
        (
            VisibleNameBinding::ReservedCoreCastTraitName,
            VisibleNameBinding::ReservedCoreCastTraitName,
        )
    ) {
        return true;
    }

    match (a, b) {
        (
            VisibleNameBinding::SameFileDeclaration {
                declaration_path: a_path,
            }
            | VisibleNameBinding::SourceDependency {
                canonical_path: a_path,
            }
            | VisibleNameBinding::TypeAlias {
                canonical_path: a_path,
            }
            | VisibleNameBinding::Trait {
                canonical_path: a_path,
            },
            VisibleNameBinding::SameFileDeclaration {
                declaration_path: b_path,
            }
            | VisibleNameBinding::SourceDependency {
                canonical_path: b_path,
            }
            | VisibleNameBinding::TypeAlias {
                canonical_path: b_path,
            }
            | VisibleNameBinding::Trait {
                canonical_path: b_path,
            },
        ) => a_path == b_path,
        (
            VisibleNameBinding::ExternalImport { symbol_id: a_id },
            VisibleNameBinding::ExternalImport { symbol_id: b_id },
        ) => a_id == b_id,
        (
            VisibleNameBinding::Prelude { symbol_id: a_id },
            VisibleNameBinding::Prelude { symbol_id: b_id },
        ) => a_id == b_id,
        (
            VisibleNameBinding::ExternalImport { symbol_id: a_id },
            VisibleNameBinding::Prelude { symbol_id: b_id },
        )
        | (
            VisibleNameBinding::Prelude { symbol_id: a_id },
            VisibleNameBinding::ExternalImport { symbol_id: b_id },
        ) => a_id == b_id,
        (
            VisibleNameBinding::NamespaceRecord {
                record_source: a_src,
            },
            VisibleNameBinding::NamespaceRecord {
                record_source: b_src,
            },
        ) => a_src == b_src,
        _ => false,
    }
}

/// Determine whether two bindings can coexist under the same visible name.
///
/// WHY: same target from two sources is harmless; different targets collide.
fn can_coexist(a: &VisibleNameBinding, b: &VisibleNameBinding) -> bool {
    is_same_target(a, b)
}

/// Generate a case-convention warning when an alias uses different leading case than the symbol.
///
/// WHY: Moth naming conventions use leading case to distinguish types from values.
/// An alias that changes leading case is allowed but warned because it misleads readers.
pub(crate) fn check_alias_case_warning(
    alias: &DependencyAlias,
    string_table: &StringTable,
    symbol_name: StringId,
) -> Option<CompilerDiagnostic> {
    let alias_str = string_table.resolve(alias.name);
    let symbol_str = string_table.resolve(symbol_name);

    let a = alias_str.chars().next()?;
    let s = symbol_str.chars().next()?;

    if !a.is_alphabetic() || !s.is_alphabetic() {
        return None;
    }

    let alias_upper = a.is_uppercase();
    let symbol_upper = s.is_uppercase();
    if alias_upper == symbol_upper {
        return None;
    }
    let span = Some(alias.span);

    Some(CompilerDiagnostic::dependency_alias_case_mismatch(
        alias.name,
        symbol_name,
        span,
    ))
}
