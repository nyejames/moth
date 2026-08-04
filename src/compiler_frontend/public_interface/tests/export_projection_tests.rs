//! Focused hidden-invariant tests for the pre-AST direct-export seed and the transient public
//! source-nominal and source-trait origin indexes.
//!
//! WHAT: exercises the invariants of [`DirectExportSeed`] and the origin-index builders that
//! integration output cannot inspect: direct export bindings cover exactly the public
//! declarations authored in the active module root, category distinctions are exact, private
//! declarations and the implicit start function are excluded, ordering is deterministic
//! independent of declaration scheduling, and the active root origin is validated from the
//! per-file source-origin table even when the public surface is empty. The source-nominal and
//! source-trait origin indexes admit direct, imported-provider and alias-targeted declarations
//! while excluding private and unowned declarations, and reject missing `FileId` failures.
//! WHY: these are construction invariants owned by `compiler_frontend::public_interface::export_projection`,
//! so they own a focused test beside the module rather than an end-to-end case.

use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::module_symbols::{
    ModuleSymbols, PublicExportEntry, PublicExportTarget,
};
use crate::compiler_frontend::headers::parse_file_headers::parse_file_headers_tests::parse_single_file_headers_with_table;
use crate::compiler_frontend::headers::parse_file_headers::parse_file_headers_tests::prepare_single_file;
use crate::compiler_frontend::headers::parse_file_headers::{FileRole, Header, HeaderKind};
use crate::compiler_frontend::public_interface::{
    DirectExportSeed, PublicDiagnosticLocation, PublicExportDiagnosticProvenance,
    PublicSemanticInterface, SourceProviderImport, SourceProviderImportSet,
    build_direct_export_seed, build_public_source_nominal_origin_index,
    build_public_source_trait_origin_index,
};
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, FunctionOriginKind, ModuleRootRole, OriginConstantId, OriginDeclarationId,
    OriginTraitId, OriginTypeCategory, OriginTypeId, StableModuleOriginIdentity,
    StablePackageIdentity,
};
use crate::compiler_frontend::source_module_origin::SourceModuleOriginTable;
use crate::compiler_frontend::symbols::identity::{FileId, ImportShellId, SourceFileTable};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use std::path::PathBuf;

/// Build the direct-export seed for one active-root source using a deterministic synthetic
/// module origin.
///
/// The receiver-method catalog defaults to empty, which exercises the free-binding projection
/// and confirms a module with no receiver methods records no receiver surfaces.
fn build_seed(source: &str) -> DirectExportSeed {
    build_seed_for_project(source, "test-project")
}

/// Build the direct-export seed for one active-root source using a configurable project name so
/// module distinction is testable without a second discovered module.
fn build_seed_for_project(source: &str, project_name: &str) -> DirectExportSeed {
    let (mut headers, mut string_table) = parse_single_file_headers_with_table(source);
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local(project_name),
        String::new(),
        ModuleRootRole::Normal,
    );

    // Build a source file table for the single synthetic test file and set the retained
    // file identity on every header so the origin projection can resolve the active root
    // from the per-file source-origin table.
    let file_path = PathBuf::from("src/@page.moth");
    let source_files = SourceFileTable::build(
        std::iter::once(file_path.clone()),
        &file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build for the synthetic test file");
    let file_id = source_files
        .get_by_canonical_path(&file_path)
        .expect("the synthetic test file should be in the source file table")
        .file_id;
    for header in &mut headers.headers {
        header.tokens.file_id = Some(file_id);
    }

    let source_module_origins =
        SourceModuleOriginTable::from_synthetic_origin(&source_files, &module_origin);

    build_direct_export_seed(
        &source_module_origins,
        file_id,
        &headers.headers,
        &headers.module_symbols,
        &SourceProviderImportSet::default(),
        &ExternalPackageRegistry::default(),
        &string_table,
    )
    .expect("the direct export seed must build for valid headers")
}

struct ExportProjectionFixture {
    headers: Vec<Header>,
    module_symbols: ModuleSymbols,
    source_module_origins: SourceModuleOriginTable,
    string_table: StringTable,
    active_root_file_id: FileId,
    module_root: InternedPath,
    module_origin: StableModuleOriginIdentity,
}

/// Prepare a small multi-file projection fixture while retaining the header-owned membership
/// facts that `collect_reexport_bindings` consumes. The focused re-export tests intentionally
/// model the already-bound public export map so they exercise semantic projection rather than
/// duplicating path-resolver setup owned by header binding.
fn build_reexport_fixture(sources: &[(&str, &str)], project_name: &str) -> ExportProjectionFixture {
    let active_path = PathBuf::from(
        sources
            .first()
            .expect("a projection fixture needs an active source")
            .0,
    );
    let mut string_table = StringTable::new();
    let mut prepared_outputs = Vec::with_capacity(sources.len());
    let mut canonical_paths = Vec::with_capacity(sources.len());

    for (path, source) in sources {
        let path = PathBuf::from(path);
        prepared_outputs.push(prepare_single_file(
            source,
            &path,
            &active_path,
            &mut string_table,
        ));
        canonical_paths.push(path);
    }

    let source_files = SourceFileTable::build(
        canonical_paths.iter(),
        &active_path,
        None,
        &mut string_table,
    )
    .expect("projection source files should build");
    let active_root_file_id = source_files
        .get_by_canonical_path(&active_path)
        .expect("the projection active root should have a file identity")
        .file_id;
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local(project_name),
        "active".to_owned(),
        ModuleRootRole::Normal,
    );
    let source_module_origins =
        SourceModuleOriginTable::from_synthetic_origin(&source_files, &module_origin);
    let module_root = InternedPath::from_single_str("module-root", &mut string_table);

    let mut headers = Vec::new();
    let mut module_symbols = ModuleSymbols::empty();
    for (output, path) in prepared_outputs.into_iter().zip(canonical_paths.iter()) {
        let source_file = output.source_file.clone();
        module_symbols
            .file_roles_by_source
            .insert(source_file.clone(), output.file_role);
        module_symbols
            .file_module_membership
            .insert(source_file.clone(), module_root.clone());

        let file_id = source_files
            .get_by_canonical_path(path)
            .expect("every prepared projection source should have a file identity")
            .file_id;
        for mut header in output.headers {
            header.tokens.file_id = Some(file_id);
            headers.push(header);
        }
    }

    ExportProjectionFixture {
        headers,
        module_symbols,
        source_module_origins,
        string_table,
        active_root_file_id,
        module_root,
        module_origin,
    }
}

fn location_scope_components(
    location: &crate::compiler_frontend::tokenizer::tokens::SourceLocation,
    string_table: &StringTable,
) -> Vec<String> {
    location
        .scope
        .as_components()
        .iter()
        .map(|component| string_table.resolve(*component).to_owned())
        .collect()
}

fn binding_for<'a>(seed: &'a DirectExportSeed, public_name: &str) -> &'a ExportBinding {
    seed.export_bindings()
        .iter()
        .find(|binding| binding.public_name() == public_name)
        .unwrap_or_else(|| panic!("no export binding for public name `{public_name}`"))
}

fn binding_names(seed: &DirectExportSeed) -> Vec<&str> {
    seed.export_bindings()
        .iter()
        .map(|binding| binding.public_name())
        .collect()
}

/// A public surface exercising every directly-defined public declaration category.
const ALL_CATEGORIES_SOURCE: &str = "\
export:\n\
    render |button Button| -> String:\n\
        return button.label\n\
    ;\n\
    Button = | label String |\n\
    Status :: Ready,\n\
    ;\n\
    Shape as Int\n\
    count #= 1\n\
    DISPLAYABLE must:\n\
        show |This| -> String\n\
    ;\n\
;\n";

#[test]
fn directly_defined_public_exports_get_export_bindings_with_exact_category() {
    let seed = build_seed(ALL_CATEGORIES_SOURCE);

    // Every public declaration category is admitted with its exact origin category.
    assert!(
        matches!(
            binding_for(&seed, "render").origin(),
            OriginDeclarationId::Function(function)
                if matches!(function.kind(), FunctionOriginKind::Free)
        ),
        "a public free function must produce a free-function origin"
    );
    assert!(
        matches!(
            binding_for(&seed, "Button").origin(),
            OriginDeclarationId::Type(type_id)
                if type_id.category() == OriginTypeCategory::Struct
        ),
        "a public struct must produce a struct type origin"
    );
    assert!(
        matches!(
            binding_for(&seed, "Status").origin(),
            OriginDeclarationId::Type(type_id)
                if type_id.category() == OriginTypeCategory::Choice
        ),
        "a public choice must produce a choice type origin"
    );
    assert!(
        matches!(
            binding_for(&seed, "Shape").origin(),
            OriginDeclarationId::Type(type_id)
                if type_id.category() == OriginTypeCategory::TransparentAlias
        ),
        "a public transparent alias must produce a transparent-alias type origin"
    );
    assert!(
        matches!(
            binding_for(&seed, "count").origin(),
            OriginDeclarationId::Constant(_)
        ),
        "a public constant must produce a constant origin"
    );
    assert!(
        matches!(
            binding_for(&seed, "DISPLAYABLE").origin(),
            OriginDeclarationId::Trait(_)
        ),
        "a public trait must produce a trait origin"
    );
}

#[test]
fn directly_defined_public_exports_retain_authored_diagnostic_provenance() {
    let seed = build_seed("export:\n    alpha #= 1\n;\n");

    assert_eq!(seed.export_diagnostic_provenance().len(), 1);
    let provenance = &seed.export_diagnostic_provenance()[0];
    assert_eq!(provenance.public_name, "alpha");
    assert_eq!(provenance.location.start_line, 1);
    assert_eq!(provenance.location.start_column, 5);
    assert_eq!(provenance.location.end_line, 1);
    assert!(provenance.location.end_column > provenance.location.start_column);
}

#[test]
fn same_module_reexport_preserves_alias_origin_and_authored_provenance() {
    let mut fixture = build_reexport_fixture(
        &[
            ("src/@page.moth", "placeholder #= 1\n"),
            ("src/impl.moth", "value #= 1\n"),
        ],
        "same-module-reexport",
    );
    let target_header = fixture
        .headers
        .iter()
        .find(|header| header.tokens.src_path.name_str(&fixture.string_table) == Some("value"))
        .expect("the private re-export target should have a header");
    let target_path = target_header.tokens.src_path.clone();
    let target_source = target_header.source_file.clone();
    let expected_location = target_header.name_location.clone();

    fixture
        .module_symbols
        .canonical_source_by_symbol_path
        .insert(target_path.clone(), target_source);
    fixture.module_symbols.module_root_public_exports.insert(
        fixture.module_root.clone(),
        [PublicExportEntry {
            export_name: fixture.string_table.intern("PublicValue"),
            target: PublicExportTarget::Source {
                path: target_path,
                import_shell_id: None,
            },
        }]
        .into_iter()
        .collect(),
    );

    let seed = build_direct_export_seed(
        &fixture.source_module_origins,
        fixture.active_root_file_id,
        &fixture.headers,
        &fixture.module_symbols,
        &SourceProviderImportSet::default(),
        &ExternalPackageRegistry::default(),
        &fixture.string_table,
    )
    .expect("same-module re-export projection should succeed");

    let binding = binding_for(&seed, "PublicValue");
    assert_eq!(
        binding.origin(),
        &OriginDeclarationId::Constant(OriginConstantId::new(
            fixture.module_origin.clone(),
            "value".to_owned(),
        ))
    );
    let provenance = seed
        .export_diagnostic_provenance()
        .iter()
        .find(|entry| entry.public_name == "PublicValue")
        .expect("same-module re-export should retain target provenance");
    assert_eq!(
        provenance.location.scope_components,
        location_scope_components(&expected_location, &fixture.string_table)
    );
    assert_eq!(
        (
            provenance.location.start_line,
            provenance.location.start_column,
            provenance.location.end_line,
            provenance.location.end_column,
        ),
        (
            expected_location.start_pos.line_number,
            expected_location.start_pos.char_column,
            expected_location.end_pos.line_number,
            expected_location.end_pos.char_column,
        )
    );
}

#[test]
fn provider_reexport_preserves_alias_and_provider_provenance() {
    let mut fixture = build_reexport_fixture(
        &[("src/@page.moth", "placeholder #= 1\n")],
        "provider-reexport",
    );
    let target_path = InternedPath::from_single_str("provider", &mut fixture.string_table)
        .join_str("Imported", &mut fixture.string_table);
    let provider_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("provider"),
        "provider/@mod.moth".to_owned(),
        ModuleRootRole::Normal,
    );
    let provider_constant_origin = OriginDeclarationId::Constant(OriginConstantId::new(
        provider_origin.clone(),
        "Imported".to_owned(),
    ));
    let provider_interface = PublicSemanticInterface {
        module_origin: provider_origin.clone(),
        export_bindings: vec![ExportBinding::new(
            provider_origin,
            "Imported".to_owned(),
            provider_constant_origin.clone(),
        )],
        export_diagnostic_provenance: vec![PublicExportDiagnosticProvenance {
            public_name: "Imported".to_owned(),
            location: PublicDiagnosticLocation {
                scope_components: vec!["provider".to_owned(), "@mod.moth".to_owned()],
                start_line: 20,
                start_column: 4,
                end_line: 20,
                end_column: 12,
            },
        }],
        binding_exports: Vec::new(),
        declarations: Vec::new(),
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    };
    let provider_imports = SourceProviderImportSet::new(vec![SourceProviderImport {
        kind: crate::compiler_frontend::public_interface::ProviderImportKind::Authored {
            shell_id: ImportShellId::new(FileId(0), 0),
        },
        interface: &provider_interface,
    }])
    .expect("one authored provider should register");
    fixture.module_symbols.module_root_public_exports.insert(
        fixture.module_root.clone(),
        [PublicExportEntry {
            export_name: fixture.string_table.intern("PublicImported"),
            target: PublicExportTarget::Source {
                path: target_path,
                import_shell_id: Some(ImportShellId::new(FileId(0), 0)),
            },
        }]
        .into_iter()
        .collect(),
    );

    let seed = build_direct_export_seed(
        &fixture.source_module_origins,
        fixture.active_root_file_id,
        &fixture.headers,
        &fixture.module_symbols,
        &provider_imports,
        &ExternalPackageRegistry::default(),
        &fixture.string_table,
    )
    .expect("provider re-export projection should succeed");

    let binding = binding_for(&seed, "PublicImported");
    assert_eq!(binding.origin(), &provider_constant_origin);
    let provenance = seed
        .export_diagnostic_provenance()
        .iter()
        .find(|entry| entry.public_name == "PublicImported")
        .expect("provider re-export should retain provider provenance under the alias");
    assert_eq!(
        provenance.location.scope_components,
        vec!["provider".to_owned(), "@mod.moth".to_owned()]
    );
    assert_eq!(
        (
            provenance.location.start_line,
            provenance.location.start_column,
            provenance.location.end_line,
            provenance.location.end_column,
        ),
        (20, 4, 20, 12)
    );
}

#[test]
fn private_declarations_and_implicit_start_are_excluded() {
    // `helper` and `Inner` are private; the implicit start function is always present for an
    // active module root. Only the public `public_fn` must be recorded.
    let source = "\
helper |value Int| -> Int:\n\
    return value\n\
;\n\
Inner = | x Int |\n\
export:\n\
    public_fn |x Int| -> Int:\n\
        return x\n\
    ;\n\
;\n";

    let seed = build_seed(source);

    assert_eq!(
        binding_names(&seed),
        vec!["public_fn"],
        "private declarations and the implicit start function must be excluded"
    );
}

#[test]
fn ordering_is_deterministic_independent_of_declaration_scheduling() {
    let order_one = "\
export:\n\
    zebra #= 1\n\
    alpha #= 2\n\
;\n";
    let order_two = "\
export:\n\
    alpha #= 2\n\
    zebra #= 1\n\
;\n";

    let first = build_seed(order_one);
    let second = build_seed(order_two);

    assert_eq!(
        binding_names(&first),
        vec!["alpha", "zebra"],
        "export bindings must be sorted by public name"
    );
    assert_eq!(
        first.module_origin(),
        second.module_origin(),
        "module origin must be independent of declaration scheduling"
    );
    assert_eq!(
        first.export_bindings(),
        second.export_bindings(),
        "semantic export bindings must be independent of declaration scheduling"
    );
    assert_eq!(
        first.public_nominal_type_origins(),
        second.public_nominal_type_origins(),
        "public nominal origins must be independent of declaration scheduling"
    );
    assert_eq!(
        first
            .export_diagnostic_provenance()
            .iter()
            .map(|entry| entry.public_name.as_str())
            .collect::<Vec<_>>(),
        second
            .export_diagnostic_provenance()
            .iter()
            .map(|entry| entry.public_name.as_str())
            .collect::<Vec<_>>(),
        "diagnostic provenance names must remain deterministic"
    );
}

#[test]
fn active_origin_missing_from_table_fails_internally() {
    // Hidden invariant: when the active root's FileId maps to no owning module origin, the
    // projection must fail through CompilerError rather than silently using a fallback origin.
    let (mut headers, mut string_table) =
        parse_single_file_headers_with_table("export:\n    alpha #= 1\n;\n");

    let file_path = PathBuf::from("src/@page.moth");
    let source_files = SourceFileTable::build(
        std::iter::once(file_path.clone()),
        &file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let file_id = source_files
        .get_by_canonical_path(&file_path)
        .expect("file should be in source file table")
        .file_id;
    for header in &mut headers.headers {
        header.tokens.file_id = Some(file_id);
    }

    // Build a table where every file maps to None (simulating a source-package file outside the
    // project module graph that somehow became an active root).
    let empty_lookup = rustc_hash::FxHashMap::default();
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &empty_lookup);

    let result = build_direct_export_seed(
        &source_module_origins,
        file_id,
        &headers.headers,
        &headers.module_symbols,
        &SourceProviderImportSet::default(),
        &ExternalPackageRegistry::default(),
        &string_table,
    );

    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("an active root with no owning module origin must fail"),
    };
    assert!(
        error.msg.contains("no owning module origin"),
        "the error must state the missing-origin violation, got: {}",
        error.msg
    );
}

#[test]
fn out_of_range_active_root_file_id_fails_internally() {
    // Hidden invariant: an out-of-range FileId is an internal CompilerError, not a silent None.
    let (mut headers, mut string_table) =
        parse_single_file_headers_with_table("export:\n    alpha #= 1\n;\n");

    let file_path = PathBuf::from("src/@page.moth");
    let source_files = SourceFileTable::build(
        std::iter::once(file_path.clone()),
        &file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let file_id = source_files
        .get_by_canonical_path(&file_path)
        .expect("file should be in source file table")
        .file_id;
    for header in &mut headers.headers {
        header.tokens.file_id = Some(file_id);
    }

    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        String::new(),
        ModuleRootRole::Normal,
    );
    let source_module_origins =
        SourceModuleOriginTable::from_synthetic_origin(&source_files, &module_origin);

    let result = build_direct_export_seed(
        &source_module_origins,
        FileId(999),
        &headers.headers,
        &headers.module_symbols,
        &SourceProviderImportSet::default(),
        &ExternalPackageRegistry::default(),
        &string_table,
    );

    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("an out-of-range active root FileId must fail"),
    };
    assert!(
        error.msg.contains("out-of-range"),
        "the error must state the out-of-range violation, got: {}",
        error.msg
    );
}

#[test]
fn conflicting_public_header_ownership_fails_internally() {
    // Hidden invariant: when two directly-defined public headers resolve to different owning
    // module origins, the projection must fail rather than picking one silently.
    let (mut headers, mut string_table) =
        parse_single_file_headers_with_table("export:\n    alpha #= 1\n    beta #= 2\n;\n");

    let file_path = PathBuf::from("src/@page.moth");
    let second_path = PathBuf::from("src/other.moth");
    let source_files = SourceFileTable::build(
        [file_path.clone(), second_path.clone()],
        &file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build with two files");
    let active_file_id = source_files
        .get_by_canonical_path(&file_path)
        .expect("active root should be in the source file table")
        .file_id;
    let other_file_id = source_files
        .get_by_canonical_path(&second_path)
        .expect("second file should be in the source file table")
        .file_id;

    let active_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "active".to_owned(),
        ModuleRootRole::Normal,
    );
    let other_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "other".to_owned(),
        ModuleRootRole::Normal,
    );

    let mut origin_by_canonical_path = rustc_hash::FxHashMap::default();
    origin_by_canonical_path.insert(file_path.clone(), active_origin.clone());
    origin_by_canonical_path.insert(second_path.clone(), other_origin.clone());
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &origin_by_canonical_path);

    // Assign every header's retained file id so the projection can resolve its origin. `alpha`
    // carries the active file id (matching the active root origin) while `beta` carries the
    // other file id, so its table origin is the other module origin and the projection detects
    // the ownership conflict.
    for header in &mut headers.headers {
        let name = header
            .tokens
            .src_path
            .name_str(&string_table)
            .expect("a public constant header must carry a defining name");
        header.tokens.file_id = Some(if name == "beta" {
            other_file_id
        } else {
            active_file_id
        });
    }

    let result = build_direct_export_seed(
        &source_module_origins,
        active_file_id,
        &headers.headers,
        &headers.module_symbols,
        &SourceProviderImportSet::default(),
        &ExternalPackageRegistry::default(),
        &string_table,
    );

    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("conflicting public header ownership must fail"),
    };
    assert!(
        error.msg.contains("does not match the active root origin"),
        "the error must state the ownership conflict, got: {}",
        error.msg
    );
}

#[test]
fn zero_public_exports_still_validates_active_origin() {
    // Hidden invariant: the active root origin is validated from the table even when the module
    // has zero directly-defined public exports. An in-range active root whose table entry is
    // None must still fail, proving lookup and validation run before any header is inspected.
    let (headers, mut string_table) = parse_single_file_headers_with_table("");

    let file_path = PathBuf::from("src/@page.moth");
    let source_files = SourceFileTable::build(
        std::iter::once(file_path.clone()),
        &file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let file_id = source_files
        .get_by_canonical_path(&file_path)
        .expect("file should be in source file table")
        .file_id;

    // Build a table where the in-range active file maps to None, simulating an unowned active
    // root. With zero public exports the header loop never runs, so only the active-root lookup
    // and validation can fail — proving they execute before any header is inspected.
    let empty_lookup = rustc_hash::FxHashMap::default();
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &empty_lookup);
    let result = build_direct_export_seed(
        &source_module_origins,
        file_id,
        &headers.headers,
        &headers.module_symbols,
        &SourceProviderImportSet::default(),
        &ExternalPackageRegistry::default(),
        &string_table,
    );

    let error = match result {
        Err(error) => error,
        Ok(_) => panic!(
            "an active root with no owning module origin must fail even with zero public exports"
        ),
    };
    assert!(
        error.msg.contains("no owning module origin"),
        "the error must state the missing-origin violation, got: {}",
        error.msg
    );
}

// ---------------------------------------------------------------------------
//  Transient expanded public source-nominal origin index (graph-derived origins)
// ---------------------------------------------------------------------------

/// Find the canonical declaration path of a struct header by defining name.
fn struct_header_path(headers: &[Header], name: &str, string_table: &StringTable) -> InternedPath {
    for header in headers {
        if matches!(header.kind, HeaderKind::Struct { .. })
            && header.tokens.src_path.name_str(string_table) == Some(name)
        {
            return header.tokens.src_path.clone();
        }
    }
    panic!("no public struct header named `{name}` in test headers")
}

/// Find the canonical declaration path of a trait header by defining name.
fn trait_header_path(headers: &[Header], name: &str, string_table: &StringTable) -> InternedPath {
    for header in headers {
        if matches!(header.kind, HeaderKind::Trait { .. })
            && header.tokens.src_path.name_str(string_table) == Some(name)
        {
            return header.tokens.src_path.clone();
        }
    }
    panic!("no trait header named `{name}` in test headers")
}

/// Build a `ModuleSymbols` whose retained module-root public exports target the given source
/// paths, modelling the header-built public export maps without standing up a full project path
/// resolver.
///
/// The origin index membership check is key-agnostic (it scans every retained entry's target), so
/// all entries live under one representative module-root key. Each export name is the target
/// path's defining name, matching the direct-public-declaration export shape produced by
/// `headers::public_exports` pass 1.
fn module_symbols_with_module_root_export_targets(
    targets: &[InternedPath],
    string_table: &mut StringTable,
) -> ModuleSymbols {
    let module_root_key = InternedPath::from_single_str("test-module-root", string_table);
    let entries: rustc_hash::FxHashSet<PublicExportEntry> = targets
        .iter()
        .map(|target| PublicExportEntry {
            export_name: target
                .name()
                .expect("an export target path must carry a defining name"),
            target: PublicExportTarget::Source {
                path: target.clone(),
                import_shell_id: None,
            },
        })
        .collect();
    let mut module_symbols = ModuleSymbols::empty();
    module_symbols
        .module_root_public_exports
        .insert(module_root_key, entries);
    module_symbols
}

/// Add a retained source-package public export entry targeting the given source path, modelling a
/// source-backed package public surface without a full project path resolver.
fn add_source_package_export_target(
    module_symbols: &mut ModuleSymbols,
    package_prefix: &str,
    target: &InternedPath,
) {
    let entry = PublicExportEntry {
        export_name: target
            .name()
            .expect("an export target path must carry a defining name"),
        target: PublicExportTarget::Source {
            path: target.clone(),
            import_shell_id: None,
        },
    };
    module_symbols
        .source_package_public_exports
        .entry(package_prefix.to_owned())
        .or_default()
        .insert(entry);
}

#[test]
fn public_source_nominal_origin_index_includes_imported_provider_origin() {
    let mut string_table = StringTable::new();
    let active_path = PathBuf::from("src/@page.moth");
    let imported_path = PathBuf::from("src/@mod.moth");

    // The active root is the entry file; the imported root is a normal module-root file compiled only to
    // validate its public declaration surface.
    let active_output = prepare_single_file(
        "export:\n    Local = | value Int |\n;\n",
        &active_path,
        &active_path,
        &mut string_table,
    );
    let imported_output = prepare_single_file(
        "export:\n    Imported = | value Int |\n;\n",
        &imported_path,
        &active_path,
        &mut string_table,
    );

    assert_eq!(active_output.file_role, FileRole::ActiveModuleRoot);
    assert_eq!(imported_output.file_role, FileRole::ImportedModuleRoot);

    let source_files = SourceFileTable::build(
        [active_path.clone(), imported_path.clone()],
        &active_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build for the two module-root files");
    let active_file_id = source_files
        .get_by_canonical_path(&active_path)
        .expect("the active root file should be in the source file table")
        .file_id;
    let imported_file_id = source_files
        .get_by_canonical_path(&imported_path)
        .expect("the imported root file should be in the source file table")
        .file_id;

    let mut headers: Vec<Header> = Vec::new();
    for mut header in active_output.headers {
        header.tokens.file_id = Some(active_file_id);
        headers.push(header);
    }
    for mut header in imported_output.headers {
        header.tokens.file_id = Some(imported_file_id);
        headers.push(header);
    }

    let active_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "active".to_owned(),
        ModuleRootRole::Normal,
    );
    let provider_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "imported".to_owned(),
        ModuleRootRole::Normal,
    );
    let mut origin_by_canonical_path = rustc_hash::FxHashMap::default();
    origin_by_canonical_path.insert(active_path.clone(), active_origin.clone());
    origin_by_canonical_path.insert(imported_path.clone(), provider_origin.clone());
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &origin_by_canonical_path);

    // Each module root's retained public export targets its own public nominal's source path:
    // the active root exports `Local` and the imported root exports `Imported`. The index admits
    // a nominal when a retained public export entry targets its canonical source path, mirroring
    // the AST `source_path_is_public_from_root_file` nameability owner.
    let local_path = struct_header_path(&headers, "Local", &string_table);
    let imported_path_decl = struct_header_path(&headers, "Imported", &string_table);
    let module_symbols = module_symbols_with_module_root_export_targets(
        &[local_path.clone(), imported_path_decl.clone()],
        &mut string_table,
    );

    let index = build_public_source_nominal_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
    )
    .expect("the expanded nominal origin index should build for active plus imported roots");

    // The active-root public nominal resolves to the active module origin.
    assert_eq!(
        index.get(&local_path),
        Some(&OriginTypeId::new(
            active_origin.clone(),
            "Local".to_owned(),
            OriginTypeCategory::Struct
        )),
        "an active-root public struct must resolve to the active module origin"
    );

    // The imported-root public nominal resolves to its defining provider module origin, not the
    // active origin.
    assert_eq!(
        index.get(&imported_path_decl),
        Some(&OriginTypeId::new(
            provider_origin.clone(),
            "Imported".to_owned(),
            OriginTypeCategory::Struct
        )),
        "an imported public struct must resolve to its provider module origin, not the active \
         module origin"
    );
}

#[test]
fn public_source_nominal_origin_index_rejects_missing_file_id() {
    let mut string_table = StringTable::new();
    let active_path = PathBuf::from("src/@page.moth");
    let imported_path = PathBuf::from("src/@mod.moth");

    let active_output = prepare_single_file(
        "export:\n    Local = | value Int |\n;\n",
        &active_path,
        &active_path,
        &mut string_table,
    );
    let imported_output = prepare_single_file(
        "export:\n    Imported = | value Int |\n;\n",
        &imported_path,
        &active_path,
        &mut string_table,
    );

    let source_files = SourceFileTable::build(
        [active_path.clone(), imported_path.clone()],
        &active_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let active_file_id = source_files
        .get_by_canonical_path(&active_path)
        .expect("active root file should be present")
        .file_id;

    let active_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "active".to_owned(),
        ModuleRootRole::Normal,
    );
    let provider_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "imported".to_owned(),
        ModuleRootRole::Normal,
    );
    let mut origin_by_canonical_path = rustc_hash::FxHashMap::default();
    origin_by_canonical_path.insert(active_path, active_origin);
    origin_by_canonical_path.insert(imported_path, provider_origin);
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &origin_by_canonical_path);

    let mut headers: Vec<Header> = Vec::new();
    for mut header in active_output.headers {
        header.tokens.file_id = Some(active_file_id);
        headers.push(header);
    }
    for mut header in imported_output.headers {
        // Deliberately keep file_id = None on the imported header.
        header.tokens.file_id = None;
        headers.push(header);
    }

    // `Imported` is targeted by a retained module-root public export entry, so the index admits
    // it; its missing retained FileId is then an internal invariant violation rather than a
    // silent skip.
    let imported_path_decl = struct_header_path(&headers, "Imported", &string_table);
    let module_symbols =
        module_symbols_with_module_root_export_targets(&[imported_path_decl], &mut string_table);

    let result = build_public_source_nominal_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
    );
    assert!(
        result.is_err(),
        "a public export-targeted nominal header with no retained FileId must be a CompilerError"
    );
}

#[test]
fn public_source_nominal_origin_index_skips_unowned_source_package_nominal() {
    let mut string_table = StringTable::new();
    let active_path = PathBuf::from("src/@page.moth");
    let package_path = PathBuf::from("src/@pkg.moth");

    let active_output = prepare_single_file(
        "export:\n    Local = | value Int |\n;\n",
        &active_path,
        &active_path,
        &mut string_table,
    );
    // A source-package module root not owned by the project graph: deliberately absent from the
    // origin map, so its table entry is None.
    let package_output = prepare_single_file(
        "export:\n    Pkg = | value Int |\n;\n",
        &package_path,
        &active_path,
        &mut string_table,
    );

    let source_files = SourceFileTable::build(
        [active_path.clone(), package_path.clone()],
        &active_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let active_file_id = source_files
        .get_by_canonical_path(&active_path)
        .expect("active root file should be present")
        .file_id;
    let package_file_id = source_files
        .get_by_canonical_path(&package_path)
        .expect("package root file should be present")
        .file_id;

    let active_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "active".to_owned(),
        ModuleRootRole::Normal,
    );
    // Only the active root is owned; the package root maps to None.
    let mut origin_by_canonical_path = rustc_hash::FxHashMap::default();
    origin_by_canonical_path.insert(active_path.clone(), active_origin.clone());
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &origin_by_canonical_path);

    let mut headers: Vec<Header> = Vec::new();
    for mut header in active_output.headers {
        header.tokens.file_id = Some(active_file_id);
        headers.push(header);
    }
    for mut header in package_output.headers {
        header.tokens.file_id = Some(package_file_id);
        headers.push(header);
    }

    // `Local` is targeted by a retained module-root public export (admitted, owned, present).
    // `Pkg` is targeted by a retained source-package public export (admitted, but its file maps
    // to None ownership, so it is skipped rather than given a fabricated origin).
    let local_path = struct_header_path(&headers, "Local", &string_table);
    let pkg_path = struct_header_path(&headers, "Pkg", &string_table);
    let mut module_symbols = module_symbols_with_module_root_export_targets(
        std::slice::from_ref(&local_path),
        &mut string_table,
    );
    add_source_package_export_target(&mut module_symbols, "pkg", &pkg_path);

    let index = build_public_source_nominal_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
    )
    .expect("the index should build; the unowned package nominal is skipped, not an error");

    // The active nominal is present; the unowned package nominal is deliberately absent.
    assert!(
        index.contains_key(&local_path),
        "the active-root public nominal must be in the index"
    );
    assert!(
        !index.contains_key(&pkg_path),
        "a source-package nominal with no project-module owner (None) must be absent from the \
         index, not assigned a fabricated origin"
    );
}

/// A privately-authored nominal in a normal file is included when a module-root public export
/// (an alias or re-export) targets its canonical source path, resolving to the normal file's
/// graph-derived module origin.
#[test]
fn public_source_nominal_origin_index_includes_alias_targeted_normal_file_nominal() {
    let mut string_table = StringTable::new();
    let active_path = PathBuf::from("src/@page.moth");
    let impl_path = PathBuf::from("src/impl.moth");

    // The active module root carries an unrelated public constant; `Counter` is authored as a
    // private struct in the normal file `impl.moth` and has no public export of its own. A
    // module-root public alias (`PublicCounter as Counter`) re-exports it, so the retained
    // module-root public export entry targets `Counter`'s canonical source path.
    let active_output = prepare_single_file(
        "export:\n    placeholder #= 1\n;\n",
        &active_path,
        &active_path,
        &mut string_table,
    );
    let impl_output = prepare_single_file(
        "Counter = | count Int |\n",
        &impl_path,
        &active_path,
        &mut string_table,
    );
    assert_eq!(active_output.file_role, FileRole::ActiveModuleRoot);
    assert_eq!(impl_output.file_role, FileRole::Normal);

    let source_files = SourceFileTable::build(
        [active_path.clone(), impl_path.clone()],
        &active_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build for the active root plus normal file");
    let active_file_id = source_files
        .get_by_canonical_path(&active_path)
        .expect("the active root file should be in the source file table")
        .file_id;
    let impl_file_id = source_files
        .get_by_canonical_path(&impl_path)
        .expect("the normal file should be in the source file table")
        .file_id;

    let mut headers: Vec<Header> = Vec::new();
    for mut header in active_output.headers {
        header.tokens.file_id = Some(active_file_id);
        headers.push(header);
    }
    for mut header in impl_output.headers {
        header.tokens.file_id = Some(impl_file_id);
        headers.push(header);
    }

    // The normal file inherits its nearest owning module origin from the project graph, the same
    // module origin as the active root, so the alias-targeted nominal resolves to that origin.
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "active".to_owned(),
        ModuleRootRole::Normal,
    );
    let mut origin_by_canonical_path = rustc_hash::FxHashMap::default();
    origin_by_canonical_path.insert(active_path.clone(), module_origin.clone());
    origin_by_canonical_path.insert(impl_path.clone(), module_origin.clone());
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &origin_by_canonical_path);

    let counter_path = struct_header_path(&headers, "Counter", &string_table);
    let module_symbols = module_symbols_with_module_root_export_targets(
        std::slice::from_ref(&counter_path),
        &mut string_table,
    );

    let index = build_public_source_nominal_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
    )
    .expect("the index should build; the alias-targeted normal-file nominal is included");

    assert_eq!(
        index.get(&counter_path),
        Some(&OriginTypeId::new(
            module_origin.clone(),
            "Counter".to_owned(),
            OriginTypeCategory::Struct
        )),
        "a privately-authored nominal exposed through a module-root public alias must resolve to its normal file's graph-derived module origin"
    );
}

/// A privately-authored nominal in a normal file with no public export target remains absent
/// from the index, while a directly-defined active-root public nominal targeted by its own export
/// is present.
#[test]
fn public_source_nominal_origin_index_excludes_private_normal_file_nominal_without_target() {
    let mut string_table = StringTable::new();
    let active_path = PathBuf::from("src/@page.moth");
    let impl_path = PathBuf::from("src/impl.moth");

    // The active root exports `Local` publicly; `Counter` is a private struct in the normal file
    // with no public export targeting it.
    let active_output = prepare_single_file(
        "export:\n    Local = | value Int |\n;\n",
        &active_path,
        &active_path,
        &mut string_table,
    );
    let impl_output = prepare_single_file(
        "Counter = | count Int |\n",
        &impl_path,
        &active_path,
        &mut string_table,
    );

    let source_files = SourceFileTable::build(
        [active_path.clone(), impl_path.clone()],
        &active_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let active_file_id = source_files
        .get_by_canonical_path(&active_path)
        .expect("the active root file should be in the source file table")
        .file_id;
    let impl_file_id = source_files
        .get_by_canonical_path(&impl_path)
        .expect("the normal file should be in the source file table")
        .file_id;

    let mut headers: Vec<Header> = Vec::new();
    for mut header in active_output.headers {
        header.tokens.file_id = Some(active_file_id);
        headers.push(header);
    }
    for mut header in impl_output.headers {
        header.tokens.file_id = Some(impl_file_id);
        headers.push(header);
    }

    let active_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "active".to_owned(),
        ModuleRootRole::Normal,
    );
    let mut origin_by_canonical_path = rustc_hash::FxHashMap::default();
    origin_by_canonical_path.insert(active_path.clone(), active_origin.clone());
    origin_by_canonical_path.insert(impl_path.clone(), active_origin.clone());
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &origin_by_canonical_path);

    let local_path = struct_header_path(&headers, "Local", &string_table);
    let counter_path = struct_header_path(&headers, "Counter", &string_table);
    // Only `Local` is targeted by a retained module-root public export; `Counter` is private
    // and has no export target.
    let module_symbols = module_symbols_with_module_root_export_targets(
        std::slice::from_ref(&local_path),
        &mut string_table,
    );

    let index = build_public_source_nominal_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
    )
    .expect("the index should build; the private untargeted nominal is excluded");

    assert!(
        index.contains_key(&local_path),
        "the active-root public nominal must be in the index"
    );
    assert!(
        !index.contains_key(&counter_path),
        "a private nominal with no public export target must be absent from the index"
    );
}

// ---------------------------------------------------------------------------
//  Transient expanded public source-trait origin index (graph-derived origins)
// ---------------------------------------------------------------------------

#[test]
fn public_source_trait_origin_index_includes_directly_defined_trait() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let output = prepare_single_file(
        "export:\n    RENDERABLE must:\n        show |This| -> String\n    ;\n;\n",
        &file_path,
        &file_path,
        &mut string_table,
    );
    let mut headers: Vec<Header> = output.headers;
    let source_files = SourceFileTable::build(
        std::iter::once(file_path.clone()),
        &file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let file_id = source_files
        .get_by_canonical_path(&file_path)
        .expect("active root file should be present")
        .file_id;
    for header in &mut headers {
        header.tokens.file_id = Some(file_id);
    }

    let active_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        String::new(),
        ModuleRootRole::Normal,
    );
    let mut origin_by_canonical_path = rustc_hash::FxHashMap::default();
    origin_by_canonical_path.insert(file_path.clone(), active_origin.clone());
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &origin_by_canonical_path);

    let trait_path = trait_header_path(&headers, "RENDERABLE", &string_table);
    let module_symbols = module_symbols_with_module_root_export_targets(
        std::slice::from_ref(&trait_path),
        &mut string_table,
    );

    let index = build_public_source_trait_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
    )
    .expect("the trait origin index should build for a directly-defined public trait");

    assert_eq!(
        index.get(&trait_path),
        Some(&OriginTraitId::new(active_origin, "RENDERABLE".to_owned())),
        "a directly-defined public trait must resolve to the active module origin"
    );
}

#[test]
fn public_source_trait_origin_index_includes_imported_provider_trait() {
    let mut string_table = StringTable::new();
    let active_path = PathBuf::from("src/@page.moth");
    let imported_path = PathBuf::from("src/@mod.moth");

    let active_output = prepare_single_file(
        "export:\n    RENDERABLE must:\n        show |This| -> String\n    ;\n;\n",
        &active_path,
        &active_path,
        &mut string_table,
    );
    let imported_output = prepare_single_file(
        "export:\n    IMPORTED_TRAIT must:\n        show |This| -> String\n    ;\n;\n",
        &imported_path,
        &active_path,
        &mut string_table,
    );

    let source_files = SourceFileTable::build(
        [active_path.clone(), imported_path.clone()],
        &active_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let active_file_id = source_files
        .get_by_canonical_path(&active_path)
        .expect("active root file should be present")
        .file_id;
    let imported_file_id = source_files
        .get_by_canonical_path(&imported_path)
        .expect("imported root file should be present")
        .file_id;

    let active_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        String::new(),
        ModuleRootRole::Normal,
    );
    let provider_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "imported".to_owned(),
        ModuleRootRole::Normal,
    );
    let mut origin_by_canonical_path = rustc_hash::FxHashMap::default();
    origin_by_canonical_path.insert(active_path.clone(), active_origin.clone());
    origin_by_canonical_path.insert(imported_path.clone(), provider_origin.clone());
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &origin_by_canonical_path);

    let mut headers: Vec<Header> = Vec::new();
    for mut header in active_output.headers {
        header.tokens.file_id = Some(active_file_id);
        headers.push(header);
    }
    for mut header in imported_output.headers {
        header.tokens.file_id = Some(imported_file_id);
        headers.push(header);
    }

    let local_trait_path = trait_header_path(&headers, "RENDERABLE", &string_table);
    let imported_trait_path = trait_header_path(&headers, "IMPORTED_TRAIT", &string_table);
    let module_symbols = module_symbols_with_module_root_export_targets(
        &[local_trait_path.clone(), imported_trait_path.clone()],
        &mut string_table,
    );

    let index = build_public_source_trait_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
    )
    .expect("the trait origin index should build for active plus imported roots");

    assert_eq!(
        index.get(&local_trait_path),
        Some(&OriginTraitId::new(active_origin, "RENDERABLE".to_owned())),
        "an active-root public trait must resolve to the active module origin"
    );
    assert_eq!(
        index.get(&imported_trait_path),
        Some(&OriginTraitId::new(
            provider_origin,
            "IMPORTED_TRAIT".to_owned()
        )),
        "an imported public trait must resolve to its provider module origin, not the active \
         module origin"
    );
}

#[test]
fn public_source_trait_origin_index_includes_alias_targeted_normal_file_trait() {
    let mut string_table = StringTable::new();
    let active_path = PathBuf::from("src/@page.moth");
    let impl_path = PathBuf::from("src/impl.moth");

    // The active root carries an unrelated public constant; `DRAWABLE` is a private trait in the
    // normal file with no public export of its own. A module-root public alias targets it, so the
    // retained module-root public export entry targets `DRAWABLE`'s canonical source path.
    let active_output = prepare_single_file(
        "export:\n    placeholder #= 1\n;\n",
        &active_path,
        &active_path,
        &mut string_table,
    );
    let impl_output = prepare_single_file(
        "DRAWABLE must:\n    draw |This| -> String\n;\n",
        &impl_path,
        &active_path,
        &mut string_table,
    );
    assert_eq!(active_output.file_role, FileRole::ActiveModuleRoot);
    assert_eq!(impl_output.file_role, FileRole::Normal);

    let source_files = SourceFileTable::build(
        [active_path.clone(), impl_path.clone()],
        &active_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let active_file_id = source_files
        .get_by_canonical_path(&active_path)
        .expect("active root file should be present")
        .file_id;
    let impl_file_id = source_files
        .get_by_canonical_path(&impl_path)
        .expect("normal file should be present")
        .file_id;

    let mut headers: Vec<Header> = Vec::new();
    for mut header in active_output.headers {
        header.tokens.file_id = Some(active_file_id);
        headers.push(header);
    }
    for mut header in impl_output.headers {
        header.tokens.file_id = Some(impl_file_id);
        headers.push(header);
    }

    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "active".to_owned(),
        ModuleRootRole::Normal,
    );
    let mut origin_by_canonical_path = rustc_hash::FxHashMap::default();
    origin_by_canonical_path.insert(active_path.clone(), module_origin.clone());
    origin_by_canonical_path.insert(impl_path.clone(), module_origin.clone());
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &origin_by_canonical_path);

    let trait_path = trait_header_path(&headers, "DRAWABLE", &string_table);
    let module_symbols = module_symbols_with_module_root_export_targets(
        std::slice::from_ref(&trait_path),
        &mut string_table,
    );

    let index = build_public_source_trait_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
    )
    .expect("the index should build; the alias-targeted normal-file trait is included");

    assert_eq!(
        index.get(&trait_path),
        Some(&OriginTraitId::new(module_origin, "DRAWABLE".to_owned())),
        "a privately-authored trait exposed through a module-root public alias must resolve to \
         its normal file's graph-derived module origin"
    );
}

#[test]
fn public_source_trait_origin_index_excludes_unexported_private_trait() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");

    let output = prepare_single_file(
        "RENDERABLE must:\n    show |This| -> String\n;\n",
        &file_path,
        &file_path,
        &mut string_table,
    );

    let source_files = SourceFileTable::build(
        std::iter::once(file_path.clone()),
        &file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let file_id = source_files
        .get_by_canonical_path(&file_path)
        .expect("active root file should be present")
        .file_id;
    let mut headers: Vec<Header> = Vec::new();
    for mut header in output.headers {
        header.tokens.file_id = Some(file_id);
        headers.push(header);
    }

    let active_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        String::new(),
        ModuleRootRole::Normal,
    );
    let mut origin_by_canonical_path = rustc_hash::FxHashMap::default();
    origin_by_canonical_path.insert(file_path, active_origin);
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &origin_by_canonical_path);

    let trait_path = trait_header_path(&headers, "RENDERABLE", &string_table);
    // No public export targets the trait path, so it is unexported.
    let module_symbols = ModuleSymbols::empty();

    let index = build_public_source_trait_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
    )
    .expect("the trait origin index should build for an unexported trait");

    assert!(
        !index.contains_key(&trait_path),
        "a private unexported trait must be absent from the trait origin index"
    );
}

#[test]
fn public_source_trait_origin_index_skips_unowned_source_package_trait() {
    let mut string_table = StringTable::new();
    let package_path = PathBuf::from("src/@pkg.moth");

    let output = prepare_single_file(
        "export:\n    PKG_TRAIT must:\n        show |This| -> String\n    ;\n;\n",
        &package_path,
        &package_path,
        &mut string_table,
    );

    let source_files = SourceFileTable::build(
        std::iter::once(package_path.clone()),
        &package_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let file_id = source_files
        .get_by_canonical_path(&package_path)
        .expect("package file should be present")
        .file_id;

    let mut headers: Vec<Header> = Vec::new();
    for mut header in output.headers {
        header.tokens.file_id = Some(file_id);
        headers.push(header);
    }

    // The package file has an explicit None owning origin (no project-module owner).
    let source_module_origins = SourceModuleOriginTable::from_graph_ownership(
        &source_files,
        &rustc_hash::FxHashMap::default(),
    );

    let trait_path = trait_header_path(&headers, "PKG_TRAIT", &string_table);
    let mut module_symbols = ModuleSymbols::empty();
    add_source_package_export_target(&mut module_symbols, "pkg", &trait_path);

    let index = build_public_source_trait_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
    )
    .expect("the trait origin index should build for an unowned source-package trait");

    assert!(
        !index.contains_key(&trait_path),
        "an unowned source-package trait must be absent from the trait origin index"
    );
}

#[test]
fn public_source_trait_origin_index_rejects_missing_file_id() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let output = prepare_single_file(
        "export:\n    RENDERABLE must:\n        show |This| -> String\n    ;\n;\n",
        &file_path,
        &file_path,
        &mut string_table,
    );
    let mut headers: Vec<Header> = output.headers;
    for header in &mut headers {
        header.tokens.file_id = None;
    }
    let source_files = SourceFileTable::build(
        std::iter::once(file_path.clone()),
        &file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");

    // Deliberately keep file_id = None on all headers.
    let active_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        String::new(),
        ModuleRootRole::Normal,
    );
    let mut origin_by_canonical_path = rustc_hash::FxHashMap::default();
    origin_by_canonical_path.insert(file_path, active_origin);
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &origin_by_canonical_path);

    let trait_path = trait_header_path(&headers, "RENDERABLE", &string_table);
    let module_symbols =
        module_symbols_with_module_root_export_targets(&[trait_path], &mut string_table);

    let result = build_public_source_trait_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
    );
    assert!(
        result.is_err(),
        "a public export-targeted trait header with no retained FileId must be a CompilerError"
    );
}
