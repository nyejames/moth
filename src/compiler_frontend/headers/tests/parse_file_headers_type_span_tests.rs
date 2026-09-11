use super::*;

#[test]
fn parsed_type_and_capacity_spans_retain_exact_ranges_after_remapping_and_rebinding() {
    let long_named = format!("Named{}", "N".repeat(1100));
    let long_qualified = format!("Qualified{}", "Q".repeat(1100));
    let long_generic = format!("Generic{}", "G".repeat(1100));
    let long_capacity = format!("capacity_{}", "c".repeat(1100));
    let source = format!(
        "-- é🦋\n\
{long_capacity} #Int = 7\n\
NamedAlias as {long_named}\n\
QualifiedAlias as {long_qualified}.Child\n\
AppliedAlias as {long_generic} of Child, String\n\
CollectionAlias as {{Child}}\n\
FixedLiteralAlias as {{7 Child}}\n\
FixedNameAlias as {{{long_capacity} Child}}\n\
MapAlias as {{Key = Value}}\n\
NestedAlias as {{Key = {{Child}}}}\n\
OptionalAlias as Child?\n\
BoolAlias as Bool\n\
IntAlias as Int\n\
FloatAlias as Float\n\
StringAlias as String\n\
CharAlias as Char\n\
INFERRED #= 1\n"
    );
    let canonical = PathBuf::from("parsed-type-spans.moth");
    let mut strings = StringTable::new();
    let sources = SourceDatabase::build([&canonical], &canonical, None, &mut strings)
        .expect("registered source");
    let source_id = sources
        .get_by_canonical_path(&canonical)
        .expect("source identity")
        .id;
    let scope =
        InternedPath::try_from_filesystem_path(&canonical, &mut strings).expect("source path");
    let mut spans = ExtendedSpanBuilder::new();
    let mut tokens = tokenize(
        &source,
        &scope,
        TokenizerEntryMode::SourceFile,
        &StyleDirectiveRegistry::built_ins(),
        &mut strings,
        source_id,
        &mut spans,
    )
    .expect("source should tokenize");
    let tokenizer_extended_span_count = spans.len();
    assert_eq!(
        tokenizer_extended_span_count, 5,
        "only the long named, qualified, generic and capacity identifier tokens should overflow"
    );
    let mut prepared = parse_file_headers_with_table(
        &mut tokens,
        &canonical,
        &HeaderParseOptions::default(),
        &mut strings,
        0,
        0,
        &mut spans,
    )
    .expect("parsed type headers should prepare");
    assert!(
        prepared.warnings.is_empty(),
        "warning-free preparation must not add diagnostic span rows"
    );
    assert_eq!(
        spans.len(),
        tokenizer_extended_span_count,
        "parsed type anchors must reuse tokenizer rows without adding extended spans"
    );

    let (type_alias_count, inferred_count, original_anchors) =
        snapshot_prepared_type_anchors(&prepared, &strings);
    assert_eq!(
        type_alias_count, 14,
        "the source should produce one header per type shape"
    );
    assert_eq!(
        inferred_count, 1,
        "inferred declarations remain location-free"
    );
    assert_eq!(
        original_anchors.len(),
        29,
        "the snapshot should cover every authored constructor and child type anchor"
    );
    let expected_texts = vec![
        "Int".to_owned(),
        long_named.clone(),
        long_qualified.clone(),
        "of".to_owned(),
        long_generic.clone(),
        "Child".to_owned(),
        "String".to_owned(),
        "{".to_owned(),
        "Child".to_owned(),
        "{".to_owned(),
        "Child".to_owned(),
        "7".to_owned(),
        "{".to_owned(),
        "Child".to_owned(),
        long_capacity.clone(),
        "{".to_owned(),
        "Key".to_owned(),
        "Value".to_owned(),
        "{".to_owned(),
        "Key".to_owned(),
        "{".to_owned(),
        "Child".to_owned(),
        "?".to_owned(),
        "Child".to_owned(),
        "Bool".to_owned(),
        "Int".to_owned(),
        "Float".to_owned(),
        "String".to_owned(),
        "Char".to_owned(),
    ];
    assert_eq!(
        original_anchors
            .iter()
            .map(|anchor| anchor.expected_text.clone())
            .collect::<Vec<_>>(),
        expected_texts,
        "each parsed type variant must retain its contract-defined anchor"
    );

    let mut merged = StringTable::new();
    merged.intern("unrelated");
    prepared
        .remap_string_ids(&merged.merge_from(&strings))
        .expect("parsed type strings should remap");

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
    let final_path = InternedPath::from_single_str("parsed-type-spans.moth", &mut merged);
    prepared
        .rebind_source_identity(final_id, final_path, canonical)
        .expect("retained source should rebind");

    let mut database = SourceDatabaseBuilder::new(final_sources);
    database
        .sources_mut()
        .retain_text(final_id, source.clone())
        .expect("retain snapshot");
    database.retain_span_builder(final_id, spans);
    let database = database.finish().expect("install original span table");

    let (_, rebound_inferred_count, rebound_anchors) =
        snapshot_prepared_type_anchors(&prepared, &merged);
    assert_eq!(rebound_inferred_count, inferred_count);
    assert_eq!(rebound_anchors.len(), original_anchors.len());
    for (original, rebound) in original_anchors.iter().zip(rebound_anchors.iter()) {
        assert_eq!(
            rebound.span.local(),
            original.span.local(),
            "span encoding changed"
        );
        assert_eq!(
            rebound.span.source(),
            final_id,
            "rebound span should carry the final source identity"
        );
        assert_eq!(rebound.expected_text, original.expected_text);
        let resolved_range = rebound.span.byte_range(&database);
        assert_eq!(
            source.get(resolved_range.start() as usize..resolved_range.end() as usize),
            Some(original.expected_text.as_str()),
            "span must resolve to the exact authored UTF-8 bytes"
        );
    }
}

#[test]
fn generic_parameter_and_bound_spans_retain_exact_ranges_after_remapping_and_rebinding() {
    let parameter_name = format!("Element{}", "E".repeat(1100));
    let display_trait_name = format!("DISPLAY_TEXT_{}", "D".repeat(1100));
    let named_trait_name = format!("NAMED_{}", "N".repeat(1100));
    let source = format!(
        "-- é🦋\n\
{display_trait_name} must:\n\
;\n\
{named_trait_name} must:\n\
;\n\
render_value type {parameter_name} is {display_trait_name} and {named_trait_name} |value {parameter_name}| -> String:\n\
    return \"ok\"\n\
;\n\
Envelope type {parameter_name} is {display_trait_name} and {named_trait_name} = |\n\
    value {parameter_name},\n\
|\n\
State type {parameter_name} is {display_trait_name} and {named_trait_name} ::\n\
    Ready | value {parameter_name} |,\n\
;\n"
    );
    let canonical = PathBuf::from("generic-anchor-spans.moth");
    let mut strings = StringTable::new();
    let sources = SourceDatabase::build([&canonical], &canonical, None, &mut strings)
        .expect("registered source");
    let source_id = sources
        .get_by_canonical_path(&canonical)
        .expect("source identity")
        .id;
    let scope =
        InternedPath::try_from_filesystem_path(&canonical, &mut strings).expect("source path");
    let mut spans = ExtendedSpanBuilder::new();
    let mut tokens = tokenize(
        &source,
        &scope,
        TokenizerEntryMode::SourceFile,
        &StyleDirectiveRegistry::built_ins(),
        &mut strings,
        source_id,
        &mut spans,
    )
    .expect("source should tokenize");
    let tokenizer_extended_span_count = spans.len();
    assert_eq!(
        tokenizer_extended_span_count, 14,
        "trait declarations, generic declarations, bounds and type uses should own the long rows"
    );

    let mut prepared = parse_file_headers_with_table(
        &mut tokens,
        &canonical,
        &HeaderParseOptions::default(),
        &mut strings,
        0,
        0,
        &mut spans,
    )
    .expect("generic headers should prepare");
    assert!(
        prepared.warnings.is_empty(),
        "valid generic names should not add warning spans"
    );
    assert_eq!(
        spans.len(),
        tokenizer_extended_span_count,
        "generic anchors must reuse the tokenizer's original extended rows"
    );

    let snapshot_anchors = |prepared: &FileFrontendPrepareOutput, table: &StringTable| {
        let mut owner_counts = [0usize; 3];
        let mut anchors = Vec::new();
        for header in &prepared.headers {
            let (owner_index, generic_parameters) = match &header.kind {
                HeaderKind::Function {
                    generic_parameters, ..
                } => (0, generic_parameters),
                HeaderKind::Struct {
                    generic_parameters, ..
                } => (1, generic_parameters),
                HeaderKind::Choice {
                    generic_parameters, ..
                } => (2, generic_parameters),
                _ => continue,
            };
            if generic_parameters.parameters.is_empty() {
                continue;
            }
            owner_counts[owner_index] += 1;
            for parameter in &generic_parameters.parameters {
                anchors.push((
                    parameter
                        .span
                        .expect("authored generic parameter should retain its span"),
                    table.resolve(parameter.name).to_owned(),
                ));
                for trait_bound in &parameter.trait_bounds {
                    anchors.push((
                        trait_bound
                            .span
                            .expect("authored generic trait bound should retain its span"),
                        table.resolve(trait_bound.trait_name).to_owned(),
                    ));
                }
            }
        }
        (owner_counts, anchors)
    };

    let (original_owner_counts, original_anchors) = snapshot_anchors(&prepared, &strings);
    assert_eq!(
        original_owner_counts,
        [1, 1, 1],
        "the regression must cover function, struct and choice generic owners"
    );
    assert_eq!(
        original_anchors.len(),
        9,
        "each generic owner must retain one parameter and two bound anchors"
    );
    assert_eq!(
        original_anchors
            .iter()
            .map(|anchor| anchor.1.clone())
            .collect::<Vec<_>>(),
        vec![
            parameter_name.clone(),
            display_trait_name.clone(),
            named_trait_name.clone(),
            parameter_name.clone(),
            display_trait_name.clone(),
            named_trait_name.clone(),
            parameter_name.clone(),
            display_trait_name.clone(),
            named_trait_name.clone(),
        ]
    );
    let original_resolver = spans.resolver();
    for (span, expected_text) in &original_anchors {
        let resolved_range = span.local().resolve_with(original_resolver);
        assert_eq!(
            source.get(resolved_range.start() as usize..resolved_range.end() as usize),
            Some(expected_text.as_str()),
            "the original span must cover the exact authored UTF-8 bytes"
        );
    }

    let mut merged = StringTable::new();
    merged.intern("unrelated");
    prepared
        .remap_string_ids(&merged.merge_from(&strings))
        .expect("generic strings should remap");

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
    let final_path = InternedPath::from_single_str("generic-anchor-spans.moth", &mut merged);
    prepared
        .rebind_source_identity(final_id, final_path, canonical)
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
        source
            .get(range.start() as usize..range.end() as usize)
            .expect("retained source range")
    };

    let (rebound_owner_counts, rebound_anchors) = snapshot_anchors(&prepared, &merged);
    assert_eq!(rebound_owner_counts, original_owner_counts);
    assert_eq!(rebound_anchors.len(), original_anchors.len());
    for ((original_span, original_text), (rebound_span, rebound_text)) in
        original_anchors.iter().zip(rebound_anchors.iter())
    {
        assert_eq!(
            rebound_span.local(),
            original_span.local(),
            "span encoding changed"
        );
        assert_eq!(rebound_span.source(), final_id);
        assert_eq!(
            rebound_text, original_text,
            "string remap changed anchor name"
        );
        let resolved_range = rebound_span.byte_range(&database);
        assert_eq!(resolve(*rebound_span), original_text);
        assert!(
            resolved_range.end() >= resolved_range.start(),
            "authored span must resolve to a valid source range"
        );
    }
}
