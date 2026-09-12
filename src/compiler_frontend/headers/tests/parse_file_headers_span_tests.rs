use super::*;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

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
    let scope =
        path_fork.try_intern_filesystem_path(&canonical, &mut strings).expect("source path");
    let mut spans = ExtendedSpanBuilder::new();
    let mut tokens = tokenize(&source, scope, TokenizerEntryMode::SourceFile, &directives, &mut strings, &mut path_fork, source_id, &mut spans)
    .expect("source should tokenize");
    let mut prepared = parse_file_headers_with_table(
        &mut tokens,
        &canonical,
        &options,
        &mut strings,
        &mut PathInternerFork::empty(),
        0,
        0,
        &mut spans,
    )
    .expect("dependency preparation should succeed");
    let original_span = prepared.file_dependency_clauses[0].dependency.span;
    assert_eq!(
        spans.len(),
        1,
        "retained dependency records reuse the lexical overflow row"
    );

    let mut merged = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    merged.intern("unrelated");
    let remap = merged.merge_from(&strings);
    prepared
        .remap_string_ids(&remap)
        .expect("prepared strings should remap");
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
    let final_path = path_fork.try_intern_portable_path("dependency-spans.moth", &mut merged).expect("test path fits");
    prepared
        .rebind_source_identity(final_id, final_path, canonical, &mut path_fork)
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
fn declaration_member_return_and_variant_spans_retain_original_ranges() {
    let member_name = format!("value_{}", "x".repeat(1100));
    let return_name = format!("Output{}", "X".repeat(1100));
    let variant_name = format!("Ready{}", "X".repeat(1100));
    let source = format!(
        "-- é🦋\nprocess |{member_name} Int| -> {return_name}:\n;\nState ::\n{variant_name} | field Int |,\n;\nRecord = | member Int |\n"
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
    let scope =
        path_fork.try_intern_filesystem_path(&canonical, &mut strings).expect("source path");
    let mut spans = ExtendedSpanBuilder::new();
    let mut tokens = tokenize(&source, scope, TokenizerEntryMode::SourceFile, &StyleDirectiveRegistry::built_ins(), &mut strings, &mut path_fork, source_id, &mut spans)
    .expect("source should tokenize");
    let mut prepared = parse_file_headers_with_table(
        &mut tokens,
        &canonical,
        &HeaderParseOptions::default(),
        &mut strings,
        &mut PathInternerFork::empty(),
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
                signature.returns[0]
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

    let mut merged = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    merged.intern("unrelated");
    prepared
        .remap_string_ids(&merged.merge_from(&strings))
        .expect("shell strings should remap");

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
    let final_path = path_fork.try_intern_portable_path("member-spans.moth", &mut merged).expect("test path fits");
    prepared
        .rebind_source_identity(final_id, final_path, canonical, &mut path_fork)
        .expect("retained source should rebind");
    assert_eq!(prepared.file_id, final_id);

    let mut database = SourceDatabaseBuilder::new(final_sources);
    database
        .sources_mut()
        .retain_text(final_id, source.clone())
        .expect("retain snapshot");
    database.retain_span_builder(final_id, spans);
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
    let rebound_return_span = signature.returns[0]
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
        path_fork
            .component(signature.parameters[0].id)
            .map(|name| merged.resolve(name)),
        Some(member_name.as_str())
    );
    assert_eq!(
        resolve(
            signature.returns[0]
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
    let scope =
        path_fork.try_intern_filesystem_path(&canonical, &mut strings).expect("source path");
    let mut spans = ExtendedSpanBuilder::new();
    let mut tokens = tokenize(&source, scope, TokenizerEntryMode::SourceFile, &StyleDirectiveRegistry::built_ins(), &mut strings, &mut path_fork, source_id, &mut spans)
    .expect("source should tokenize");
    let mut prepared = parse_file_headers_with_table(
        &mut tokens,
        &canonical,
        &HeaderParseOptions::default(),
        &mut strings,
        &mut PathInternerFork::empty(),
        0,
        0,
        &mut spans,
    )
    .expect("trait shells should prepare");

    let snapshot = |prepared: &FileFrontendPrepareOutput, table: &StringTable| {
        let mut anchors = Vec::new();
        let mut generated_header_names = Vec::new();
        let mut header_counts = [0usize; 4];
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
                    generated_header_names.push(format!("{:?}", header.tokens.src_path));
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
                    generated_header_names.push(format!("{:?}", header.tokens.src_path));
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
        snapshot(&prepared, &strings);
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
    let mut path_fork = PathInternerFork::empty();
    merged.intern("unrelated");
    prepared
        .remap_string_ids(&merged.merge_from(&strings))
        .expect("trait shell strings should remap");

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
    let final_path = path_fork.try_intern_portable_path("trait-spans.moth", &mut merged).expect("test path fits");
    prepared
        .rebind_source_identity(final_id, final_path, canonical, &mut path_fork)
        .expect("retained source should rebind");

    let mut database = SourceDatabaseBuilder::new(final_sources);
    database
        .sources_mut()
        .retain_text(final_id, source.clone())
        .expect("retain snapshot");
    database.retain_span_builder(final_id, spans);
    let database = database.finish().expect("install original span table");
    let resolve = |span: SourceSpan| {
        let range = span.byte_range(&database);
        &source[range.start() as usize..range.end() as usize]
    };

    let (rebound_anchors, rebound_header_names, rebound_header_counts) =
        snapshot(&prepared, &merged);
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
