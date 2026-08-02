//! Header-stage visible-name registry.
//!
//! WHAT: tracks which names are visible in one source file, classifies their binding kind,
//! and detects collisions.
//! WHY: same-file declarations, imports, aliases, builtins, and prelude symbols share one
//! namespace; silent shadowing must be rejected before AST body parsing.
//! MUST NOT: resolve import paths to files or external packages (that belongs in target
//! and public export resolution).

use crate::compiler_frontend::builtins::casts::traits::for_each_core_cast_trait_name;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, ReservedNameOwner};
use crate::compiler_frontend::external_packages::ExternalSymbolId;
use crate::compiler_frontend::headers::import_environment::NamespaceRecordSource;
use crate::compiler_frontend::headers::import_environment::diagnostics;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use rustc_hash::FxHashMap;

/// Classification of a visible name binding.
///
/// WHY: collision logic depends on whether two bindings refer to the same underlying target.
/// Enums make the resolution path explicit in type names and match arms.
pub(crate) enum VisibleNameBinding {
    SameFileDeclaration {
        declaration_path: InternedPath,
    },
    SourceImport {
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
    ///      without imports and must not be shadowed by declarations, aliases,
    ///      imports, or namespace records.
    ReservedCoreCastTraitName,
}

/// Stored entry for a registered visible name, including the binding and its source location.
///
/// WHY: preserving the original location lets collision diagnostics emit secondary labels
/// pointing to the first declaration or import.
struct VisibleNameEntry {
    binding: VisibleNameBinding,
    location: Option<SourceLocation>,
}

/// Per-file registry of visible names.
///
/// WHAT: maintains a map from local spelling to its binding classification and source location.
/// WHY: centralizing collision checks prevents drift between same-file declarations,
/// imports, builtins, and prelude symbols.
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
    /// shadowed by later declarations, imports, aliases, or namespace records.
    ///
    /// WHY: core cast trait names are globally visible without imports; treating
    ///      them as pre-existing bindings lets the normal collision path reject
    ///      any user-visible spelling that would claim one of those names.
    pub(crate) fn reserve_core_cast_trait_names(&mut self, string_table: &mut StringTable) {
        for_each_core_cast_trait_name(|trait_name| {
            let name_id = string_table.intern(trait_name);
            self.names.insert(
                name_id,
                VisibleNameEntry {
                    binding: VisibleNameBinding::ReservedCoreCastTraitName,
                    location: None,
                },
            );
        });
    }

    /// Attempt to register a visible name.
    ///
    /// WHY: same target from two sources (e.g., re-importing the same symbol) is harmless.
    /// Different targets with the same local spelling is a collision.
    ///
    /// Returns `Ok(())` when the name is registered or already present with the same target.
    /// Returns `Err` with a structured diagnostic when the name collides with a different target.
    ///
    /// The diagnostic is boxed at this registry boundary because every connected
    /// import-environment caller already propagates boxed diagnostics. Keeping the same
    /// error shape lets collisions travel directly to the header accumulation boundary.
    pub(crate) fn register(
        &mut self,
        local_name: StringId,
        binding: VisibleNameBinding,
        location: Option<SourceLocation>,
    ) -> Result<(), Box<CompilerDiagnostic>> {
        if let Some(entry) = self.names.get(&local_name) {
            if can_coexist(&entry.binding, &binding) {
                return Ok(());
            }
            let current_location = location
                .clone()
                .or_else(|| entry.location.clone())
                .unwrap_or_default();
            if matches!(entry.binding, VisibleNameBinding::ReservedCoreCastTraitName) {
                return Err(Box::new(CompilerDiagnostic::reserved_name_collision(
                    local_name,
                    ReservedNameOwner::CoreTrait,
                    current_location,
                )));
            }
            let previous_location = if location.is_some() {
                entry.location.clone()
            } else {
                None
            };
            return Err(Box::new(diagnostics::import_name_collision(
                local_name,
                current_location,
                previous_location,
            )));
        }
        self.names
            .insert(local_name, VisibleNameEntry { binding, location });
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
/// WHY: importing the same symbol twice (without alias, or with the same alias) is not an error.
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
            | VisibleNameBinding::SourceImport {
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
            | VisibleNameBinding::SourceImport {
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
    alias_location: &Option<SourceLocation>,
    path_location: &SourceLocation,
    local_name: StringId,
    symbol_name: StringId,
    string_table: &StringTable,
) -> Option<CompilerDiagnostic> {
    let alias_str = string_table.resolve(local_name);
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

    let location = alias_location
        .clone()
        .unwrap_or_else(|| path_location.clone());

    Some(CompilerDiagnostic::import_alias_case_mismatch(
        local_name,
        symbol_name,
        location,
    ))
}
