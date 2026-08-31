//! String-ID remapping tests for flat header metadata types.
//!
//! WHAT: verifies that `TopLevelConstFragment` and `RetainedDependencyClause`
//!      can be remapped from local string tables into a merged global table.
//! WHY: per-file frontend preparation produces these flat metadata structures using local
//!      string tables; remapping must preserve all paths, names, aliases, and source locations.

use crate::compiler_frontend::arena::TokenStats;
use crate::compiler_frontend::compiler_messages::source_location::{CharPosition, SourceLocation};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticKind, DiagnosticPayload, NameNamespace, RuleDiagnosticKind,
};
use crate::compiler_frontend::datatypes::generic_parameters::{
    GenericParameter, GenericParameterList, TypeParameterId,
};
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::binding_mode::BindingMode;
use crate::compiler_frontend::declaration_syntax::choice::{
    ChoiceVariantPayloadSyntax, ChoiceVariantSyntax,
};
use crate::compiler_frontend::declaration_syntax::declaration_shell::{
    DeclarationSyntax, InitializerReference,
};
use crate::compiler_frontend::declaration_syntax::signature_members::{
    FunctionReturnSyntax, FunctionSignatureSyntax, ReturnChannelSyntax, ReturnSlotSyntax,
    SignatureMemberSyntax,
};
use crate::compiler_frontend::headers::dependency_clause_syntax::{
    DependencyAlias, RetainedDependencyPath,
};
use crate::compiler_frontend::headers::types::{
    DependencyBindingSyntax, DependencySelection, DependencySelectionRange,
    FileFrontendPrepareError, FileFrontendPrepareOutput, FileRole, Header, HeaderExportMode,
    HeaderKind, LocalDeclarationOrderingHint, PreparedFilePathSyntax, RetainedDependencyClause,
    TopLevelConstFragment,
};
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::public_interface::{
    ProviderDependencyKind, PublicSemanticInterface, SourceProviderDependency,
    SourceProviderDependencySet,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::identity::{DependencyShellId, FileId};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, Token, TokenKind};
use crate::compiler_frontend::traits::syntax::{
    TraitDeclarationSyntax, TraitRequirementSyntax, TraitThisUsage,
};
use crate::compiler_frontend::value_mode::ValueMode;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn make_location(path_name: &str, string_table: &mut StringTable) -> SourceLocation {
    let path = InternedPath::from_single_str(path_name, string_table);
    SourceLocation::new(path, CharPosition::default(), CharPosition::default())
}

fn assert_location_resolves_to(
    location: &SourceLocation,
    expected: &str,
    string_table: &StringTable,
) {
    let scope_components = location
        .scope
        .as_components()
        .iter()
        .map(|id| string_table.resolve(*id))
        .collect::<Vec<_>>();

    assert_eq!(scope_components, vec![expected]);
}

fn make_signature_member(name: &str, string_table: &mut StringTable) -> SignatureMemberSyntax {
    let location = make_location("test.moth", string_table);

    SignatureMemberSyntax {
        id: InternedPath::from_single_str(name, string_table),
        value_mode: ValueMode::ImmutableOwned,
        is_reactive: false,
        type_annotation: ParsedTypeRef::BuiltinInt {
            location: location.clone(),
        },
        default_tokens: vec![],
        location,
    }
}

fn make_generic_parameter_list(name: &str, string_table: &mut StringTable) -> GenericParameterList {
    GenericParameterList {
        parameters: vec![GenericParameter {
            id: TypeParameterId(0),
            name: string_table.intern(name),
            location: make_location("test.moth", string_table),
            trait_bounds: Vec::new(),
        }],
    }
}

fn make_file_tokens(symbol_name: &str, string_table: &mut StringTable) -> FileTokens {
    let src_path = InternedPath::from_single_str("test.moth", string_table);
    let token = Token::new(
        TokenKind::Symbol(string_table.intern(symbol_name)),
        make_location("test.moth", string_table),
    );
    FileTokens::new(src_path, vec![token])
}

fn make_prepared_header(
    source_file: &InternedPath,
    file_id: FileId,
    canonical_os_path: &Path,
    tokens: Vec<Token>,
) -> Header {
    Header {
        kind: HeaderKind::StartFunction,
        file_role: FileRole::Normal,
        export_mode: HeaderExportMode::Private,
        local_ordering_hints: HashSet::new(),
        name_location: SourceLocation::new(
            source_file.clone(),
            CharPosition::default(),
            CharPosition::default(),
        ),
        tokens: FileTokens::new_deferred_with_identity(
            source_file.clone(),
            Some(file_id),
            Some(canonical_os_path.to_path_buf()),
            tokens,
        ),
        source_file: source_file.clone(),
        capacity_references: Vec::new(),
    }
}

fn make_prepared_output(
    source_file: InternedPath,
    file_id: FileId,
    canonical_os_path: PathBuf,
    headers: Vec<Header>,
) -> FileFrontendPrepareOutput {
    FileFrontendPrepareOutput {
        source_file,
        file_id: Some(file_id),
        path_syntax: PreparedFilePathSyntax::empty(),
        token_count: 0,
        token_stats: TokenStats::default(),
        file_role: FileRole::Normal,
        file_dependency_clauses: Vec::new(),
        structural_file_references: Default::default(),
        dependency_selections: Vec::new(),
        canonical_os_path: Some(canonical_os_path),
        headers,
        top_level_const_fragments: Vec::new(),
        const_template_count: 0,
        runtime_fragment_count: 0,
        has_non_trivial_root_body: false,
        warnings: Vec::new(),
    }
}

#[test]
fn top_level_const_fragment_remaps_path_and_location() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let header_path = InternedPath::from_single_str("src/@page.moth", &mut local);
    let location = make_location("src/@page.moth", &mut local);

    let mut fragment = TopLevelConstFragment {
        runtime_insertion_index: 3,
        header_path,
        location,
    };

    let remap = global.merge_from(&local);
    fragment.remap_string_ids(&remap);

    assert_eq!(fragment.runtime_insertion_index, 3);
    assert_eq!(
        fragment.header_path.to_portable_string(&global),
        "src/@page.moth"
    );
    assert_location_resolves_to(&fragment.location, "src/@page.moth", &global);
}

#[test]
fn file_dependency_clause_remaps_all_fields_without_alias() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let header_path = InternedPath::from_single_str("@html/head", &mut local);
    let location = make_location("test.moth", &mut local);
    let path_location = make_location("test.moth", &mut local);

    let provider = RetainedDependencyPath {
        path: header_path,
        path_syntax: crate::compiler_frontend::paths::path_syntax::PathSyntaxId::NONE,
        target: crate::compiler_frontend::headers::dependency_target::DependencyTargetKind::Source,
        location: path_location,
        dependency_shell_id: DependencyShellId::new(FileId(0), 0),
    };
    let mut dependency = RetainedDependencyClause {
        dependency: provider.clone(),
        binding: DependencyBindingSyntax::Namespace { alias: None },
        location,
        export_mode: HeaderExportMode::Private,
    };

    let remap = global.merge_from(&local);
    dependency.remap_string_ids(&remap);

    assert_eq!(
        dependency.dependency.path.to_portable_string(&global),
        "@html/head"
    );
    assert!(matches!(
        dependency.binding,
        DependencyBindingSyntax::Namespace { alias: None }
    ));
    assert_location_resolves_to(&dependency.location, "test.moth", &global);
    assert_location_resolves_to(&dependency.dependency.location, "test.moth", &global);
    assert_eq!(dependency.export_mode, HeaderExportMode::Private);
}

#[test]
fn file_dependency_clause_remaps_all_fields_with_alias() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let alias_name = local.intern("h");
    let header_path = InternedPath::from_single_str("@html/head", &mut local);
    let location = make_location("test.moth", &mut local);
    let path_location = make_location("test.moth", &mut local);
    let alias_location = make_location("test.moth", &mut local);

    let provider = RetainedDependencyPath {
        path: header_path,
        path_syntax: crate::compiler_frontend::paths::path_syntax::PathSyntaxId::NONE,
        target: crate::compiler_frontend::headers::dependency_target::DependencyTargetKind::Source,
        location: path_location,
        dependency_shell_id: DependencyShellId::new(FileId(0), 1),
    };
    let mut dependency = RetainedDependencyClause {
        dependency: provider.clone(),
        binding: DependencyBindingSyntax::Namespace {
            alias: Some(DependencyAlias {
                name: alias_name,
                location: alias_location,
            }),
        },
        location,
        export_mode: HeaderExportMode::Public,
    };

    let remap = global.merge_from(&local);
    dependency.remap_string_ids(&remap);

    assert_eq!(
        dependency.dependency.path.to_portable_string(&global),
        "@html/head"
    );
    let DependencyBindingSyntax::Namespace { alias: Some(alias) } = &dependency.binding else {
        panic!("expected namespace binding with alias");
    };
    assert_eq!(global.resolve(alias.name), "h");
    assert_location_resolves_to(&dependency.location, "test.moth", &global);
    assert_location_resolves_to(&dependency.dependency.location, "test.moth", &global);
    assert_location_resolves_to(&alias.location, "test.moth", &global);
    assert_eq!(dependency.export_mode, HeaderExportMode::Public);
}

#[test]
fn remap_preserves_correct_ids_when_global_has_preexisting_strings() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    // Preexisting strings in global ensure the merge is non-identity.
    global.intern("preexisting_a");
    global.intern("preexisting_b");

    let alias_name = local.intern("my_alias");
    let mut header_path = InternedPath::new();
    header_path.push_str("utils", &mut local);
    header_path.push_str("helpers", &mut local);
    let local_path_components = header_path.as_components().to_vec();
    let location = make_location("file.moth", &mut local);
    let path_location = make_location("file.moth", &mut local);
    let alias_location = make_location("file.moth", &mut local);
    let original_shell = DependencyShellId::new(FileId(0), 2);

    let provider = RetainedDependencyPath {
        path: header_path,
        path_syntax: crate::compiler_frontend::paths::path_syntax::PathSyntaxId::NONE,
        target: crate::compiler_frontend::headers::dependency_target::DependencyTargetKind::Source,
        location: path_location,
        dependency_shell_id: original_shell,
    };
    let mut dependency = RetainedDependencyClause {
        dependency: provider.clone(),
        binding: DependencyBindingSyntax::Namespace {
            alias: Some(DependencyAlias {
                name: alias_name,
                location: alias_location,
            }),
        },
        location,
        export_mode: HeaderExportMode::Public,
    };

    let mut fragment = TopLevelConstFragment {
        runtime_insertion_index: 7,
        header_path: InternedPath::from_single_str("file.moth", &mut local),
        location: make_location("file.moth", &mut local),
    };

    let remap = global.merge_from(&local);
    dependency.remap_string_ids(&remap);
    fragment.remap_string_ids(&remap);

    // Verify the alias resolves to the correct string in the global table.
    let DependencyBindingSyntax::Namespace { alias: Some(alias) } = &dependency.binding else {
        panic!("expected namespace binding with alias");
    };
    assert_eq!(global.resolve(alias.name), "my_alias");

    // Remap must change component IDs when the merged table is not identity.
    assert_ne!(
        dependency.dependency.path.as_components(),
        local_path_components.as_slice(),
        "merged string IDs must not keep the local table indexes"
    );
    assert_eq!(
        dependency.dependency.dependency_shell_id, original_shell,
        "shell identity is file-local and must survive string remapping unchanged"
    );
    assert_eq!(
        dependency.dependency.path.to_portable_string(&global),
        "utils/helpers"
    );
    assert_eq!(dependency.dependency.path.as_components().len(), 2);
    assert_eq!(
        dependency
            .dependency
            .path
            .as_components()
            .iter()
            .map(|component| global.resolve(*component))
            .collect::<Vec<_>>(),
        ["utils", "helpers"]
    );
    assert_eq!(
        dependency.dependency.target,
        crate::compiler_frontend::headers::dependency_target::DependencyTargetKind::Source
    );

    // Verify fragment path resolves correctly.
    assert_eq!(
        fragment.header_path.to_portable_string(&global),
        "file.moth"
    );

    // Verify all locations still resolve.
    assert_location_resolves_to(&dependency.location, "file.moth", &global);
    assert_location_resolves_to(&dependency.dependency.location, "file.moth", &global);
    assert_location_resolves_to(&alias.location, "file.moth", &global);
    assert_location_resolves_to(&fragment.location, "file.moth", &global);
}

// -----------------------------------------------------------
//  HeaderKind remapping tests
// -----------------------------------------------------------

#[test]
fn header_kind_function_remaps_generic_parameters_and_signature() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let generic_parameters = make_generic_parameter_list("T", &mut local);
    let signature = FunctionSignatureSyntax::default();

    let mut kind = HeaderKind::Function {
        generic_parameters,
        signature,
    };

    let remap = global.merge_from(&local);
    kind.remap_string_ids(&remap);

    let HeaderKind::Function {
        generic_parameters, ..
    } = kind
    else {
        panic!("expected Function kind");
    };
    assert_eq!(global.resolve(generic_parameters.parameters[0].name), "T");
    assert_location_resolves_to(
        &generic_parameters.parameters[0].location,
        "test.moth",
        &global,
    );
}

#[test]
fn header_kind_constant_remaps_declaration() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let type_name = local.intern("MyType");
    let declaration = DeclarationSyntax {
        binding_mode: BindingMode::ImmutableRuntime,
        type_annotation: ParsedTypeRef::Named {
            name: type_name,
            location: make_location("test.moth", &mut local),
        },
        initializer_tokens: vec![],
        initializer_references: vec![],
        location: make_location("test.moth", &mut local),
    };

    let mut kind = HeaderKind::Constant { declaration };

    let remap = global.merge_from(&local);
    kind.remap_string_ids(&remap);

    let HeaderKind::Constant { declaration, .. } = kind else {
        panic!("expected Constant kind");
    };

    let ParsedTypeRef::Named { name, location } = &declaration.type_annotation else {
        panic!("expected Named type annotation");
    };
    assert_eq!(global.resolve(*name), "MyType");
    assert_location_resolves_to(location, "test.moth", &global);
    assert_location_resolves_to(&declaration.location, "test.moth", &global);
}

#[test]
fn header_kind_struct_remaps_generic_parameters_and_fields() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let generic_parameters = make_generic_parameter_list("T", &mut local);
    let field = make_signature_member("field", &mut local);

    let mut kind = HeaderKind::Struct {
        generic_parameters,
        fields: vec![field],
    };

    let remap = global.merge_from(&local);
    kind.remap_string_ids(&remap);

    let HeaderKind::Struct {
        generic_parameters,
        fields,
    } = kind
    else {
        panic!("expected Struct kind");
    };
    assert_eq!(global.resolve(generic_parameters.parameters[0].name), "T");
    assert_location_resolves_to(&fields[0].location, "test.moth", &global);
}

#[test]
fn header_kind_choice_remaps_generic_parameters_and_variants() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let generic_parameters = make_generic_parameter_list("T", &mut local);
    let variant = ChoiceVariantSyntax {
        id: local.intern("SomeVariant"),
        payload: ChoiceVariantPayloadSyntax::Unit,
        location: make_location("test.moth", &mut local),
    };

    let mut kind = HeaderKind::Choice {
        generic_parameters,
        variants: vec![variant],
    };

    let remap = global.merge_from(&local);
    kind.remap_string_ids(&remap);

    let HeaderKind::Choice {
        generic_parameters,
        variants,
    } = kind
    else {
        panic!("expected Choice kind");
    };
    assert_eq!(global.resolve(generic_parameters.parameters[0].name), "T");
    assert_eq!(global.resolve(variants[0].id), "SomeVariant");
    assert_location_resolves_to(&variants[0].location, "test.moth", &global);
}

#[test]
fn header_kind_type_alias_remaps_target() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let target = ParsedTypeRef::Named {
        name: local.intern("TargetType"),
        location: make_location("test.moth", &mut local),
    };

    let mut kind = HeaderKind::TypeAlias { target };

    let remap = global.merge_from(&local);
    kind.remap_string_ids(&remap);

    let HeaderKind::TypeAlias { target } = kind else {
        panic!("expected TypeAlias kind");
    };

    let ParsedTypeRef::Named { name, location } = target else {
        panic!("expected Named target");
    };
    assert_eq!(global.resolve(name), "TargetType");
    assert_location_resolves_to(&location, "test.moth", &global);
}

#[test]
fn header_kind_const_template_remaps_condition_references() {
    let mut global = StringTable::new();
    let mut local = StringTable::new();
    let show_banner = local.intern("show_banner");

    let mut kind = HeaderKind::ConstTemplate {
        condition_references: vec![InitializerReference {
            name: show_banner,
            dot_member: None,
            location: make_location("test.moth", &mut local),
            followed_by_call: false,
            followed_by_choice_namespace: false,
        }],
    };

    let remap = global.merge_from(&local);
    kind.remap_string_ids(&remap);

    let HeaderKind::ConstTemplate {
        condition_references,
        ..
    } = kind
    else {
        panic!("expected ConstTemplate kind");
    };
    assert_eq!(global.resolve(condition_references[0].name), "show_banner");
    assert_location_resolves_to(&condition_references[0].location, "test.moth", &global);
}

#[test]
fn header_kind_start_function_is_no_op() {
    let mut kind = HeaderKind::StartFunction;
    let identity_remap = {
        let mut global = StringTable::new();
        global.merge_from(&StringTable::new())
    };
    kind.remap_string_ids(&identity_remap);
    // No panic and no fields to assert.
}

// -----------------------------------------------------------
//  Header container remapping tests
// -----------------------------------------------------------

#[test]
fn header_remaps_kind_dependencies_locations_tokens_and_source_file() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let generic_parameters = make_generic_parameter_list("T", &mut local);
    let mut dependencies = HashSet::new();
    dependencies.insert(LocalDeclarationOrderingHint::provider_spelling(
        InternedPath::from_single_str("@core/prelude", &mut local),
    ));

    let mut header = Header {
        kind: HeaderKind::Function {
            generic_parameters,
            signature: FunctionSignatureSyntax::default(),
        },
        file_role: FileRole::Normal,
        export_mode: HeaderExportMode::Private,
        local_ordering_hints: dependencies,
        name_location: make_location("test.moth", &mut local),
        tokens: make_file_tokens("my_symbol", &mut local),
        source_file: InternedPath::from_single_str("test.moth", &mut local),
        capacity_references: Vec::new(),
    };

    let remap = global.merge_from(&local);
    header.remap_string_ids(&remap);

    // Verify kind remapped.
    let HeaderKind::Function {
        generic_parameters, ..
    } = &header.kind
    else {
        panic!("expected Function kind");
    };
    assert_eq!(global.resolve(generic_parameters.parameters[0].name), "T");

    // Verify dependencies remapped.
    assert_eq!(header.local_ordering_hints.len(), 1);
    let dep = header.local_ordering_hints.iter().next().unwrap();
    assert_eq!(dep.path().to_portable_string(&global), "@core/prelude");

    // Verify name location remapped.
    assert_location_resolves_to(&header.name_location, "test.moth", &global);

    // Verify tokens remapped.
    assert_eq!(
        header.tokens.src_path.to_portable_string(&global),
        "test.moth"
    );
    let token_kind = &header.tokens.tokens[0].kind;
    let TokenKind::Symbol(symbol_id) = token_kind else {
        panic!("expected Symbol token");
    };
    assert_eq!(global.resolve(*symbol_id), "my_symbol");

    // Verify source file remapped.
    assert_eq!(header.source_file.to_portable_string(&global), "test.moth");
}

#[test]
fn header_remap_preserves_correct_ids_when_global_has_preexisting_strings() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    // Preexisting strings in global ensure the merge is non-identity.
    global.intern("preexisting_a");
    global.intern("preexisting_b");

    let generic_parameters = make_generic_parameter_list("T", &mut local);
    let mut dependencies = HashSet::new();
    dependencies.insert(LocalDeclarationOrderingHint::provider_spelling(
        InternedPath::from_single_str("@core/prelude", &mut local),
    ));

    let mut header = Header {
        kind: HeaderKind::Function {
            generic_parameters,
            signature: FunctionSignatureSyntax::default(),
        },
        file_role: FileRole::Normal,
        export_mode: HeaderExportMode::Public,
        local_ordering_hints: dependencies,
        name_location: make_location("test.moth", &mut local),
        tokens: make_file_tokens("my_symbol", &mut local),
        source_file: InternedPath::from_single_str("test.moth", &mut local),
        capacity_references: Vec::new(),
    };

    let remap = global.merge_from(&local);
    header.remap_string_ids(&remap);

    // Verify generic parameter name resolves correctly after non-identity merge.
    let HeaderKind::Function {
        generic_parameters, ..
    } = &header.kind
    else {
        panic!("expected Function kind");
    };
    assert_eq!(global.resolve(generic_parameters.parameters[0].name), "T");

    // Verify dependency resolves correctly.
    assert_eq!(
        header
            .local_ordering_hints
            .iter()
            .next()
            .unwrap()
            .path()
            .to_portable_string(&global),
        "@core/prelude"
    );

    // Verify token symbol resolves correctly.
    let TokenKind::Symbol(symbol_id) = &header.tokens.tokens[0].kind else {
        panic!("expected Symbol token");
    };
    assert_eq!(global.resolve(*symbol_id), "my_symbol");
}

// -----------------------------------------------------------
//  FileFrontendPrepareOutput remapping tests
// -----------------------------------------------------------

fn make_unknown_name_diagnostic(name: &str, string_table: &mut StringTable) -> CompilerDiagnostic {
    let name_id = string_table.intern(name);
    let location = make_location("test.moth", string_table);
    CompilerDiagnostic::new(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        location,
        DiagnosticPayload::UnknownName {
            name: name_id,
            namespace: NameNamespace::Value,
        },
    )
}

#[test]
fn file_frontend_prepare_output_remaps_all_string_id_fields() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    // Preexisting strings in global ensure the merge is non-identity.
    global.intern("preexisting_a");
    global.intern("preexisting_b");

    let source_file = InternedPath::from_single_str("src/main.moth", &mut local);

    let generic_parameters = make_generic_parameter_list("T", &mut local);
    let mut dependencies = HashSet::new();
    dependencies.insert(LocalDeclarationOrderingHint::provider_spelling(
        InternedPath::from_single_str("@core/prelude", &mut local),
    ));

    let header = Header {
        kind: HeaderKind::Function {
            generic_parameters,
            signature: FunctionSignatureSyntax::default(),
        },
        file_role: FileRole::Normal,
        export_mode: HeaderExportMode::Private,
        local_ordering_hints: dependencies,
        name_location: make_location("test.moth", &mut local),
        tokens: make_file_tokens("my_func", &mut local),
        source_file: InternedPath::from_single_str("test.moth", &mut local),
        capacity_references: Vec::new(),
    };

    let fragment = TopLevelConstFragment {
        runtime_insertion_index: 2,
        header_path: InternedPath::from_single_str("src/@page.moth", &mut local),
        location: make_location("src/@page.moth", &mut local),
    };

    let warning = make_unknown_name_diagnostic("warn_name", &mut local);

    let provider = RetainedDependencyPath {
        path: InternedPath::from_single_str("@html/head", &mut local),
        path_syntax: crate::compiler_frontend::paths::path_syntax::PathSyntaxId::NONE,
        target: crate::compiler_frontend::headers::dependency_target::DependencyTargetKind::Source,
        location: make_location("test.moth", &mut local),
        dependency_shell_id: DependencyShellId::new(FileId(0), 0),
    };
    let dependency = RetainedDependencyClause {
        dependency: provider.clone(),
        binding: DependencyBindingSyntax::Namespace {
            alias: Some(DependencyAlias {
                name: local.intern("h"),
                location: make_location("test.moth", &mut local),
            }),
        },
        location: make_location("test.moth", &mut local),
        export_mode: HeaderExportMode::Public,
    };

    let mut output = FileFrontendPrepareOutput {
        source_file,
        file_id: None,
        path_syntax: PreparedFilePathSyntax::empty(),
        token_count: 12,
        token_stats: TokenStats::default(),
        file_role: FileRole::Normal,
        file_dependency_clauses: vec![dependency],
        structural_file_references: Default::default(),
        dependency_selections: Vec::new(),
        canonical_os_path: None,
        headers: vec![header],
        top_level_const_fragments: vec![fragment],
        const_template_count: 5,
        runtime_fragment_count: 3,
        has_non_trivial_root_body: false,
        warnings: vec![warning],
    };

    let remap = global.merge_from(&local);
    output
        .remap_string_ids(&remap)
        .expect("complete output should remap into the merged string table");

    // source_file remapped.
    assert_eq!(
        output.source_file.to_portable_string(&global),
        "src/main.moth"
    );

    // file_id unchanged.
    assert!(output.file_id.is_none());

    // Header nested fields remapped.
    assert_eq!(output.headers.len(), 1);
    let header = &output.headers[0];
    let HeaderKind::Function {
        generic_parameters, ..
    } = &header.kind
    else {
        panic!("expected Function kind");
    };
    assert_eq!(global.resolve(generic_parameters.parameters[0].name), "T");
    assert_eq!(
        header
            .local_ordering_hints
            .iter()
            .next()
            .unwrap()
            .path()
            .to_portable_string(&global),
        "@core/prelude"
    );
    assert_location_resolves_to(&header.name_location, "test.moth", &global);
    let TokenKind::Symbol(symbol_id) = &header.tokens.tokens[0].kind else {
        panic!("expected Symbol token");
    };
    assert_eq!(global.resolve(*symbol_id), "my_func");
    assert_eq!(header.source_file.to_portable_string(&global), "test.moth");

    // Per-file dependency clauses remapped.
    assert_eq!(output.file_dependency_clauses.len(), 1);
    let dependency = &output.file_dependency_clauses[0];
    assert_eq!(
        dependency.dependency.path.to_portable_string(&global),
        "@html/head"
    );
    let DependencyBindingSyntax::Namespace { alias: Some(alias) } = &dependency.binding else {
        panic!("expected namespace binding with alias");
    };
    assert_eq!(global.resolve(alias.name), "h");
    assert_eq!(dependency.export_mode, HeaderExportMode::Public);

    // Const fragment remapped.
    assert_eq!(output.top_level_const_fragments.len(), 1);
    let fragment = &output.top_level_const_fragments[0];
    assert_eq!(fragment.runtime_insertion_index, 2);
    assert_eq!(
        fragment.header_path.to_portable_string(&global),
        "src/@page.moth"
    );
    assert_location_resolves_to(&fragment.location, "src/@page.moth", &global);

    // Counters unchanged.
    assert_eq!(output.const_template_count, 5);
    assert_eq!(output.runtime_fragment_count, 3);

    // Warnings remapped.
    assert_eq!(output.warnings.len(), 1);
    let warning = &output.warnings[0];
    let DiagnosticPayload::UnknownName { name, .. } = &warning.payload else {
        panic!("expected UnknownName payload");
    };
    assert_eq!(global.resolve(*name), "warn_name");
    assert_location_resolves_to(&warning.primary_location, "test.moth", &global);
}

#[test]
fn file_frontend_prepare_output_identity_remap_preserves_payload() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let source_file = InternedPath::from_single_str("src/main.moth", &mut local);
    let warning = make_unknown_name_diagnostic("warn_name", &mut local);

    let mut output = FileFrontendPrepareOutput {
        source_file,
        file_id: None,
        path_syntax: PreparedFilePathSyntax::empty(),
        token_count: 0,
        token_stats: TokenStats::default(),
        file_role: FileRole::Normal,
        file_dependency_clauses: Vec::new(),
        structural_file_references: Default::default(),
        dependency_selections: Vec::new(),
        canonical_os_path: None,
        headers: Vec::new(),
        top_level_const_fragments: Vec::new(),
        const_template_count: 0,
        runtime_fragment_count: 0,
        has_non_trivial_root_body: false,
        warnings: vec![warning],
    };

    let remap = global.merge_from(&local);
    assert!(remap.is_identity());

    output
        .remap_string_ids(&remap)
        .expect("identity remapping should preserve the complete output");

    assert_eq!(
        output.source_file.to_portable_string(&global),
        "src/main.moth"
    );

    let DiagnosticPayload::UnknownName { name, .. } = &output.warnings[0].payload else {
        panic!("expected UnknownName payload");
    };
    assert_eq!(global.resolve(*name), "warn_name");
    assert_location_resolves_to(&output.warnings[0].primary_location, "test.moth", &global);
}

#[test]
fn file_frontend_prepare_output_rebinds_complete_nested_payload_atomically() {
    let mut string_table = StringTable::new();
    let provisional_source = InternedPath::from_components(vec![
        string_table.intern("src"),
        string_table.intern("main.moth"),
    ]);
    let final_source = InternedPath::from_components(vec![
        string_table.intern("logical"),
        string_table.intern("main.moth"),
    ]);
    let provisional_location = make_location("src/main.moth", &mut string_table);

    let mut local_ordering_hints = HashSet::new();
    local_ordering_hints.insert(LocalDeclarationOrderingHint::source_owned(
        provisional_source.append(string_table.intern("ordering_hint")),
    ));

    let parameter_name = string_table.intern("parameter");
    let parameter = SignatureMemberSyntax {
        id: provisional_source.append(parameter_name),
        value_mode: ValueMode::ImmutableOwned,
        is_reactive: false,
        type_annotation: ParsedTypeRef::Named {
            name: string_table.intern("Input"),
            location: provisional_location.clone(),
        },
        default_tokens: vec![Token::new(
            TokenKind::Symbol(string_table.intern("default")),
            provisional_location.clone(),
        )],
        location: provisional_location.clone(),
    };
    let return_slot = ReturnSlotSyntax {
        value: FunctionReturnSyntax {
            type_annotation: ParsedTypeRef::Named {
                name: string_table.intern("Output"),
                location: provisional_location.clone(),
            },
            location: provisional_location.clone(),
        },
        channel: ReturnChannelSyntax::Success,
        location: provisional_location.clone(),
    };
    let function_signature = FunctionSignatureSyntax {
        parameters: vec![parameter],
        returns: vec![return_slot],
    };

    let constant_declaration = DeclarationSyntax {
        binding_mode: BindingMode::ImmutableRuntime,
        type_annotation: ParsedTypeRef::Named {
            name: string_table.intern("Value"),
            location: provisional_location.clone(),
        },
        initializer_tokens: vec![Token::new(
            TokenKind::Symbol(string_table.intern("initializer")),
            provisional_location.clone(),
        )],
        initializer_references: vec![InitializerReference {
            name: string_table.intern("dependency"),
            dot_member: None,
            location: provisional_location.clone(),
            followed_by_call: false,
            followed_by_choice_namespace: false,
        }],
        location: provisional_location.clone(),
    };

    let trait_requirement = TraitRequirementSyntax {
        name: string_table.intern("required"),
        name_location: provisional_location.clone(),
        this_usage: TraitThisUsage::Immutable,
        signature: FunctionSignatureSyntax {
            parameters: vec![SignatureMemberSyntax {
                id: provisional_source.append(string_table.intern("This")),
                value_mode: ValueMode::ImmutableOwned,
                is_reactive: false,
                type_annotation: ParsedTypeRef::This {
                    location: provisional_location.clone(),
                },
                default_tokens: Vec::new(),
                location: provisional_location.clone(),
            }],
            returns: Vec::new(),
        },
        location: provisional_location.clone(),
    };

    let warning = CompilerDiagnostic::import_name_collision(
        string_table.intern("conflict"),
        Some(provisional_location.clone()),
        provisional_location.clone(),
    );
    let provider_name = string_table.intern("provider");
    let provider_path = InternedPath::from_components(vec![provider_name]);
    let provider = RetainedDependencyPath {
        path: provider_path.clone(),
        path_syntax: crate::compiler_frontend::paths::path_syntax::PathSyntaxId::NONE,
        target: crate::compiler_frontend::headers::dependency_target::DependencyTargetKind::Source,
        location: provisional_location.clone(),
        dependency_shell_id: DependencyShellId::new(FileId(7), 0),
    };
    let clause_location = provisional_location.clone();

    let mut output = FileFrontendPrepareOutput {
        source_file: provisional_source.clone(),
        file_id: Some(FileId(7)),
        path_syntax: PreparedFilePathSyntax::empty(),
        token_count: 3,
        token_stats: TokenStats::default(),
        file_role: FileRole::Normal,
        file_dependency_clauses: vec![RetainedDependencyClause {
            dependency: provider,
            binding: DependencyBindingSyntax::Namespace { alias: None },
            location: clause_location,
            export_mode: HeaderExportMode::Private,
        }],
        structural_file_references: Default::default(),
        dependency_selections: Vec::new(),
        canonical_os_path: Some(PathBuf::from("/provisional/src/main.moth")),
        headers: vec![
            Header {
                kind: HeaderKind::Function {
                    generic_parameters: make_generic_parameter_list("T", &mut string_table),
                    signature: function_signature,
                },
                file_role: FileRole::Normal,
                export_mode: HeaderExportMode::Private,
                local_ordering_hints: local_ordering_hints.clone(),
                name_location: provisional_location.clone(),
                tokens: FileTokens::new_deferred_with_identity(
                    provisional_source.clone(),
                    Some(FileId(7)),
                    Some(PathBuf::from("/provisional/src/main.moth")),
                    vec![Token::new(
                        TokenKind::Symbol(string_table.intern("function_body")),
                        provisional_location.clone(),
                    )],
                ),
                source_file: provisional_source.clone(),
                capacity_references: vec![InitializerReference {
                    name: string_table.intern("capacity"),
                    dot_member: None,
                    location: provisional_location.clone(),
                    followed_by_call: false,
                    followed_by_choice_namespace: false,
                }],
            },
            Header {
                kind: HeaderKind::Constant {
                    declaration: constant_declaration,
                },
                file_role: FileRole::Normal,
                export_mode: HeaderExportMode::Private,
                local_ordering_hints: HashSet::new(),
                name_location: provisional_location.clone(),
                tokens: FileTokens::new_deferred_with_identity(
                    provisional_source.clone(),
                    Some(FileId(7)),
                    Some(PathBuf::from("/provisional/src/main.moth")),
                    vec![Token::new(
                        TokenKind::Symbol(string_table.intern("constant_body")),
                        provisional_location.clone(),
                    )],
                ),
                source_file: provisional_source.clone(),
                capacity_references: Vec::new(),
            },
            Header {
                kind: HeaderKind::Trait {
                    declaration: TraitDeclarationSyntax {
                        name: string_table.intern("Trait"),
                        name_location: provisional_location.clone(),
                        requirements: vec![trait_requirement],
                        location: provisional_location.clone(),
                    },
                },
                file_role: FileRole::Normal,
                export_mode: HeaderExportMode::Private,
                local_ordering_hints: HashSet::new(),
                name_location: provisional_location.clone(),
                tokens: FileTokens::new_deferred_with_identity(
                    provisional_source.clone(),
                    Some(FileId(7)),
                    Some(PathBuf::from("/provisional/src/main.moth")),
                    vec![Token::new(
                        TokenKind::Symbol(string_table.intern("trait_body")),
                        provisional_location.clone(),
                    )],
                ),
                source_file: provisional_source.clone(),
                capacity_references: Vec::new(),
            },
        ],
        top_level_const_fragments: vec![TopLevelConstFragment {
            runtime_insertion_index: 0,
            header_path: provisional_source.clone(),
            location: provisional_location.clone(),
        }],
        const_template_count: 1,
        runtime_fragment_count: 0,
        has_non_trivial_root_body: false,
        warnings: vec![warning],
    };

    let final_os_path = PathBuf::from("/project/src/main.moth");
    output
        .rebind_source_identity(FileId(42), final_source.clone(), final_os_path.clone())
        .expect("complete retained output should rebind atomically");
    output
        .freeze_path_syntax(&string_table)
        .expect("the fully rebound retained output should satisfy the file invariant gate");

    assert_eq!(output.source_file, final_source);
    assert_eq!(output.file_id, Some(FileId(42)));
    assert_eq!(output.canonical_os_path, Some(final_os_path.clone()));

    for header in &output.headers {
        assert_eq!(header.source_file, output.source_file);
        assert_eq!(header.tokens.src_path, output.source_file);
        assert_eq!(header.tokens.file_id, Some(FileId(42)));
        assert_eq!(header.tokens.canonical_os_path, Some(final_os_path.clone()));
        assert_eq!(header.name_location.scope, output.source_file);
        assert!(
            header
                .tokens
                .tokens
                .iter()
                .all(|token| token.location.scope == output.source_file)
        );
    }

    let HeaderKind::Function {
        generic_parameters,
        signature,
    } = &output.headers[0].kind
    else {
        panic!("expected function header");
    };
    assert_eq!(
        generic_parameters.parameters[0].location.scope,
        output.source_file
    );
    assert_eq!(
        signature.parameters[0].id,
        output.source_file.append(parameter_name)
    );
    assert_eq!(signature.parameters[0].location.scope, output.source_file);
    let ParsedTypeRef::Named { location, .. } = &signature.parameters[0].type_annotation else {
        panic!("expected named parameter type");
    };
    assert_eq!(location.scope, output.source_file);
    assert_eq!(
        signature.parameters[0].default_tokens[0].location.scope,
        output.source_file
    );
    assert_eq!(
        signature.returns[0].value.location.scope,
        output.source_file
    );

    let HeaderKind::Constant { declaration } = &output.headers[1].kind else {
        panic!("expected constant header");
    };
    assert_eq!(declaration.location.scope, output.source_file);
    assert_eq!(
        declaration.initializer_tokens[0].location.scope,
        output.source_file
    );
    assert_eq!(
        declaration.initializer_references[0].location.scope,
        output.source_file
    );

    let HeaderKind::Trait { declaration } = &output.headers[2].kind else {
        panic!("expected trait header");
    };
    assert_eq!(declaration.location.scope, output.source_file);
    assert_eq!(
        declaration.requirements[0].name_location.scope,
        output.source_file
    );
    assert_eq!(
        declaration.requirements[0].signature.parameters[0].id,
        output.source_file.append(string_table.intern("This"))
    );
    assert_eq!(
        declaration.requirements[0].signature.parameters[0]
            .location
            .scope,
        output.source_file
    );

    assert_eq!(
        output.top_level_const_fragments[0].header_path,
        output.source_file
    );
    assert_eq!(
        output.top_level_const_fragments[0].location.scope,
        output.source_file
    );
    assert!(
        output.headers[0]
            .local_ordering_hints
            .iter()
            .all(|hint| hint.path().starts_with(&output.source_file))
    );

    let warning = &output.warnings[0];
    assert_eq!(warning.primary_location.scope, output.source_file);
    assert!(
        warning
            .labels
            .iter()
            .all(|label| label.location.scope == output.source_file)
    );
    let DiagnosticPayload::ImportNameCollision {
        previous_location: Some(previous_location),
        ..
    } = &warning.payload
    else {
        panic!("expected collision payload with previous location");
    };
    assert_eq!(previous_location.scope, output.source_file);

    let clause = &output.file_dependency_clauses[0];
    assert_eq!(
        clause.dependency.dependency_shell_id,
        DependencyShellId::new(FileId(42), 0)
    );
    assert_eq!(clause.location.scope, output.source_file);
    assert_eq!(clause.dependency.path, provider_path);
    assert_eq!(
        clause.dependency.path.to_portable_string(&string_table),
        "provider"
    );
    assert_eq!(clause.dependency.location.scope, output.source_file);
}

#[test]
fn rebased_prepared_shell_joins_one_provider_interface() {
    let mut string_table = StringTable::new();
    let provisional_source = InternedPath::from_single_str("src/main.moth", &mut string_table);
    let final_source = InternedPath::from_single_str("logical/main.moth", &mut string_table);
    let location = make_location("src/main.moth", &mut string_table);
    let provider = RetainedDependencyPath {
        path: InternedPath::from_single_str("provider", &mut string_table),
        path_syntax: crate::compiler_frontend::paths::path_syntax::PathSyntaxId::NONE,
        target: crate::compiler_frontend::headers::dependency_target::DependencyTargetKind::Source,
        location: location.clone(),
        dependency_shell_id: DependencyShellId::new(FileId(3), 0),
    };
    let mut output = FileFrontendPrepareOutput {
        source_file: provisional_source,
        file_id: Some(FileId(3)),
        path_syntax: PreparedFilePathSyntax::empty(),
        token_count: 0,
        token_stats: TokenStats::default(),
        file_role: FileRole::Normal,
        file_dependency_clauses: vec![RetainedDependencyClause {
            dependency: provider.clone(),
            binding: DependencyBindingSyntax::Namespace { alias: None },
            location,
            export_mode: HeaderExportMode::Private,
        }],
        structural_file_references: Default::default(),
        dependency_selections: Vec::new(),
        canonical_os_path: Some(PathBuf::from("/provisional/src/main.moth")),
        headers: Vec::new(),
        top_level_const_fragments: Vec::new(),
        const_template_count: 0,
        runtime_fragment_count: 0,
        has_non_trivial_root_body: false,
        warnings: Vec::new(),
    };

    output
        .rebind_source_identity(
            FileId(19),
            final_source.clone(),
            PathBuf::from("/project/src/main.moth"),
        )
        .expect("complete retained output should rebind atomically");
    let rebound = &output.file_dependency_clauses[0].dependency;
    let shell = rebound.dependency_shell_id;
    assert_eq!(shell, DependencyShellId::new(FileId(19), 0));
    assert_eq!(rebound.location.scope, output.source_file);
    assert_eq!(output.source_file, final_source);
    assert_eq!(rebound.path.to_portable_string(&string_table), "provider");

    let provider = PublicSemanticInterface {
        module_origin: StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("rebased-provider"),
            "rebased-provider/@mod.moth".to_owned(),
            ModuleRootRole::Normal,
        ),
        export_bindings: Vec::new(),
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: Vec::new(),
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    };
    let provider_dependencies = SourceProviderDependencySet::new(vec![SourceProviderDependency {
        kind: ProviderDependencyKind::Authored { shell },
        interface: &provider,
    }])
    .expect("the rebased shell should register exactly one provider interface");
    let resolved = provider_dependencies
        .resolve_clause(shell)
        .expect("the rebased shell should join its provider interface");
    assert!(std::ptr::eq(
        provider_dependencies
            .interface(resolved.provider)
            .expect("the joined provider interface should resolve"),
        &provider
    ));
}

#[test]
fn file_frontend_prepare_output_remaps_flat_dependency_selections() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();
    global.intern("preexisting");

    let source_file = InternedPath::from_single_str("src/main.moth", &mut local);
    let dependency_location = make_location("src/main.moth", &mut local);
    let provider = RetainedDependencyPath {
        path: InternedPath::from_single_str("provider", &mut local),
        path_syntax: crate::compiler_frontend::paths::path_syntax::PathSyntaxId::NONE,
        target: crate::compiler_frontend::headers::dependency_target::DependencyTargetKind::Source,
        location: dependency_location.clone(),
        dependency_shell_id: DependencyShellId::new(FileId(0), 0),
    };
    let dependency = RetainedDependencyClause {
        dependency: provider.clone(),
        binding: DependencyBindingSyntax::DirectSelections {
            range: DependencySelectionRange::new(0, 1),
        },
        location: dependency_location.clone(),
        export_mode: HeaderExportMode::Private,
    };
    let mut output = FileFrontendPrepareOutput {
        source_file,
        file_id: None,
        path_syntax: PreparedFilePathSyntax::empty(),
        token_count: 0,
        token_stats: TokenStats::default(),
        file_role: FileRole::Normal,
        file_dependency_clauses: vec![dependency],
        structural_file_references: Default::default(),
        dependency_selections: vec![DependencySelection {
            source_name: local.intern("source"),
            source_location: dependency_location.clone(),
            local_alias: Some(DependencyAlias {
                name: local.intern("local"),
                location: dependency_location,
            }),
        }],
        canonical_os_path: None,
        headers: Vec::new(),
        top_level_const_fragments: Vec::new(),
        const_template_count: 0,
        runtime_fragment_count: 0,
        has_non_trivial_root_body: false,
        warnings: Vec::new(),
    };

    let remap = global.merge_from(&local);
    output
        .remap_string_ids(&remap)
        .expect("flat dependency selections should remap with their file output");

    let dependency = &output.file_dependency_clauses[0];
    let selections = dependency
        .selections(&output.dependency_selections)
        .expect("selection range should remain valid after remapping");
    assert_eq!(selections.len(), 1);
    assert_eq!(global.resolve(selections[0].source_name), "source");
    assert_eq!(global.resolve(selections[0].local_name()), "local");
    assert_location_resolves_to(&selections[0].source_location, "src/main.moth", &global);
    assert_location_resolves_to(
        selections[0]
            .local_alias()
            .map(|alias| &alias.location)
            .expect("selected alias should retain its location"),
        "src/main.moth",
        &global,
    );
    assert_eq!(
        dependency
            .selection_id(&output.dependency_selections, 0)
            .expect("selection identity should be valid")
            .selected_index,
        0
    );
}

// ----------------------------------------------------------------
//  Prepared-file invariant and source-identity boundary tests
// ----------------------------------------------------------------

#[test]
fn prepared_file_invariant_anchors_header_identity_to_file_id() {
    let mut string_table = StringTable::new();
    let source_file = InternedPath::from_single_str("logical/main.moth", &mut string_table);
    let canonical_os_path = PathBuf::from("/project/src/main.moth");
    let header = make_prepared_header(&source_file, FileId(7), &canonical_os_path, Vec::new());
    let mut output = make_prepared_output(source_file, FileId(6), canonical_os_path, vec![header]);

    let error = output
        .freeze_path_syntax(&string_table)
        .expect_err("a header stream with another file identity is malformed retained state");
    assert!(
        error
            .msg
            .contains("header token stream does not match the prepared file identity"),
        "unexpected invariant error: {error:?}"
    );
}

#[test]
fn prepared_file_invariant_anchors_clause_shell_identity_to_file_id() {
    let mut string_table = StringTable::new();
    let source_file = InternedPath::from_single_str("logical/main.moth", &mut string_table);
    let location = SourceLocation::new(
        source_file.clone(),
        CharPosition::default(),
        CharPosition::default(),
    );
    let provider = RetainedDependencyPath {
        path: InternedPath::from_single_str("@core/math", &mut string_table),
        path_syntax: crate::compiler_frontend::paths::path_syntax::PathSyntaxId::NONE,
        target: crate::compiler_frontend::headers::dependency_target::DependencyTargetKind::Source,
        location: location.clone(),
        dependency_shell_id: DependencyShellId::new(FileId(7), 0),
    };
    let mut output = make_prepared_output(
        source_file,
        FileId(6),
        PathBuf::from("/project/src/main.moth"),
        Vec::new(),
    );
    output
        .file_dependency_clauses
        .push(RetainedDependencyClause {
            dependency: provider.clone(),
            binding: DependencyBindingSyntax::Namespace { alias: None },
            location,
            export_mode: HeaderExportMode::Private,
        });

    let error = output
        .freeze_path_syntax(&string_table)
        .expect_err("a clause shell with another file identity is malformed retained state");
    assert!(
        error
            .msg
            .contains("shell identity does not match the prepared file identity"),
        "unexpected invariant error: {error:?}"
    );
}

#[test]
fn prepared_file_invariant_rejects_duplicate_clause_shell_ordinals() {
    let mut string_table = StringTable::new();
    let source_file = InternedPath::from_single_str("logical/main.moth", &mut string_table);
    let location = SourceLocation::new(
        source_file.clone(),
        CharPosition::default(),
        CharPosition::default(),
    );
    let provider_path = InternedPath::from_single_str("@core/math", &mut string_table);
    let provider = |ordinal| RetainedDependencyPath {
        path: provider_path.clone(),
        path_syntax: crate::compiler_frontend::paths::path_syntax::PathSyntaxId::NONE,
        target: crate::compiler_frontend::headers::dependency_target::DependencyTargetKind::Source,
        location: location.clone(),
        dependency_shell_id: DependencyShellId::new(FileId(6), ordinal),
    };
    let clause = |provider: RetainedDependencyPath| RetainedDependencyClause {
        dependency: provider.clone(),
        binding: DependencyBindingSyntax::Namespace { alias: None },
        location: location.clone(),
        export_mode: HeaderExportMode::Private,
    };
    let mut output = make_prepared_output(
        source_file,
        FileId(6),
        PathBuf::from("/project/src/main.moth"),
        Vec::new(),
    );
    output.file_dependency_clauses = vec![clause(provider(0)), clause(provider(0))];

    let error = output
        .freeze_path_syntax(&string_table)
        .expect_err("duplicate shell ordinals must fail at the total invariant boundary");
    assert!(
        error.msg.contains("dense file-local clause position"),
        "unexpected invariant error: {error:?}"
    );
}

#[test]
fn prepared_file_invariant_rejects_stale_path_handles() {
    let mut string_table = StringTable::new();
    let source_file = InternedPath::from_single_str("logical/main.moth", &mut string_table);
    let canonical_os_path = PathBuf::from("/project/src/main.moth");
    let location = SourceLocation::new(
        source_file.clone(),
        CharPosition::default(),
        CharPosition::default(),
    );
    let header = make_prepared_header(
        &source_file,
        FileId(6),
        &canonical_os_path,
        vec![Token::new(TokenKind::Path(PathSyntaxId::NONE), location)],
    );
    let mut output = make_prepared_output(source_file, FileId(6), canonical_os_path, vec![header]);

    let error = output
        .freeze_path_syntax(&string_table)
        .expect_err("an absent path handle is malformed retained state");
    assert!(
        error.msg.contains("absent PathSyntaxId marker"),
        "unexpected invariant error: {error:?}"
    );
}

#[test]
fn prepared_file_invariant_rejects_unclaimed_dependency_selection_rows() {
    let mut string_table = StringTable::new();
    let source_file = InternedPath::from_single_str("logical/main.moth", &mut string_table);
    let location = SourceLocation::new(
        source_file.clone(),
        CharPosition::default(),
        CharPosition::default(),
    );
    let mut output = make_prepared_output(
        source_file,
        FileId(6),
        PathBuf::from("/project/src/main.moth"),
        Vec::new(),
    );
    output.dependency_selections.push(DependencySelection {
        source_name: string_table.intern("sin"),
        source_location: location,
        local_alias: None,
    });

    let error = output
        .freeze_path_syntax(&string_table)
        .expect_err("a selection row without an owning direct clause is malformed retained state");
    assert!(
        error
            .msg
            .contains("selection table contains unclaimed rows"),
        "unexpected invariant error: {error:?}"
    );
}

#[test]
fn prepared_file_invariant_rejects_malformed_provider_prefix_count() {
    let mut string_table = StringTable::new();
    let source_file = InternedPath::from_single_str("logical/main.moth", &mut string_table);
    let location = SourceLocation::new(
        source_file.clone(),
        CharPosition::default(),
        CharPosition::default(),
    );
    let mut output = make_prepared_output(
        source_file,
        FileId(6),
        PathBuf::from("/project/src/main.moth"),
        Vec::new(),
    );
    output.file_dependency_clauses.push(RetainedDependencyClause {
        dependency: RetainedDependencyPath {
            path: InternedPath::from_single_str("drawing.js", &mut string_table),
            path_syntax: crate::compiler_frontend::paths::path_syntax::PathSyntaxId::NONE,
            target: crate::compiler_frontend::headers::dependency_target::DependencyTargetKind::ExternalProvider {
                prefix_component_count: 4,
                extension: string_table.intern("js"),
            },
            location: location.clone(),
            dependency_shell_id: DependencyShellId::new(FileId(6), 0),
        },
        binding: DependencyBindingSyntax::Namespace { alias: None },
        location,
        export_mode: HeaderExportMode::Private,
    });

    let error = output
        .freeze_path_syntax(&string_table)
        .expect_err("a prefix count outside the path is malformed retained state");
    assert!(
        error.msg.contains("outside the path"),
        "unexpected invariant error: {error:?}"
    );
}

#[test]
fn prepared_file_invariant_rejects_empty_retained_dependency_path() {
    let mut string_table = StringTable::new();
    let source_file = InternedPath::from_single_str("logical/main.moth", &mut string_table);
    let location = SourceLocation::new(
        source_file.clone(),
        CharPosition::default(),
        CharPosition::default(),
    );
    let mut output = make_prepared_output(
        source_file,
        FileId(6),
        PathBuf::from("/project/src/main.moth"),
        Vec::new(),
    );
    output
        .file_dependency_clauses
        .push(RetainedDependencyClause {
        dependency: RetainedDependencyPath {
            path: InternedPath::new(),
            path_syntax: crate::compiler_frontend::paths::path_syntax::PathSyntaxId::NONE,
            target:
                crate::compiler_frontend::headers::dependency_target::DependencyTargetKind::Source,
            location: location.clone(),
            dependency_shell_id: DependencyShellId::new(FileId(6), 0),
        },
        binding: DependencyBindingSyntax::Namespace { alias: None },
        location,
        export_mode: HeaderExportMode::Private,
    });

    let error = output
        .freeze_path_syntax(&string_table)
        .expect_err("an empty retained dependency path is malformed retained state");
    assert!(
        error.msg.contains("empty path"),
        "unexpected invariant error: {error:?}"
    );
}

#[test]
fn prepared_file_rebinding_preflights_required_paths_without_partial_mutation() {
    let mut string_table = StringTable::new();
    let provisional_source = InternedPath::from_single_str("src/main.moth", &mut string_table);
    let final_source = InternedPath::from_single_str("logical/main.moth", &mut string_table);
    let provisional_os_path = PathBuf::from("/provisional/src/main.moth");
    let final_os_path = PathBuf::from("/project/src/main.moth");
    let malformed_header_path = InternedPath::from_single_str("orphan", &mut string_table);
    let header = Header {
        kind: HeaderKind::StartFunction,
        file_role: FileRole::Normal,
        export_mode: HeaderExportMode::Private,
        local_ordering_hints: HashSet::new(),
        name_location: SourceLocation::new(
            provisional_source.clone(),
            CharPosition::default(),
            CharPosition::default(),
        ),
        tokens: FileTokens::new_deferred_with_identity(
            malformed_header_path.clone(),
            Some(FileId(6)),
            Some(provisional_os_path.clone()),
            Vec::new(),
        ),
        source_file: provisional_source.clone(),
        capacity_references: Vec::new(),
    };
    let mut output = make_prepared_output(
        provisional_source.clone(),
        FileId(6),
        provisional_os_path.clone(),
        vec![header],
    );

    let error = output
        .rebind_source_identity(FileId(9), final_source, final_os_path)
        .expect_err("a source-owned header path must carry the provisional source prefix");
    assert!(
        error.msg.contains("missing its provisional source prefix"),
        "unexpected rebind error: {error:?}"
    );
    assert_eq!(output.source_file, provisional_source);
    assert_eq!(output.file_id, Some(FileId(6)));
    assert_eq!(output.canonical_os_path, Some(provisional_os_path));
    assert_eq!(output.headers[0].source_file, output.source_file);
    assert_eq!(output.headers[0].tokens.src_path, malformed_header_path);
}

#[test]
fn prepared_file_rebinding_keeps_provider_spelling_prefix_free() {
    let mut string_table = StringTable::new();
    let provisional_source = InternedPath::from_single_str("src/main.moth", &mut string_table);
    let final_source = InternedPath::from_single_str("logical/main.moth", &mut string_table);
    let provisional_os_path = PathBuf::from("/provisional/src/main.moth");
    let final_os_path = PathBuf::from("/project/src/main.moth");
    let provider_spelling = InternedPath::from_single_str("@core/math", &mut string_table);
    let mut header = make_prepared_header(
        &provisional_source,
        FileId(6),
        &provisional_os_path,
        Vec::new(),
    );
    header
        .local_ordering_hints
        .insert(LocalDeclarationOrderingHint::provider_spelling(
            provider_spelling.clone(),
        ));
    let mut output = make_prepared_output(
        provisional_source,
        FileId(6),
        provisional_os_path,
        vec![header],
    );

    output
        .rebind_source_identity(FileId(9), final_source, final_os_path)
        .expect("provider spellings intentionally remain outside the source prefix");
    assert_eq!(
        output.headers[0]
            .local_ordering_hints
            .iter()
            .next()
            .expect("provider hint should be retained")
            .path(),
        &provider_spelling
    );
}

// -----------------------------------------------------------
//  FileFrontendPrepareError remapping tests
// -----------------------------------------------------------

#[test]
fn file_frontend_prepare_error_remaps_warnings_and_diagnostic() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    // Preexisting strings in global ensure the merge is non-identity.
    global.intern("preexisting_x");
    global.intern("preexisting_y");

    let warning_a = make_unknown_name_diagnostic("warn_a", &mut local);
    let warning_b = make_unknown_name_diagnostic("warn_b", &mut local);
    let diagnostic = make_unknown_name_diagnostic("error_name", &mut local);

    let mut error = FileFrontendPrepareError {
        warnings: vec![warning_a, warning_b],
        diagnostic: Box::new(diagnostic),
    };

    let remap = global.merge_from(&local);
    error.remap_string_ids(&remap);

    // Warnings remapped.
    assert_eq!(error.warnings.len(), 2);

    let DiagnosticPayload::UnknownName { name: name_a, .. } = &error.warnings[0].payload else {
        panic!("expected UnknownName payload");
    };
    assert_eq!(global.resolve(*name_a), "warn_a");
    assert_location_resolves_to(&error.warnings[0].primary_location, "test.moth", &global);

    let DiagnosticPayload::UnknownName { name: name_b, .. } = &error.warnings[1].payload else {
        panic!("expected UnknownName payload");
    };
    assert_eq!(global.resolve(*name_b), "warn_b");
    assert_location_resolves_to(&error.warnings[1].primary_location, "test.moth", &global);

    // Primary diagnostic remapped.
    let DiagnosticPayload::UnknownName {
        name: error_name, ..
    } = &error.diagnostic.payload
    else {
        panic!("expected UnknownName payload");
    };
    assert_eq!(global.resolve(*error_name), "error_name");
    assert_location_resolves_to(&error.diagnostic.primary_location, "test.moth", &global);
}

#[test]
fn file_frontend_prepare_error_identity_remap_preserves_payload() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let warning = make_unknown_name_diagnostic("warn_name", &mut local);
    let diagnostic = make_unknown_name_diagnostic("error_name", &mut local);

    let mut error = FileFrontendPrepareError {
        warnings: vec![warning],
        diagnostic: Box::new(diagnostic),
    };

    let remap = global.merge_from(&local);
    assert!(remap.is_identity());

    error.remap_string_ids(&remap);

    let DiagnosticPayload::UnknownName { name, .. } = &error.warnings[0].payload else {
        panic!("expected UnknownName payload");
    };
    assert_eq!(global.resolve(*name), "warn_name");
    assert_location_resolves_to(&error.warnings[0].primary_location, "test.moth", &global);

    let DiagnosticPayload::UnknownName {
        name: error_name, ..
    } = &error.diagnostic.payload
    else {
        panic!("expected UnknownName payload");
    };
    assert_eq!(global.resolve(*error_name), "error_name");
    assert_location_resolves_to(&error.diagnostic.primary_location, "test.moth", &global);
}
