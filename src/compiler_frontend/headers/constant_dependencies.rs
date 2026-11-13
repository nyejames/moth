//! Header-stage constant dependency extraction.
//!
//! WHAT: classifies symbol-shaped references captured from constant initializer tokens and adds
//! top-level dependency edges between constants.
//! WHY: dependency sorting must order constants before AST folds their initializer expressions.
//! MUST NOT: type-check expressions or decide whether a full initializer is foldable.

use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, DiagnosticBag,
};
use crate::compiler_frontend::datatypes::CallableEvaluation;
use crate::compiler_frontend::external_packages::ExternalSymbolId;
use crate::compiler_frontend::headers::binding_environment::{
    FileVisibility, HeaderBindingEnvironment, NamespaceTypeMember, NamespaceValueMember,
    cross_file_root_local_declaration,
};
use crate::compiler_frontend::headers::module_symbols::{GenericDeclarationKind, ModuleSymbols};
use crate::compiler_frontend::headers::parse_file_headers::{
    Header, HeaderKind, HeaderPreparationFailure,
};
use crate::compiler_frontend::headers::types::{
    LocalDeclarationOrderingHint, LocalDeclarationOrderingHintOrigin, SourceTokenOwners,
    SyntheticContentPayload,
};
use crate::compiler_frontend::public_interface::{
    PublicDeclarationRecord, PublicDeclarationSemantics,
};
use crate::compiler_frontend::semantic_identity::OriginDeclarationId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::utilities::token_scan::{
    InitializerReference, TokenFactView, collect_body_local_binding_names,
    collect_scanned_symbol_references,
};
use rustc_hash::{FxHashMap, FxHashSet};

pub(crate) struct ConstantDependencyInput<'a> {
    pub(crate) headers: &'a mut [Header],
    pub(crate) module_symbols: &'a ModuleSymbols,
    pub(crate) binding_environment: &'a HeaderBindingEnvironment,
    pub(crate) string_table: &'a mut StringTable,
    pub(crate) path_fork: &'a PathInternerFork,
    pub(crate) source_token_owners: &'a SourceTokenOwners,
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
#[derive(Clone, Copy, Debug)]
struct ConstantPosition {
    source_file: PathId,
    header_index: usize,
    kind: OrderedValueKind,
}

/// Which ordered value participant owns a dependency position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OrderedValueKind {
    Constant,
    ConstFunction,
}

pub(crate) enum ConstantReferenceResolution {
    SourceConstant { path: PathId },
    SourceConstFunction { path: PathId },
    ImportedConstant,
    ImportedConstFunction { _path: PathId },
    SourceNonConstant { _path: PathId },
    SourceTypeAlias { _path: PathId },
    ExternalConstant { _symbol_id: ExternalSymbolId },
    ExternalNonConstant { _symbol_id: ExternalSymbolId },
    ConstructorLikeSource { _path: PathId },
    NotVisible { name: StringId },
}

/// Borrowed facts one header's qualified-reference diagnostic pass needs.
///
/// WHY: the pass must be callable with synthetic hints in a focused test without building a full
///      `Header`; grouping the read-only facts keeps the function's argument list small.
struct QualifiedReferenceDiagnosticContext<'a> {
    declaration_path: PathId,
    name_span: Option<SourceSpan>,
    header_index: usize,
    current_source_file: PathId,
    is_moth_template_content: bool,
    constant_positions: &'a FxHashMap<PathId, ConstantPosition>,
    path_fork: &'a PathInternerFork,
}

/// Push the same-file qualified-reference diagnostics for one header in authored source order.
///
/// WHAT: reports self-references and same-file forward references among the header's retained
///       qualified-reference hints. The MOTH template content self-cycle skip is preserved.
/// WHY: hint storage is a hash set used to deduplicate graph edges, so this pass orders the
///      qualifying hints by authored span before pushing. Diagnostics preserve insertion order
///      and bounded error reporting keeps the first retained errors.
fn push_qualified_reference_diagnostics<'a>(
    context: QualifiedReferenceDiagnosticContext<'_>,
    hints: impl IntoIterator<Item = &'a LocalDeclarationOrderingHint>,
    diagnostic_bag: &mut DiagnosticBag,
    byte_range: impl FnMut(SourceSpan) -> Option<(u32, u32)>,
) {
    let QualifiedReferenceDiagnosticContext {
        declaration_path,
        name_span,
        header_index,
        current_source_file,
        is_moth_template_content,
        constant_positions,
        path_fork,
    } = context;

    for hint in qualified_reference_hints_in_diagnostic_order(hints, name_span, byte_range) {
        let Some(position) = constant_positions.get(&hint.path()) else {
            continue;
        };
        if position.source_file != current_source_file {
            continue;
        }
        // Const functions may be called before their declaration; the dependency graph owns
        // their ordering and cycle rejection. Only constants keep the source-order rule.
        if position.kind == OrderedValueKind::ConstFunction {
            continue;
        }
        let span = hint.occurrence_span().or(name_span);

        if hint.path() == declaration_path {
            if is_moth_template_content {
                // Stage 3's existing cycle diagnostic owns the explicit self-qualified
                // template content reference; the unqualified self filter does not apply.
                continue;
            }
            diagnostic_bag.push(self_reference_error_for(
                path_fork.component(declaration_path),
                span,
            ));
            continue;
        }

        if position.header_index > header_index {
            diagnostic_bag.push(same_file_forward_reference_error_for(
                declaration_path,
                hint.path(),
                span,
                path_fork,
            ));
        }
    }
}

/// Inline byte range for diagnostic ordering, without resolving an extended span.
///
/// WHY: header binding runs before the source database installs extended-span tables, so
/// `SourceSpan::byte_range` can panic on a span that does not fit inline. An extended span
/// stays in the spanned group. Its allocation index is not a byte offset, so two extended
/// spans in one shell are deterministic by path, not necessarily in authored order. Restoring
/// that order needs the preparation-time occurrence list, which this pass does not own.
fn hint_span_order_range(span: SourceSpan) -> (u32, u32) {
    if let Some(range) = span.local().inline_range() {
        return (range.start(), range.end());
    }
    (u32::MAX - 1, u32::MAX - 1)
}
/// Authored order for one qualified-reference diagnostic.
///
/// Spanned hints sort first. Hints with only a header name span follow. Hints with no
/// resolvable span sort last. The path breaks ties so equal spans stay deterministic.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct QualifiedDiagnosticOrder {
    category: u8,
    start: u32,
    end: u32,
    path: PathId,
}

/// Order the qualified-reference hints a diagnostic pass examines by authored source position.
///
/// WHAT: returns hints with a resolvable occurrence span first, ordered by span start then end,
///       then hints without one in their header name-span order. The referenced path is the final
///       tie-breaker for equal spans.
/// WHY: `Header::local_ordering_hints` is a `HashSet` retained to deduplicate graph edges, so its
///      iteration order carries randomized hash state. `DiagnosticBag` preserves insertion order
///      and bounded error reporting keeps the first retained errors, so diagnostics must be pushed
///      in authored source order instead of hash order.
fn qualified_reference_hints_in_diagnostic_order<'a>(
    hints: impl IntoIterator<Item = &'a LocalDeclarationOrderingHint>,
    name_span: Option<SourceSpan>,
    mut byte_range: impl FnMut(SourceSpan) -> Option<(u32, u32)>,
) -> Vec<&'a LocalDeclarationOrderingHint> {
    let mut name_range: Option<Option<(u32, u32)>> = None;
    let mut keyed: Vec<(QualifiedDiagnosticOrder, &LocalDeclarationOrderingHint)> = hints
        .into_iter()
        .filter(|hint| hint.origin() == LocalDeclarationOrderingHintOrigin::QualifiedReference)
        .map(|hint| {
            let key = match hint.occurrence_span().and_then(&mut byte_range) {
                Some((start, end)) => QualifiedDiagnosticOrder {
                    category: 0,
                    start,
                    end,
                    path: hint.path(),
                },
                None => {
                    let name_range =
                        *name_range.get_or_insert_with(|| name_span.and_then(&mut byte_range));
                    match name_range {
                        Some((start, end)) => QualifiedDiagnosticOrder {
                            category: 1,
                            start,
                            end,
                            path: hint.path(),
                        },
                        None => QualifiedDiagnosticOrder {
                            category: 2,
                            start: u32::MAX,
                            end: u32::MAX,
                            path: hint.path(),
                        },
                    }
                }
            };
            (key, hint)
        })
        .collect();
    keyed.sort_unstable_by_key(|(key, _)| *key);
    keyed.into_iter().map(|(_, hint)| hint).collect()
}

pub(crate) fn add_constant_initializer_dependencies(
    input: ConstantDependencyInput<'_>,
) -> Result<ConstantDependencyReport, HeaderPreparationFailure> {
    let ConstantDependencyInput {
        headers,
        module_symbols,
        binding_environment,
        string_table,
        path_fork,
        source_token_owners,
    } = input;

    let mut diagnostic_bag = DiagnosticBag::new();
    let mut report = ConstantDependencyReport {
        added_edges: 0,
        same_file_edges: 0,
        cross_file_edges: 0,
    };

    // Build indexes for fast constant and struct/choice lookups.
    let mut constants_by_name: FxHashMap<StringId, Vec<PathId>> = FxHashMap::default();
    let mut constant_positions: FxHashMap<PathId, ConstantPosition> = FxHashMap::default();
    let mut struct_or_choice_paths: FxHashSet<PathId> = FxHashSet::default();

    for (header_index, header) in headers.iter().enumerate() {
        match &header.kind {
            HeaderKind::Function { signature, .. }
                if signature.evaluation == CallableEvaluation::Const =>
            {
                constant_positions.insert(
                    header.declaration_path,
                    ConstantPosition {
                        source_file: module_symbols
                            .source_path_for_header(header)
                            .expect("header body range has no prepared source-path identity"),
                        header_index,
                        kind: OrderedValueKind::ConstFunction,
                    },
                );
            }
            HeaderKind::Constant { .. } => {
                let path = header.declaration_path;
                constant_positions.insert(
                    path,
                    ConstantPosition {
                        source_file: module_symbols
                            .source_path_for_header(header)
                            .expect("header body range has no prepared source-path identity"),
                        header_index,
                        kind: OrderedValueKind::Constant,
                    },
                );
                if let Some(name) = path_fork.component(path) {
                    constants_by_name.entry(name).or_default().push(path);
                }
            }
            HeaderKind::Struct { .. } | HeaderKind::Choice { .. } => {
                struct_or_choice_paths.insert(header.declaration_path);
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
        let mut const_body_references: Vec<InitializerReference>;
        let (initializer_refs, reference_header_index) = match &header.kind {
            HeaderKind::Constant { declaration, .. } => {
                (&declaration.initializer_references[..], header_index)
            }
            HeaderKind::Function { signature, .. }
                if signature.evaluation == CallableEvaluation::Const =>
            {
                let source_path = module_symbols
                    .source_path_for_header(header)
                    .expect("header body range has no prepared source-path identity");
                let source_id = *module_symbols
                    .source_ids_by_source
                    .get(&source_path)
                    .ok_or_else(|| {
                        HeaderPreparationFailure::Infrastructure(
                            InfrastructureFailure::internal(
                                CompilerStage::HeaderParsing,
                                "const function body header has no prepared source identity",
                            )
                            .into(),
                        )
                    })?;
                let owner = source_token_owners.get(&source_id).ok_or_else(|| {
                    HeaderPreparationFailure::Infrastructure(
                        InfrastructureFailure::internal(
                            CompilerStage::HeaderParsing,
                            "const function body header has no canonical token owner",
                        )
                        .into(),
                    )
                })?;
                let view = TokenFactView::from_source_range(owner.tokens_ref(), header.tokens)
                    .map_err(|error| {
                        HeaderPreparationFailure::Infrastructure(
                            InfrastructureFailure::internal(
                                CompilerStage::HeaderParsing,
                                format!(
                                    "const function body token range could not be viewed: {error:?}"
                                ),
                            )
                            .into(),
                        )
                    })?;
                const_body_references =
                    collect_scanned_symbol_references(view, source_id, string_table);
                // Parameter and local names belong to the callable's body scope. The shallow
                // token scan cannot resolve them, so exclude them here before they can create an
                // edge to a top-level declaration with the same spelling. Moth forbids shadowing,
                // so a body-local spelling cannot be a top-level reference.
                let mut local_names = collect_body_local_binding_names(view, string_table)
                    .map_err(|error| HeaderPreparationFailure::Infrastructure(error.into()))?;
                let parameter_names: FxHashSet<StringId> = signature
                    .parameters
                    .iter()
                    .filter_map(|parameter| path_fork.component(parameter.id))
                    .collect();
                local_names.extend(parameter_names.iter().copied());
                const_body_references.retain(|reference| !local_names.contains(&reference.name));
                // A parameter default folds while this signature resolves in Stage 3, before a
                // later const function body prepares. Record the same ordering edge as a body
                // reference so the default's callee is prepared first. Defaults cannot name
                // parameters, so only parameter spellings are excluded from their scan.
                for parameter in &signature.parameters {
                    let Some(default_range) = parameter.default_range else {
                        continue;
                    };
                    let default_view =
                        TokenFactView::from_source_range(owner.tokens_ref(), default_range)
                            .map_err(|error| {
                                HeaderPreparationFailure::Infrastructure(
                                    InfrastructureFailure::internal(
                                        CompilerStage::HeaderParsing,
                                        format!(
                                            "const function parameter default token range could not be viewed: {error:?}"
                                        ),
                                    )
                                    .into(),
                                )
                            })?;
                    const_body_references.extend(
                        collect_scanned_symbol_references(default_view, source_id, string_table)
                            .into_iter()
                            .filter(|reference| !parameter_names.contains(&reference.name)),
                    );
                }
                (const_body_references.as_slice(), header_index)
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

        let visibility = match binding_environment.visibility_for(
            &module_symbols
                .source_path_for_header(header)
                .expect("header body range has no prepared source-path identity"),
        ) {
            Ok(v) => v,
            Err(error) => return Err(HeaderPreparationFailure::Infrastructure(error.into())),
        };
        let current_source_file = module_symbols
            .source_path_for_header(header)
            .expect("header body range has no prepared source-path identity");

        let current_path = header.declaration_path;

        let current_is_const_function = matches!(
            &header.kind,
            HeaderKind::Function { signature, .. }
                if signature.evaluation
                    == CallableEvaluation::Const
        );
        // Signature capacities keep their established source-order rule even on a const
        // function. Only initializer, const-body and const-function default references may
        // point forward.
        let all_refs = initializer_refs
            .iter()
            .map(|reference| (reference, current_is_const_function))
            .chain(
                header
                    .capacity_references
                    .iter()
                    .map(|reference| (reference, false)),
            );
        for (reference, is_const_required_reference) in all_refs {
            let resolution = classify_reference(
                reference,
                visibility,
                &constant_positions,
                &struct_or_choice_paths,
                module_symbols,
                &binding_environment.imported_declarations_by_local_path,
                &binding_environment.imported_declarations_by_origin,
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
                        return Err(HeaderPreparationFailure::Infrastructure(
                            missing_constant_position_error(&path, path_fork, string_table).into(),
                        ));
                    };
                    if position.source_file == current_source_file {
                        if position.header_index > reference_header_index
                            && !is_const_required_reference
                        {
                            diagnostic_bag.push(same_file_forward_reference_error(
                                &current_path,
                                &path,
                                reference,
                                path_fork,
                            ));
                            continue;
                        }
                        report.same_file_edges += 1;
                    } else {
                        report.cross_file_edges += 1;
                    }

                    edges_to_add.push((
                        header_index,
                        LocalDeclarationOrderingHint::source_owned(path),
                    ));
                }

                // Const functions create ordering edges in either source direction. The graph
                // walk owns direct and transitive cycles, including a self edge.
                ConstantReferenceResolution::SourceConstFunction { path } => {
                    let Some(position) = constant_positions.get(&path) else {
                        return Err(HeaderPreparationFailure::Infrastructure(
                            missing_constant_position_error(&path, path_fork, string_table).into(),
                        ));
                    };
                    if position.source_file == current_source_file {
                        report.same_file_edges += 1;
                    } else {
                        report.cross_file_edges += 1;
                    }
                    edges_to_add.push((
                        header_index,
                        LocalDeclarationOrderingHint::source_owned(path),
                    ));
                }

                // Type aliases live in the type namespace. They do not create value dependency edges.
                // Imported constants and const functions are ordered by the module dependency
                // graph; their provider executable runs before this consumer.
                ConstantReferenceResolution::SourceTypeAlias { .. }
                | ConstantReferenceResolution::ImportedConstant
                | ConstantReferenceResolution::ImportedConstFunction { .. }
                | ConstantReferenceResolution::ExternalConstant { .. }
                | ConstantReferenceResolution::ConstructorLikeSource { .. } => {}

                // Source non-constants are structurally invalid in constant initializers.
                // External non-constants are deferred to AST because header stage cannot
                // determine whether an external call is foldable or valid in all contexts.
                ConstantReferenceResolution::SourceNonConstant { .. } => {
                    // A const body's or const-signature default's own eligibility pass reports
                    // its runtime call or binding with a call-site label; the dependency scan
                    // must not duplicate it. Signature capacities keep their established
                    // header-stage diagnostics.
                    if !is_const_required_reference {
                        diagnostic_bag.push(non_constant_reference_error(reference));
                    }
                }

                // External references are deferred to AST folding validation.
                ConstantReferenceResolution::ExternalNonConstant { .. } => {}

                // A constant with this name exists in the module but is not visible to this file.
                // When the invisible declaration is authored directly in an active root file,
                // name the root-local isolation rule with both sites instead of the generic
                // constant-visibility error.
                ConstantReferenceResolution::NotVisible { name } => {
                    // A const body may read its own parameter or local, and a const-signature
                    // default may name a spelling that is not visible here. Scope-aware AST
                    // resolution owns that decision, so an unresolved header name must not
                    // become a false dependency error.
                    if is_const_required_reference {
                        continue;
                    }
                    let Some(candidates) = constants_by_name.get(&name) else {
                        // No constant with this name exists anywhere. AST produces a more precise
                        // diagnostic during expression parsing.
                        continue;
                    };
                    // A Moth template body is an ordinary reader here. Qualified selection of a
                    // root facade is rejected earlier in binding with MOTH-IMPORT-0005, while
                    // naming a root-local declaration reaches this rule and reports
                    // MOTH-IMPORT-0030 like any other reader.
                    let root_local = candidates.iter().copied().find_map(|path| {
                        let position = constant_positions.get(&path)?;
                        (!module_symbols.authored_declarations_visible_from(
                            &current_source_file,
                            &position.source_file,
                        ))
                        .then_some((path, position.source_file))
                    });
                    match root_local {
                        Some((path, target_source_file)) => {
                            let diagnostic = cross_file_root_local_declaration(
                                module_symbols,
                                &current_source_file,
                                &target_source_file,
                                &path,
                                reference.span,
                            )
                            .expect(
                                "an invisible constant authored in an active root file is not visible from this consumer",
                            );
                            diagnostic_bag.push(diagnostic);
                        }
                        None => diagnostic_bag.push(not_visible_constant_error(reference)),
                    }
                }
            }
        }
    }

    // Same-file forward and self-reference diagnostics apply to constant shells only.
    // A parameter or field default may name a later constant in the same file; the ordering
    // edge sorts that constant first, matching the unqualified default. Diagnosing it here
    // would reject a program the unqualified form accepts.
    for (header_index, header) in headers.iter().enumerate() {
        let constant_shell = match &header.kind {
            HeaderKind::Constant { declaration } if declaration.config_contract.is_none() => true,
            HeaderKind::ConstTemplate { .. } => true,
            _ => false,
        };
        if !constant_shell {
            continue;
        }
        let Some(current_source_file) = module_symbols.source_path_for_header(header) else {
            continue;
        };
        let is_moth_template_content = matches!(
            header.synthetic_content_payload,
            Some(SyntheticContentPayload::MothTemplate { .. })
        );

        push_qualified_reference_diagnostics(
            QualifiedReferenceDiagnosticContext {
                declaration_path: header.declaration_path,
                name_span: header.name_span,
                header_index,
                current_source_file,
                is_moth_template_content,
                constant_positions: &constant_positions,
                path_fork,
            },
            &header.local_ordering_hints,
            &mut diagnostic_bag,
            |span| Some(hint_span_order_range(span)),
        );
    }

    for (header_index, hint) in edges_to_add {
        let header = &mut headers[header_index];
        if header.local_ordering_hints.insert(hint) {
            report.added_edges += 1;
        }
    }

    if diagnostic_bag.has_errors() {
        return Err(HeaderPreparationFailure::Diagnosed(diagnostic_bag));
    }

    Ok(report)
}

fn classify_reference(
    reference: &InitializerReference,
    visibility: &FileVisibility,
    constant_positions: &FxHashMap<PathId, ConstantPosition>,
    struct_or_choice_paths: &FxHashSet<PathId>,
    module_symbols: &ModuleSymbols,
    imported_declarations: &FxHashMap<PathId, OriginDeclarationId>,
    imported_declarations_by_origin: &FxHashMap<OriginDeclarationId, PublicDeclarationRecord>,
) -> ConstantReferenceResolution {
    // External symbols: constants are valid references; non-constants are errors.
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

    if let Some(path) = visibility.visible_type_alias_names.get(&reference.name) {
        return ConstantReferenceResolution::SourceTypeAlias {
            _path: *path.local_path(),
        };
    }

    // Namespace records: a shallow `namespace.member` access can name a source constant that
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
                imported_declarations,
                imported_declarations_by_origin,
            );
        }

        if let Some(NamespaceTypeMember::SourceDeclaration(path)) =
            record.type_members.get(&member_name)
        {
            return ConstantReferenceResolution::SourceTypeAlias {
                _path: *path.local_path(),
            };
        }

        return ConstantReferenceResolution::NotVisible { name: member_name };
    };

    // Source-visible names: may be constants, constructors, or non-constants.
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
        imported_declarations,
        imported_declarations_by_origin,
    )
}

fn classify_namespace_value_member(
    member: &NamespaceValueMember,
    constant_positions: &FxHashMap<PathId, ConstantPosition>,
    struct_or_choice_paths: &FxHashSet<PathId>,
    module_symbols: &ModuleSymbols,
    reference: &InitializerReference,
    imported_declarations: &FxHashMap<PathId, OriginDeclarationId>,
    imported_declarations_by_origin: &FxHashMap<OriginDeclarationId, PublicDeclarationRecord>,
) -> ConstantReferenceResolution {
    match member {
        NamespaceValueMember::SourceDeclaration(target_path) => {
            classify_source_declaration_reference(
                target_path.local_path(),
                constant_positions,
                struct_or_choice_paths,
                module_symbols,
                reference,
                imported_declarations,
                imported_declarations_by_origin,
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
    target_path: &PathId,
    constant_positions: &FxHashMap<PathId, ConstantPosition>,
    struct_or_choice_paths: &FxHashSet<PathId>,
    module_symbols: &ModuleSymbols,
    reference: &InitializerReference,
    imported_declarations: &FxHashMap<PathId, OriginDeclarationId>,
    imported_declarations_by_origin: &FxHashMap<OriginDeclarationId, PublicDeclarationRecord>,
) -> ConstantReferenceResolution {
    if let Some(origin) = imported_declarations.get(target_path)
        && let Some(record) = imported_declarations_by_origin.get(origin)
    {
        return match &record.semantics {
            PublicDeclarationSemantics::Constant(_) => {
                ConstantReferenceResolution::ImportedConstant
            }
            PublicDeclarationSemantics::Function(function)
                if function.evaluation == CallableEvaluation::Const
                    && reference.followed_by_call =>
            {
                ConstantReferenceResolution::ImportedConstFunction {
                    _path: *target_path,
                }
            }
            PublicDeclarationSemantics::Struct(_) | PublicDeclarationSemantics::Choice(_)
                if reference.followed_by_call || reference.followed_by_choice_namespace =>
            {
                ConstantReferenceResolution::ConstructorLikeSource {
                    _path: *target_path,
                }
            }
            PublicDeclarationSemantics::TransparentAlias(_) => {
                ConstantReferenceResolution::SourceTypeAlias {
                    _path: *target_path,
                }
            }
            _ => ConstantReferenceResolution::SourceNonConstant {
                _path: *target_path,
            },
        };
    }

    let position_kind = constant_positions
        .get(target_path)
        .map(|position| position.kind);

    if position_kind == Some(OrderedValueKind::ConstFunction) {
        return if reference.followed_by_call {
            ConstantReferenceResolution::SourceConstFunction { path: *target_path }
        } else {
            ConstantReferenceResolution::SourceNonConstant {
                _path: *target_path,
            }
        };
    }

    if position_kind.is_some() {
        // Even if the target is a constant, it might be used as a constructor-like
        // nominal if followed by a call or namespace accessor.
        if (reference.followed_by_call || reference.followed_by_choice_namespace)
            && is_nominal_constructor(target_path, struct_or_choice_paths, module_symbols)
        {
            return ConstantReferenceResolution::ConstructorLikeSource {
                _path: *target_path,
            };
        }

        return ConstantReferenceResolution::SourceConstant { path: *target_path };
    }

    // Not a constant: check if it's a legitimate constructor-like reference.
    if (reference.followed_by_call || reference.followed_by_choice_namespace)
        && is_nominal_constructor(target_path, struct_or_choice_paths, module_symbols)
    {
        return ConstantReferenceResolution::ConstructorLikeSource {
            _path: *target_path,
        };
    }

    ConstantReferenceResolution::SourceNonConstant {
        _path: *target_path,
    }
}

/// Determine whether a visible source name refers to a struct or choice declaration
/// and can therefore be used as a nominal constructor in a constant initializer.
///
/// WHY: constants may construct struct/choice literals at compile time, but function calls
/// and other non-constant references are not valid in constant initializers.
fn is_nominal_constructor(
    target_path: &PathId,
    struct_or_choice_paths: &FxHashSet<PathId>,
    module_symbols: &ModuleSymbols,
) -> bool {
    // Fast path: the header itself is a struct or choice.
    if struct_or_choice_paths.contains(target_path) {
        return true;
    }

    // Fallback: generic declarations with struct/choice kinds are also constructors.
    if let Some(kind) = module_symbols.generic_declarations_by_path.get(target_path) {
        return matches!(
            kind,
            GenericDeclarationKind::Struct | GenericDeclarationKind::Choice
        );
    }

    false
}

// ---------------------------------------------------------------------------
// Diagnostic helpers
// ---------------------------------------------------------------------------

fn self_reference_error(reference: &InitializerReference) -> DiagnosticDraft {
    self_reference_error_for(Some(reference.name), reference.span)
}

fn self_reference_error_for(name: Option<StringId>, span: Option<SourceSpan>) -> DiagnosticDraft {
    choices_control_flow::compile_time_evaluation_error(
        CompileTimeEvaluationErrorReason::ConstantSelfReference,
        name,
        span,
    )
}

fn not_visible_constant_error(reference: &InitializerReference) -> DiagnosticDraft {
    choices_control_flow::compile_time_evaluation_error(
        CompileTimeEvaluationErrorReason::ConstantNotVisible,
        Some(reference.name),
        reference.span,
    )
}

fn non_constant_reference_error(reference: &InitializerReference) -> DiagnosticDraft {
    choices_control_flow::compile_time_evaluation_error(
        CompileTimeEvaluationErrorReason::NonConstantReferenceInConstant,
        Some(reference.name),
        reference.span,
    )
}

fn same_file_forward_reference_error(
    constant_path: &PathId,
    target_path: &PathId,
    reference: &InitializerReference,
    path_fork: &PathInternerFork,
) -> DiagnosticDraft {
    same_file_forward_reference_error_for(*constant_path, *target_path, reference.span, path_fork)
}

fn same_file_forward_reference_error_for(
    constant_path: PathId,
    target_path: PathId,
    span: Option<SourceSpan>,
    path_fork: &PathInternerFork,
) -> DiagnosticDraft {
    let target_name = path_fork
        .component(target_path)
        .or_else(|| path_fork.component(constant_path));
    choices_control_flow::compile_time_evaluation_error(
        CompileTimeEvaluationErrorReason::SameFileForwardConstantReference,
        target_name,
        span,
    )
}

/// Produce an internal compiler error for a classified source constant whose position record
/// is missing from the inventory map.
///
/// WHY: constant classification reads `constant_positions` directly, so a classified
/// `SourceConstant` must always have a position record. A missing record means the map is
/// corrupted, which is a compiler bug rather than a user source error.
fn missing_constant_position_error(
    constant_path: &PathId,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> InfrastructureFailure {
    let mut scratch = Vec::new();
    InfrastructureFailure::internal(
        CompilerStage::HeaderParsing,
        format!(
            "Missing constant position metadata for classified source constant '{}' - \
         the constant inventory map is corrupted",
            path_fork.render_portable(*constant_path, string_table, &mut scratch),
        ),
    )
}

#[cfg(test)]
#[path = "tests/constant_dependencies_tests.rs"]
mod constant_dependencies_tests;
