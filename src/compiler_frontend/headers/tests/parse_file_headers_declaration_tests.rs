use super::*;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::tokenizer::tokens::{TokenCursor, TokenTag};

fn cursor_has_tag(mut cursor: TokenCursor<'_>, expected: TokenTag) -> bool {
    while let Some(token) = cursor.advance() {
        if token.tag() == expected {
            return true;
        }
        if token.is_eof() {
            break;
        }
    }
    false
}

fn cursor_first_tag(mut cursor: TokenCursor<'_>) -> Option<TokenTag> {
    cursor.advance().map(|token| token.tag())
}

fn cursor_last_tag(mut cursor: TokenCursor<'_>) -> Option<TokenTag> {
    let mut last = None;
    while let Some(token) = cursor.advance() {
        last = Some(token.tag());
        if token.is_eof() {
            break;
        }
    }
    last
}

fn cursor_has_string(
    mut cursor: TokenCursor<'_>,
    expected_tag: TokenTag,
    string_table: &StringTable,
    expected: &str,
) -> bool {
    while let Some(token) = cursor.advance() {
        if token.tag() == expected_tag
            && token
                .string_id()
                .is_some_and(|id| string_table.resolve(id) == expected)
        {
            return true;
        }
        if token.is_eof() {
            break;
        }
    }
    false
}

fn cursor_find_path(mut cursor: TokenCursor<'_>) -> Option<PathSyntaxId> {
    while let Some(token) = cursor.advance() {
        if token.tag() == TokenTag::PATH {
            return token.path_syntax_id();
        }
        if token.is_eof() {
            break;
        }
    }
    None
}

#[test]
fn exported_untyped_constant_has_no_header_provided_dependencies() {
    let headers = parse_single_file_headers("theme #= navbar\n");
    let constant_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Constant { .. }))
        .expect("expected constant header");

    assert!(
        constant_header.local_ordering_hints.is_empty(),
        "header-provided constant dependencies come from declared type syntax only"
    );
}

#[test]
fn exported_typed_constant_headers_are_parsed_and_follow_on_constant_stays_header() {
    let headers = parse_single_file_headers("page #String = [: world]\n\ntest #= [page: Hello ]\n");

    assert!(
        matches!(
            headers.headers.first().map(|header| &header.kind),
            Some(HeaderKind::Constant { .. })
        ),
        "first header should be parsed as a constant"
    );
    assert!(
        matches!(
            headers.headers.get(1).map(|header| &header.kind),
            Some(HeaderKind::Constant { .. })
        ),
        "follow-on 'test #= ...' should remain a constant header"
    );
}

#[test]
fn non_generic_headers_keep_generic_parameter_lists_empty() {
    let headers = parse_single_file_headers(
        "identity |value Int| -> Int:\n\
             return value\n\
         ;\n\
         Box = |\n\
             value Int,\n\
         |\n\
         Status :: Ready,\n\
         ;\n\
         Alias as Int\n",
    );

    for header in &headers.headers {
        match &header.kind {
            HeaderKind::Function {
                generic_parameters, ..
            }
            | HeaderKind::Struct {
                generic_parameters, ..
            }
            | HeaderKind::Choice {
                generic_parameters, ..
            } => {
                assert!(
                    generic_parameters.parameters.is_empty(),
                    "non-generic declarations should keep generic parameter lists empty"
                );
            }
            _ => {}
        }
    }
}

#[test]
fn generic_declaration_headers_parse_parameter_lists() {
    let (headers, string_table) = parse_single_file_headers_with_table(
        "identity type T |value T| -> T:\n\
             return value\n\
         ;\n\
         Box type Item = |\n\
             value Item,\n\
         |\n\
         ResultShape type OkType, ErrType ::\n\
             Ok | value OkType |,\n\
             Err | error ErrType |,\n\
         ;\n",
    );

    let mut generic_parameter_counts = Vec::new();
    for header in &headers.headers {
        match &header.kind {
            HeaderKind::Function {
                generic_parameters, ..
            }
            | HeaderKind::Struct {
                generic_parameters, ..
            }
            | HeaderKind::Choice {
                generic_parameters, ..
            } => generic_parameter_counts.push(generic_parameters.parameters.len()),
            _ => {}
        }
    }

    assert_eq!(generic_parameter_counts, vec![1, 1, 2]);
    assert_eq!(
        headers.module_symbols.generic_declarations_by_path.len(),
        3,
        "only declarations with generic parameters should be registered as generic declarations"
    );

    let generic_declarations = &headers.module_symbols.generic_declarations_by_path;
    assert_eq!(
        generic_declarations
            .values()
            .filter(|kind| matches!(kind, GenericDeclarationKind::Function))
            .count(),
        1,
        "the generic function must be registered by declaration kind"
    );
    assert_eq!(
        generic_declarations
            .values()
            .filter(|kind| matches!(kind, GenericDeclarationKind::Struct))
            .count(),
        1,
        "the generic struct must be registered by declaration kind"
    );
    assert_eq!(
        generic_declarations
            .values()
            .filter(|kind| matches!(kind, GenericDeclarationKind::Choice))
            .count(),
        1,
        "the generic choice must be registered by declaration kind"
    );

    let parsed_generic_names = headers
        .headers
        .iter()
        .filter_map(|header| match &header.kind {
            HeaderKind::Function {
                generic_parameters, ..
            }
            | HeaderKind::Struct {
                generic_parameters, ..
            }
            | HeaderKind::Choice {
                generic_parameters, ..
            } => Some(generic_parameters),
            _ => None,
        })
        .flat_map(|generic_parameters| {
            generic_parameters
                .parameters
                .iter()
                .map(|parameter| string_table.resolve(parameter.name).to_owned())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        parsed_generic_names,
        vec!["T", "Item", "OkType", "ErrType"],
        "parameter names remain owned by the parsed HeaderKind lists"
    );
}

#[test]
fn malformed_generic_bound_retains_exact_multibyte_extended_span() {
    let long_bound = format!("d{}", "é".repeat(600));
    let source = format!("-- é🦋\nidentity type T is {long_bound} |value T| -> T:\n;\n");
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let options = HeaderParseOptions::default();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let file_path = Path::new("src/@page.moth");
    let source_id = SourceId::from_index(12);
    let context = HeaderTestPrepareContext {
        source_id,
        entry_file_path: file_path,
        options: &options,
        style_directives: &style_directives,
    };

    let failure = prepare_test_source_file(
        &source,
        file_path,
        &context,
        &mut string_table,
        0,
        0,
        &mut span_builder,
        &mut path_fork,
    )
    .err()
    .expect("a lowercase generic bound should be rejected");
    let FileFrontendPrepareFailure::Diagnosed(error) = failure else {
        panic!("expected the malformed generic bound to remain a source diagnostic");
    };

    assert!(matches!(
        error.diagnostic.payload,
        DiagnosticPayload::InvalidDeclaration {
            reason: InvalidDeclarationReason::InvalidTraitName,
            ..
        }
    ));
    let span = error
        .diagnostic
        .primary_span
        .expect("generic-bound diagnostics should retain their token span");
    assert_eq!(span.source(), source_id);
    let range = span.resolve_with(span_builder.resolver_for(source_id));
    assert_eq!(
        &source[range.start() as usize..range.end() as usize],
        long_bound
    );
    assert!(
        range.end() - range.start() > 1023,
        "the multibyte bound should exercise the extended span table"
    );
}

#[test]
fn top_level_const_template_outside_entry_file_errors() {
    let result = parse_single_file_headers_with_entry(
        "#[html.head: [\"x\"]]\n",
        "src/lib.moth",
        "src/@page.moth",
    );

    assert!(
        result.is_err(),
        "const templates outside the entry file should error"
    );
}

#[test]
fn top_level_const_template_tokens_keep_close_and_eof_for_ast_parser() {
    let headers = parse_single_file_headers("#[3]\n");

    let const_template_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::ConstTemplate { .. }))
        .expect("expected top-level const template header");

    assert_eq!(
        cursor_first_tag(header_body_tokens(&headers, const_template_header)),
        Some(TokenTag::TEMPLATE_HEAD),
        "const template token stream should start with template opener"
    );
    assert!(
        cursor_has_tag(
            header_body_tokens(&headers, const_template_header),
            TokenTag::TEMPLATE_CLOSE,
        ),
        "const template token stream should preserve template close token"
    );
    assert_eq!(
        cursor_last_tag(header_body_tokens(&headers, const_template_header)),
        Some(TokenTag::EOF),
        "const template token stream should end with EOF sentinel"
    );
}

#[test]
fn header_const_fragment_and_source_contract_spans_keep_authored_ranges() {
    let long_fragment_name = "fragment_".to_owned() + &"x".repeat(1300);
    let source = format!("value #Config of Int = 1\n#[{long_fragment_name}]\n");
    let mut source_context = TestSourceContext::new("src/@page.moth");
    let mut path_fork = PathInternerFork::empty();
    let file_path = source_context.path().to_path_buf();
    let source_id = source_context.source_id();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let file_tokens = source_context
        .tokenize(
            &source,
            &mut path_fork,
            &style_directives,
            TokenizerEntryMode::SourceFile,
        )
        .expect("source should tokenize");
    let interned_path = file_tokens.logical_path;
    let (owner, path_syntax) = super::owner_from_lexed_source(file_tokens);
    let tokenizer_span_count = {
        let (string_table, span_builder) = source_context.preparation_parts();
        let count = span_builder.len();
        let output = prepare_file_from_tokens(
            owner,
            interned_path,
            path_syntax,
            &file_path,
            &HeaderParseOptions::default(),
            string_table,
            0,
            0,
            span_builder,
            &mut path_fork,
        )
        .expect("headers should prepare");
        let header_name_span = output
            .headers
            .iter()
            .find_map(|header| {
                matches!(header.kind, HeaderKind::Constant { .. }).then_some(header.name_span)
            })
            .expect("source config header");
        let fragment_span = output
            .top_level_const_fragments
            .first()
            .expect("const fragment metadata")
            .span;
        let mut outputs = [output];
        let prepared = prepare_header_syntax(
            &mut outputs,
            string_table,
            &mut |source, diagnostic| diagnostic.capture_preparation_span(source),
            &mut path_fork,
        )
        .expect("header syntax should aggregate");

        let resolver = span_builder.resolver_for(source_id);
        let header_range = header_name_span
            .expect("source config header must retain its declaration-name span")
            .resolve_with(resolver);
        assert_eq!(
            source.get(header_range.start() as usize..header_range.end() as usize),
            Some("value"),
            "header name span should retain the declaration-name token"
        );

        let fragment_range = fragment_span.resolve_with(resolver);
        assert_eq!(fragment_span.source(), SourceId::COMPILATION_ROOT);
        assert_eq!(
            fragment_range.start() as usize,
            source.find(&long_fragment_name).expect("fragment name")
        );
        assert_eq!(fragment_range.end() as usize, source.len());
        let expected_fragment = format!("{long_fragment_name}]\n");
        assert_eq!(
            source.get(fragment_range.start() as usize..fragment_range.end() as usize),
            Some(expected_fragment.as_str()),
            "const fragment span should include the post-close source boundary"
        );

        let contract = prepared
            .source_build_config_contracts
            .first()
            .expect("source config contract");
        let contract_range = contract.span.resolve_with(resolver);
        assert_eq!(contract.span.source(), SourceId::COMPILATION_ROOT);
        assert_eq!(
            source.get(contract_range.start() as usize..contract_range.end() as usize),
            Some("#"),
            "source config contract span should use the qualifier anchor"
        );

        count
    };

    assert_eq!(
        source_context.span_builder().len(),
        tokenizer_span_count + 1,
        "joining the long const fragment should append exactly one source-owned row"
    );
}

#[test]
fn const_fragment_selection_failure_stays_in_the_infrastructure_lane() {
    let source = "#[value]\n";
    let mut source_context = TestSourceContext::new("src/@page.moth");
    let mut path_fork = PathInternerFork::empty();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let token_stream = source_context
        .tokenize(
            source,
            &mut path_fork,
            &style_directives,
            TokenizerEntryMode::SourceFile,
        )
        .expect("source should tokenize");
    let lexed = token_stream;
    let scope = lexed.logical_path;
    let owner = SourceTokenOwner::new(lexed.tokens);
    let (string_table, span_builder) = source_context.preparation_parts();
    let mut cursor = owner
        .cursor(
            owner
                .full_range()
                .expect("canonical source range should fit"),
        )
        .expect("tokenized source must expose canonical tokens");
    let opening = loop {
        let token = cursor.current().expect("template opener");
        if token.tag() == TokenTag::TEMPLATE_HEAD {
            break token;
        }
        assert!(!token.is_eof(), "template opener must be present");
        cursor.advance();
    };
    cursor.advance();
    let malformed_clause =
        super::malformed_direct_selection_clause(DependencySelectionRange::new(0, 1));
    let mut warnings = Vec::new();
    let mut context = HeaderBuildContext {
        warnings: &mut warnings,
        source_file: scope,
        file_dependency_clauses: std::slice::from_ref(&malformed_clause),
        dependency_selections: &[],
        string_table,
        path_fork: &mut path_fork,
        file_role: FileRole::ActiveModuleRoot,
    };
    let failure =
        create_top_level_const_template(scope, opening, 0, &mut cursor, &mut context, span_builder)
            .expect_err("malformed retained selection should fail");

    match failure {
        HeaderParseFailure::Infrastructure(error) => {
            assert!(
                error.msg.contains("outside a table"),
                "unexpected error: {error:?}"
            );
        }
        HeaderParseFailure::Diagnostic(diagnostic) => {
            panic!("retained-data corruption must not become a source diagnostic: {diagnostic:?}");
        }
    }
}

#[test]
fn top_level_const_template_uses_selected_dependency_alias_path() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let file_path = PathBuf::from("src/@page.moth");
    let (output, _span_builder) = prepare_single_file_with_fork(
        "@widgets content as panel\n#[panel]\n",
        &file_path,
        &file_path,
        &mut string_table,
        &mut path_fork,
    );

    let const_template_header = output
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::ConstTemplate { .. }))
        .expect("expected top-level const template header");
    let mut path_scratch = Vec::new();
    let hint_paths = const_template_header
        .local_ordering_hints
        .iter()
        .map(|hint| path_fork.render_portable(hint.path(), &string_table, &mut path_scratch))
        .collect::<Vec<_>>();

    assert_eq!(hint_paths, vec!["widgets/content"]);
}

#[test]
fn top_level_const_template_collects_if_condition_dependency_refs() {
    let (headers, string_table) = parse_single_file_headers_with_table(
        "#[if show_banner:
            [if maybe_name is |name|:
                [name]
            ]
        ]\n",
    );

    let const_template_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::ConstTemplate { .. }))
        .expect("expected top-level const template header");
    let HeaderKind::ConstTemplate {
        condition_references,
        ..
    } = &const_template_header.kind
    else {
        panic!("expected const template header");
    };

    let names = condition_references
        .iter()
        .map(|reference| string_table.resolve(reference.name))
        .collect::<Vec<_>>();

    assert_eq!(names, vec!["show_banner", "maybe_name"]);
}

#[test]
fn start_function_local_references_do_not_create_module_dependencies() {
    let headers = parse_single_file_headers(
        "value = 1\n\
         another = value + 1\n\
         io.line([: [another]])\n",
    );

    let start_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::StartFunction))
        .expect("expected start function header");

    assert!(
        start_header.local_ordering_hints.is_empty(),
        "local start-function symbols must not be tracked as inter-header/module dependencies"
    );
}

#[test]
fn loop_binding_symbols_remain_in_start_function_body() {
    let (headers, string_table) = parse_single_file_headers_with_table(
        "items = {1, 2, 3}\n\
         \n\
         loop items |item, index|:\n\
             io.line([: [item]])\n\
         ;\n",
    );

    assert_eq!(
        headers.headers.len(),
        1,
        "loop-only top-level files should emit only the implicit start header"
    );
    assert!(matches!(headers.headers[0].kind, HeaderKind::StartFunction));

    let start_header = start_function_header(&headers);
    let start_symbols = symbol_tokens_in_header_body(&headers, start_header, &string_table);

    assert!(
        start_symbols.iter().any(|symbol| symbol == "item"),
        "loop item binding should stay in the implicit start body token stream"
    );
    assert!(
        start_symbols.iter().any(|symbol| symbol == "index"),
        "loop index binding should stay in the implicit start body token stream"
    );
    assert!(
        cursor_has_tag(header_body_tokens(&headers, start_header), TokenTag::LOOP),
        "start header should preserve the top-level loop statement tokens"
    );
}
#[test]
fn top_level_expression_symbols_stay_in_implicit_start_body() {
    let (headers, string_table) = parse_single_file_headers_with_table(
        "func basic()\n\
         items = {1, 2, 3}\n\
         loop items |item, index|:\n\
             io.line([: [item]])\n\
         ;\n\
         [basic]\n\
         basic()\n\
         items\n",
    );

    assert_eq!(
        headers.headers.len(),
        1,
        "dependency clauses and top-level expressions should still collapse into one start header here"
    );
    assert!(matches!(headers.headers[0].kind, HeaderKind::StartFunction));

    let start_header = start_function_header(&headers);
    let start_symbols = symbol_tokens_in_header_body(&headers, start_header, &string_table);
    let header_names = non_start_header_names(&headers, &string_table);

    assert!(
        start_symbols.iter().any(|symbol| symbol == "basic"),
        "imported symbol usage in expression/template position should stay in start body"
    );
    assert!(
        start_symbols.iter().any(|symbol| symbol == "item")
            && start_symbols.iter().any(|symbol| symbol == "index"),
        "loop binding symbols inside top-level loops should remain start-body tokens"
    );
    assert!(
        cursor_has_tag(
            header_body_tokens(&headers, start_header),
            TokenTag::TEMPLATE_HEAD,
        ),
        "runtime top-level templates should remain in the start-function token stream"
    );
    assert!(
        !header_names
            .iter()
            .any(|name| name == "basic" || name == "items" || name == "item" || name == "index"),
        "expression-position symbols must not be misclassified as top-level declaration headers"
    );
}

#[test]
fn compile_time_declarations_parse_as_headers_without_elevating_body_symbols() {
    let (headers, string_table) = parse_single_file_headers_with_table(
        "theme #= \"dark\"\n\
         items = {theme}\n\
         loop items |item, index|:\n\
             io.line([: [item]])\n\
         ;\n\
         [theme]\n\
         theme\n",
    );

    let header_names = non_start_header_names(&headers, &string_table);
    assert_eq!(
        header_names,
        vec![String::from("theme")],
        "the `theme #= ...` declaration should remain a real top-level constant header"
    );
    assert_eq!(
        headers.headers.len(),
        2,
        "expected one compile-time constant header plus the implicit start header"
    );
    assert!(
        headers
            .headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::Constant { .. })),
        "compile-time binding syntax should classify as a constant header"
    );

    let start_header = start_function_header(&headers);
    let start_symbols = symbol_tokens_in_header_body(&headers, start_header, &string_table);

    assert!(
        start_symbols.iter().any(|symbol| symbol == "theme"),
        "same-name symbol uses later in top-level expressions should stay in start body"
    );
    assert!(
        start_symbols.iter().any(|symbol| symbol == "item")
            && start_symbols.iter().any(|symbol| symbol == "index"),
        "loop-binding symbols in start-body statements must not become headers"
    );
    assert!(
        !header_names
            .iter()
            .any(|name| name == "items" || name == "item" || name == "index"),
        "only legitimate '#'-prefixed declarations should become headers"
    );
}

#[test]
fn function_without_arrow_has_zero_return_slots() {
    let headers = parse_single_file_headers("f||:\n;\n");
    let signature = first_function_signature(&headers);

    assert!(signature.returns.is_empty());
}

#[test]
fn function_value_return_is_preserved_as_return_syntax_shell() {
    let headers = parse_single_file_headers("f|| -> Int:\n;\n");
    let signature = first_function_signature(&headers);

    assert!(matches!(
        signature.returns.as_slice(),
        [ReturnSlotSyntax {
            value: FunctionReturnSyntax {
                type_annotation: ParsedTypeRef::BuiltinInt { .. },
                ..
            },
            channel: ReturnChannelSyntax::Success,
            ..
        }]
    ));
}

#[test]
fn function_named_return_is_preserved_for_ast_resolution() {
    let headers = parse_single_file_headers("f|| -> Point:\n;\n");
    let signature = first_function_signature(&headers);

    assert!(matches!(
        signature.returns.as_slice(),
        [ReturnSlotSyntax {
            value: FunctionReturnSyntax {
                type_annotation: ParsedTypeRef::Named { .. },
                ..
            },
            channel: ReturnChannelSyntax::Success,
            ..
        }]
    ));
}

#[test]
fn function_parameter_default_stays_in_header_syntax_tokens() {
    let (headers, string_table) =
        parse_single_file_headers_with_table("label |prefix String = \"item\"| -> String:\n;\n");
    let signature = first_function_signature(&headers);

    let parameter = signature
        .parameters
        .first()
        .expect("expected one parameter shell");
    assert!(matches!(
        parameter.type_annotation,
        ParsedTypeRef::BuiltinString { .. }
    ));
    assert!(
        cursor_has_string(
            default_tokens(&headers, parameter.default_range),
            TokenTag::STRING_SLICE_LITERAL,
            &string_table,
            "item",
        ),
        "header should capture default expression tokens without building an AST expression"
    );
}

#[test]
fn struct_field_default_stays_in_header_syntax_tokens() {
    let (headers, string_table) = parse_single_file_headers_with_table(
        "DEFAULT_WIDTH #= 80\nOptions = |\n    width Int = DEFAULT_WIDTH,\n|\n",
    );
    let struct_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Struct { .. }))
        .expect("expected struct header");

    let HeaderKind::Struct { fields, .. } = &struct_header.kind else {
        panic!("expected Struct header kind");
    };
    let field = fields.first().expect("expected width field shell");

    assert!(matches!(
        field.type_annotation,
        ParsedTypeRef::BuiltinInt { .. }
    ));
    assert!(
        cursor_has_string(
            default_tokens(&headers, field.default_range),
            TokenTag::SYMBOL,
            &string_table,
            "DEFAULT_WIDTH",
        ),
        "header should preserve struct default tokens for AST-time constant resolution"
    );
}

#[test]
fn function_parameter_default_path_rows_use_the_file_owned_table() {
    let (headers, _string_table) = parse_single_file_headers_with_table(
        "label |prefix String = [: [@docs/intro.md] ]| -> String:\n;\n",
    );
    let function_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Function { .. }))
        .expect("expected function header");

    let HeaderKind::Function { signature, .. } = &function_header.kind else {
        panic!("expected Function header kind");
    };
    let parameter = signature
        .parameters
        .first()
        .expect("expected one parameter shell");

    let path_id = cursor_find_path(default_tokens(&headers, parameter.default_range))
        .expect("expected a path token in the default");
    assert_ne!(path_id, PathSyntaxId::NONE);
}

#[test]
fn struct_field_default_path_rows_use_the_file_owned_table() {
    let (headers, _string_table) = parse_single_file_headers_with_table(
        "Options = |\n    path String = [: [@docs/intro.md] ],\n|\n",
    );
    let struct_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Struct { .. }))
        .expect("expected struct header");

    let HeaderKind::Struct { fields, .. } = &struct_header.kind else {
        panic!("expected Struct header kind");
    };
    let field = fields.first().expect("expected one field shell");

    let path_id = cursor_find_path(default_tokens(&headers, field.default_range))
        .expect("expected a path token in the default");
    assert_ne!(path_id, PathSyntaxId::NONE);
}

#[test]
fn function_default_and_body_path_rows_stay_distinct() {
    let (headers, _string_table) = parse_single_file_headers_with_table(
        "label |prefix String = [: [@docs/intro.md] ]| -> String:\n    io.line([: [@docs/body.md]])\n;\n",
    );
    let function_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Function { .. }))
        .expect("expected function header");

    let HeaderKind::Function { signature, .. } = &function_header.kind else {
        panic!("expected Function header kind");
    };
    let parameter = signature
        .parameters
        .first()
        .expect("expected one parameter shell");

    let default_path_id = cursor_find_path(default_tokens(&headers, parameter.default_range))
        .expect("expected a path token in the default");
    assert_ne!(default_path_id, PathSyntaxId::NONE);

    let body_path_id = cursor_find_path(header_body_tokens(&headers, function_header))
        .expect("expected a path token in the body");
    assert_ne!(body_path_id, PathSyntaxId::NONE);
}

#[test]
fn retained_header_substreams_share_one_frozen_file_path_table() {
    let (headers, _string_table) = parse_single_file_headers_with_table(
        "label |prefix String = [: [@docs/default.md] ]| -> String:\n;\nio.line([: [@docs/start.md]])\n",
    );
    let function_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Function { .. }))
        .expect("expected function header");
    let start_header = start_function_header(&headers);

    let function_owner = headers
        .source_token_owners
        .get(&function_header.tokens.source())
        .expect("function header source owner");
    let start_owner = headers
        .source_token_owners
        .get(&start_header.tokens.source())
        .expect("start header source owner");
    let function_table = function_owner
        .tokens_ref()
        .path_syntax_arc()
        .expect("prepared function source should receive the frozen file table");
    let start_table = start_owner
        .tokens_ref()
        .path_syntax_arc()
        .expect("prepared start source should receive the frozen file table");
    assert!(
        Arc::ptr_eq(&function_table, &start_table),
        "ordinary retained header substreams must share one immutable file-owned path table"
    );

    let HeaderKind::Function { signature, .. } = &function_header.kind else {
        panic!("expected Function header kind");
    };
    let default_path_id = cursor_find_path(default_tokens(
        &headers,
        signature.parameters[0].default_range,
    ))
    .expect("expected a default path token");
    let start_path_id = cursor_find_path(header_body_tokens(&headers, start_header))
        .expect("expected a start-body path token");

    assert_ne!(default_path_id, PathSyntaxId::NONE);
    assert_ne!(start_path_id, PathSyntaxId::NONE);
}

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
#[test]
fn retained_header_substreams_do_not_count_copied_path_rows() {
    use crate::compiler_frontend::instrumentation::{
        capture_frontend_counters_for_test, log_frontend_counters, reset_frontend_counters,
    };
    use crate::timing::start_benchmark_collection;

    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _counter_capture = capture_frontend_counters_for_test();
    reset_frontend_counters();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");

    parse_single_file_headers_with_table(
        "label |prefix String = [: [@docs/default.md] ]| -> String:\n;\nio.line([: [@docs/start.md]])\n",
    );

    log_frontend_counters();
    let observations = timing_session.finish();
    let counter_value = |name: &str| {
        observations
            .counters
            .iter()
            .find(|counter| counter.name == name)
            .map(|counter| counter.value)
            .unwrap_or(-1.0)
    };

    assert_eq!(counter_value("path_syntax_row_count"), 2.0);
    assert_eq!(counter_value("token_rescan_count"), 0.0);
}

#[test]
fn function_signature_rejects_void_return_syntax() {
    let source = format!("f|| {}{}:\n;\n", "-> ", "Void");
    let result = parse_single_file_headers_with_entry(&source, "src/@page.moth", "src/@page.moth");
    assert!(result.is_err(), "void return syntax must be rejected");
    let errors = result.err().expect("expected parse errors");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFunctionSignature {
            reason: InvalidFunctionSignatureReason::VoidNotAllowed
        }
    )));
}

#[test]
fn function_signature_rejects_none_return_syntax() {
    let source = format!("f|| {}{}:\n;\n", "-> ", "None");
    let result = parse_single_file_headers_with_entry(&source, "src/@page.moth", "src/@page.moth");
    assert!(result.is_err(), "none return syntax must be rejected");
    let errors = result.err().expect("expected parse errors");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidTypeAnnotation {
            reason: InvalidTypeAnnotationReason::NoneNotAllowed,
            ..
        }
    )));
}

#[test]
fn function_signature_preserves_unknown_symbolic_return_for_ast_resolution() {
    let headers = parse_single_file_headers("f|| -> MissingType:\n;\n");
    let signature = first_function_signature(&headers);

    assert!(matches!(
        signature.returns.as_slice(),
        [ReturnSlotSyntax {
            value: FunctionReturnSyntax {
                type_annotation: ParsedTypeRef::Named { .. },
                ..
            },
            channel: ReturnChannelSyntax::Success,
            ..
        }]
    ));
}
