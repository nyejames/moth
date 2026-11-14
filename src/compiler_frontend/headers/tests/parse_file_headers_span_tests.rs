use super::*;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

/// Preparation retains exact dependency and file-path ranges across identity normalization.
#[test]
fn dependency_ranges_survive_string_remapping_and_source_rebinding() {
    let long_name = "é".repeat(700);
    let path_text = format!("@vendor/\"{long_name}.js\"");
    let source = format!(
        "-- 🦋\n{path_text} render as render_other, Button as UiButton\n@core/math as maths\nlogo #= @images/logo.svg\n"
    );
    let canonical = PathBuf::from("dependency-spans.moth");
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let sources = SourceDatabase::build([&canonical], &canonical, None, &mut strings)
        .expect("source should register");
    let source_id = sources
        .get_by_canonical_path(&canonical)
        .expect("registered source")
        .id;
    let options = HeaderParseOptions::default();
    let directives = StyleDirectiveRegistry::built_ins();
    let scope = path_fork
        .try_intern_filesystem_path(&canonical, &mut strings)
        .expect("source path");
    let mut spans = ExtendedSpanBuilder::new();
    let tokens = tokenize(
        &source,
        scope,
        TokenizerEntryMode::SourceFile,
        &directives,
        &mut strings,
        &mut path_fork,
        source_id,
        &mut spans,
    )
    .expect("source should tokenize");
    let tokenizer_span_count = spans.len();
    let (owner, path_syntax) = super::canonical_handoff(tokens);
    let mut prepared = parse_file_headers_with_table(
        owner,
        scope,
        path_syntax,
        &options,
        &mut strings,
        &mut path_fork,
        0,
        0,
        &mut spans,
    )
    .expect("dependency preparation should succeed");
    let original_span = prepared.file_dependency_clauses[0].dependency.span;
    assert_eq!(
        spans.len(),
        tokenizer_span_count,
        "dependency header records should reuse the lexical overflow rows"
    );
    let dependency_range = original_span.resolve_with(spans.resolver_for(source_id));
    assert_eq!(
        source.get(dependency_range.start() as usize..dependency_range.end() as usize),
        Some(path_text.as_str()),
        "the retained dependency span should resolve to its authored path"
    );

    let mut merged = StringTable::new();
    let mut remapped_path_fork = PathInternerFork::empty();
    merged.intern("unrelated");
    let remap = merged.merge_from(&strings);
    let path_remap = remapped_path_fork
        .merge_delta_from(&path_fork, &remap)
        .expect("source paths should remap");
    prepared
        .remap_string_ids(&remap)
        .expect("prepared strings should remap");
    prepared
        .remap_path_ids(&path_remap)
        .expect("prepared paths should remap");
    drop(sources);
    let earlier_source = PathBuf::from("a-earlier.moth");
    let final_sources =
        SourceDatabase::build([&earlier_source, &canonical], &canonical, None, &mut merged)
            .expect("final source membership should register");
    let final_id = final_sources
        .get_by_canonical_path(&canonical)
        .expect("final source")
        .id;
    assert_ne!(
        final_id, source_id,
        "canonical membership changes the provisional identity"
    );
    let final_path = remapped_path_fork
        .try_intern_portable_path("dependency-spans.moth", &mut merged)
        .expect("test path fits");
    prepared
        .rebind_source_identity(final_id, final_path, &mut remapped_path_fork)
        .expect("retained source should rebind");
    assert_eq!(
        prepared.file_dependency_clauses[0].dependency.span.local(),
        original_span.local()
    );
    assert_eq!(
        prepared.file_dependency_clauses[0]
            .dependency
            .dependency_shell_id
            .source,
        final_id
    );

    let mut database = SourceDatabaseBuilder::new(final_sources);
    database
        .sources_mut()
        .retain_text(final_id, source.clone())
        .expect("retain source snapshot");
    database.retain_span_builder(final_id, spans);
    database
        .finish_source(final_id)
        .expect("test source should finalize");
    database
        .assemble_finalized_tables()
        .expect("the original span table should assemble");
    let database = database.finish().expect("install original span table");
    let resolve = |span: SourceSpan| {
        let range = span.byte_range(&database);
        &source[range.start() as usize..range.end() as usize]
    };
    let rebound_dependency_span = prepared.file_dependency_clauses[0].dependency.span;
    assert_eq!(rebound_dependency_span.source(), final_id);
    assert_eq!(resolve(rebound_dependency_span), path_text);
    let selections = &prepared.dependency_selections;
    assert_eq!(resolve(selections[0].source_span), "render");
    assert_eq!(
        resolve(
            selections[0]
                .local_alias
                .as_ref()
                .expect("entry alias")
                .span
        ),
        "render_other"
    );
    assert_eq!(resolve(selections[1].source_span), "Button");
    assert_eq!(
        resolve(
            selections[1]
                .local_alias
                .as_ref()
                .expect("entry alias")
                .span
        ),
        "UiButton"
    );
    let DependencyBindingSyntax::Namespace { alias: Some(alias) } =
        &prepared.file_dependency_clauses[1].binding
    else {
        panic!("expected namespace alias");
    };
    assert_eq!(resolve(alias.span), "maths");
    let reference = prepared
        .structural_file_references
        .iter()
        .next()
        .expect("structural file reference");
    assert_eq!(reference.source_file, final_id);
    assert_eq!(resolve(reference.span), "@images/logo.svg");
}

#[test]
fn declaration_initializer_reference_member_return_and_variant_spans_retain_original_ranges() {
    let member_name = format!("value_{}", "x".repeat(1100));
    let return_name = format!("Output{}", "X".repeat(1100));
    let variant_name = format!("Ready{}", "X".repeat(1100));
    let source = format!(
        "-- é🦋\nprocess |{member_name} I32| -> {return_name}:\n;\nState ::\n{variant_name} | field I32 |,\n;\nRecord = | member I32 |\nsource_reference #= target_value\n"
    );
    let canonical = PathBuf::from("member-spans.moth");
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let sources = SourceDatabase::build([&canonical], &canonical, None, &mut strings)
        .expect("registered source");
    let source_id = sources
        .get_by_canonical_path(&canonical)
        .expect("source identity")
        .id;
    let scope = path_fork
        .try_intern_filesystem_path(&canonical, &mut strings)
        .expect("source path");
    let mut spans = ExtendedSpanBuilder::new();
    let tokens = tokenize(
        &source,
        scope,
        TokenizerEntryMode::SourceFile,
        &StyleDirectiveRegistry::built_ins(),
        &mut strings,
        &mut path_fork,
        source_id,
        &mut spans,
    )
    .expect("source should tokenize");
    let (owner, path_syntax) = super::canonical_handoff(tokens);
    let options =
        crate::compiler_frontend::tests::parse_support::synthetic_entry_options(source_id);
    let mut prepared = parse_file_headers_with_table(
        owner,
        scope,
        path_syntax,
        &options,
        &mut strings,
        &mut path_fork,
        0,
        0,
        &mut spans,
    )
    .expect("shells should prepare");
    assert_eq!(
        spans.len(),
        3,
        "member, return and variant retain their original overflow rows"
    );

    let (original_member_span, original_return_span) = prepared
        .headers
        .iter()
        .find_map(|header| match &header.kind {
            HeaderKind::Function { signature, .. } => Some((
                signature.parameters[0]
                    .span
                    .expect("authored parameter should retain its span"),
                signature.ordinary_return_slots()[0]
                    .value
                    .span
                    .expect("authored return should retain its span"),
            )),
            _ => None,
        })
        .expect("function shell");
    let original_variant_span = prepared
        .headers
        .iter()
        .find_map(|header| match &header.kind {
            HeaderKind::Choice { variants, .. } => Some(
                variants[0]
                    .span
                    .expect("authored choice variant should retain its span"),
            ),
            _ => None,
        })
        .expect("choice shell");
    let original_initializer_reference_span = prepared
        .headers
        .iter()
        .find_map(|header| match &header.kind {
            HeaderKind::Constant { declaration, .. } => declaration
                .initializer_references
                .first()
                .and_then(|reference| reference.span),
            _ => None,
        })
        .expect("constant initializer reference should retain its source span");
    assert_eq!(original_initializer_reference_span.source(), source_id);

    let mut merged = StringTable::new();
    let mut remapped_path_fork = PathInternerFork::empty();
    merged.intern("unrelated");
    let remap = merged.merge_from(&strings);
    let path_remap = remapped_path_fork
        .merge_delta_from(&path_fork, &remap)
        .expect("source paths should remap");
    prepared
        .remap_string_ids(&remap)
        .expect("shell strings should remap");
    prepared
        .remap_path_ids(&path_remap)
        .expect("shell paths should remap");
    drop(sources);
    let earlier_source = PathBuf::from("a-earlier.moth");
    let final_sources =
        SourceDatabase::build([&earlier_source, &canonical], &canonical, None, &mut merged)
            .expect("final source membership should register");
    let final_id = final_sources
        .get_by_canonical_path(&canonical)
        .expect("final source")
        .id;
    assert_ne!(
        final_id, source_id,
        "canonical membership changes the provisional identity"
    );
    let final_path = remapped_path_fork
        .try_intern_portable_path("member-spans.moth", &mut merged)
        .expect("test path fits");
    prepared
        .rebind_source_identity(final_id, final_path, &mut remapped_path_fork)
        .expect("retained source should rebind");
    assert_eq!(prepared.file_id, final_id);

    let mut database = SourceDatabaseBuilder::new(final_sources);
    database
        .sources_mut()
        .retain_text(final_id, source.clone())
        .expect("retain snapshot");
    database.retain_span_builder(final_id, spans);
    database
        .finish_source(final_id)
        .expect("test source should finalize");
    database
        .assemble_finalized_tables()
        .expect("the original span table should assemble");
    let database = database.finish().expect("install original span table");
    let resolve = |span: SourceSpan| {
        let range = span.byte_range(&database);
        &source[range.start() as usize..range.end() as usize]
    };
    let signature = prepared
        .headers
        .iter()
        .find_map(|header| match &header.kind {
            HeaderKind::Function { signature, .. } => Some(signature),
            _ => None,
        })
        .expect("function shell");
    let rebound_member_span = signature.parameters[0]
        .span
        .expect("authored parameter should retain its span");
    let rebound_return_span = signature.ordinary_return_slots()[0]
        .value
        .span
        .expect("authored return should retain its span");
    assert_eq!(rebound_member_span.local(), original_member_span.local());
    assert_eq!(rebound_member_span.source(), final_id);
    assert_eq!(rebound_return_span.local(), original_return_span.local());
    assert_eq!(rebound_return_span.source(), final_id);
    assert_eq!(
        resolve(
            signature.parameters[0]
                .span
                .expect("authored parameter should retain its span")
        ),
        member_name
    );
    assert_eq!(
        remapped_path_fork
            .component(signature.parameters[0].id)
            .map(|name| merged.resolve(name)),
        Some(member_name.as_str())
    );
    assert_eq!(
        resolve(
            signature.ordinary_return_slots()[0]
                .value
                .span
                .expect("authored return should retain its span")
        ),
        return_name
    );
    let variants = prepared
        .headers
        .iter()
        .find_map(|header| match &header.kind {
            HeaderKind::Choice { variants, .. } => Some(variants),
            _ => None,
        })
        .expect("choice shell");
    let rebound_variant_span = variants[0]
        .span
        .expect("authored choice variant should retain its span");
    assert_eq!(rebound_variant_span.local(), original_variant_span.local());
    assert_eq!(rebound_variant_span.source(), final_id);
    assert_eq!(
        resolve(
            variants[0]
                .span
                .expect("authored choice variant should retain its span")
        ),
        variant_name
    );
    assert_eq!(merged.resolve(variants[0].id), variant_name);
    let ChoiceVariantPayloadSyntax::Record { fields } = &variants[0].payload else {
        panic!("record payload");
    };
    assert_eq!(
        resolve(
            fields[0]
                .span
                .expect("authored variant field should retain its span")
        ),
        "field"
    );
    let fields = prepared
        .headers
        .iter()
        .find_map(|header| match &header.kind {
            HeaderKind::Struct { fields, .. } => Some(fields),
            _ => None,
        })
        .expect("struct shell");
    assert_eq!(
        resolve(
            fields[0]
                .span
                .expect("authored struct field should retain its span")
        ),
        "member"
    );
    let rebound_initializer_reference_span = prepared
        .headers
        .iter()
        .find_map(|header| match &header.kind {
            HeaderKind::Constant { declaration, .. } => declaration
                .initializer_references
                .first()
                .and_then(|reference| reference.span),
            _ => None,
        })
        .expect("constant initializer reference should retain its rebound source span");
    assert_eq!(
        rebound_initializer_reference_span.source(),
        final_id,
        "initializer references must use their declaration's finalized source identity"
    );
    assert_eq!(
        rebound_initializer_reference_span.local(),
        original_initializer_reference_span.local(),
        "source rebinding must preserve the exact initializer-reference range"
    );
    assert_eq!(
        resolve(rebound_initializer_reference_span),
        "target_value",
        "the final source database must resolve the original reference text"
    );
}

#[test]
fn const_template_and_config_spans_rebind_before_whole_file_freeze() {
    let source = "$config\nrelease_version #I32 = 1\n[if show_banner: Hello]\n";
    let canonical = PathBuf::from("src/@entry.moth");
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let sources = SourceDatabase::build([&canonical], &canonical, None, &mut strings)
        .expect("registered source");
    let source_id = sources
        .get_by_canonical_path(&canonical)
        .expect("source identity")
        .id;
    let scope = path_fork
        .try_intern_filesystem_path(&canonical, &mut strings)
        .expect("source path");
    let mut spans = ExtendedSpanBuilder::new();
    let tokens = tokenize(
        source,
        scope,
        TokenizerEntryMode::SourceFile,
        &StyleDirectiveRegistry::built_ins(),
        &mut strings,
        &mut path_fork,
        source_id,
        &mut spans,
    )
    .expect("source should tokenize");
    let (owner, path_syntax) = super::canonical_handoff(tokens);
    let options =
        crate::compiler_frontend::tests::parse_support::synthetic_entry_options(source_id);
    let mut prepared = parse_file_headers_with_table(
        owner,
        scope,
        path_syntax,
        &options,
        &mut strings,
        &mut path_fork,
        0,
        0,
        &mut spans,
    )
    .expect("headers should prepare");

    let original_condition_span = prepared
        .headers
        .iter()
        .find_map(|header| match &header.kind {
            HeaderKind::ConstTemplate {
                condition_references,
            } => condition_references
                .first()
                .and_then(|reference| reference.span),
            _ => None,
        })
        .expect("const-template condition reference span");
    let (original_marker_span, original_declaration_type_span) = prepared
        .headers
        .iter()
        .find_map(|header| match &header.kind {
            HeaderKind::Constant { declaration } => {
                let declaration_type_span = match &declaration.type_annotation {
                    ParsedTypeRef::BuiltinFixedScalar {
                        scalar: FixedScalar::I32,
                        span: Some(span),
                    } => *span,
                    _ => panic!("declaration should retain its parsed I32 span"),
                };
                Some((
                    declaration.config_contract.as_ref()?.marker_span,
                    declaration_type_span,
                ))
            }
            _ => None,
        })
        .expect("source config declaration");
    for span in [
        original_condition_span,
        original_marker_span,
        original_declaration_type_span,
    ] {
        assert_eq!(span.source(), source_id, "span should begin provisional");
    }

    let mut merged = StringTable::new();
    let mut remapped_path_fork = PathInternerFork::empty();
    merged.intern("unrelated");
    let remap = merged.merge_from(&strings);
    let path_remap = remapped_path_fork
        .merge_delta_from(&path_fork, &remap)
        .expect("source paths should remap");
    prepared
        .remap_string_ids(&remap)
        .expect("header strings should remap");
    prepared
        .remap_path_ids(&path_remap)
        .expect("header paths should remap");
    drop(sources);

    let earlier_source = PathBuf::from("src/0-earlier.moth");
    let final_sources =
        SourceDatabase::build([&earlier_source, &canonical], &canonical, None, &mut merged)
            .expect("final source membership should register");
    let final_id = final_sources
        .get_by_canonical_path(&canonical)
        .expect("final source")
        .id;
    assert_ne!(
        final_id, source_id,
        "the finalized source identity should change"
    );
    let final_path = remapped_path_fork
        .try_intern_portable_path("src/@entry.moth", &mut merged)
        .expect("test path fits");
    prepared
        .rebind_source_identity(final_id, final_path, &mut remapped_path_fork)
        .expect("retained source should rebind");

    let rebound_condition_span = prepared
        .headers
        .iter()
        .find_map(|header| match &header.kind {
            HeaderKind::ConstTemplate {
                condition_references,
            } => condition_references
                .first()
                .and_then(|reference| reference.span),
            _ => None,
        })
        .expect("rebound condition reference span");
    let rebound_config_spans = prepared
        .headers
        .iter()
        .find_map(|header| match &header.kind {
            HeaderKind::Constant { declaration } => {
                let declaration_type_span = match &declaration.type_annotation {
                    ParsedTypeRef::BuiltinFixedScalar {
                        scalar: FixedScalar::I32,
                        span: Some(span),
                    } => *span,
                    _ => panic!("declaration should retain its parsed I32 span"),
                };
                Some((
                    declaration.config_contract.as_ref()?.marker_span,
                    declaration_type_span,
                ))
            }
            _ => None,
        })
        .expect("rebound config spans");
    assert_eq!(rebound_condition_span.source(), final_id);
    assert_eq!(
        rebound_condition_span.local(),
        original_condition_span.local()
    );
    for (rebound, original) in [
        (rebound_config_spans.0, original_marker_span),
        (rebound_config_spans.1, original_declaration_type_span),
    ] {
        assert_eq!(rebound.source(), final_id);
        assert_eq!(rebound.local(), original.local());
    }

    prepared
        .freeze_path_syntax(&merged, &mut remapped_path_fork)
        .expect("whole-file invariants should hold at freeze");
    let mut database = SourceDatabaseBuilder::new(final_sources);
    database
        .sources_mut()
        .retain_text(final_id, source.to_owned())
        .expect("retain source snapshot");
    database.retain_span_builder(final_id, spans);
    database
        .finish_source(final_id)
        .expect("test source should finalize");
    database
        .assemble_finalized_tables()
        .expect("the original span table should assemble");
    let database = database.finish().expect("install original span table");
    let resolve = |span: SourceSpan| {
        let range = span.byte_range(&database);
        &source[range.start() as usize..range.end() as usize]
    };
    assert_eq!(resolve(rebound_condition_span), "show_banner");
    assert_eq!(resolve(rebound_config_spans.0), "$config");
    assert_eq!(resolve(rebound_config_spans.1), "I32");
}

#[test]
fn trait_shell_spans_retain_original_ranges_after_remapping_and_rebinding() {
    let trait_name = format!("DISPLAYABLE_{}", "X".repeat(1100));
    let requirement_name = format!("render_{}", "x".repeat(1100));
    let target_name = format!("Target{}", "X".repeat(1100));
    let incompatible_name = format!("INCOMPATIBLE_{}", "Y".repeat(1100));
    let source = format!(
        "-- é🦋\n\
{trait_name} must:\n\
    {requirement_name} |This| -> String\n\
;\n\
{target_name} must {trait_name}, SERIALIZABLE\n\
{trait_name} must not {incompatible_name}, OTHER_TRAIT\n\
Generic of A must {trait_name}\n"
    );
    let canonical = PathBuf::from("trait-spans.moth");
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let sources = SourceDatabase::build([&canonical], &canonical, None, &mut strings)
        .expect("registered source");
    let source_id = sources
        .get_by_canonical_path(&canonical)
        .expect("source identity")
        .id;
    let scope = path_fork
        .try_intern_filesystem_path(&canonical, &mut strings)
        .expect("source path");
    let mut spans = ExtendedSpanBuilder::new();
    let tokens = tokenize(
        &source,
        scope,
        TokenizerEntryMode::SourceFile,
        &StyleDirectiveRegistry::built_ins(),
        &mut strings,
        &mut path_fork,
        source_id,
        &mut spans,
    )
    .expect("source should tokenize");
    let (owner, path_syntax) = super::canonical_handoff(tokens);
    let mut prepared = parse_file_headers_with_table(
        owner,
        scope,
        path_syntax,
        &HeaderParseOptions::default(),
        &mut strings,
        &mut path_fork,
        0,
        0,
        &mut spans,
    )
    .expect("trait shells should prepare");

    let snapshot = |prepared: &FileFrontendPrepareOutput,
                    table: &StringTable,
                    path_fork: &PathInternerFork| {
        let mut anchors = Vec::new();
        let mut generated_header_names = Vec::new();
        let mut header_counts = [0usize; 4];
        let mut path_scratch = Vec::new();
        for header in &prepared.headers {
            match &header.kind {
                HeaderKind::Trait { declaration } => {
                    header_counts[0] += 1;
                    anchors.push((declaration.span, table.resolve(declaration.name).to_owned()));
                    let requirement = declaration.requirements.first().expect("trait requirement");
                    anchors.push((requirement.span, table.resolve(requirement.name).to_owned()));
                }
                HeaderKind::TraitConformance { conformance } => {
                    match conformance.target.kind {
                        ConformanceTargetKind::Named => {
                            header_counts[1] += 1;
                        }
                        ConformanceTargetKind::SpecializedGenericInstance => {
                            header_counts[3] += 1;
                        }
                    }
                    generated_header_names.push(path_fork.render_portable(
                        header.declaration_path,
                        table,
                        &mut path_scratch,
                    ));
                    anchors.push((
                        conformance.target.span,
                        table.resolve(conformance.target.name).to_owned(),
                    ));
                    for trait_ref in &conformance.traits {
                        anchors.push((trait_ref.span, table.resolve(trait_ref.name).to_owned()));
                    }
                }
                HeaderKind::TraitIncompatibility { incompatibility } => {
                    header_counts[2] += 1;
                    generated_header_names.push(path_fork.render_portable(
                        header.declaration_path,
                        table,
                        &mut path_scratch,
                    ));
                    anchors.push((
                        incompatibility.subject.span,
                        table.resolve(incompatibility.subject.name).to_owned(),
                    ));
                    for trait_ref in &incompatibility.incompatible_traits {
                        anchors.push((trait_ref.span, table.resolve(trait_ref.name).to_owned()));
                    }
                }
                _ => {}
            }
        }
        (anchors, generated_header_names, header_counts)
    };

    let (original_anchors, original_header_names, original_header_counts) =
        snapshot(&prepared, &strings, &path_fork);
    assert_eq!(original_header_counts, [1, 1, 1, 1]);
    assert_eq!(
        original_anchors.len(),
        10,
        "all trait shell anchors are captured"
    );
    assert_eq!(
        spans.len(),
        7,
        "only the seven long authored identifier tokens should need extended rows"
    );

    let mut merged = StringTable::new();
    let mut remapped_path_fork = PathInternerFork::empty();
    merged.intern("unrelated");
    let remap = merged.merge_from(&strings);
    let path_remap = remapped_path_fork
        .merge_delta_from(&path_fork, &remap)
        .expect("source paths should remap");
    prepared
        .remap_string_ids(&remap)
        .expect("trait shell strings should remap");
    prepared
        .remap_path_ids(&path_remap)
        .expect("trait shell paths should remap");

    drop(sources);
    let earlier_source = PathBuf::from("a-earlier.moth");
    let final_sources =
        SourceDatabase::build([&earlier_source, &canonical], &canonical, None, &mut merged)
            .expect("final source membership should register");
    let final_id = final_sources
        .get_by_canonical_path(&canonical)
        .expect("final source")
        .id;
    assert_ne!(
        final_id, source_id,
        "canonical membership changes the provisional identity"
    );
    let final_path = remapped_path_fork
        .try_intern_portable_path("trait-spans.moth", &mut merged)
        .expect("test path fits");
    prepared
        .rebind_source_identity(final_id, final_path, &mut remapped_path_fork)
        .expect("retained source should rebind");

    let mut database = SourceDatabaseBuilder::new(final_sources);
    database
        .sources_mut()
        .retain_text(final_id, source.clone())
        .expect("retain snapshot");
    database.retain_span_builder(final_id, spans);
    database
        .finish_source(final_id)
        .expect("test source should finalize");
    database
        .assemble_finalized_tables()
        .expect("the original span table should assemble");
    let database = database.finish().expect("install original span table");
    let resolve = |span: SourceSpan| {
        let range = span.byte_range(&database);
        &source[range.start() as usize..range.end() as usize]
    };

    let (rebound_anchors, rebound_header_names, rebound_header_counts) =
        snapshot(&prepared, &merged, &remapped_path_fork);
    assert_eq!(rebound_header_counts, original_header_counts);
    assert_eq!(rebound_header_names, original_header_names);
    assert_eq!(rebound_anchors.len(), original_anchors.len());
    for ((original_span, expected_text), (rebound_span, rebound_text)) in
        original_anchors.iter().zip(rebound_anchors.iter())
    {
        assert_eq!(
            rebound_span.local(),
            original_span.local(),
            "span encoding changed"
        );
        assert_eq!(
            rebound_span.source(),
            final_id,
            "rebound span should carry the final source identity"
        );
        assert_eq!(
            rebound_text, expected_text,
            "string remap changed anchor name"
        );
        let resolved_range = rebound_span.byte_range(&database);
        assert_eq!(
            resolve(*rebound_span),
            expected_text,
            "span no longer resolves to the exact authored UTF-8 bytes"
        );
        assert!(
            resolved_range.end() >= resolved_range.start(),
            "authored span must resolve to a valid source range"
        );
    }
}

/// Qualified-reference occurrences survive string remapping and source rebinding unchanged.
///
/// WHAT: retains `(path row, selected name, path span, selected span, shell)` through the exact
///       per-file merge and final identity rebinding the module pipeline performs.
/// WHY: occurrence keys are `(SourceId, PathSyntaxId)` and the selected name is a local string
///       id; a stale id or span source would silently detach the later provider binding.
#[test]
fn qualified_reference_occurrences_survive_string_remapping_and_source_rebinding() {
    let source = "value #= @math square\n";
    let canonical = PathBuf::from("qualified-spans.moth");
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let sources = SourceDatabase::build([&canonical], &canonical, None, &mut strings)
        .expect("source should register");
    let source_id = sources
        .get_by_canonical_path(&canonical)
        .expect("registered source")
        .id;
    let scope = path_fork
        .try_intern_filesystem_path(&canonical, &mut strings)
        .expect("source path");
    let mut spans = ExtendedSpanBuilder::new();
    let tokens = tokenize(
        source,
        scope,
        TokenizerEntryMode::SourceFile,
        &StyleDirectiveRegistry::built_ins(),
        &mut strings,
        &mut path_fork,
        source_id,
        &mut spans,
    )
    .expect("source should tokenize");
    let (owner, path_syntax) = super::canonical_handoff(tokens);
    let mut prepared = parse_file_headers_with_table(
        owner,
        scope,
        path_syntax,
        &HeaderParseOptions::default(),
        &mut strings,
        &mut path_fork,
        0,
        0,
        &mut spans,
    )
    .expect("qualified reference preparation should succeed");
    let original = prepared
        .qualified_references
        .first()
        .expect("the initializer should retain one occurrence")
        .clone();
    assert_eq!(strings.resolve(original.selected), "square");
    assert_eq!(original.dependency.span.source(), source_id);

    let mut merged = StringTable::new();
    let mut remapped_path_fork = PathInternerFork::empty();
    merged.intern("unrelated");
    let remap = merged.merge_from(&strings);
    let path_remap = remapped_path_fork
        .merge_delta_from(&path_fork, &remap)
        .expect("source paths should remap");
    prepared
        .remap_string_ids(&remap)
        .expect("prepared strings should remap");
    prepared
        .remap_path_ids(&path_remap)
        .expect("prepared paths should remap");
    drop(sources);

    let earlier_source = PathBuf::from("a-earlier.moth");
    let final_sources =
        SourceDatabase::build([&earlier_source, &canonical], &canonical, None, &mut merged)
            .expect("final source membership should register");
    let final_id = final_sources
        .get_by_canonical_path(&canonical)
        .expect("final source")
        .id;
    assert_ne!(
        final_id, source_id,
        "canonical membership changes the provisional identity"
    );
    let final_path = remapped_path_fork
        .try_intern_portable_path("qualified-spans.moth", &mut merged)
        .expect("test path fits");
    prepared
        .rebind_source_identity(final_id, final_path, &mut remapped_path_fork)
        .expect("retained source should rebind");

    let rebound = prepared
        .qualified_references
        .first()
        .expect("the occurrence must survive remapping")
        .clone();
    assert_eq!(rebound.selected, remap.get(original.selected));
    assert_eq!(merged.resolve(rebound.selected), "square");
    assert_eq!(
        rebound.dependency.path_syntax,
        original.dependency.path_syntax
    );
    assert_eq!(
        rebound.dependency.span.local(),
        original.dependency.span.local()
    );
    assert_eq!(
        rebound.selected_span.local(),
        original.selected_span.local()
    );
    assert_eq!(rebound.dependency.span.source(), final_id);
    assert_eq!(rebound.selected_span.source(), final_id);
    assert_eq!(rebound.dependency.dependency_shell_id.source, final_id);
    assert_eq!(
        rebound.dependency.dependency_shell_id.ordinal,
        original.dependency.dependency_shell_id.ordinal
    );
}

/// The Stage 2 function owner accepts every legal closer placement on the same source shapes.
///
/// WHAT: inline and standalone closers, comments, blank/comment-only lines before a closer,
///       CRLF, bare CR, multibyte body text, a split signature and both empty-body forms.
/// WHY: a layout predicate that keyed on one line-ending spelling or on the closer's own line
///      alone would reject legal source before AST or the tokenizer could report its own facts.
#[test]
fn function_scope_layout_accepts_inline_and_standalone_closers() {
    let cases = [
        ("inline body", "ready_check || -> Bool: return true;\n"),
        ("standalone closer", "run ||:\n    io.line(\"ready\")\n;\n"),
        (
            "closer followed by a comment",
            "run ||:\n    io.line(\"ready\")\n; -- finished\n",
        ),
        (
            "blank and comment-only lines before the closer",
            "run ||:\n    io.line(\"ready\")\n\n    -- finished\n;\n",
        ),
        ("CRLF source", "run ||:\r\n    io.line(\"ready\")\r\n;\r\n"),
        ("bare CR source", "run ||:\r    io.line(\"ready\")\r;\r"),
        (
            "multibyte body text",
            "run ||:\n    io.line(\"héllo 🦋\")\n;\n",
        ),
        (
            "split multiline signature",
            "sum |left I32,\n     right I32| -> I32:\n    return left\n;\n",
        ),
        ("empty inline body", "run ||:;\n"),
        ("empty multiline body", "run ||:\n;\n"),
        (
            "else branch closer shares its line",
            "run ||:\n    if ready:\n        work()\n    else other();\n;\n",
        ),
    ];

    for (case, source) in cases {
        let outcome =
            parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");
        expect_header_success(outcome, case);
    }
}

#[test]
fn multiline_function_closer_after_code_reports_the_closer_and_signature_colon() {
    let source = "run ||:\n    io.line(\"ready\");\n";
    let outcome =
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");

    assert_scope_layout_diagnostic(
        outcome,
        "multiline function closer after code",
        "invalid_scope_layout.multiline_closer_after_code",
        source_span_for(source, ";", 0),
        &[(
            ExpectedScopeLayoutLabel::OpenerStartsHere,
            source_span_for(source, ":", 0),
        )],
    );
}

#[test]
fn function_closer_trailing_code_reports_the_first_trailing_token() {
    let source = "run ||:\n    io.line(\"ready\")\n; io.line(\"after\")\n";
    let outcome =
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");

    assert_scope_layout_diagnostic(
        outcome,
        "function closer trailing code",
        "invalid_scope_layout.content_after_closer",
        source_span_for(source, "io", 1),
        &[(
            ExpectedScopeLayoutLabel::EarlierCloser,
            source_span_for(source, ";", 0),
        )],
    );
}

#[test]
fn stacked_function_closers_report_the_second_closer() {
    let source = "run ||: io.line(\"x\");;\n";
    let outcome =
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");

    assert_scope_layout_diagnostic(
        outcome,
        "stacked inline function closers",
        "invalid_scope_layout.additional_closer_on_line",
        source_span_for(source, ";", 1),
        &[(
            ExpectedScopeLayoutLabel::EarlierCloser,
            source_span_for(source, ";", 0),
        )],
    );
}

#[test]
fn inner_and_function_closers_stacked_on_one_line_report_the_second_closer() {
    let source = "run ||:\n    if a: b();;\n";
    let outcome =
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");

    assert_scope_layout_diagnostic(
        outcome,
        "inner closer then function closer",
        "invalid_scope_layout.additional_closer_on_line",
        source_span_for(source, ";", 1),
        &[(
            ExpectedScopeLayoutLabel::EarlierCloser,
            source_span_for(source, ";", 0),
        )],
    );
}

#[test]
fn stacked_function_closers_on_a_multiline_inner_closer_report_the_inner_closer() {
    // `if ready:` opens a block on an earlier physical line, so its `;` is the first provable
    // error. The function closer stacked on it must not hide that fact.
    let source = "run ||:\n    if ready:\n        work();;\n";
    let outcome =
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");

    assert_scope_layout_diagnostic(
        outcome,
        "function closer stacked after a multiline inner closer",
        "invalid_scope_layout.multiline_closer_after_code",
        source_span_for(source, ";", 0),
        &[(
            ExpectedScopeLayoutLabel::OpenerStartsHere,
            source_span_for(source, ":", 1),
        )],
    );
}

#[test]
fn stacked_function_closers_on_a_stacked_inner_closer_report_the_earlier_inner_closer() {
    // The middle `;` closes `if a` and is itself stacked on the `;` that closed `if b`. The
    // first misplaced closer is the middle one, so it wins over the function closer.
    let source = "run ||:\n    if a: if b: c();;;\n";
    let outcome =
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");

    assert_scope_layout_diagnostic(
        outcome,
        "function closer stacked on an already-stacked inner closer",
        "invalid_scope_layout.additional_closer_on_line",
        source_span_for(source, ";", 1),
        &[(
            ExpectedScopeLayoutLabel::EarlierCloser,
            source_span_for(source, ";", 0),
        )],
    );
}

#[test]
fn legal_branch_continuation_keeps_the_stacked_check_on_the_function_closer() {
    // An `else` body's final `;` may share the branch's line under the accepted contract, so
    // that inner closer is legal and the function closer stacked on it is the reported one.
    let source = "run ||:\n    if ready:\n        work()\n    else other();;\n";
    let outcome =
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");

    assert_scope_layout_diagnostic(
        outcome,
        "else branch closer sharing its line",
        "invalid_scope_layout.additional_closer_on_line",
        source_span_for(source, ";", 1),
        &[(
            ExpectedScopeLayoutLabel::EarlierCloser,
            source_span_for(source, ";", 0),
        )],
    );
}

#[test]
fn legal_match_arm_continuation_keeps_the_stacked_check_on_the_function_closer() {
    // A match arm ending in `=>` may close on the arm's line under the accepted contract.
    let source = "run ||:\n    if value is:\n        Foo => work();;\n";
    let outcome =
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");

    assert_scope_layout_diagnostic(
        outcome,
        "match arm closer sharing its line",
        "invalid_scope_layout.additional_closer_on_line",
        source_span_for(source, ";", 1),
        &[(
            ExpectedScopeLayoutLabel::EarlierCloser,
            source_span_for(source, ";", 0),
        )],
    );
}

#[test]
fn else_continuation_on_a_later_line_does_not_excuse_a_multiline_inner_closer() {
    // The `else` sits on its own line, so the inner closer does not share the branch's physical
    // line. `if true:` opened on an earlier line, making the first `;` the first error.
    let source = "run ||:\n    if true:\n        return\n    else\n        return;;\n";
    let outcome =
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");

    assert_scope_layout_diagnostic(
        outcome,
        "else continuation on a later line",
        "invalid_scope_layout.multiline_closer_after_code",
        source_span_for(source, ";", 0),
        &[(
            ExpectedScopeLayoutLabel::OpenerStartsHere,
            source_span_for(source, ":", 1),
        )],
    );
}

#[test]
fn else_continuation_on_the_same_line_keeps_the_inner_closer_legal() {
    // `else return;;` shares one physical line, so the first `;` legally closes the branch and
    // the stacked function closer is the reported error.
    let source = "run ||:\n    if true:\n        return\n    else return;;\n";
    let outcome =
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");

    assert_scope_layout_diagnostic(
        outcome,
        "else continuation on the closer line",
        "invalid_scope_layout.additional_closer_on_line",
        source_span_for(source, ";", 1),
        &[(
            ExpectedScopeLayoutLabel::EarlierCloser,
            source_span_for(source, ";", 0),
        )],
    );
}

#[test]
fn the_earliest_invalid_inner_closer_wins_when_the_function_closer_is_invalid() {
    // The inner `;` follows code on a later line than `if true:` and the function closer also
    // stands after code. Stage 2 reports the earlier inner error, not the function closer.
    let source = "run ||:\n    if true:\n        return;\n    return;\n";
    let outcome =
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");

    assert_scope_layout_diagnostic(
        outcome,
        "earliest invalid inner closer",
        "invalid_scope_layout.multiline_closer_after_code",
        source_span_for(source, ";", 0),
        &[(
            ExpectedScopeLayoutLabel::OpenerStartsHere,
            source_span_for(source, ":", 1),
        )],
    );
}

#[test]
fn the_earliest_invalid_inner_closer_of_three_stacked_closers_wins() {
    // The first `;` closes the multiline `if b` after code and is the first error. The other two
    // closers are also invalid, but Stage 2 must not report them ahead of it.
    let source = "run ||:\n    if a:\n        if b:\n            x();;;\n";
    let outcome =
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");

    assert_scope_layout_diagnostic(
        outcome,
        "three stacked closers",
        "invalid_scope_layout.multiline_closer_after_code",
        source_span_for(source, ";", 0),
        &[(
            ExpectedScopeLayoutLabel::OpenerStartsHere,
            source_span_for(source, ":", 2),
        )],
    );
}

#[test]
fn code_after_an_inner_closer_wins_when_the_function_closer_is_invalid() {
    // The inline `if` closes legally, but code follows its `;` on the same line. That trailing
    // code is the first structural error, ahead of the misplaced function closer.
    let source = "run ||:\n    if true: return; return\n    return;\n";
    let outcome =
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");

    assert_scope_layout_diagnostic(
        outcome,
        "code after inner closer",
        "invalid_scope_layout.content_after_closer",
        source_span_for(source, "return", 1),
        &[(
            ExpectedScopeLayoutLabel::EarlierCloser,
            source_span_for(source, ";", 0),
        )],
    );
}

#[test]
fn an_inline_value_if_inside_a_multiline_block_keeps_standalone_closers_valid() {
    // The value-if's `else` is on the assignment line, but both real source scopes close
    // standalone. The branch exception must not turn valid layout into an error.
    let source = "run ||:\n    if ready:\n        x = if a then 1 else 2\n    ;\n;\n";
    let outcome =
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");

    expect_header_success(outcome, "inline value-if inside a multiline block");
}

#[test]
fn a_grouped_semicolon_is_left_to_the_expression_parser() {
    // A `;` inside call parentheses already names a precise expression error. Stage 2 must not
    // preempt it with the trailing-code rule for the function closer on the next line.
    let source = "run ||:\n    call(;)\n; next\n";
    let outcome =
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");

    expect_header_success(outcome, "grouped semicolon before the function closer");
}

#[test]
fn inline_function_with_a_multiline_literal_closer_stands_after_code() {
    // The literal's own physical line break puts the closer on a later physical line than the
    // signature colon, so the closer cannot claim the inline form.
    let source = "run ||: io.line(\"a\nb\");\n";
    let outcome =
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");

    assert_scope_layout_diagnostic(
        outcome,
        "inline function with a multiline literal",
        "invalid_scope_layout.multiline_closer_after_code",
        source_span_for(source, ";", 0),
        &[(
            ExpectedScopeLayoutLabel::OpenerStartsHere,
            source_span_for(source, ":", 0),
        )],
    );
}

#[test]
fn a_discarded_template_body_line_break_is_not_inline_code() {
    // The discarded `$note` body spans a physical line, but only the lexer sees that text. Its
    // closing `]` owns the break, so the function closer stands after code, not inline.
    let source = "run ||: value = 1 [$note: first\nsecond];\n";
    let outcome =
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");

    assert_scope_layout_diagnostic(
        outcome,
        "function closer after a discarded multiline template body",
        "invalid_scope_layout.multiline_closer_after_code",
        source_span_for(source, ";", 0),
        &[(
            ExpectedScopeLayoutLabel::OpenerStartsHere,
            source_span_for(source, ":", 0),
        )],
    );
}

/// Const and receiver-method functions share the ordinary signature-to-body capture route.
///
/// WHAT: the inline form is legal and the multiline-closer-after-code form reports the same
///       MOTH-SYNTAX-0041 reason and labels as an ordinary function.
/// WHY: all function-like routes funnel through one capture, so a route-specific body handoff
///      regression would otherwise only surface for the `#|` marker or a receiver signature.
#[test]
fn const_and_receiver_function_routes_use_the_same_closer_contract() {
    let valid_cases = [
        (
            "const function inline body",
            "square #|value I32| -> I32: return value;\n",
        ),
        (
            "receiver method inline body",
            "add |this ~Accumulator, amount I32|: return amount;\n",
        ),
    ];
    for (case, source) in valid_cases {
        let outcome =
            parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");
        expect_header_success(outcome, case);
    }

    let invalid_cases = [
        (
            "const function multiline closer after code",
            "square #|value I32| -> I32:\n    return value;\n",
        ),
        (
            "receiver method multiline closer after code",
            "add |this ~Accumulator, amount I32|:\n    return amount;\n",
        ),
    ];
    for (case, source) in invalid_cases {
        let outcome =
            parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");
        assert_scope_layout_diagnostic(
            outcome,
            case,
            "invalid_scope_layout.multiline_closer_after_code",
            source_span_for(source, ";", 0),
            &[(
                ExpectedScopeLayoutLabel::OpenerStartsHere,
                source_span_for(source, ":", 0),
            )],
        );
    }
}

#[test]
fn second_function_on_a_closer_line_reports_the_first_trailing_token() {
    let source = "first ||: return 1; second ||: return 2;\n";
    let outcome =
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");

    assert_scope_layout_diagnostic(
        outcome,
        "second function after a closer",
        "invalid_scope_layout.content_after_closer",
        source_span_for(source, "second", 0),
        &[(
            ExpectedScopeLayoutLabel::EarlierCloser,
            source_span_for(source, ";", 0),
        )],
    );
}
