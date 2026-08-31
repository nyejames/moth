//! Shared HTML builder test support.
//!
//! WHAT: centralises the small HIR/module fixtures and artifact helpers used across the
//!       HTML builder tests.
//! WHY: the refactor split tests by module responsibility, so common scaffolding should
//!      live in one place instead of being redefined in every test file.

use crate::build_system::build::{FileKind, OutputFile};
use crate::builder_surface::PackageOrigin;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::{
    CallTarget, ExternalFunctionLowerings, ExternalFunctionSpec, ExternalJsLowering,
    ExternalPackageRegistry,
};
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirNodeId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::collect_module_function_link_facts;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::module_compilation::artefact::{
    ModuleCompilerMetadata, ModuleExecutable, ModuleLinkFacts,
};
use crate::compiler_frontend::module_compilation::{
    Module, ModuleExternalImport, ModuleRootActivity,
};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_tests::integration_test_runner::assertions::html_shell_violation;
use std::path::PathBuf;
use std::sync::Arc;

/// Create the smallest valid HIR module with one entry start function.
pub(crate) fn create_test_hir_module() -> HirModule {
    let mut module = HirModule::new();
    let unit_type = crate::compiler_frontend::datatypes::ids::builtin_type_ids::NONE;

    module.regions = vec![HirRegion::lexical(RegionId(0), None)];
    module.blocks = vec![HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(HirExpression {
            id: crate::compiler_frontend::hir::ids::HirValueId(0),
            kind: HirExpressionKind::TupleConstruct { elements: vec![] },
            ty: unit_type,
            value_kind: ValueKind::Const,
            region: RegionId(0),
        }),
    }];
    module.functions = vec![HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: unit_type,
    }];
    module.start_function = Some(FunctionId(0));
    module
        .function_origins
        .insert(FunctionId(0), HirFunctionOrigin::EntryStart);

    module
}

/// Wrap the base HIR fixture in the build-system `Module` shape used by the HTML builder.
///
/// WHAT: binds the test module's names into the caller-owned shared string table.
/// WHY: HTML builder tests now need the same one-table diagnostic model as production builds.
pub(crate) fn create_test_module(entry_point: PathBuf, string_table: &mut StringTable) -> Module {
    let mut hir_module = create_test_hir_module();
    hir_module.side_table.bind_function_name(
        FunctionId(0),
        InternedPath::from_single_str("start_entry", string_table),
    );
    let function_link_facts = collect_module_function_link_facts(&hir_module)
        .expect("test HIR should produce function link facts");

    Module {
        executable: ModuleExecutable {
            hir: hir_module,
            resource_table: ModuleResourceTable::new(),
            type_environment: TypeEnvironment::new(),
            borrow_analysis: BorrowCheckReport::default(),
        },
        link_facts: ModuleLinkFacts {
            external_package_registry: Arc::new(ExternalPackageRegistry::new()),
            external_import_candidates: vec![],
            functions: function_link_facts,
        },
        metadata: ModuleCompilerMetadata {
            entry_point,
            warnings: vec![],
            const_top_level_fragments: vec![],
            // Most builder fixtures represent active page roots. API-only modules opt into the
            // explicit default when a test needs to verify artifact filtering.
            root_activity: ModuleRootActivity {
                has_non_trivial_root_body: true,
                ..ModuleRootActivity::default()
            },
            doc_fragments: vec![],
            materialisation_context: None,
        },
    }
}

/// Attach one candidate import to a reachable external call in the test entry function.
///
/// WHY: runtime-asset tests exercise HTML emission rather than reachability policy, so their
/// fixtures must model a package that entry assembly has legitimately selected.
pub(crate) fn add_reachable_external_import(
    module: &mut Module,
    mut external_import: ModuleExternalImport,
) {
    let import_index = module.link_facts.external_import_candidates.len();
    let registry = Arc::make_mut(&mut module.link_facts.external_package_registry);
    let package_id = registry
        .register_package(
            format!("@test/runtime-{import_index}"),
            PackageOrigin::ProjectLocal,
        )
        .expect("test runtime package should register");
    let function_id = registry
        .register_external_function(
            package_id,
            ExternalFunctionSpec {
                name: format!("runtime_call_{import_index}"),
                parameters: vec![],
                returns: vec![],
                error_return_type: None,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::InlineExpression("undefined".to_owned())),
                    wasm: None,
                },
            },
        )
        .expect("test runtime function should register");

    let start_block = module
        .executable
        .hir
        .blocks
        .iter_mut()
        .find(|block| block.id == BlockId(0))
        .expect("test entry block should exist");
    start_block.statements.push(HirStatement {
        id: HirNodeId(10_000 + import_index as u32),
        kind: HirStatementKind::Call {
            target: CallTarget::External(function_id),
            args: vec![],
            result: None,
        },
        location: SourceLocation::default(),
    });

    external_import.package_id = package_id;
    module
        .link_facts
        .external_import_candidates
        .push(external_import);
    module.link_facts.functions = collect_module_function_link_facts(&module.executable.hir)
        .expect("test HIR should refresh function link facts");
}

/// Collect output paths so tests can assert artifact layout without repeating iterator plumbing.
pub(crate) fn collect_output_paths(output_files: &[OutputFile]) -> Vec<PathBuf> {
    output_files
        .iter()
        .map(|file| file.relative_output_path().to_path_buf())
        .collect()
}

/// Extract an emitted HTML artifact by relative path.
pub(crate) fn expect_html_output<'a>(
    output_files: &'a [OutputFile],
    relative_path: &str,
) -> &'a str {
    let expected_path = PathBuf::from(relative_path);
    output_files
        .iter()
        .find_map(|file| match file.file_kind() {
            FileKind::Html(content) if file.relative_output_path() == expected_path.as_path() => {
                Some(content.as_str())
            }
            _ => None,
        })
        .expect("expected HTML output artifact")
}

/// Extract an emitted JS artifact by relative path.
pub(crate) fn expect_js_output<'a>(output_files: &'a [OutputFile], relative_path: &str) -> &'a str {
    let expected_path = PathBuf::from(relative_path);
    output_files
        .iter()
        .find_map(|file| match file.file_kind() {
            FileKind::Js(content) if file.relative_output_path() == expected_path.as_path() => {
                Some(content.as_str())
            }
            _ => None,
        })
        .expect("expected JS output artifact")
}

/// Assert the full-document shell contract shared by all HTML builder outputs.
///
/// The contract itself is owned by `html_shell_violation`, which the integration HTML baselines
/// also consume, so the builder's own tests and the canonical suite cannot drift apart.
#[track_caller]
pub(crate) fn assert_has_basic_shell(html: &str) {
    if let Some(violation) = html_shell_violation(html) {
        panic!("emitted HTML violates the document shell contract: {violation}\n{html}");
    }
}

/// Assert that a fragment appears before the closing body tag.
pub(crate) fn assert_fragment_before_body_close(html: &str, fragment: &str) {
    let fragment_pos = html.find(fragment).expect("expected fragment to exist");
    let body_close = html.find("</body>").expect("expected </body> to exist");
    assert!(
        fragment_pos < body_close,
        "expected '{fragment}' to appear before </body>"
    );
}
