//! Header-stage constant dependency extraction.
//!
//! WHAT: classifies symbol-shaped references captured from constant initializer tokens and adds
//! top-level dependency edges between constants.
//! WHY: dependency sorting must order constants before AST folds their initializer expressions.
//! MUST NOT: type-check expressions or decide whether a full initializer is foldable.

use crate::compiler_frontend::compiler_errors::{CompilerError, compiler_error_to_diagnostic};
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic, DiagnosticBag,
};
use crate::compiler_frontend::external_packages::ExternalSymbolId;
use crate::compiler_frontend::headers::import_environment::{
    FileVisibility, HeaderImportEnvironment, NamespaceTypeMember, NamespaceValueMember,
};
use crate::compiler_frontend::headers::module_symbols::{GenericDeclarationKind, ModuleSymbols};
use crate::compiler_frontend::headers::parse_file_headers::{Header, HeaderKind};
use crate::compiler_frontend::headers::types::LocalDeclarationOrderingHint;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::utilities::token_scan::InitializerReference;
use rustc_hash::{FxHashMap, FxHashSet};

pub(crate) struct ConstantDependencyInput<'a> {
    pub(crate) headers: &'a mut [Header],
    pub(crate) module_symbols: &'a ModuleSymbols,
    pub(crate) import_environment: &'a HeaderImportEnvironment,
    pub(crate) string_table: &'a mut StringTable,
}

pub(crate) struct ConstantDependencyReport {
    pub(crate) added_edges: usize,
    pub(crate) same_file_edges: usize,
    pub(crate) cross_file_edges: usize,
}

/// Canonical source file and header index for one source constant.
///
/// WHY: dependency ordering needs both the canonical source file (to distinguish same-file
/// from cross-file references) and the header index (to enforce source order within a file).
/// Building both in one inventory pass avoids a separate lookup that could silently fall back
/// to header index zero when compiler-owned metadata is missing.
#[derive(Clone, Debug)]
struct ConstantPosition {
    source_file: InternedPath,
    header_index: usize,
}

pub(crate) enum ConstantReferenceResolution {
    SourceConstant { path: InternedPath },
    SourceNonConstant { _path: InternedPath },
    SourceTypeAlias { _path: InternedPath },
    ExternalConstant { _symbol_id: ExternalSymbolId },
    ExternalNonConstant { _symbol_id: ExternalSymbolId },
    ConstructorLikeSource { _path: InternedPath },
    NotVisible { name: StringId },
}

pub(crate) fn add_constant_initializer_dependencies(
    input: ConstantDependencyInput<'_>,
) -> Result<ConstantDependencyReport, DiagnosticBag> {
    let ConstantDependencyInput {
        headers,
        module_symbols,
        import_environment,
        string_table,
    } = input;

    let mut diagnostic_bag = DiagnosticBag::new();
    let mut report = ConstantDependencyReport {
        added_edges: 0,
        same_file_edges: 0,
        cross_file_edges: 0,
    };

    // Build indexes for fast constant and struct/choice lookups.
    let mut constants_by_name: FxHashMap<StringId, Vec<InternedPath>> = FxHashMap::default();
    let mut constant_positions: FxHashMap<InternedPath, ConstantPosition> = FxHashMap::default();
    let mut struct_or_choice_paths: FxHashSet<InternedPath> = FxHashSet::default();

    for (header_index, header) in headers.iter().enumerate() {
        match &header.kind {
            HeaderKind::Constant { .. } => {
                let path = header.tokens.src_path.clone();
                constant_positions.insert(
                    path.clone(),
                    ConstantPosition {
                        source_file: header.canonical_source_file(string_table),
                        header_index,
                    },
                );
                if let Some(name) = path.name() {
                    constants_by_name.entry(name).or_default().push(path);
                }
            }
            HeaderKind::Struct { .. } | HeaderKind::Choice { .. } => {
                struct_or_choice_paths.insert(header.tokens.src_path.clone());
            }
            HeaderKind::ConstTemplate { .. } => {}
            _ => {}
        }
    }

    // Collect edges and errors in a first pass, then apply edges in a second pass.
    // WHY: avoids borrowing headers both immutably (for reading kind/source_file) and mutably
    // (for inserting into dependencies) at the same time.
    let mut edges_to_add: Vec<(usize, LocalDeclarationOrderingHint)> = Vec::new();

    for (header_index, header) in headers.iter().enumerate() {
        let (initializer_refs, reference_header_index) = match &header.kind {
            HeaderKind::Constant { declaration, .. } => {
                (&declaration.initializer_references[..], header_index)
            }

            HeaderKind::ConstTemplate {
                condition_references,
                ..
            } => (&condition_references[..], header_index),

            _ => (&[][..], header_index),
        };
        let has_initializer_refs = !initializer_refs.is_empty();
        let has_capacity_refs = !header.capacity_references.is_empty();
        if !has_initializer_refs && !has_capacity_refs {
            continue;
        }

        let visibility = match import_environment.visibility_for(&header.source_file) {
            Ok(v) => v,
            Err(error) => {
                diagnostic_bag.push(compiler_error_to_diagnostic(&error));
                continue;
            }
        };

        let current_path = header.tokens.src_path.clone();

        let all_refs = initializer_refs.iter().chain(&header.capacity_references);
        for reference in all_refs {
            let resolution = classify_reference(
                reference,
                visibility,
                &constant_positions,
                &struct_or_choice_paths,
                module_symbols,
            );

            match resolution {
                // Constants create ordering edges. Same-file edges are still constrained by source order.
                ConstantReferenceResolution::SourceConstant { path } => {
                    if path == current_path {
                        diagnostic_bag.push(self_reference_error(reference));
                        continue;
                    }

                    // The position record was built in the same inventory pass that classified
                    // this path as a constant, so a missing record is a compiler invariant
                    // violation - not a user-facing source diagnostic.
                    let Some(position) = constant_positions.get(&path) else {
                        diagnostic_bag.push(missing_constant_position_error(&path, string_table));
                        continue;
                    };

                    // Compare canonical source files to distinguish same-file from cross-file
                    // references. Both sides use canonical OS paths, not logical source paths.
                    let current_canonical_source = header.canonical_source_file(string_table);
                    if position.source_file == current_canonical_source {
                        if position.header_index > reference_header_index {
                            diagnostic_bag.push(same_file_forward_reference_error(
                                &current_path,
                                &path,
                                reference,
                            ));
                            continue;
                        }
                        report.same_file_edges += 1;
                    } else {
                        report.cross_file_edges += 1;
                    }

                    edges_to_add.push((header_index, LocalDeclarationOrderingHint::new(path)));
                }

                // Type aliases live in the type namespace. They do not create value dependency edges.
                ConstantReferenceResolution::SourceTypeAlias { .. }
                | ConstantReferenceResolution::ExternalConstant { .. }
                | ConstantReferenceResolution::ConstructorLikeSource { .. } => {}

                // Source non-constants are structurally invalid in constant initializers.
                // External non-constants are deferred to AST because header stage cannot
                // determine whether an external call is foldable or valid in all contexts.
                ConstantReferenceResolution::SourceNonConstant { .. } => {
                    diagnostic_bag.push(non_constant_reference_error(reference));
                }

                // External references are deferred to AST folding validation.
                ConstantReferenceResolution::ExternalNonConstant { .. } => {}

                // A constant with this name exists in the module but is not visible to this file.
                ConstantReferenceResolution::NotVisible { name } => {
                    if constants_by_name.contains_key(&name) {
                        diagnostic_bag.push(not_visible_constant_error(reference));
                    }
                    // If no constant with this name exists anywhere, treat as Unknown so AST
                    // can produce a more precise diagnostic during expression parsing.
                }
            }
        }
    }

    for (header_index, hint) in edges_to_add {
        let header = &mut headers[header_index];
        if header.local_ordering_hints.insert(hint) {
            report.added_edges += 1;
        }
    }

    if diagnostic_bag.has_errors() {
        return Err(diagnostic_bag);
    }

    Ok(report)
}

fn classify_reference(
    reference: &InitializerReference,
    visibility: &FileVisibility,
    constant_positions: &FxHashMap<InternedPath, ConstantPosition>,
    struct_or_choice_paths: &FxHashSet<InternedPath>,
    module_symbols: &ModuleSymbols,
) -> ConstantReferenceResolution {
    // 1. External symbols: constants are valid references; non-constants are errors.
    if let Some(symbol_id) = visibility.visible_external_symbols.get(&reference.name) {
        return if matches!(symbol_id, ExternalSymbolId::Constant(_)) {
            ConstantReferenceResolution::ExternalConstant {
                _symbol_id: *symbol_id,
            }
        } else {
            ConstantReferenceResolution::ExternalNonConstant {
                _symbol_id: *symbol_id,
            }
        };
    }

    // 2. Type aliases: valid to resolve but do not create value dependency edges.
    if let Some(path) = visibility.visible_type_alias_names.get(&reference.name) {
        return ConstantReferenceResolution::SourceTypeAlias {
            _path: path.local_path().clone(),
        };
    }

    // 3. Namespace records: a shallow `namespace.member` access can name a source constant that
    // must be ordered before this initializer folds. Full member semantics stay in AST.
    if let Some(record) = visibility.visible_namespace_records.get(&reference.name) {
        let Some(member_name) = reference.dot_member else {
            return ConstantReferenceResolution::NotVisible {
                name: reference.name,
            };
        };

        if let Some(member) = record.value_members.get(&member_name) {
            return classify_namespace_value_member(
                member,
                constant_positions,
                struct_or_choice_paths,
                module_symbols,
                reference,
            );
        }

        if let Some(NamespaceTypeMember::SourceDeclaration(path)) =
            record.type_members.get(&member_name)
        {
            return ConstantReferenceResolution::SourceTypeAlias {
                _path: path.local_path().clone(),
            };
        }

        return ConstantReferenceResolution::NotVisible { name: member_name };
    };

    // 4. Source-visible names: may be constants, constructors, or non-constants.
    let Some(target_path) = visibility.visible_source_names.get(&reference.name) else {
        return ConstantReferenceResolution::NotVisible {
            name: reference.name,
        };
    };

    classify_source_declaration_reference(
        target_path.local_path(),
        constant_positions,
        struct_or_choice_paths,
        module_symbols,
        reference,
    )
}

fn classify_namespace_value_member(
    member: &NamespaceValueMember,
    constant_positions: &FxHashMap<InternedPath, ConstantPosition>,
    struct_or_choice_paths: &FxHashSet<InternedPath>,
    module_symbols: &ModuleSymbols,
    reference: &InitializerReference,
) -> ConstantReferenceResolution {
    match member {
        NamespaceValueMember::SourceDeclaration(target_path) => {
            classify_source_declaration_reference(
                target_path.local_path(),
                constant_positions,
                struct_or_choice_paths,
                module_symbols,
                reference,
            )
        }

        NamespaceValueMember::ExternalSymbol(symbol_id) => {
            if matches!(symbol_id, ExternalSymbolId::Constant(_)) {
                ConstantReferenceResolution::ExternalConstant {
                    _symbol_id: *symbol_id,
                }
            } else {
                ConstantReferenceResolution::ExternalNonConstant {
                    _symbol_id: *symbol_id,
                }
            }
        }
    }
}

fn classify_source_declaration_reference(
    target_path: &InternedPath,
    constant_positions: &FxHashMap<InternedPath, ConstantPosition>,
    struct_or_choice_paths: &FxHashSet<InternedPath>,
    module_symbols: &ModuleSymbols,
    reference: &InitializerReference,
) -> ConstantReferenceResolution {
    let is_constant = constant_positions.contains_key(target_path);

    if is_constant {
        // Even if the target is a constant, it might be used as a constructor-like
        // nominal if followed by a call or namespace accessor.
        if (reference.followed_by_call || reference.followed_by_choice_namespace)
            && is_nominal_constructor(target_path, struct_or_choice_paths, module_symbols)
        {
            return ConstantReferenceResolution::ConstructorLikeSource {
                _path: target_path.clone(),
            };
        }

        return ConstantReferenceResolution::SourceConstant {
            path: target_path.clone(),
        };
    }

    // Not a constant: check if it's a legitimate constructor-like reference.
    if (reference.followed_by_call || reference.followed_by_choice_namespace)
        && is_nominal_constructor(target_path, struct_or_choice_paths, module_symbols)
    {
        return ConstantReferenceResolution::ConstructorLikeSource {
            _path: target_path.clone(),
        };
    }

    ConstantReferenceResolution::SourceNonConstant {
        _path: target_path.clone(),
    }
}

/// Determine whether a visible source name refers to a struct or choice declaration
/// and can therefore be used as a nominal constructor in a constant initializer.
///
/// WHY: constants may construct struct/choice literals at compile time, but function calls
/// and other non-constant references are not valid in constant initializers.
fn is_nominal_constructor(
    target_path: &InternedPath,
    struct_or_choice_paths: &FxHashSet<InternedPath>,
    module_symbols: &ModuleSymbols,
) -> bool {
    // Fast path: the header itself is a struct or choice.
    if struct_or_choice_paths.contains(target_path) {
        return true;
    }

    // Fallback: generic declarations with struct/choice kinds are also constructors.
    if let Some(metadata) = module_symbols.generic_declarations_by_path.get(target_path) {
        return matches!(
            metadata.kind,
            GenericDeclarationKind::Struct | GenericDeclarationKind::Choice
        );
    }

    false
}

// ---------------------------------------------------------------------------
// Diagnostic helpers
// ---------------------------------------------------------------------------

fn self_reference_error(reference: &InitializerReference) -> CompilerDiagnostic {
    CompilerDiagnostic::compile_time_evaluation_error(
        CompileTimeEvaluationErrorReason::ConstantSelfReference,
        Some(reference.name),
        reference.location.clone(),
    )
}

fn not_visible_constant_error(reference: &InitializerReference) -> CompilerDiagnostic {
    CompilerDiagnostic::compile_time_evaluation_error(
        CompileTimeEvaluationErrorReason::ConstantNotVisible,
        Some(reference.name),
        reference.location.clone(),
    )
}

fn non_constant_reference_error(reference: &InitializerReference) -> CompilerDiagnostic {
    CompilerDiagnostic::compile_time_evaluation_error(
        CompileTimeEvaluationErrorReason::NonConstantReferenceInConstant,
        Some(reference.name),
        reference.location.clone(),
    )
}

fn same_file_forward_reference_error(
    constant_path: &InternedPath,
    target_path: &InternedPath,
    reference: &InitializerReference,
) -> CompilerDiagnostic {
    let target_name = target_path.name().or_else(|| constant_path.name());
    CompilerDiagnostic::compile_time_evaluation_error(
        CompileTimeEvaluationErrorReason::SameFileForwardConstantReference,
        target_name,
        reference.location.clone(),
    )
}

/// Produce an internal compiler error for a classified source constant whose position record
/// is missing from the inventory map.
///
/// WHY: constant classification reads `constant_positions` directly, so a classified
/// `SourceConstant` must always have a position record. A missing record means the map is
/// corrupted, which is a compiler bug rather than a user source error.
fn missing_constant_position_error(
    constant_path: &InternedPath,
    string_table: &StringTable,
) -> CompilerDiagnostic {
    compiler_error_to_diagnostic(&CompilerError::compiler_error(format!(
        "Missing constant position metadata for classified source constant '{}' - \
         the constant inventory map is corrupted",
        constant_path.to_portable_string(string_table),
    )))
}

#[cfg(test)]
#[path = "tests/constant_dependencies_tests.rs"]
mod constant_dependencies_tests;
