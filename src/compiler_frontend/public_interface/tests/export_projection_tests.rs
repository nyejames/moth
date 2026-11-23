//! Focused hidden-invariant tests for the pre-AST direct-export seed and the transient public
//! source-nominal and source-trait origin indexes.
//!
//! WHAT: exercises the invariants of [`DirectExportSeed`] and the origin-index builders that
//! integration output cannot inspect: direct export bindings cover exactly the public
//! declarations authored in the active module root, category distinctions are exact, private
//! declarations and the compiler-owned entry are excluded, ordering is deterministic
//! independent of declaration scheduling, and the active root origin is validated from the
//! per-file source-origin table even when the public surface is empty. The source-nominal and
//! source-trait origin indexes admit direct, imported-provider and alias-targeted declarations
//! while excluding private and unowned declarations, and reject source identities that cannot resolve
//! to a physical module origin.
//! WHY: these are construction invariants owned by `compiler_frontend::public_interface::export_projection`,
//! so they own a focused test beside the module rather than an end-to-end case.

use crate::builder_surface::core_packages::register_core_text_package;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::compiler_errors::InfrastructureFailureKind;
use crate::compiler_frontend::external_packages::external_returns;
use crate::compiler_frontend::external_packages::{
    ExternalConstEvalOp, ExternalFunctionDef, ExternalFunctionLowerings, ExternalNumericContract,
    ExternalPackageRegistry, ExternalSymbolId, TextConstEvalOp,
};
use crate::compiler_frontend::headers::module_symbols::{
    ModuleSymbols, PublicExportEntry, PublicExportTarget,
};
use crate::compiler_frontend::headers::parse_file_headers::parse_file_headers_tests::{
    prepare_active_root_with_role, prepare_single_file_with_fork,
};
use crate::compiler_frontend::headers::parse_file_headers::{
    BoundModuleHeaders, FileRole, Header, HeaderKind, bind_module_headers, prepare_header_syntax,
};
use crate::compiler_frontend::public_interface::{
    DirectExportSeed, PublicBindingExport, PublicExportDiagnosticProvenance,
    PublicSemanticInterface, SourceProviderDependency, SourceProviderDependencySet,
    build_direct_export_seed, build_public_source_nominal_origin_index,
    build_public_source_trait_origin_index,
};
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, FunctionOriginKind, ModuleRootRole, OriginConstantId, OriginDeclarationId,
    OriginFunctionId, OriginTraitId, OriginTypeCategory, OriginTypeId, PublicExportRoute,
    StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, LocalSpan, SourceDatabase, SourceId, SourceSpan,
};
use crate::compiler_frontend::source_module_origin::SourceModuleOriginTable;
use crate::compiler_frontend::symbols::identity::{DependencySelectionId, DependencyShellId};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;

use std::path::PathBuf;

/// Build the direct-export seed for one active-root source using a deterministic synthetic
/// module origin.
///
/// The receiver-method catalog defaults to empty, which exercises the free-binding projection.
fn parse_single_file_headers_with_fork(
    source: &str,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> BoundModuleHeaders {
    let file_path = PathBuf::from("src/@mod.moth");
    let mut span_builder = ExtendedSpanBuilder::new();
    let output = prepare_active_root_with_role(
        source,
        &file_path,
        ModuleRootRole::Normal,
        string_table,
        &mut span_builder,
        path_fork,
    )
    .expect("a normal facade fixture should accept its `$export` declarations");
    let mut prepared_outputs = vec![output];
    let prepared = prepare_header_syntax(
        &mut prepared_outputs,
        string_table,
        &mut |_, _| Ok(()),
        path_fork,
    )
    .expect("header syntax should prepare");
    bind_module_headers(
        prepared,
        &ExternalPackageRegistry::new(),
        &ExternalImportResolutionTable::default(),
        &SourceProviderDependencySet::default(),
        None,
        &SourceDatabase::empty(),
        string_table,
        path_fork,
    )
    .expect("headers should bind")
}
/// and confirms a module with no receiver methods records no receiver surfaces.
fn build_seed(source: &str) -> DirectExportSeed {
    build_seed_for_project(source, "test-project")
}
fn build_seed_for_project(source: &str, project_name: &str) -> DirectExportSeed {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local(project_name),
        String::new(),
        ModuleRootRole::Normal,
    );
    let mut headers =
        parse_single_file_headers_with_fork(source, &mut string_table, &mut path_fork);

    // Build a source file table for the single synthetic test file and set the retained
    // file identity on every header so the origin projection can resolve the active root
    // from the per-file source-origin table.
    let file_path = PathBuf::from("src/@mod.moth");
    let source_files = SourceDatabase::build(
        std::iter::once(file_path.clone()),
        &file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build for the synthetic test file");
    let file_id = source_files
        .get_by_canonical_path(&file_path)
        .expect("the synthetic test file should be in the source file table")
        .id;
    for header in &mut headers.headers {
        header.tokens = header.tokens.rebind_source(file_id);
        header.name_span = header
            .name_span
            .map(|span| SourceSpan::new(file_id, span.local()));
    }

    let source_module_origins =
        SourceModuleOriginTable::from_synthetic_origin(&source_files, &module_origin);

    build_direct_export_seed(
        &source_module_origins,
        file_id,
        &headers.headers,
        &headers.module_symbols,
        &SourceProviderDependencySet::default(),
        &ExternalPackageRegistry::default(),
        &string_table,
        &path_fork,
    )
    .expect("the direct export seed must build for valid headers")
}

struct ExportProjectionFixture {
    headers: Vec<Header>,
    module_symbols: ModuleSymbols,
    source_module_origins: SourceModuleOriginTable,
    string_table: StringTable,
    path_fork: PathInternerFork,
    active_root_file_id: SourceId,
    module_origin: StableModuleOriginIdentity,
}

/// Prepare a small multi-file projection fixture while retaining the header-owned membership
/// facts that `collect_reexport_bindings` consumes. The focused re-export tests intentionally
/// model the already-bound public export map so they exercise semantic projection rather than
/// duplicating path-resolver setup owned by header binding.
fn build_reexport_fixture(sources: &[(&str, &str)], project_name: &str) -> ExportProjectionFixture {
    build_reexport_fixture_with_shared_sources(sources, project_name, &[])
}

fn build_reexport_fixture_with_shared_sources(
    sources: &[(&str, &str)],
    project_name: &str,
    shared_source_paths: &[&str],
) -> ExportProjectionFixture {
    let active_path = PathBuf::from(
        sources
            .first()
            .expect("a projection fixture needs an active source")
            .0,
    );
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut prepared_outputs = Vec::with_capacity(sources.len());
    let mut canonical_paths = Vec::with_capacity(sources.len());
    let mut span_builders = Vec::with_capacity(sources.len());

    for (path, source) in sources {
        let path = PathBuf::from(path);
        if path == active_path {
            let mut builder = ExtendedSpanBuilder::new();
            let output = prepare_active_root_with_role(
                source,
                &path,
                ModuleRootRole::Normal,
                &mut string_table,
                &mut builder,
                &mut path_fork,
            )
            .expect("the active normal facade fixture should accept its `$export` declarations");
            prepared_outputs.push(output);
            span_builders.push(builder);
        } else {
            let (output, builder) = prepare_single_file_with_fork(
                source,
                &path,
                &active_path,
                &mut string_table,
                &mut path_fork,
            );
            prepared_outputs.push(output);
            span_builders.push(builder);
        }
        canonical_paths.push(path);
    }

    let source_files = SourceDatabase::build(
        canonical_paths.iter(),
        &active_path,
        None,
        &mut string_table,
    )
    .expect("projection source files should build");
    let active_root_file_id = source_files
        .get_by_canonical_path(&active_path)
        .expect("the projection active root should have a file identity")
        .id;
    let package = StablePackageIdentity::project_local(project_name);
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        package.clone(),
        "active".to_owned(),
        ModuleRootRole::Normal,
    );
    let mut origin_by_path = rustc_hash::FxHashMap::default();
    for path in &canonical_paths {
        let origin = if shared_source_paths
            .iter()
            .any(|shared_path| std::path::Path::new(shared_path) == path)
        {
            StableModuleOriginIdentity::from_portable_path(
                package.clone(),
                "active".to_owned(),
                ModuleRootRole::Shared,
            )
        } else {
            module_origin.clone()
        };
        origin_by_path.insert(path.clone(), origin);
    }
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &origin_by_path);
    let module_root = path_fork
        .try_intern_portable_path("module-root", &mut string_table)
        .expect("test path fits");

    let mut headers = Vec::new();
    let mut module_symbols = ModuleSymbols::empty();
    for (output, path) in prepared_outputs.into_iter().zip(canonical_paths.iter()) {
        let source_file = output.source_file;
        module_symbols
            .file_roles_by_source
            .insert(source_file, output.file_role);
        module_symbols
            .file_module_membership
            .insert(source_file, module_root);

        let file_id = source_files
            .get_by_canonical_path(path)
            .expect("every prepared projection source should have a file identity")
            .id;
        module_symbols
            .source_paths_by_source_id
            .insert(file_id, source_file);
        for mut header in output.headers {
            header.tokens = header.tokens.rebind_source(file_id);
            header.name_span = header
                .name_span
                .map(|span| SourceSpan::new(file_id, span.local()));
            headers.push(header);
        }
    }

    ExportProjectionFixture {
        headers,
        module_symbols,
        source_module_origins,
        string_table,
        path_fork,
        active_root_file_id,
        module_origin,
    }
}
fn header_named<'a>(
    headers: &'a [Header],
    name: &str,
    string_table: &StringTable,
    path_fork: &PathInternerFork,
) -> &'a Header {
    headers
        .iter()
        .find(|header| {
            path_fork
                .try_component(header.declaration_path)
                .map(|component| string_table.resolve(component) == name)
                .unwrap_or(false)
        })
        .unwrap_or_else(|| panic!("no header named `{name}` in test headers"))
}

fn authored_span(source: &str, public_name: &str) -> Option<SourceSpan> {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let headers = parse_single_file_headers_with_fork(source, &mut string_table, &mut path_fork);
    let file_path = PathBuf::from("src/@mod.moth");
    let source_files = SourceDatabase::build(
        std::iter::once(file_path.clone()),
        &file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build for authored span");
    let file_id = source_files
        .get_by_canonical_path(&file_path)
        .expect("authored span source file should exist")
        .id;
    header_named(&headers.headers, public_name, &string_table, &path_fork)
        .name_span
        .map(|span| SourceSpan::new(file_id, span.local()))
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
const ALL_CATEGORIES_SOURCE: &str = "$export\nrender |button Button| -> String:\n    return button.label\n;\n\
$export\nButton = | label String |\n\
$export\nStatus :: Ready,\n;\n\
$export\nShape as I32\n\
$export\ncount #= 1\n\
$export\nDISPLAYABLE must:\n    show |This| -> String\n;\n";

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
    let source = "$export\nalpha #= 1\n";
    let seed = build_seed(source);

    assert_eq!(seed.export_diagnostic_provenance().len(), 1);
    let provenance = &seed.export_diagnostic_provenance()[0];
    assert_eq!(provenance.public_name, "alpha");
    assert_eq!(provenance.span, authored_span(source, "alpha"));
}

#[test]
fn same_module_reexport_preserves_alias_origin_and_authored_provenance() {
    let mut fixture = build_reexport_fixture(
        &[
            ("src/@mod.moth", "placeholder #= 1\n"),
            ("src/impl.moth", "value #= 1\n"),
        ],
        "same-module-reexport",
    );
    let target_header = header_named(
        &fixture.headers,
        "value",
        &fixture.string_table,
        &fixture.path_fork,
    );
    let target_path = target_header.declaration_path;
    let target_source = fixture
        .module_symbols
        .source_paths_by_source_id
        .get(&target_header.tokens.source())
        .copied()
        .expect("target header source path should be prepared");
    let expected_span = target_header.name_span;

    fixture
        .module_symbols
        .canonical_source_by_symbol_path
        .insert(target_path, target_source);
    fixture.module_symbols.module_root_public_exports.insert(
        *fixture
            .module_symbols
            .source_paths_by_source_id
            .get(&fixture.active_root_file_id)
            .expect("active root source path should be prepared"),
        [PublicExportEntry {
            route: None,
            export_name: fixture.string_table.intern("PublicValue"),
            target: PublicExportTarget::SourceDeclaration { path: target_path },
        }]
        .into_iter()
        .collect(),
    );

    let seed = build_direct_export_seed(
        &fixture.source_module_origins,
        fixture.active_root_file_id,
        &fixture.headers,
        &fixture.module_symbols,
        &SourceProviderDependencySet::default(),
        &ExternalPackageRegistry::default(),
        &fixture.string_table,
        &fixture.path_fork,
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
    assert_eq!(provenance.span, expected_span);
}

#[test]
fn helper_reexport_preserves_role_neutral_defining_origin() {
    let mut fixture = build_reexport_fixture_with_shared_sources(
        &[
            ("src/@mod.moth", "placeholder #= 1\n"),
            ("src/impl.moth", "value #= 1\n"),
        ],
        "shared-helper-reexport",
        &["src/impl.moth"],
    );
    let target_header = header_named(
        &fixture.headers,
        "value",
        &fixture.string_table,
        &fixture.path_fork,
    );
    let target_path = target_header.declaration_path;
    let target_source = fixture
        .module_symbols
        .source_paths_by_source_id
        .get(&target_header.tokens.source())
        .copied()
        .expect("target header source path should be prepared");
    let expected_span = target_header.name_span;

    fixture
        .module_symbols
        .canonical_source_by_symbol_path
        .insert(target_path, target_source);
    fixture.module_symbols.module_root_public_exports.insert(
        *fixture
            .module_symbols
            .source_paths_by_source_id
            .get(&fixture.active_root_file_id)
            .expect("active root source path should be prepared"),
        [PublicExportEntry {
            route: None,
            export_name: fixture.string_table.intern("PublicValue"),
            target: PublicExportTarget::SourceDeclaration { path: target_path },
        }]
        .into_iter()
        .collect(),
    );

    let seed = build_direct_export_seed(
        &fixture.source_module_origins,
        fixture.active_root_file_id,
        &fixture.headers,
        &fixture.module_symbols,
        &SourceProviderDependencySet::default(),
        &ExternalPackageRegistry::default(),
        &fixture.string_table,
        &fixture.path_fork,
    )
    .expect("shared-helper re-export projection should succeed");

    let shared_origin = StableModuleOriginIdentity::from_portable_path(
        fixture.module_origin.package().clone(),
        "active".to_owned(),
        ModuleRootRole::Shared,
    );
    let binding = binding_for(&seed, "PublicValue");
    assert_eq!(
        binding.origin(),
        &OriginDeclarationId::Constant(OriginConstantId::new(shared_origin, "value".to_owned())),
        "a re-exported helper declaration must keep its role-neutral defining identity"
    );
    let provenance = seed
        .export_diagnostic_provenance()
        .iter()
        .find(|entry| entry.public_name == "PublicValue")
        .expect("helper re-export should retain target provenance");
    assert_eq!(provenance.span, expected_span);
}

#[test]
fn changing_only_the_public_route_keeps_origin_but_changes_interface_identity() {
    let root_seed = build_seed("$export\nvalue #= 1\n");
    let routed_seed = build_seed("$export @forms\nvalue #= 1\n");
    let root_binding = binding_for(&root_seed, "value");
    let routed_binding = binding_for(&routed_seed, "value");

    assert!(root_binding.route().is_root());
    assert_eq!(routed_binding.route().as_str(), "forms");
    assert_eq!(
        routed_binding.origin(),
        root_binding.origin(),
        "the route changes where a declaration publishes, never its origin identity"
    );
    assert_ne!(
        routed_binding, root_binding,
        "the route is part of the stable export binding equality"
    );

    let interface = |seed: DirectExportSeed| {
        let parts = seed.into_parts();
        PublicSemanticInterface {
            module_origin: parts.module_origin,
            export_bindings: parts.export_bindings,
            export_diagnostic_provenance: parts.export_diagnostic_provenance,
            binding_exports: parts.binding_exports,
            declarations: Vec::new(),
            reusable_evidence: Vec::new(),
            concrete_call_summaries: Vec::new(),
        }
    };
    assert_ne!(
        interface(routed_seed),
        interface(root_seed),
        "the public-interface fingerprint includes each binding's route"
    );
}

#[test]
fn provider_reexport_preserves_alias_and_provider_provenance() {
    let mut fixture = build_reexport_fixture(
        &[("src/@mod.moth", "placeholder #= 1\n")],
        "provider-reexport",
    );
    let target_path = fixture
        .path_fork
        .try_intern_portable_path("provider/Imported", &mut fixture.string_table)
        .expect("test path fits");
    let reexport_route = fixture
        .path_fork
        .try_intern_portable_path("forms", &mut fixture.string_table)
        .expect("test route path fits");
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
            PublicExportRoute::ROOT,
            "Imported".to_owned(),
            provider_constant_origin.clone(),
        )],
        export_diagnostic_provenance: vec![PublicExportDiagnosticProvenance {
            route: PublicExportRoute::ROOT,
            public_name: "Imported".to_owned(),
            span: None,
        }],
        binding_exports: Vec::new(),
        declarations: Vec::new(),
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    };
    let provider_dependencies = SourceProviderDependencySet::new(vec![SourceProviderDependency {
        kind: crate::compiler_frontend::public_interface::ProviderDependencyKind::Authored {
            shell: DependencyShellId::new(SourceId::from_index(0), 0),
            route: crate::compiler_frontend::semantic_identity::PublicExportRoute::ROOT,
        },
        interface: &provider_interface,
    }])
    .expect("one authored provider should register");
    fixture.module_symbols.module_root_public_exports.insert(
        *fixture
            .module_symbols
            .source_paths_by_source_id
            .get(&fixture.active_root_file_id)
            .expect("active root source path should be prepared"),
        [PublicExportEntry {
            route: Some(reexport_route),
            export_name: fixture.string_table.intern("PublicImported"),
            target: PublicExportTarget::ProviderSelection {
                diagnostic_path: target_path,
                selection: DependencySelectionId::new(
                    DependencyShellId::new(SourceId::from_index(0), 0),
                    0,
                ),
                source_name: fixture.string_table.intern("Imported"),
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
        &provider_dependencies,
        &ExternalPackageRegistry::default(),
        &fixture.string_table,
        &fixture.path_fork,
    )
    .expect("provider re-export projection should succeed");

    let binding = binding_for(&seed, "PublicImported");
    assert_eq!(
        binding.origin(),
        &provider_constant_origin,
        "the re-export publishes under a route without changing the provider origin",
    );
    assert_eq!(binding.route().as_str(), "forms");
    let provenance = seed
        .export_diagnostic_provenance()
        .iter()
        .find(|entry| entry.public_name == "PublicImported")
        .expect("provider re-export should retain provider provenance under the alias");
    assert_eq!(provenance.route.as_str(), "forms");
    assert_eq!(
        provenance.span, None,
        "synthetic provider fixtures carry no authored source span",
    );
}

#[test]
fn external_binding_export_identity_resolves_in_independent_registry() {
    // Both registries start from the same builtin set, so the donor's Core Text `length` takes
    // the first id after the builtins.
    let mut donor_registry = ExternalPackageRegistry::new();
    register_core_text_package(&mut donor_registry);
    let (donor_length, _) = donor_registry
        .resolve_package_function("@core/text", "length")
        .expect("donor should register Core Text length");

    // The consumer registers a same-named function in another package first, so its id for
    // Core Text `length` differs from the donor's, and the donor's raw id names the decoy.
    let mut consumer_registry = ExternalPackageRegistry::new();
    let decoy_id = consumer_registry
        .register_function(ExternalFunctionDef {
            name: "length".to_owned(),
            parameters: Vec::new(),
            return_contract: external_returns(Vec::new(), None),
            const_eval: None,
            numeric_contract: ExternalNumericContract::Unknown,
            lowerings: ExternalFunctionLowerings::default(),
        })
        .expect("decoy test function should register");
    register_core_text_package(&mut consumer_registry);
    // The name lookup below is the test oracle for the expected consumer id. Production
    // resolution goes through the canonical identity only.
    let (consumer_length, _) = consumer_registry
        .resolve_package_function("@core/text", "length")
        .expect("consumer should register Core Text length");
    assert_eq!(
        donor_length, decoy_id,
        "precondition: the donor's raw id names the consumer's decoy",
    );
    assert_ne!(consumer_length, decoy_id);

    let mut fixture = build_reexport_fixture(
        &[("src/@mod.moth", "placeholder #= 1\n")],
        "external-binding-reexport",
    );
    let export_name = fixture.string_table.intern("PublicLength");
    let forms_route = fixture
        .path_fork
        .try_intern_portable_path("forms", &mut fixture.string_table)
        .expect("test route path fits");
    fixture.module_symbols.module_root_public_exports.insert(
        *fixture
            .module_symbols
            .source_paths_by_source_id
            .get(&fixture.active_root_file_id)
            .expect("active root source path should be prepared"),
        [PublicExportEntry {
            route: Some(forms_route),
            export_name,
            target: PublicExportTarget::External(ExternalSymbolId::Function(donor_length)),
        }]
        .into_iter()
        .collect(),
    );
    let seed = build_direct_export_seed(
        &fixture.source_module_origins,
        fixture.active_root_file_id,
        &fixture.headers,
        &fixture.module_symbols,
        &SourceProviderDependencySet::default(),
        &donor_registry,
        &fixture.string_table,
        &fixture.path_fork,
    )
    .expect("external binding re-export projection should succeed");
    let binding_exports = seed.into_parts().binding_exports;

    let [export] = binding_exports.as_slice() else {
        panic!("exactly one binding export should be projected");
    };
    assert_eq!(export.public_name, "PublicLength");
    assert_eq!(
        export.route.as_str(),
        "forms",
        "the authored route is where the binding publishes, not part of its binding identity",
    );
    assert_eq!(
        export.exporting_module, fixture.module_origin,
        "the re-exporting facade owns the published binding, not the binding package",
    );

    // Identity resolves to the consumer's own function; the donor's raw id does not cross.
    assert_eq!(
        consumer_registry.resolve_canonical_symbol(&export.target),
        Some(ExternalSymbolId::Function(consumer_length)),
    );
    let resolved = consumer_registry
        .get_function_by_id(consumer_length)
        .expect("consumer Core Text length should be registered");
    assert_eq!(
        resolved.const_eval,
        Some(ExternalConstEvalOp::Text(TextConstEvalOp::Length)),
    );
    // A donor-local id used directly would dispatch to the decoy, which has no const-eval op.
    assert_eq!(
        consumer_registry
            .get_function_by_id(donor_length)
            .and_then(|function| function.const_eval),
        None,
    );
}

#[test]
fn two_publishing_hops_keep_original_origins_and_authored_provenance() {
    let provider_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("origin-package"),
        "origin/@mod.moth".to_owned(),
        ModuleRootRole::Normal,
    );
    let function_origin = OriginDeclarationId::Function(OriginFunctionId::new_free(
        provider_origin.clone(),
        "twice".to_owned(),
    ));
    let type_origin = OriginDeclarationId::Type(OriginTypeId::new(
        provider_origin.clone(),
        "Counter".to_owned(),
        OriginTypeCategory::Struct,
    ));
    let constant_origin = OriginDeclarationId::Constant(OriginConstantId::new(
        provider_origin.clone(),
        "BASE".to_owned(),
    ));
    let provider_route = PublicExportRoute::from_canonical_spelling("nested/inner".to_owned());
    let mut span_builder = ExtendedSpanBuilder::new();
    let authored_spans: Vec<SourceSpan> = ["twice", "Counter", "BASE"]
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            SourceSpan::new(
                SourceId::from_index(7),
                LocalSpan::exact(10 + index as u32 * 10, name.len() as u32, &mut span_builder)
                    .expect("test span fits"),
            )
        })
        .collect();
    let provider_interface = PublicSemanticInterface {
        module_origin: provider_origin.clone(),
        export_bindings: vec![
            ExportBinding::new(
                provider_origin.clone(),
                provider_route.clone(),
                "twice".to_owned(),
                function_origin.clone(),
            ),
            ExportBinding::new(
                provider_origin.clone(),
                provider_route.clone(),
                "Counter".to_owned(),
                type_origin.clone(),
            ),
            ExportBinding::new(
                provider_origin,
                provider_route.clone(),
                "BASE".to_owned(),
                constant_origin.clone(),
            ),
        ],
        export_diagnostic_provenance: vec![
            PublicExportDiagnosticProvenance {
                route: provider_route.clone(),
                public_name: "twice".to_owned(),
                span: Some(authored_spans[0]),
            },
            PublicExportDiagnosticProvenance {
                route: provider_route.clone(),
                public_name: "Counter".to_owned(),
                span: Some(authored_spans[1]),
            },
            PublicExportDiagnosticProvenance {
                route: provider_route.clone(),
                public_name: "BASE".to_owned(),
                span: Some(authored_spans[2]),
            },
        ],
        binding_exports: Vec::new(),
        declarations: Vec::new(),
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    };
    let provider_dependencies = SourceProviderDependencySet::new(vec![SourceProviderDependency {
        kind: crate::compiler_frontend::public_interface::ProviderDependencyKind::Authored {
            shell: DependencyShellId::new(SourceId::from_index(0), 0),
            route: provider_route.clone(),
        },
        interface: &provider_interface,
    }])
    .expect("one authored provider should register");

    // First publishing facade: republishes the origin provider's `nested/inner` surface at
    // `inner`, adding one hop without changing any origin.
    let mut first_fixture = build_reexport_fixture(
        &[("src/@mod.moth", "placeholder #= 1\n")],
        "first-publisher",
    );
    let inner_path = first_fixture
        .path_fork
        .try_intern_portable_path("inner", &mut first_fixture.string_table)
        .expect("test route path fits");
    let first_diagnostic_path = first_fixture
        .path_fork
        .try_intern_portable_path("origin/nested", &mut first_fixture.string_table)
        .expect("test path fits");
    let selection_shell = DependencyShellId::new(SourceId::from_index(0), 0);
    let mut first_entry = |index: u32, name: &str| PublicExportEntry {
        route: Some(inner_path),
        export_name: first_fixture.string_table.intern(name),
        target: PublicExportTarget::ProviderSelection {
            selection: DependencySelectionId::new(selection_shell, index),
            source_name: first_fixture.string_table.intern(name),
            diagnostic_path: first_diagnostic_path,
        },
    };
    first_fixture
        .module_symbols
        .module_root_public_exports
        .insert(
            *first_fixture
                .module_symbols
                .source_paths_by_source_id
                .get(&first_fixture.active_root_file_id)
                .expect("active root source path should be prepared"),
            [
                first_entry(0, "twice"),
                first_entry(1, "Counter"),
                first_entry(2, "BASE"),
            ]
            .into_iter()
            .collect(),
        );
    let first_seed = build_direct_export_seed(
        &first_fixture.source_module_origins,
        first_fixture.active_root_file_id,
        &first_fixture.headers,
        &first_fixture.module_symbols,
        &provider_dependencies,
        &ExternalPackageRegistry::default(),
        &first_fixture.string_table,
        &first_fixture.path_fork,
    )
    .expect("first publishing hop should project");
    let first_parts = first_seed.into_parts();
    assert_eq!(
        first_parts.export_bindings.len(),
        3,
        "the first publisher should republish every selected origin",
    );
    for binding in &first_parts.export_bindings {
        assert_eq!(binding.route().as_str(), "inner");
        assert_eq!(binding.exporting_module(), &first_fixture.module_origin);
    }
    assert_eq!(
        first_parts.export_diagnostic_provenance.len(),
        3,
        "the first publisher must retain the origin's authored provenance",
    );
    for (name, expected_span) in [
        ("twice", authored_spans[0]),
        ("Counter", authored_spans[1]),
        ("BASE", authored_spans[2]),
    ] {
        let entry = first_parts
            .export_diagnostic_provenance
            .iter()
            .find(|entry| entry.public_name == name)
            .unwrap_or_else(|| panic!("no first-hop provenance for `{name}`"));
        assert_eq!(entry.route.as_str(), "inner");
        assert_eq!(entry.span, Some(expected_span));
    }
    let first_interface = PublicSemanticInterface {
        module_origin: first_parts.module_origin,
        export_bindings: first_parts.export_bindings,
        export_diagnostic_provenance: first_parts.export_diagnostic_provenance,
        binding_exports: first_parts.binding_exports,
        declarations: Vec::new(),
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    };

    // Second publishing facade: republishes the first publisher's `inner` surface at
    // `mirror/deep`. A second distinct module identity must own this binding while the origins
    // and provenance still name the original declarations.
    let second_dependencies = SourceProviderDependencySet::new(vec![SourceProviderDependency {
        kind: crate::compiler_frontend::public_interface::ProviderDependencyKind::Authored {
            shell: DependencyShellId::new(SourceId::from_index(0), 0),
            route: PublicExportRoute::from_canonical_spelling("inner".to_owned()),
        },
        interface: &first_interface,
    }])
    .expect("the first publisher should register as a provider");
    let mut second_fixture = build_reexport_fixture(
        &[("src/@mod.moth", "placeholder #= 1\n")],
        "second-publisher",
    );
    let final_path = second_fixture
        .path_fork
        .try_intern_portable_path("mirror/deep", &mut second_fixture.string_table)
        .expect("test route path fits");
    let second_diagnostic_path = second_fixture
        .path_fork
        .try_intern_portable_path("first/inner", &mut second_fixture.string_table)
        .expect("test path fits");
    let mut second_entry = |index: u32, source_name: &str, export_name: &str| PublicExportEntry {
        route: Some(final_path),
        export_name: second_fixture.string_table.intern(export_name),
        target: PublicExportTarget::ProviderSelection {
            selection: DependencySelectionId::new(selection_shell, index),
            source_name: second_fixture.string_table.intern(source_name),
            diagnostic_path: second_diagnostic_path,
        },
    };
    second_fixture
        .module_symbols
        .module_root_public_exports
        .insert(
            *second_fixture
                .module_symbols
                .source_paths_by_source_id
                .get(&second_fixture.active_root_file_id)
                .expect("active root source path should be prepared"),
            [
                second_entry(0, "twice", "twice"),
                second_entry(1, "Counter", "Counter"),
                second_entry(2, "BASE", "FALLBACK_BASE"),
            ]
            .into_iter()
            .collect(),
        );
    let second_seed = build_direct_export_seed(
        &second_fixture.source_module_origins,
        second_fixture.active_root_file_id,
        &second_fixture.headers,
        &second_fixture.module_symbols,
        &second_dependencies,
        &ExternalPackageRegistry::default(),
        &second_fixture.string_table,
        &second_fixture.path_fork,
    )
    .expect("second publishing hop should project");
    let second_parts = second_seed.into_parts();
    assert_eq!(
        second_parts.export_bindings.len(),
        3,
        "the second publisher should republish every selected name",
    );
    for binding in &second_parts.export_bindings {
        assert_eq!(binding.route().as_str(), "mirror/deep");
        assert_eq!(
            binding.exporting_module(),
            &second_fixture.module_origin,
            "the second publisher owns the binding, not either earlier facade",
        );
        assert_ne!(binding.exporting_module(), &first_fixture.module_origin);
    }
    let origin_for = |name: &str| {
        second_parts
            .export_bindings
            .iter()
            .find(|binding| binding.public_name() == name)
            .unwrap_or_else(|| panic!("no binding for `{name}`"))
            .origin()
            .clone()
    };
    assert_eq!(origin_for("twice"), function_origin);
    assert_eq!(origin_for("Counter"), type_origin);
    assert_eq!(origin_for("FALLBACK_BASE"), constant_origin);
    assert_eq!(
        second_parts.export_diagnostic_provenance.len(),
        3,
        "the second publisher must retain the original authored provenance",
    );
    let span_for = |name: &str| {
        second_parts
            .export_diagnostic_provenance
            .iter()
            .find(|entry| entry.public_name == name)
            .unwrap_or_else(|| panic!("no provenance for `{name}`"))
            .clone()
    };
    for (name, expected_span) in [
        ("twice", authored_spans[0]),
        ("Counter", authored_spans[1]),
        ("FALLBACK_BASE", authored_spans[2]),
    ] {
        let entry = span_for(name);
        assert_eq!(entry.route.as_str(), "mirror/deep");
        assert_eq!(entry.span, Some(expected_span));
    }
}

#[test]
fn binding_backed_reexport_preserves_canonical_identity_and_route() {
    let mut donor_registry = ExternalPackageRegistry::new();
    register_core_text_package(&mut donor_registry);
    let (donor_length, _) = donor_registry
        .resolve_package_function("@core/text", "length")
        .expect("donor should register Core Text length");
    let canonical_target = donor_registry
        .canonical_symbol_identity(ExternalSymbolId::Function(donor_length))
        .expect("donor length should have a canonical identity");

    // A second registry registers an unrelated `length` first, so its local id for Core Text
    // `length` differs from the donor's. Resolution must use the canonical identity, never the
    // donor's raw id.
    let mut consumer_registry = ExternalPackageRegistry::new();
    let decoy_id = consumer_registry
        .register_function(ExternalFunctionDef {
            name: "length".to_owned(),
            parameters: Vec::new(),
            return_contract: external_returns(Vec::new(), None),
            const_eval: None,
            numeric_contract: ExternalNumericContract::Unknown,
            lowerings: ExternalFunctionLowerings::default(),
        })
        .expect("decoy test function should register");
    register_core_text_package(&mut consumer_registry);
    let (consumer_length, _) = consumer_registry
        .resolve_package_function("@core/text", "length")
        .expect("consumer should register Core Text length");
    assert_eq!(
        donor_length, decoy_id,
        "precondition: the donor's raw id names the consumer's decoy",
    );
    assert_ne!(consumer_length, donor_length);

    let mut fixture = build_reexport_fixture(
        &[("src/@mod.moth", "placeholder #= 1\n")],
        "binding-backed-two-hop-reexport",
    );
    let provider_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("provider"),
        "provider/@mod.moth".to_owned(),
        ModuleRootRole::Normal,
    );
    let provider_route = PublicExportRoute::from_canonical_spelling("nested".to_owned());
    let provider_interface = PublicSemanticInterface {
        module_origin: provider_origin.clone(),
        export_bindings: Vec::new(),
        export_diagnostic_provenance: Vec::new(),
        binding_exports: vec![PublicBindingExport {
            exporting_module: provider_origin,
            route: provider_route.clone(),
            public_name: "length".to_owned(),
            target: canonical_target.clone(),
        }],
        declarations: Vec::new(),
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    };
    let provider_dependencies = SourceProviderDependencySet::new(vec![SourceProviderDependency {
        kind: crate::compiler_frontend::public_interface::ProviderDependencyKind::Authored {
            shell: DependencyShellId::new(SourceId::from_index(0), 0),
            route: provider_route,
        },
        interface: &provider_interface,
    }])
    .expect("one authored provider should register");

    let ext_route = fixture
        .path_fork
        .try_intern_portable_path("ext", &mut fixture.string_table)
        .expect("test route path fits");
    let diagnostic_path = fixture
        .path_fork
        .try_intern_portable_path("provider/nested", &mut fixture.string_table)
        .expect("test path fits");
    fixture.module_symbols.module_root_public_exports.insert(
        *fixture
            .module_symbols
            .source_paths_by_source_id
            .get(&fixture.active_root_file_id)
            .expect("active root source path should be prepared"),
        [PublicExportEntry {
            route: Some(ext_route),
            export_name: fixture.string_table.intern("public_length"),
            target: PublicExportTarget::ProviderSelection {
                selection: DependencySelectionId::new(
                    DependencyShellId::new(SourceId::from_index(0), 0),
                    0,
                ),
                source_name: fixture.string_table.intern("length"),
                diagnostic_path,
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
        &provider_dependencies,
        &donor_registry,
        &fixture.string_table,
        &fixture.path_fork,
    )
    .expect("binding-backed two-hop re-export projection should succeed");

    let parts = seed.into_parts();
    let [export] = parts.binding_exports.as_slice() else {
        panic!("exactly one binding export should be projected");
    };
    assert_eq!(
        export.public_name, "public_length",
        "the re-exporter's public alias is the published leaf",
    );
    assert_eq!(
        export.exporting_module, fixture.module_origin,
        "the re-exporter owns the published binding, not the binding package",
    );
    assert_eq!(
        export.route.as_str(),
        "ext",
        "the re-exporter's route replaces the provider route",
    );
    assert_eq!(
        parts.export_bindings.len(),
        0,
        "a binding-backed selection has no source declaration binding",
    );
    assert_eq!(
        consumer_registry.resolve_canonical_symbol(&export.target),
        Some(ExternalSymbolId::Function(consumer_length)),
        "the canonical identity resolves in a registry whose local ids differ",
    );
    assert_eq!(
        consumer_registry
            .get_function_by_id(consumer_length)
            .expect("consumer Core Text length should be registered")
            .const_eval,
        Some(ExternalConstEvalOp::Text(TextConstEvalOp::Length)),
    );
    assert_eq!(
        consumer_registry
            .get_function_by_id(donor_length)
            .and_then(|function| function.const_eval),
        None,
        "the donor raw id names the decoy in the consumer registry",
    );
}

#[test]
fn private_declarations_and_non_export_headers_are_excluded() {
    // `helper` and `Inner` are private; the compiler-owned start never exists on an API facade.
    // Only the public `public_fn` must be recorded.
    let source = "\
helper |value I32| -> I32:\n\
    return value\n\
;\n\
Inner = | x I32 |\n\
$export\n\
public_fn |x I32| -> I32:\n\
    return x\n\
;\n";

    let seed = build_seed(source);

    assert_eq!(
        binding_names(&seed),
        vec!["public_fn"],
        "private declarations and non-export headers must be excluded"
    );
}

#[test]
fn ordering_is_deterministic_independent_of_declaration_scheduling() {
    let order_one = "\
$export\n\
zebra #= 1\n\
$export\n\
alpha #= 2\n";
    let order_two = "\
$export\n\
alpha #= 2\n\
$export\n\
zebra #= 1\n";

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
    // Hidden invariant: when the active root's SourceId maps to no owning module origin, the
    // projection must fail through InfrastructureFailure rather than silently using a fallback origin.
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut headers = parse_single_file_headers_with_fork(
        "$export\nalpha #= 1\n",
        &mut string_table,
        &mut path_fork,
    );

    let file_path = PathBuf::from("src/@mod.moth");
    let source_files = SourceDatabase::build(
        std::iter::once(file_path.clone()),
        &file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let file_id = source_files
        .get_by_canonical_path(&file_path)
        .expect("file should be in source file table")
        .id;
    for header in &mut headers.headers {
        header.tokens = header.tokens.rebind_source(file_id);
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
        &SourceProviderDependencySet::default(),
        &ExternalPackageRegistry::default(),
        &string_table,
        &path_fork,
    );

    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("an active root with no owning module origin must fail"),
    };
    assert!(
        error.message.contains("no owning module origin"),
        "the error must state the missing-origin violation, got: {}",
        error.message
    );
}

#[test]
fn out_of_range_active_root_file_id_fails_internally() {
    // Hidden invariant: an out-of-range SourceId is an internal InfrastructureFailure, not a silent None.
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut headers = parse_single_file_headers_with_fork(
        "$export\nalpha #= 1\n",
        &mut string_table,
        &mut path_fork,
    );

    let file_path = PathBuf::from("src/@mod.moth");
    let source_files = SourceDatabase::build(
        std::iter::once(file_path.clone()),
        &file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let file_id = source_files
        .get_by_canonical_path(&file_path)
        .expect("file should be in source file table")
        .id;
    for header in &mut headers.headers {
        header.tokens = header.tokens.rebind_source(file_id);
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
        SourceId::from_index(999),
        &headers.headers,
        &headers.module_symbols,
        &SourceProviderDependencySet::default(),
        &ExternalPackageRegistry::default(),
        &string_table,
        &path_fork,
    );

    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("an out-of-range active root SourceId must fail"),
    };
    assert!(
        error.message.contains("out-of-range"),
        "the error must state the out-of-range violation, got: {}",
        error.message
    );
}

#[test]
fn zero_public_exports_still_validates_active_origin() {
    // Hidden invariant: the active root origin is validated from the table even when the module
    // has zero directly-defined public exports. An in-range active root whose table entry is
    // None must still fail, proving lookup and validation run before any header is inspected.
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let headers = parse_single_file_headers_with_fork("", &mut string_table, &mut path_fork);

    let file_path = PathBuf::from("src/@mod.moth");
    let source_files = SourceDatabase::build(
        std::iter::once(file_path.clone()),
        &file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let file_id = source_files
        .get_by_canonical_path(&file_path)
        .expect("file should be in source file table")
        .id;

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
        &SourceProviderDependencySet::default(),
        &ExternalPackageRegistry::default(),
        &string_table,
        &path_fork,
    );

    let error = match result {
        Err(error) => error,
        Ok(_) => panic!(
            "an active root with no owning module origin must fail even with zero public exports"
        ),
    };
    assert!(
        error.message.contains("no owning module origin"),
        "the error must state the missing-origin violation, got: {}",
        error.message
    );
}

// ---------------------------------------------------------------------------
//  Transient expanded public source-nominal origin index (graph-derived origins)
// ---------------------------------------------------------------------------

fn struct_header_path(
    headers: &[Header],
    name: &str,
    string_table: &StringTable,
    path_fork: &PathInternerFork,
) -> PathId {
    for header in headers {
        if matches!(header.kind, HeaderKind::Struct { .. })
            && path_fork
                .try_component(header.declaration_path)
                .map(|component| string_table.resolve(component) == name)
                .unwrap_or(false)
        {
            return header.declaration_path;
        }
    }
    panic!("no public struct header named `{name}` in test headers")
}

/// Find the canonical declaration path of a trait header by defining name.
fn trait_header_path(
    headers: &[Header],
    name: &str,
    string_table: &StringTable,
    path_fork: &PathInternerFork,
) -> PathId {
    for header in headers {
        if matches!(header.kind, HeaderKind::Trait { .. })
            && path_fork
                .try_component(header.declaration_path)
                .map(|component| string_table.resolve(component) == name)
                .unwrap_or(false)
        {
            return header.declaration_path;
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
/// Build a `ModuleSymbols` whose retained module-root public exports target the given source
/// paths, modelling the header-built public export maps without standing up a full project path
/// resolver.
fn module_symbols_with_module_root_export_targets(
    targets: &[PathId],
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> ModuleSymbols {
    let module_root_key = path_fork
        .try_intern_portable_path("test-module-root", string_table)
        .expect("test path fits");
    let entries: rustc_hash::FxHashSet<PublicExportEntry> = targets
        .iter()
        .filter_map(|target| {
            Some(PublicExportEntry {
                route: None,
                export_name: path_fork.try_component(*target)?,
                target: PublicExportTarget::SourceDeclaration { path: *target },
            })
        })
        .collect();
    let mut module_symbols = ModuleSymbols::empty();
    module_symbols
        .module_root_public_exports
        .insert(module_root_key, entries);
    module_symbols
}

/// Add a retained source-package public export entry targeting the given source path.
fn add_source_package_export_target(
    module_symbols: &mut ModuleSymbols,
    package_prefix: &str,
    target: &PathId,
    export_name: crate::compiler_frontend::symbols::string_interning::StringId,
) {
    let entry = PublicExportEntry {
        route: None,
        export_name,
        target: PublicExportTarget::SourceDeclaration { path: *target },
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
    let mut path_fork = PathInternerFork::empty();
    let active_path = PathBuf::from("src/@mod.moth");
    let imported_path = PathBuf::from("provider/@mod.moth");

    // The active root is the entry file; the imported root is a normal module-root file compiled only to
    // validate its public declaration surface.
    let mut active_span_builder = ExtendedSpanBuilder::new();
    let active_output = prepare_active_root_with_role(
        "$export\nLocal = | value I32 |\n",
        &active_path,
        ModuleRootRole::Normal,
        &mut string_table,
        &mut active_span_builder,
        &mut path_fork,
    )
    .expect("a normal facade fixture should accept its `$export` declarations");
    let mut imported_span_builder = ExtendedSpanBuilder::new();
    let imported_output = prepare_active_root_with_role(
        "$export\nImported = | value I32 |\n",
        &imported_path,
        ModuleRootRole::Normal,
        &mut string_table,
        &mut imported_span_builder,
        &mut path_fork,
    )
    .expect("a normal facade fixture should accept its `$export` declarations");

    assert_eq!(
        active_output.file_role,
        FileRole::ActiveApiOnlyModuleRoot(ModuleRootRole::Normal)
    );
    assert_eq!(
        imported_output.file_role,
        FileRole::ActiveApiOnlyModuleRoot(ModuleRootRole::Normal)
    );

    let source_files = SourceDatabase::build(
        [active_path.clone(), imported_path.clone()],
        &active_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build for the two module-root files");
    let active_file_id = source_files
        .get_by_canonical_path(&active_path)
        .expect("the active root file should be in the source file table")
        .id;
    let imported_file_id = source_files
        .get_by_canonical_path(&imported_path)
        .expect("the imported root file should be in the source file table")
        .id;

    let mut headers: Vec<Header> = Vec::new();
    for mut header in active_output.headers {
        header.tokens = header.tokens.rebind_source(active_file_id);
        headers.push(header);
    }
    for mut header in imported_output.headers {
        header.tokens = header.tokens.rebind_source(imported_file_id);
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
    let local_path = struct_header_path(&headers, "Local", &string_table, &path_fork);
    let imported_path_decl = struct_header_path(&headers, "Imported", &string_table, &path_fork);
    let module_symbols = module_symbols_with_module_root_export_targets(
        &[local_path, imported_path_decl],
        &mut string_table,
        &mut path_fork,
    );

    let index = build_public_source_nominal_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
        &path_fork,
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
fn public_source_nominal_origin_index_rejects_compilation_root_file_id() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let active_path = PathBuf::from("src/@mod.moth");
    let imported_path = PathBuf::from("provider/@mod.moth");

    let mut active_span_builder = ExtendedSpanBuilder::new();
    let active_output = prepare_active_root_with_role(
        "$export\nLocal = | value I32 |\n",
        &active_path,
        ModuleRootRole::Normal,
        &mut string_table,
        &mut active_span_builder,
        &mut path_fork,
    )
    .expect("a normal facade fixture should accept its `$export` declarations");
    let mut imported_span_builder = ExtendedSpanBuilder::new();
    let imported_output = prepare_active_root_with_role(
        "$export\nImported = | value I32 |\n",
        &imported_path,
        ModuleRootRole::Normal,
        &mut string_table,
        &mut imported_span_builder,
        &mut path_fork,
    )
    .expect("a normal facade fixture should accept its `$export` declarations");

    let source_files = SourceDatabase::build(
        [active_path.clone(), imported_path.clone()],
        &active_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let active_file_id = source_files
        .get_by_canonical_path(&active_path)
        .expect("active root file should be present")
        .id;

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
        header.tokens = header.tokens.rebind_source(active_file_id);
        headers.push(header);
    }
    for mut header in imported_output.headers {
        // Deliberately retain the compilation-root identity from the unregistered preparation
        // stream.
        header.tokens = header.tokens.rebind_source(SourceId::COMPILATION_ROOT);
        headers.push(header);
    }

    // `Imported` is targeted by a retained module-root public export entry, so the index admits
    // it; its compilation-root identity is then an internal invariant violation rather than a
    // silent skip.
    let imported_path_decl = struct_header_path(&headers, "Imported", &string_table, &path_fork);
    let module_symbols = module_symbols_with_module_root_export_targets(
        &[imported_path_decl],
        &mut string_table,
        &mut path_fork,
    );

    let result = build_public_source_nominal_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
        &path_fork,
    );
    assert!(
        result.is_err(),
        "a public export-targeted nominal header with a compilation-root identity must be an InfrastructureFailure"
    );
}

#[test]
fn public_source_nominal_origin_index_skips_unowned_source_package_nominal() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let active_path = PathBuf::from("src/@mod.moth");
    let package_path = PathBuf::from("packages/pkg/@mod.moth");

    let mut active_span_builder = ExtendedSpanBuilder::new();
    let active_output = prepare_active_root_with_role(
        "$export\nLocal = | value I32 |\n",
        &active_path,
        ModuleRootRole::Normal,
        &mut string_table,
        &mut active_span_builder,
        &mut path_fork,
    )
    .expect("a normal facade fixture should accept its `$export` declarations");
    // A source-package module root not owned by the project graph: deliberately absent from the
    // origin map, so its table entry is None.
    let mut package_span_builder = ExtendedSpanBuilder::new();
    let package_output = prepare_active_root_with_role(
        "$export\nPkg = | value I32 |\n",
        &package_path,
        ModuleRootRole::Normal,
        &mut string_table,
        &mut package_span_builder,
        &mut path_fork,
    )
    .expect("a facade fixture should accept its `$export` declarations");

    let source_files = SourceDatabase::build(
        [active_path.clone(), package_path.clone()],
        &active_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let active_file_id = source_files
        .get_by_canonical_path(&active_path)
        .expect("active root file should be present")
        .id;
    let package_file_id = source_files
        .get_by_canonical_path(&package_path)
        .expect("package root file should be present")
        .id;

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
        header.tokens = header.tokens.rebind_source(active_file_id);
        headers.push(header);
    }
    for mut header in package_output.headers {
        header.tokens = header.tokens.rebind_source(package_file_id);
        headers.push(header);
    }

    // `Local` is targeted by a retained module-root public export (admitted, owned, present).
    // `Pkg` is targeted by a retained source-package public export (admitted, but its file maps
    // to None ownership, so it is skipped rather than given a fabricated origin).
    let local_path = struct_header_path(&headers, "Local", &string_table, &path_fork);
    let pkg_path = struct_header_path(&headers, "Pkg", &string_table, &path_fork);
    let mut module_symbols = module_symbols_with_module_root_export_targets(
        std::slice::from_ref(&local_path),
        &mut string_table,
        &mut path_fork,
    );
    add_source_package_export_target(
        &mut module_symbols,
        "pkg",
        &pkg_path,
        string_table.intern("Pkg"),
    );

    let index = build_public_source_nominal_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
        &path_fork,
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
    let mut path_fork = PathInternerFork::empty();
    let active_path = PathBuf::from("src/@mod.moth");
    let impl_path = PathBuf::from("src/impl.moth");

    // The active module root carries an unrelated public constant; `Counter` is authored as a
    // private struct in the normal file `impl.moth` and has no public export of its own. A
    // module-root public alias (`PublicCounter as Counter`) re-exports it, so the retained
    // module-root public export entry targets `Counter`'s canonical source path.
    let mut active_span_builder = ExtendedSpanBuilder::new();
    let active_output = prepare_active_root_with_role(
        "$export\nplaceholder #= 1\n",
        &active_path,
        ModuleRootRole::Normal,
        &mut string_table,
        &mut active_span_builder,
        &mut path_fork,
    )
    .expect("a normal facade fixture should accept its `$export` declarations");
    let (impl_output, _span_builder) = prepare_single_file_with_fork(
        "Counter = | count I32 |\n",
        &impl_path,
        &active_path,
        &mut string_table,
        &mut path_fork,
    );
    assert_eq!(
        active_output.file_role,
        FileRole::ActiveApiOnlyModuleRoot(ModuleRootRole::Normal)
    );
    assert_eq!(impl_output.file_role, FileRole::Normal);

    let source_files = SourceDatabase::build(
        [active_path.clone(), impl_path.clone()],
        &active_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build for the active root plus normal file");
    let active_file_id = source_files
        .get_by_canonical_path(&active_path)
        .expect("the active root file should be in the source file table")
        .id;
    let impl_file_id = source_files
        .get_by_canonical_path(&impl_path)
        .expect("the normal file should be in the source file table")
        .id;

    let mut headers: Vec<Header> = Vec::new();
    for mut header in active_output.headers {
        header.tokens = header.tokens.rebind_source(active_file_id);
        headers.push(header);
    }
    for mut header in impl_output.headers {
        header.tokens = header.tokens.rebind_source(impl_file_id);
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

    let counter_path = struct_header_path(&headers, "Counter", &string_table, &path_fork);
    let module_symbols = module_symbols_with_module_root_export_targets(
        std::slice::from_ref(&counter_path),
        &mut string_table,
        &mut path_fork,
    );

    let index = build_public_source_nominal_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
        &path_fork,
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
    let mut path_fork = PathInternerFork::empty();
    let active_path = PathBuf::from("src/@mod.moth");
    let impl_path = PathBuf::from("src/impl.moth");

    // The active root exports `Local` publicly; `Counter` is a private struct in the normal file
    // with no public export targeting it.
    let mut active_span_builder = ExtendedSpanBuilder::new();
    let active_output = prepare_active_root_with_role(
        "$export\nLocal = | value I32 |\n",
        &active_path,
        ModuleRootRole::Normal,
        &mut string_table,
        &mut active_span_builder,
        &mut path_fork,
    )
    .expect("a normal facade fixture should accept its `$export` declarations");
    let (impl_output, _span_builder) = prepare_single_file_with_fork(
        "Counter = | count I32 |\n",
        &impl_path,
        &active_path,
        &mut string_table,
        &mut path_fork,
    );

    let source_files = SourceDatabase::build(
        [active_path.clone(), impl_path.clone()],
        &active_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let active_file_id = source_files
        .get_by_canonical_path(&active_path)
        .expect("the active root file should be in the source file table")
        .id;
    let impl_file_id = source_files
        .get_by_canonical_path(&impl_path)
        .expect("the normal file should be in the source file table")
        .id;

    let mut headers: Vec<Header> = Vec::new();
    for mut header in active_output.headers {
        header.tokens = header.tokens.rebind_source(active_file_id);
        headers.push(header);
    }
    for mut header in impl_output.headers {
        header.tokens = header.tokens.rebind_source(impl_file_id);
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

    let local_path = struct_header_path(&headers, "Local", &string_table, &path_fork);
    let counter_path = struct_header_path(&headers, "Counter", &string_table, &path_fork);
    // Only `Local` is targeted by a retained module-root public export; `Counter` is private
    // and has no export target.
    let module_symbols = module_symbols_with_module_root_export_targets(
        std::slice::from_ref(&local_path),
        &mut string_table,
        &mut path_fork,
    );

    let index = build_public_source_nominal_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
        &path_fork,
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
    let mut path_fork = PathInternerFork::empty();
    let file_path = PathBuf::from("src/@mod.moth");
    let mut span_builder = ExtendedSpanBuilder::new();
    let output = prepare_active_root_with_role(
        "$export\nRENDERABLE must:\n    show |This| -> String\n;\n",
        &file_path,
        ModuleRootRole::Normal,
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    )
    .expect("a normal facade fixture should accept its `$export` declarations");
    let mut headers: Vec<Header> = output.headers;
    let source_files = SourceDatabase::build(
        std::iter::once(file_path.clone()),
        &file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let file_id = source_files
        .get_by_canonical_path(&file_path)
        .expect("active root file should be present")
        .id;
    for header in &mut headers {
        header.tokens = header.tokens.rebind_source(file_id);
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

    let trait_path = trait_header_path(&headers, "RENDERABLE", &string_table, &path_fork);
    let module_symbols = module_symbols_with_module_root_export_targets(
        std::slice::from_ref(&trait_path),
        &mut string_table,
        &mut path_fork,
    );

    let index = build_public_source_trait_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
        &path_fork,
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
    let mut path_fork = PathInternerFork::empty();
    let active_path = PathBuf::from("src/@mod.moth");
    let imported_path = PathBuf::from("provider/@mod.moth");

    let mut active_span_builder = ExtendedSpanBuilder::new();
    let active_output = prepare_active_root_with_role(
        "$export\nRENDERABLE must:\n    show |This| -> String\n;\n",
        &active_path,
        ModuleRootRole::Normal,
        &mut string_table,
        &mut active_span_builder,
        &mut path_fork,
    )
    .expect("a normal facade fixture should accept its `$export` declarations");
    let mut imported_span_builder = ExtendedSpanBuilder::new();
    let imported_output = prepare_active_root_with_role(
        "$export\nIMPORTED_TRAIT must:\n    show |This| -> String\n;\n",
        &imported_path,
        ModuleRootRole::Normal,
        &mut string_table,
        &mut imported_span_builder,
        &mut path_fork,
    )
    .expect("a normal facade fixture should accept its `$export` declarations");

    let source_files = SourceDatabase::build(
        [active_path.clone(), imported_path.clone()],
        &active_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let active_file_id = source_files
        .get_by_canonical_path(&active_path)
        .expect("active root file should be present")
        .id;
    let imported_file_id = source_files
        .get_by_canonical_path(&imported_path)
        .expect("imported root file should be present")
        .id;

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
        header.tokens = header.tokens.rebind_source(active_file_id);
        headers.push(header);
    }
    for mut header in imported_output.headers {
        header.tokens = header.tokens.rebind_source(imported_file_id);
        headers.push(header);
    }

    let local_trait_path = trait_header_path(&headers, "RENDERABLE", &string_table, &path_fork);
    let imported_trait_path =
        trait_header_path(&headers, "IMPORTED_TRAIT", &string_table, &path_fork);
    let module_symbols = module_symbols_with_module_root_export_targets(
        &[local_trait_path, imported_trait_path],
        &mut string_table,
        &mut path_fork,
    );

    let index = build_public_source_trait_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
        &path_fork,
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
    let mut path_fork = PathInternerFork::empty();
    let active_path = PathBuf::from("src/@mod.moth");
    let impl_path = PathBuf::from("src/impl.moth");

    // The active root carries an unrelated public constant; `DRAWABLE` is a private trait in the
    // normal file with no public export of its own. A module-root public alias targets it, so the
    // retained module-root public export entry targets `DRAWABLE`'s canonical source path.
    let mut active_span_builder = ExtendedSpanBuilder::new();
    let active_output = prepare_active_root_with_role(
        "$export\nplaceholder #= 1\n",
        &active_path,
        ModuleRootRole::Normal,
        &mut string_table,
        &mut active_span_builder,
        &mut path_fork,
    )
    .expect("a normal facade fixture should accept its `$export` declarations");
    let (impl_output, _span_builder) = prepare_single_file_with_fork(
        "DRAWABLE must:\n    draw |This| -> String\n;\n",
        &impl_path,
        &active_path,
        &mut string_table,
        &mut path_fork,
    );
    assert_eq!(
        active_output.file_role,
        FileRole::ActiveApiOnlyModuleRoot(ModuleRootRole::Normal)
    );
    assert_eq!(impl_output.file_role, FileRole::Normal);

    let source_files = SourceDatabase::build(
        [active_path.clone(), impl_path.clone()],
        &active_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let active_file_id = source_files
        .get_by_canonical_path(&active_path)
        .expect("active root file should be present")
        .id;
    let impl_file_id = source_files
        .get_by_canonical_path(&impl_path)
        .expect("normal file should be present")
        .id;

    let mut headers: Vec<Header> = Vec::new();
    for mut header in active_output.headers {
        header.tokens = header.tokens.rebind_source(active_file_id);
        headers.push(header);
    }
    for mut header in impl_output.headers {
        header.tokens = header.tokens.rebind_source(impl_file_id);
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

    let trait_path = trait_header_path(&headers, "DRAWABLE", &string_table, &path_fork);
    let module_symbols = module_symbols_with_module_root_export_targets(
        std::slice::from_ref(&trait_path),
        &mut string_table,
        &mut path_fork,
    );

    let index = build_public_source_trait_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
        &path_fork,
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
    let mut path_fork = PathInternerFork::empty();
    let file_path = PathBuf::from("src/@mod.moth");

    let (output, _span_builder) = prepare_single_file_with_fork(
        "RENDERABLE must:\n    show |This| -> String\n;\n",
        &file_path,
        &file_path,
        &mut string_table,
        &mut path_fork,
    );

    let source_files = SourceDatabase::build(
        std::iter::once(file_path.clone()),
        &file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let file_id = source_files
        .get_by_canonical_path(&file_path)
        .expect("active root file should be present")
        .id;
    let mut headers: Vec<Header> = Vec::new();
    for mut header in output.headers {
        header.tokens = header.tokens.rebind_source(file_id);
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

    let trait_path = trait_header_path(&headers, "RENDERABLE", &string_table, &path_fork);
    // No public export targets the trait path, so it is unexported.
    let module_symbols = ModuleSymbols::empty();

    let index = build_public_source_trait_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
        &path_fork,
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
    let mut path_fork = PathInternerFork::empty();
    let package_path = PathBuf::from("packages/pkg/@mod.moth");

    let mut span_builder = ExtendedSpanBuilder::new();
    let output = prepare_active_root_with_role(
        "$export\nPKG_TRAIT must:\n    show |This| -> String\n;\n",
        &package_path,
        ModuleRootRole::Normal,
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    )
    .expect("a normal facade fixture should accept its `$export` declarations");

    let source_files = SourceDatabase::build(
        std::iter::once(package_path.clone()),
        &package_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let file_id = source_files
        .get_by_canonical_path(&package_path)
        .expect("package file should be present")
        .id;

    let mut headers: Vec<Header> = Vec::new();
    for mut header in output.headers {
        header.tokens = header.tokens.rebind_source(file_id);
        headers.push(header);
    }

    // The package file has an explicit None owning origin (no project-module owner).
    let source_module_origins = SourceModuleOriginTable::from_graph_ownership(
        &source_files,
        &rustc_hash::FxHashMap::default(),
    );
    let trait_path = trait_header_path(&headers, "PKG_TRAIT", &string_table, &path_fork);
    let mut module_symbols = ModuleSymbols::empty();
    add_source_package_export_target(
        &mut module_symbols,
        "pkg",
        &trait_path,
        string_table.intern("PKG_TRAIT"),
    );

    let index = build_public_source_trait_origin_index(
        &source_module_origins,
        &headers,
        &module_symbols,
        &string_table,
        &path_fork,
    )
    .expect("the trait origin index should build for an unowned source-package trait");

    assert!(
        !index.contains_key(&trait_path),
        "an unowned source-package trait must be absent from the trait origin index"
    );
}

#[test]
fn public_source_trait_origin_index_rejects_unowned_source_identity() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let file_path = PathBuf::from("src/@mod.moth");
    let mut span_builder = ExtendedSpanBuilder::new();
    let output = prepare_active_root_with_role(
        "$export\nRENDERABLE must:\n    show |This| -> String\n;\n",
        &file_path,
        ModuleRootRole::Normal,
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    )
    .expect("a normal facade fixture should accept its `$export` declarations");
    let mut headers: Vec<Header> = output.headers;
    let source_files = SourceDatabase::build(
        std::iter::once(file_path.clone()),
        &file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");

    let active_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        String::new(),
        ModuleRootRole::Normal,
    );
    let mut origin_by_canonical_path = rustc_hash::FxHashMap::default();
    origin_by_canonical_path.insert(file_path, active_origin);
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &origin_by_canonical_path);

    let trait_path = trait_header_path(&headers, "RENDERABLE", &string_table, &path_fork);
    let module_symbols = module_symbols_with_module_root_export_targets(
        &[trait_path],
        &mut string_table,
        &mut path_fork,
    );

    for source in [SourceId::COMPILATION_ROOT] {
        for header in &mut headers {
            header.tokens = header.tokens.rebind_source(source);
        }
        let error = build_public_source_trait_origin_index(
            &source_module_origins,
            &headers,
            &module_symbols,
            &string_table,
            &path_fork,
        )
        .expect_err("a public trait must have a registered physical source identity");
        assert_eq!(error.kind, InfrastructureFailureKind::Internal);
    }
}
