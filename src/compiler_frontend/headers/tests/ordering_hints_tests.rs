//! Content-source ordering hints recorded from declaration shells.
//!
//! WHAT: verifies that every pre-body declaration shell records one content ordering hint per
//!       content-class file-value occurrence, and that runtime bodies, resource paths and
//!       dependency-clause rows record none.
//! WHY: direct `.mtf`/`.md` value paths reuse the synthetic `content` constant, so the shells that
//!       fold before body emission must carry the ordering fact at token level.

use crate::compiler_frontend::headers::binding_environment::{
    HeaderBindingEnvironment, QualifiedReferenceTargets, ResolvedQualifiedReference,
    ResolvedQualifiedTarget,
};
use crate::compiler_frontend::headers::module_symbols::ModuleSymbols;
use crate::compiler_frontend::headers::ordering_hints::{
    collect_content_source_ordering_hints, collect_qualified_reference_ordering_hints,
};
use crate::compiler_frontend::headers::parse_file_headers::{
    Header, HeaderKind, LocalDeclarationOrderingHint, LocalDeclarationOrderingHintOrigin,
    PreparedHeaderSyntax, prepare_file_from_tokens, prepare_header_syntax,
};
use crate::compiler_frontend::headers::types::{
    FileFrontendPrepareOutput, FileRole, HeaderParseOptions, SourceTokenOwner,
};
use crate::compiler_frontend::paths::file_references::PreparedFileReferenceClass;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceId};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::TokenizerEntryMode;
use rustc_hash::FxHashMap;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

fn prepare_source(
    source: &str,
) -> (
    crate::compiler_frontend::headers::types::FileFrontendPrepareOutput,
    StringTable,
    ExtendedSpanBuilder,
    PathInternerFork,
) {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let file_path = Path::new("@entry.moth");
    let interned_path = path_fork
        .try_intern_filesystem_path(file_path, &mut string_table)
        .expect("test path should be UTF-8");
    let mut span_builder = ExtendedSpanBuilder::new();
    let lexed = tokenize(
        source,
        interned_path,
        TokenizerEntryMode::SourceFile,
        &StyleDirectiveRegistry::built_ins(),
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("source should tokenize");
    let owner = SourceTokenOwner::new(lexed.tokens);
    let options = crate::compiler_frontend::tests::parse_support::synthetic_entry_options(
        SourceId::COMPILATION_ROOT,
    );
    let output = prepare_file_from_tokens(
        owner,
        interned_path,
        lexed.path_syntax,
        &options,
        &mut string_table,
        0,
        0,
        &mut span_builder,
        &mut path_fork,
    )
    .expect("preparation should succeed");
    (output, string_table, span_builder, path_fork)
}

fn content_hint(
    path_text: &str,
    occurrence: PathSyntaxId,
    strings: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> LocalDeclarationOrderingHint {
    let components = path_text
        .split('/')
        .map(|component| strings.intern(component))
        .collect::<Vec<_>>();
    LocalDeclarationOrderingHint::content_source(
        path_fork
            .try_intern_components(&components)
            .expect("test path fits"),
        occurrence,
    )
}

fn occurrences_of_class(
    output: &crate::compiler_frontend::headers::types::FileFrontendPrepareOutput,
    class: PreparedFileReferenceClass,
) -> Vec<PathSyntaxId> {
    output
        .structural_file_references
        .iter()
        .filter(|reference| reference.class == class)
        .map(|reference| reference.path_syntax)
        .collect()
}

fn header_of_kind<'a>(
    headers: &'a [Header],
    kind_label: &str,
    kind_matches: impl Fn(&HeaderKind) -> bool,
) -> &'a Header {
    headers
        .iter()
        .find(|header| kind_matches(&header.kind))
        .unwrap_or_else(|| panic!("expected one {kind_label} header"))
}

fn assert_hints(header: &Header, expected: &HashSet<LocalDeclarationOrderingHint>) {
    let actual_keys = header
        .local_ordering_hints
        .iter()
        .map(|hint| (hint.path(), hint.origin(), hint.occurrence()))
        .collect::<HashSet<_>>();
    let expected_keys = expected
        .iter()
        .map(|hint| (hint.path(), hint.origin(), hint.occurrence()))
        .collect::<HashSet<_>>();
    assert_eq!(
        actual_keys, expected_keys,
        "unexpected ordering hints on the declaration shell"
    );
    assert!(
        header
            .local_ordering_hints
            .iter()
            .all(|hint| hint.occurrence_span().is_some()),
        "every retained content occurrence must carry its exact authored span"
    );
}

#[test]
fn constant_initializer_content_value_records_content_hint() {
    let (output, mut strings, _span_builder, mut path_fork) =
        prepare_source("intro #= @docs/intro.mtf\n");

    let header = header_of_kind(&output.headers, "constant", |kind| {
        matches!(kind, HeaderKind::Constant { .. })
    });

    let occurrences = occurrences_of_class(&output, PreparedFileReferenceClass::ContentSource);
    assert_eq!(occurrences.len(), 1);
    let mut expected = HashSet::new();
    expected.insert(content_hint(
        "docs/intro.mtf/content",
        occurrences[0],
        &mut strings,
        &mut path_fork,
    ));
    assert_hints(header, &expected);
}

#[test]
fn source_directive_operand_records_content_hint() {
    let (output, mut strings, _span_builder, mut path_fork) =
        prepare_source("$doc [@docs/intro.mtf]\nvalue #= 1\n");

    let header = header_of_kind(&output.headers, "constant", |kind| {
        matches!(kind, HeaderKind::Constant { .. })
    });

    let occurrences = occurrences_of_class(&output, PreparedFileReferenceClass::ContentSource);
    assert_eq!(occurrences.len(), 1);
    let mut expected = HashSet::new();
    expected.insert(content_hint(
        "docs/intro.mtf/content",
        occurrences[0],
        &mut strings,
        &mut path_fork,
    ));
    assert_hints(header, &expected);
}

#[test]
fn repeated_content_values_in_one_shell_retain_each_occurrence_hint() {
    let (output, mut strings, _span_builder, mut path_fork) =
        prepare_source("[: [@docs/intro.mtf] [@docs/intro.mtf]]\n");

    let fragment_header = header_of_kind(&output.headers, "const-template", |kind| {
        matches!(kind, HeaderKind::ConstTemplate { .. })
    });

    let occurrences = occurrences_of_class(&output, PreparedFileReferenceClass::ContentSource);
    assert_eq!(occurrences.len(), 2);
    let mut expected = HashSet::new();
    for occurrence in occurrences {
        expected.insert(content_hint(
            "docs/intro.mtf/content",
            occurrence,
            &mut strings,
            &mut path_fork,
        ));
    }
    assert_hints(fragment_header, &expected);
}

#[test]
fn function_parameter_default_records_content_hint_and_body_stays_unhinted() {
    let (output, mut strings, _span_builder, mut path_fork) = prepare_source(
        "label |prefix String = [: [@docs/intro.md] ]| -> String:\n\
     \x20   io.line([: [@docs/body.md]])\n\
     ;\n",
    );

    let function_header = header_of_kind(&output.headers, "function", |kind| {
        matches!(kind, HeaderKind::Function { .. })
    });

    let occurrences = occurrences_of_class(&output, PreparedFileReferenceClass::ContentSource);
    assert_eq!(occurrences.len(), 2);
    let mut expected = HashSet::new();
    expected.insert(content_hint(
        "docs/intro.md/content",
        occurrences[0],
        &mut strings,
        &mut path_fork,
    ));
    assert_hints(function_header, &expected);
}

#[test]
fn struct_field_default_records_content_hint() {
    let (output, mut strings, _span_builder, mut path_fork) =
        prepare_source("Options = |\n    path String = [: [@docs/intro.md] ],\n|\n");

    let struct_header = header_of_kind(&output.headers, "struct", |kind| {
        matches!(kind, HeaderKind::Struct { .. })
    });

    let occurrences = occurrences_of_class(&output, PreparedFileReferenceClass::ContentSource);
    assert_eq!(occurrences.len(), 1);
    let mut expected = HashSet::new();
    expected.insert(content_hint(
        "docs/intro.md/content",
        occurrences[0],
        &mut strings,
        &mut path_fork,
    ));
    assert_hints(struct_header, &expected);
}

#[test]
fn top_level_const_fragment_records_content_hint() {
    let (output, mut strings, _span_builder, mut path_fork) =
        prepare_source("[: [@docs/intro.md]]\n");

    let fragment_header = header_of_kind(&output.headers, "const-template", |kind| {
        matches!(kind, HeaderKind::ConstTemplate { .. })
    });

    let occurrences = occurrences_of_class(&output, PreparedFileReferenceClass::ContentSource);
    assert_eq!(occurrences.len(), 1);
    let mut expected = HashSet::new();
    expected.insert(content_hint(
        "docs/intro.md/content",
        occurrences[0],
        &mut strings,
        &mut path_fork,
    ));
    assert_hints(fragment_header, &expected);
}

#[test]
fn runtime_start_body_content_value_records_no_content_hint() {
    let (output, _, _span_builder, _path_fork) = prepare_source("io.line([: [@docs/intro.mtf]])\n");

    let start_header = header_of_kind(&output.headers, "start", |kind| {
        matches!(kind, HeaderKind::StartFunction)
    });

    // The entry body must genuinely contain a classified content occurrence, so the no-hint
    // assertion below cannot pass on a shape the collector never visits.
    assert_eq!(
        occurrences_of_class(&output, PreparedFileReferenceClass::ContentSource).len(),
        1,
        "the entry body should carry exactly one classified content occurrence"
    );
    assert!(
        start_header.local_ordering_hints.is_empty(),
        "runtime start bodies fold after content constants and must carry no content hints"
    );
}

#[test]
fn resource_file_value_records_no_content_hint() {
    let (output, _, _span_builder, _path_fork) = prepare_source("icon #= @assets/logo.svg\n");

    let header = header_of_kind(&output.headers, "constant", |kind| {
        matches!(kind, HeaderKind::Constant { .. })
    });

    assert!(
        header.local_ordering_hints.is_empty(),
        "resource-only file values resolve through Stage 0 without a content ordering edge"
    );
}

#[test]
fn struct_field_resource_default_records_no_content_hint() {
    let (output, _strings, _span_builder, _path_fork) =
        prepare_source("Options = |\n    icon_url String = @assets/logo.svg,\n|\n");

    let struct_header = header_of_kind(&output.headers, "struct", |kind| {
        matches!(kind, HeaderKind::Struct { .. })
    });

    // The field default must genuinely classify as a resource occurrence, so the no-hint
    // assertion below cannot pass on a shape preparation stopped visiting.
    assert_eq!(
        occurrences_of_class(&output, PreparedFileReferenceClass::ResourceFile).len(),
        1,
        "the field default should carry exactly one classified resource occurrence"
    );
    assert!(
        struct_header.local_ordering_hints.is_empty(),
        "a resource-only field default needs no content ordering edge, got {:?} in {:?}",
        struct_header.local_ordering_hints,
        struct_header.declaration_path
    );
}

#[test]
fn dependency_clause_rows_record_no_content_hint() {
    let (output, _strings, _span_builder, _path_fork) = prepare_source(
        "@core/math sin\n\
     unused #= @assets/logo.svg\n",
    );

    for header in &output.headers {
        assert!(
            header.local_ordering_hints.is_empty(),
            "clause-consumed and resource rows must record no hints, got {:?} on {:?}",
            header.local_ordering_hints,
            header.declaration_path
        );
    }
}

#[test]
fn recollecting_content_hints_deduplicates_into_the_hint_set() {
    // Re-running the collector over the same shells and rows must be a no-op, so no ordering or
    // diagnostic difference can depend on worklist insertion order.
    let (mut output, mut strings, _span_builder, mut path_fork) = prepare_source(
        "intro #= @docs/intro.mtf\n\
     other #= @docs/other.mtf\n",
    );

    let before: Vec<usize> = output
        .headers
        .iter()
        .map(|header| header.local_ordering_hints.len())
        .collect();
    let source_tokens = output
        .source_token_stream
        .as_ref()
        .expect("prepared source retains its token owner")
        .clone();
    collect_content_source_ordering_hints(
        &mut output.headers,
        &source_tokens,
        &output.structural_file_references,
        output.path_syntax.table(),
        &mut strings,
        &mut path_fork,
    )
    .expect("re-collecting valid path rows should succeed");
    let after: Vec<usize> = output
        .headers
        .iter()
        .map(|header| header.local_ordering_hints.len())
        .collect();

    assert_eq!(
        before, after,
        "re-collection must deduplicate into the hint set"
    );
    let constants = output
        .headers
        .iter()
        .filter(|header| matches!(header.kind, HeaderKind::Constant { .. }));
    assert!(
        constants
            .map(|header| header.local_ordering_hints.len())
            .eq(std::iter::repeat_n(1usize, 2)),
        "each shell with one content value should hold exactly one hint"
    );
}

#[test]
fn config_declaration_directive_operand_records_content_hint() {
    let (output, mut strings, _span_builder, mut path_fork) = prepare_source(
        "$config\n$doc [@docs/intro.mtf]\napi_url #String = \"https://example.test\"\n",
    );

    let header = header_of_kind(&output.headers, "constant", |kind| {
        matches!(kind, HeaderKind::Constant { .. })
    });

    let occurrences = occurrences_of_class(&output, PreparedFileReferenceClass::ContentSource);
    assert_eq!(occurrences.len(), 1);
    let mut expected = HashSet::new();
    expected.insert(content_hint(
        "docs/intro.mtf/content",
        occurrences[0],
        &mut strings,
        &mut path_fork,
    ));
    assert_hints(header, &expected);
}

// ------------------------
//  Qualified-reference ordering across rebased file headers
// ------------------------

fn prepare_module_file(
    source: &str,
    file_path: &str,
    source_id: SourceId,
    role: Option<FileRole>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> FileFrontendPrepareOutput {
    let file_path = Path::new(file_path);
    let interned_path = path_fork
        .try_intern_filesystem_path(file_path, string_table)
        .expect("test path should be UTF-8");
    let mut span_builder = ExtendedSpanBuilder::new();
    let lexed = tokenize(
        source,
        interned_path,
        TokenizerEntryMode::SourceFile,
        &StyleDirectiveRegistry::built_ins(),
        string_table,
        path_fork,
        source_id,
        &mut span_builder,
    )
    .expect("source should tokenize");
    let owner = SourceTokenOwner::new(lexed.tokens);
    let options = match role {
        Some(role) => HeaderParseOptions {
            selected_file: Some((source_id, role)),
            ..Default::default()
        },
        None => HeaderParseOptions::default(),
    };
    prepare_file_from_tokens(
        owner,
        interned_path,
        lexed.path_syntax,
        &options,
        string_table,
        0,
        0,
        &mut span_builder,
        path_fork,
    )
    .expect("preparation should succeed")
}

fn prepare_qualified_hint_module(
    sources: &[(&str, &str, SourceId, Option<FileRole>)],
) -> (PreparedHeaderSyntax, StringTable, PathInternerFork) {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut outputs = Vec::with_capacity(sources.len());
    for (source, file_path, source_id, role) in sources {
        outputs.push(prepare_module_file(
            source,
            file_path,
            *source_id,
            *role,
            &mut string_table,
            &mut path_fork,
        ));
    }

    let mut capture = |source, diagnostic: &mut crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft| {
        crate::compiler_frontend::compiler_messages::diagnostics::draft::validate_preparation_draft_spans(
            diagnostic, source,
        )
    };
    let prepared = prepare_header_syntax(
        &mut outputs,
        &mut string_table,
        &mut capture,
        &mut path_fork,
    )
    .expect("module headers should prepare");
    (prepared, string_table, path_fork)
}

fn declaration_paths_by_name(
    headers: &[Header],
    string_table: &StringTable,
    path_fork: &PathInternerFork,
) -> FxHashMap<String, PathId> {
    let mut paths = FxHashMap::default();
    for header in headers {
        let Some(name) = path_fork.component(header.declaration_path) else {
            continue;
        };
        paths.insert(
            string_table.resolve(name).to_owned(),
            header.declaration_path,
        );
    }
    paths
}

/// Build a synthetic binding environment from the module's prepared occurrences.
///
/// WHAT: resolves every selected name with a same-module declaration to a `Source` target.
/// WHY: this test isolates header-index ownership in ordering-hint collection; provider binding
///      semantics are covered by the binding-environment and namespace tests.
fn qualified_targets_for_module(
    module_symbols: &ModuleSymbols,
    declaration_paths: &FxHashMap<String, PathId>,
    string_table: &StringTable,
) -> QualifiedReferenceTargets {
    let mut targets = QualifiedReferenceTargets::default();
    for occurrences in module_symbols.qualified_references_by_source.values() {
        for occurrence in occurrences {
            let Some(declaration_path) =
                declaration_paths.get(string_table.resolve(occurrence.selected))
            else {
                continue;
            };
            targets.insert(
                (
                    occurrence.selected_span.source(),
                    occurrence.dependency.path_syntax,
                ),
                ResolvedQualifiedReference {
                    target: ResolvedQualifiedTarget::Source {
                        declaration_path: *declaration_path,
                    },
                    selected_span: occurrence.selected_span,
                },
            );
        }
    }
    targets
}

fn header_named<'a>(
    headers: &'a [Header],
    string_table: &StringTable,
    path_fork: &PathInternerFork,
    name: &str,
) -> &'a Header {
    headers
        .iter()
        .find(|header| {
            path_fork
                .component(header.declaration_path)
                .is_some_and(|component| string_table.resolve(component) == name)
        })
        .unwrap_or_else(|| panic!("expected a header named {name}"))
}

fn qualified_hint_paths(header: &Header) -> Vec<PathId> {
    let mut paths: Vec<PathId> = header
        .local_ordering_hints
        .iter()
        .filter(|hint| hint.origin() == LocalDeclarationOrderingHintOrigin::QualifiedReference)
        .map(LocalDeclarationOrderingHint::path)
        .collect();
    paths.sort();
    paths
}

#[test]
fn qualified_hints_follow_rebased_header_indexes_and_const_eligibility() {
    let entry_source = "\
a_target #= 1
b_target #= 2
c_target #= 3
d_target #= 4
runtime_target #= 5
helper_fn || -> I32:
    return 1
;
alpha #= @helper a_target
beta #= [: [@helper b_target] [@helper c_target] ]
delta #= @helper d_target
missing_ref #= @helper absent_target
function_ref #= @helper helper_fn
render || -> String:
    return [: [@helper runtime_target] ]
;
";
    let helper_source = "\
e_target #= 6
gamma #= @helper e_target
";
    let (prepared, string_table, path_fork) = prepare_qualified_hint_module(&[
        (
            entry_source,
            "src/@entry.moth",
            SourceId::COMPILATION_ROOT,
            Some(FileRole::ActiveEntryRoot),
        ),
        (
            helper_source,
            "src/helper.moth",
            SourceId::from_index(1),
            None,
        ),
    ]);
    let PreparedHeaderSyntax {
        mut headers,
        mut module_symbols,
        ..
    } = prepared;

    let declaration_paths = declaration_paths_by_name(&headers, &string_table, &path_fork);
    for header in &headers {
        if matches!(header.kind, HeaderKind::Constant { .. }) {
            module_symbols
                .constant_paths
                .insert(header.declaration_path);
        }
    }

    let environment = HeaderBindingEnvironment {
        qualified_reference_targets: Arc::new(qualified_targets_for_module(
            &module_symbols,
            &declaration_paths,
            &string_table,
        )),
        ..HeaderBindingEnvironment::default()
    };
    assert!(
        environment
            .qualified_reference_targets
            .keys()
            .any(|(source, _)| *source == SourceId::from_index(1)),
        "the helper file must contribute a rebased occurrence"
    );

    collect_qualified_reference_ordering_hints(&mut headers, &module_symbols, &environment)
        .expect("valid rebased indexes should collect");

    // Every const-required, resolved constant target records exactly one hint on its own shell.
    assert_eq!(
        qualified_hint_paths(header_named(&headers, &string_table, &path_fork, "alpha")),
        vec![declaration_paths["a_target"]]
    );
    assert_eq!(
        qualified_hint_paths(header_named(&headers, &string_table, &path_fork, "beta")),
        vec![declaration_paths["b_target"], declaration_paths["c_target"]]
    );
    assert_eq!(
        qualified_hint_paths(header_named(&headers, &string_table, &path_fork, "delta")),
        vec![declaration_paths["d_target"]]
    );

    // The rebased helper-file index must attach to the helper shell, not an entry shell.
    let gamma = header_named(&headers, &string_table, &path_fork, "gamma");
    assert_eq!(
        qualified_hint_paths(gamma),
        vec![declaration_paths["e_target"]],
        "a rebased occurrence must land on its own file's header"
    );
    assert!(
        gamma.local_ordering_hints.iter().all(|hint| hint
            .occurrence_span()
            .is_some_and(|span| span.source() == SourceId::from_index(1))),
        "the helper shell must retain the authored helper-file span"
    );

    // Missing targets, non-constant targets and runtime-only occurrences add no ordering edge.
    for name in ["missing_ref", "function_ref", "render", "runtime_target"] {
        assert!(
            qualified_hint_paths(header_named(&headers, &string_table, &path_fork, name))
                .is_empty(),
            "{name} must not carry a qualified ordering hint"
        );
    }
    assert_eq!(
        headers
            .iter()
            .map(|header| qualified_hint_paths(header).len())
            .sum::<usize>(),
        5,
        "exactly the five const-required constant targets emit hints"
    );
}

#[test]
fn qualified_hints_cover_many_headers_without_cross_attachment() {
    let mut entry_source = String::new();
    let mut helper_source = String::new();
    let mut expected_refs = Vec::new();
    for index in 0..16 {
        entry_source.push_str(&format!("entry_target_{index} #= {index}\n"));
        helper_source.push_str(&format!("helper_target_{index} #= {index}\n"));
    }
    for index in 0..16 {
        entry_source.push_str(&format!(
            "entry_ref_{index} #= @helper entry_target_{index}\n"
        ));
        helper_source.push_str(&format!(
            "helper_ref_{index} #= @helper helper_target_{index}\n"
        ));
        expected_refs.push(format!("entry_ref_{index}"));
        expected_refs.push(format!("helper_ref_{index}"));
    }

    let (prepared, string_table, path_fork) = prepare_qualified_hint_module(&[
        (
            &entry_source,
            "src/@entry.moth",
            SourceId::COMPILATION_ROOT,
            Some(FileRole::ActiveEntryRoot),
        ),
        (
            &helper_source,
            "src/helper.moth",
            SourceId::from_index(1),
            None,
        ),
    ]);
    let PreparedHeaderSyntax {
        mut headers,
        mut module_symbols,
        ..
    } = prepared;
    let declaration_paths = declaration_paths_by_name(&headers, &string_table, &path_fork);
    for header in &headers {
        if matches!(header.kind, HeaderKind::Constant { .. }) {
            module_symbols
                .constant_paths
                .insert(header.declaration_path);
        }
    }

    let environment = HeaderBindingEnvironment {
        qualified_reference_targets: Arc::new(qualified_targets_for_module(
            &module_symbols,
            &declaration_paths,
            &string_table,
        )),
        ..HeaderBindingEnvironment::default()
    };

    collect_qualified_reference_ordering_hints(&mut headers, &module_symbols, &environment)
        .expect("many rebased indexes should collect");

    for reference in &expected_refs {
        let expected_target = if reference.starts_with("entry_ref") {
            format!(
                "entry_target_{}",
                reference.trim_start_matches("entry_ref_")
            )
        } else {
            format!(
                "helper_target_{}",
                reference.trim_start_matches("helper_ref_")
            )
        };
        assert_eq!(
            qualified_hint_paths(header_named(&headers, &string_table, &path_fork, reference)),
            vec![declaration_paths[&expected_target]],
            "{reference} must point at {expected_target}"
        );
    }
}
