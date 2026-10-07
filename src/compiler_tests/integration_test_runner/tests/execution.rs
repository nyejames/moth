//! Integration execution regressions over real source and the existing build/runtime harness.
//!
//! Owns panic bookkeeping and the synthetic-start contract across frontend, target validation
//! and emitted projects. Backend-local bootstrap shape tests remain with the HTML builder.

use super::super::FailureKind;
use super::super::assertions::{
    RuntimeEvent, execute_html_harness_for_test, validate_success_result,
};
use super::super::execution::execute_test_case;
use super::super::execution::panic_case_result;
use super::super::types::{GoldenExpectation, RenderedOutputExpectation};
use super::super::{BackendId, SuccessExpectation, WarningExpectation};
use super::synthetic_build_results::success_test_case;
use crate::backends::js::ENTRY_FAILURE_NOTICE;
use crate::build_system::BuildProfile;
use crate::build_system::build::{
    BackendBuilder, BuildResult, FileKind, ProjectBuilder, build_project,
};
use crate::build_system::create_project_modules::{
    ProjectFrontendCompilation, compile_project_frontend,
};
use crate::builder_surface::BuilderSurface;
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidControlFlowStatementReason, TypeMismatchContext,
    UnsupportedBackendFeatureReason,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::hir::utils::terminator_targets;
use crate::compiler_frontend::module_compilation::Module;
use crate::compiler_frontend::source::line_index::LinePosition;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_tests::test_support::frontend_test_style_directives;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
use crate::projects::html_project::js_path::RELEASE_ENTRY_FAILURE_NOTICE;
use crate::projects::settings::Config;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[test]
fn panic_execution_results_are_always_failures() {
    let result = panic_case_result(Box::new("boom".to_string()));
    assert!(!result.passed);
    assert_eq!(result.panic_message.as_deref(), Some("boom"));
    assert!(result.failure_reason.is_some());
}

#[test]
fn panic_execution_result_has_harness_failed_kind() {
    let result = panic_case_result(Box::new("boom".to_string()));
    assert!(!result.passed);
    assert_eq!(result.failure_kind, Some(FailureKind::HarnessFailed));
}

/// The same root body is compiled as a directory module or an explicitly selected single file.
struct EntryFixture {
    directory: TempDir,
    entry: PathBuf,
    root_source: PathBuf,
}

impl EntryFixture {
    fn new(source: &str, directory_entry: bool) -> Self {
        let directory = tempfile::tempdir().expect("should create entry fixture");
        fs::create_dir(directory.path().join("src")).expect("should create source directory");
        fs::write(
            directory.path().join("config.moth"),
            "project #= (name = \"entry_contract\", entry_root = \"src\")\nhtml #= ()\n",
        )
        .expect("should write directory config");
        // The existing runtime harness owns index.html plus root page.js/page.wasm.
        // @page is the single-file homepage convention; selection still dispatches by file path.
        let root_source = directory.path().join("src/@page.moth");
        fs::write(&root_source, source).expect("should write authored root");
        let entry = if directory_entry {
            directory.path().to_path_buf()
        } else {
            root_source.clone()
        };
        Self {
            directory,
            entry,
            root_source,
        }
    }

    fn select_ordinary_page_file(&mut self) {
        let ordinary_page = self.root_source.with_file_name("page.moth");
        fs::copy(&self.root_source, &ordinary_page).expect("should copy root into ordinary page");
        self.entry = ordinary_page;
    }

    fn frontend(&self) -> ProjectFrontendCompilation {
        let mut config = Config::new(self.entry.clone());
        // This config-free compiler seam consumes settings rather than loading config.moth.
        config.project_name = "entry_contract".to_owned();
        if self.entry.is_dir() {
            config.entry_root = PathBuf::from("src");
        }

        let mut string_table = StringTable::new();
        let mut frontend = compile_project_frontend(
            &mut config,
            BuildProfile::Dev,
            None,
            &frontend_test_style_directives(),
            &mut BuilderSurface::with_mandatory_core(),
            &mut string_table,
        )
        .expect("entry frontend should complete without infrastructure failure");
        if frontend.has_diagnosed_or_blocked() {
            let source_database = frontend.project_source_database.take();
            let messages = frontend
                .into_render_messages_with_frozen_identity(&mut string_table, source_database, None)
                .expect("frontend diagnostics must retain their frozen source identity");
            let rendered = crate::compiler_frontend::compiler_messages::render::terse::
                format_terse_compiler_messages(&messages);
            let source = fs::read_to_string(&self.root_source).expect("fixture source must exist");
            panic!(
                "entry source must compile: {}\nsource:\n{source}\ndiagnostics:\n{}\nstructured: {:?}",
                self.entry.display(),
                rendered.join("\n"),
                messages.error_diagnostics().collect::<Vec<_>>(),
            );
        }
        frontend
    }

    fn html_frontend(&self) -> ProjectFrontendCompilation {
        let mut config = Config::new(self.entry.clone());
        config.project_name = "entry_contract".to_owned();
        if self.entry.is_dir() {
            config.entry_root = PathBuf::from("src");
        }

        let builder = HtmlProjectBuilder::new();
        let style_directives = StyleDirectiveRegistry::merged(&builder.frontend_style_directives())
            .expect("HTML style directives should merge");
        let mut builder_surface = builder.frontend_surface();
        let mut string_table = StringTable::new();
        let mut frontend = compile_project_frontend(
            &mut config,
            BuildProfile::Dev,
            None,
            &style_directives,
            &mut builder_surface,
            &mut string_table,
        )
        .expect("HTML entry frontend should complete without infrastructure failure");
        if frontend.has_diagnosed_or_blocked() {
            let source_database = frontend.project_source_database.take();
            let messages = frontend
                .into_render_messages_with_frozen_identity(&mut string_table, source_database, None)
                .expect("frontend diagnostics must retain their frozen source identity");
            let rendered = crate::compiler_frontend::compiler_messages::render::terse::
                format_terse_compiler_messages(&messages);
            panic!(
                "HTML entry source must compile: {}\ndiagnostics:\n{}\nstructured: {:?}",
                self.entry.display(),
                rendered.join("\n"),
                messages.error_diagnostics().collect::<Vec<_>>(),
            );
        }
        frontend
    }

    fn build_result(&self, flags: &[Flag]) -> Result<BuildResult, CompilerMessages> {
        build_project(
            &ProjectBuilder::new(Box::new(HtmlProjectBuilder::new())),
            self.entry.to_str().expect("fixture entry should be UTF-8"),
            flags,
            &BuildConfigInputSet::new(),
        )
    }

    fn build(&self, flags: &[Flag]) -> BuildResult {
        self.build_result(flags)
            .expect("entry project should build")
    }
}

fn start_function(module: &Module) -> Option<&HirFunction> {
    let hir = &module.executable.hir;
    hir.functions.iter().find(|function| {
        hir.function_origins.get(&function.id) == Some(&HirFunctionOrigin::EntryStart)
    })
}

#[track_caller]
fn assert_start_result(module: &Module, fallible: bool) {
    let start = start_function(module).expect("runtime root must retain start");
    let types = &module.executable.type_environment;
    let carrier = types.fallible_carrier_slots(start.return_type);
    assert_eq!(carrier.is_some(), fallible);
    let success_type = carrier.map_or(start.return_type, |(success, _)| success);
    assert_eq!(
        types.collection_element_type(success_type),
        Some(builtin_type_ids::STRING)
    );

    let hir = &module.executable.hir;
    let mut pending = vec![start.entry];
    let mut visited = Vec::new();
    let mut reaches_error = false;
    let mut reaches_success = false;
    while let Some(block_id) = pending.pop() {
        if visited.contains(&block_id) {
            continue;
        }
        visited.push(block_id);
        let terminator = &hir.blocks[block_id.0 as usize].terminator;
        match terminator {
            HirTerminator::ReturnError(_) => reaches_error = true,
            HirTerminator::ReturnSuccess(_) => {
                assert!(
                    fallible,
                    "narrowed start must return a plain fragment collection"
                );
                reaches_success = true;
            }
            HirTerminator::Return(_) => {
                assert!(
                    !fallible,
                    "fallible start must return through the success channel"
                );
                reaches_success = true;
            }
            _ => {}
        }
        pending.extend(terminator_targets(terminator));
    }
    assert_eq!(reaches_error, fallible);
    assert!(
        reaches_success,
        "start must retain the fragment-collection success return"
    );
}

fn output_expectation(text: &str) -> SuccessExpectation {
    SuccessExpectation {
        warnings: WarningExpectation::Forbid,
        success_contract: None,
        artifact_assertions: Vec::new(),
        golden: GoldenExpectation::default(),
        rendered_output: RenderedOutputExpectation {
            contains_exactly_once: vec![text.to_owned()],
            ..Default::default()
        },
        artifacts_must_not_exist: Vec::new(),
    }
}

#[test]
fn synthetic_single_file_registered_html_package_matches_directory_build() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    for (entry_mode, directory_entry, ordinary_page_entry) in [
        ("directory", true, false),
        ("selected @page.moth", false, false),
        ("selected page.moth", false, true),
    ] {
        let mut fixture = EntryFixture::new(
            "@html p, center, canvas\n\
             @wrappers draw\n\
             #[p, center: source-package-wrapper]\n\
             #[canvas:\n\
                 [$insert(\"id\"):game_canvas]\n\
             ]\n\
             result = draw(\"game_canvas\")!\n\
             [:[result]]\n",
            directory_entry,
        );
        if ordinary_page_entry {
            fixture.select_ordinary_page_file();
        }
        fs::write(
            fixture.directory.path().join("src/wrappers.moth"),
            "@html Canvas, get_canvas\n\
             @labels marker\n\
             paint |drawing ~Canvas|:\n\
                 ~drawing.set_fill_style(\"#202020\")\n\
                 ~drawing.fill_rect(0.0, 0.0, 10.0, 10.0)\n\
             ;\n\
             draw |id String| -> String, Error!:\n\
                 drawing ~= get_canvas(id)!\n\
                 paint(~drawing)\n\
                 return marker\n\
             ;\n",
        )
        .expect("should write Canvas wrapper module");
        fs::write(
            fixture.directory.path().join("src/labels.moth"),
            "marker #String = \"remapped-label\"\n",
        )
        .expect("should write source dependency");
        fs::write(
            fixture.directory.path().join("src/unused.moth"),
            "@missing/unreferenced Missing\n",
        )
        .expect("should write unreferenced source with a missing dependency");

        let frontend = fixture.html_frontend();
        assert!(!frontend.has_diagnosed_or_blocked());
        assert_eq!(
            frontend.project.structure.nodes().len(),
            1,
            "{entry_mode}: the selected page and its ordinary source dependencies belong to one consumer module"
        );
        assert_eq!(
            frontend.project.successful_module_views().count(),
            1,
            "the consumer graph must not absorb the source-backed @html package"
        );
        assert_eq!(frontend.source_packages.len(), 1);
        let html_package = frontend
            .source_packages
            .get(0)
            .expect("the registered HTML source package should publish separately");
        assert_eq!(html_package.package_prefix(), "html");
        assert!(
            html_package
                .boundary
                .successful_module_views()
                .any(|module| module
                    .metadata
                    .entry_point
                    .ends_with("packages/html/@mod.moth")),
            "the HTML package root must remain in its own compilation boundary"
        );
        let source_root = fs::canonicalize(fixture.directory.path().join("src"))
            .expect("source root should canonicalize");
        let project_sources = frontend
            .project_source_database
            .as_ref()
            .expect("project consumer should retain its source inventory");
        assert!(
            project_sources.iter().all(|slot| slot
                .canonical_os_path
                .as_deref()
                .is_some_and(|path| path.starts_with(&source_root))),
            "the synthetic consumer source inventory must stay below the selected source root"
        );
        assert_eq!(
            project_sources.iter().any(|slot| slot
                .canonical_os_path
                .as_deref()
                .is_some_and(|path| path.ends_with("unused.moth"))),
            directory_entry,
            "directory inventory sees the orphan, while bounded single-file discovery excludes it"
        );
        assert_eq!(
            project_sources
                .iter()
                .any(
                    |slot| slot.canonical_os_path.as_deref().is_some_and(|path| {
                        path.file_name().and_then(|name| name.to_str()) == Some("page.moth")
                    })
                ),
            ordinary_page_entry,
            "{entry_mode}: bounded single-file inventory includes only its selected ordinary page"
        );
        assert_eq!(
            project_sources.iter().any(|slot| slot
                .canonical_os_path
                .as_deref()
                .is_some_and(|path| path.ends_with("@page.moth"))),
            !ordinary_page_entry || directory_entry,
            "{entry_mode}: an unselected @page.moth stays outside ordinary-file discovery"
        );

        let built = fixture.build(&[]);
        let html = built
            .project
            .output_files
            .iter()
            .find_map(|output| {
                if let FileKind::Html(html) = output.file_kind() {
                    Some(html)
                } else {
                    None
                }
            })
            .expect("registered HTML package should emit a page");
        assert!(html.contains("source-package-wrapper"));
        assert!(html.contains("text-align: center"));
        assert!(html.contains("<canvas id=\"game_canvas\""));
        assert!(html.contains("<script type=\"importmap\">"));
        assert!(
            html.contains("__moth_src_fn_"),
            "the source-owned Canvas wrapper functions must be linked into the page bundle"
        );
        let canvas_glue_is_emitted = built.project.output_files.iter().any(|output| {
            if let FileKind::Js(script) = output.file_kind() {
                script.contains("getCanvas as __moth_external_fn")
                    && script.contains("context2d as __moth_external_fn")
                    && script.contains("setFillStyle as __moth_external_fn")
                    && script.contains("fillRect as __moth_external_fn")
            } else {
                false
            }
        });
        assert!(
            canvas_glue_is_emitted,
            "the reachable source-owned Canvas methods must select their browser runtime wrappers"
        );
    }
}

fn assert_ambiguous_package_diagnostic(
    fixture: &EntryFixture,
    dependency_clause: &str,
    dependency_path: &str,
    scenario: &str,
) {
    let messages = match fixture.build_result(&[]) {
        Err(messages) => messages,
        Ok(_) => panic!("{scenario}: conflicting local target unexpectedly built"),
    };
    assert!(
        !messages.has_infrastructure_error(),
        "{scenario}: {messages:?}"
    );
    let diagnostics = messages.error_diagnostics().collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 1, "{scenario}: {messages:?}");
    assert_eq!(
        diagnostics[0].kind.code(),
        "MOTH-IMPORT-0006",
        "{scenario}: {messages:?}"
    );

    let position = messages
        .diagnostic_render_context(0)
        .primary_position(diagnostics[0])
        .expect("ambiguous package diagnostic should retain its authored span");
    assert_eq!(position.line, dependency_clause, "{scenario}");
    assert_eq!(
        position.start,
        LinePosition { line: 0, column: 0 },
        "{scenario}"
    );
    assert_eq!(
        position.end,
        LinePosition {
            line: 0,
            column: dependency_path.chars().count() as u32,
        },
        "{scenario}"
    );
}

#[test]
fn synthetic_entry_modes_reject_registered_package_namespace_collisions() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let html_clause = "@html p";
    let html_dependency_path = "@html";
    let html_collisions = [
        ("html.moth", "p #String = \"local html\"\n"),
        ("html.mtf", "[:local html content]\n"),
        ("html.md", "Local html content\n"),
        ("Html.moth", "p #String = \"case-only local html\"\n"),
    ];

    // A direct @page entry and an ordinary selected filename both use bounded synthetic discovery.
    for (entry_mode, directory_entry, ordinary_page_entry) in [
        ("selected @page.moth", false, false),
        ("selected page.moth", false, true),
        ("directory", true, false),
    ] {
        for (local_file, local_source) in html_collisions {
            let mut fixture = EntryFixture::new(
                &format!("{html_clause}\n[:collision reproduction]\n"),
                directory_entry,
            );
            if ordinary_page_entry {
                fixture.select_ordinary_page_file();
            }
            fs::write(
                fixture.directory.path().join("src").join(local_file),
                local_source,
            )
            .expect("should write local HTML namespace collision");

            assert_ambiguous_package_diagnostic(
                &fixture,
                html_clause,
                html_dependency_path,
                &format!("{entry_mode} with {local_file}"),
            );
        }
    }

    let binding_clause = "@core/math PI";
    let binding_dependency_path = "@core/math";
    for (entry_mode, directory_entry, ordinary_page_entry) in [
        ("selected @page.moth", false, false),
        ("selected page.moth", false, true),
        ("directory", true, false),
    ] {
        let mut fixture = EntryFixture::new(
            &format!("{binding_clause}\n[:binding collision reproduction]\n"),
            directory_entry,
        );
        if ordinary_page_entry {
            fixture.select_ordinary_page_file();
        }
        fs::write(
            fixture.directory.path().join("src/core.moth"),
            "placeholder #String = \"local core\"\n",
        )
        .expect("should write local Core namespace collision");

        assert_ambiguous_package_diagnostic(
            &fixture,
            binding_clause,
            binding_dependency_path,
            &format!("{entry_mode} with core.moth"),
        );
    }
}

#[test]
fn synthetic_entry_modes_reject_child_local_package_namespace_collisions() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let child_clause = "@html p";
    for (entry_mode, directory_entry) in [("selected @page.moth", false), ("directory", true)] {
        let fixture = EntryFixture::new("@child label\n[:[label]]\n", directory_entry);
        let child_directory = fixture.directory.path().join("src/child");
        fs::create_dir(&child_directory).expect("should create child module directory");
        fs::write(
            child_directory.join("@child.moth"),
            format!("{child_clause}\nexport:\n    label #String = \"child\"\n;\n"),
        )
        .expect("should write child module dependency");
        fs::write(
            child_directory.join("html.moth"),
            "p #String = \"child-local html\"\n",
        )
        .expect("should write child-local HTML namespace collision");

        assert_ambiguous_package_diagnostic(
            &fixture,
            child_clause,
            "@html",
            &format!("{entry_mode} child module with html.moth"),
        );
    }
}

#[test]
fn synthetic_single_file_missing_package_keeps_its_authored_diagnostic_span() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let import_clause = "@missing/graphics Canvas";
    let dependency_path = "@missing/graphics";
    for directory_entry in [true, false] {
        let fixture = EntryFixture::new(
            &format!("{import_clause}\n[:missing dependency reproduction]\n"),
            directory_entry,
        );
        let messages = match fixture.build_result(&[]) {
            Err(messages) => messages,
            Ok(_) => panic!("unregistered source package unexpectedly built"),
        };
        assert!(!messages.has_infrastructure_error(), "{messages:?}");
        let diagnostics = messages.error_diagnostics().collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 1, "{messages:?}");
        assert_eq!(diagnostics[0].kind.code(), "MOTH-IMPORT-0005");

        let position = messages
            .diagnostic_render_context(0)
            .primary_position(diagnostics[0])
            .expect("missing package diagnostic should retain its authored span");
        assert_eq!(position.line, import_clause);
        assert_eq!(position.start, LinePosition { line: 0, column: 0 });
        assert_eq!(
            position.end,
            LinePosition {
                line: 0,
                column: dependency_path.chars().count() as u32,
            }
        );

        let rendered = crate::compiler_frontend::compiler_messages::render::dev_server::
            render_compiler_messages_html(&messages, fixture.directory.path());
        assert!(rendered.contains("data-diagnostic-code=\"MOTH-IMPORT-0005\""));
        assert!(rendered.contains(&format!(
            "<span class=\"source-line\">{import_clause}</span>"
        )));
        assert!(rendered.contains(&format!(
            "<span class=\"source-caret\">{}</span>",
            "^".repeat(dependency_path.chars().count())
        )));
    }
}

#[test]
fn synthetic_start_builtin_propagation_and_arithmetic_execute_directory_and_single_file() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    for directory_entry in [true, false] {
        for (source, expected) in [
            (
                "load || -> String, Error!:\n    return \"loaded\"\n;\n\
                 value = load()!\n[:[value]]\n",
                "loaded",
            ),
            (
                "left ~= 6\nright ~= 7\nvalue = left * right\n[:[value]]\n",
                "42",
            ),
            (
                "product |left Int, right Int| -> Int:\n    return left * right\n;\n\
                 value = product(6, 7)\n[:[value]]\n",
                "42",
            ),
        ] {
            let fixture = EntryFixture::new(source, directory_entry);
            let frontend = fixture.frontend();
            let module = frontend
                .successful_module_views()
                .next()
                .expect("entry must compile");
            assert_start_result(module, true);

            let expectation = output_expectation(expected);
            let mut case = success_test_case(BackendId::Html, expectation);
            case.entry_path = fixture.entry.clone();
            case.fixture_root = fixture.directory.path().to_path_buf();
            let result = execute_test_case(&case);
            assert!(
                result.passed,
                "{}: {:?} {:?}",
                source, result.failure_reason, result.messages
            );
        }
    }
}

#[test]
fn synthetic_start_local_catch_narrows_and_executes_directory_and_single_file() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    for directory_entry in [true, false] {
        for (source, expected) in [
            (
                "load || -> String, Error!:\n    return! Error(\"failed\", code = 0)\n;\n\
                 value = load() catch then \"recovered\"\n[:[value]]\n",
                "recovered",
            ),
            (
                "left ~= 1\nright ~= 0\nvalue = left // right catch then 17\n[:[value]]\n",
                "17",
            ),
        ] {
            let fixture = EntryFixture::new(source, directory_entry);
            let frontend = fixture.frontend();
            let module = frontend
                .successful_module_views()
                .next()
                .expect("entry must compile");
            assert_start_result(module, false);

            let built = fixture.build(&[]);
            let html = built
                .project
                .output_files
                .iter()
                .find_map(|output| {
                    if let FileKind::Html(html) = output.file_kind() {
                        Some(html)
                    } else {
                        None
                    }
                })
                .expect("entry must emit HTML");
            assert!(
                !html.contains("moth_entry_result"),
                "infallible entry needs no result branch"
            );
            assert!(
                !html.contains("if (moth_result.tag"),
                "narrowed entry needs no error branch"
            );
            let expectation = output_expectation(expected);
            let case = success_test_case(BackendId::Html, expectation.clone());
            let result = validate_success_result(&case, built, &expectation);
            assert!(result.passed, "{:?}", result.failure_reason);
        }
    }
}

#[test]
fn synthetic_start_custom_error_propagation_is_a_source_diagnostic() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    for directory_entry in [true, false] {
        let fixture = EntryFixture::new(
            "Failure = | message String |\n\
             load || -> String, Failure!:\n    return! Failure(\"custom\")\n;\n\
             value = load()!\n[:[value]]\n",
            directory_entry,
        );
        let mut case = success_test_case(BackendId::Html, output_expectation("unreachable"));
        case.entry_path = fixture.entry.clone();
        case.fixture_root = fixture.directory.path().to_path_buf();
        let result = execute_test_case(&case);
        assert!(!result.passed);
        assert_eq!(result.failure_kind, Some(FailureKind::ExpectationViolation));
        assert!(
            result.build_result.is_none(),
            "source diagnostics must precede runtime execution"
        );
        let messages = result
            .messages
            .expect("source rejection must retain compiler diagnostics");
        assert!(!messages.has_infrastructure_error(), "{messages:?}");
        let diagnostics = messages.error_diagnostics().collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].kind.code(), "MOTH-TYPE-0001");
        assert!(matches!(
            diagnostics[0].payload,
            DiagnosticPayload::TypeMismatch {
                context: TypeMismatchContext::ErrorReturn,
                ..
            }
        ));
        assert!(diagnostics[0].primary_span.is_some());
    }
}

#[test]
fn synthetic_start_authored_return_stays_a_source_diagnostic() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    for directory_entry in [true, false] {
        for source in [
            "return\n",
            "if true:\n    return\n;\n",
            "return! Error(\"root\")\n",
        ] {
            let fixture = EntryFixture::new(source, directory_entry);
            let result = build_project(
                &ProjectBuilder::new(Box::new(HtmlProjectBuilder::new())),
                fixture
                    .entry
                    .to_str()
                    .expect("fixture entry should be UTF-8"),
                &[],
                &BuildConfigInputSet::new(),
            );
            let messages = match result {
                Err(messages) => messages,
                Ok(_) => {
                    panic!("authored return must not become a synthetic start return: {source}")
                }
            };
            assert!(
                !messages.has_infrastructure_error(),
                "{source}: {messages:?}"
            );
            let diagnostics = messages.error_diagnostics().collect::<Vec<_>>();
            assert_eq!(diagnostics.len(), 1, "{source}: {messages:?}");
            assert_eq!(diagnostics[0].kind.code(), "MOTH-RULE-0042");
            assert!(matches!(
                diagnostics[0].payload,
                DiagnosticPayload::InvalidControlFlowStatement {
                    reason: InvalidControlFlowStatementReason::ReturnOutsideFunction,
                }
            ));
        }
    }
}

#[test]
fn synthetic_start_preserves_nested_returns_inside_authored_functions() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let fixture = EntryFixture::new(
        "choose || -> String, Error!:\n\
             if true:\n\
                 return \"nested\"\n\
             ;\n\
             return! Error(\"unused\")\n\
         ;\n\
         value = choose()!\n[:[value]]\n",
        true,
    );
    let frontend = fixture.frontend();
    assert!(
        !frontend.has_diagnosed_or_blocked(),
        "nested authored return must stay legal"
    );
    let expectation = output_expectation("nested");
    let mut case = success_test_case(BackendId::Html, expectation);
    case.entry_path = fixture.entry.clone();
    case.fixture_root = fixture.directory.path().to_path_buf();
    let result = execute_test_case(&case);
    assert!(
        result.passed,
        "{:?} {:?}",
        result.failure_reason, result.messages
    );
}
#[test]
fn synthetic_start_wasm_rejects_reachable_failure_before_lir() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    for directory_entry in [true, false] {
        for (source, expected_reason) in [
            (
                "load || -> String, Error!:\n    return! Error(\"failed\")\n;\n\
                 value = load()!\n[:[value]]\n",
                UnsupportedBackendFeatureReason::ErrorValues,
            ),
            (
                "left ~= 6\nright ~= 7\nvalue = left * right\n[:[value]]\n",
                UnsupportedBackendFeatureReason::RecoverableNumericFailure,
            ),
        ] {
            let fixture = EntryFixture::new(source, directory_entry);
            let result = build_project(
                &ProjectBuilder::new(Box::new(HtmlProjectBuilder::new())),
                fixture
                    .entry
                    .to_str()
                    .expect("fixture entry should be UTF-8"),
                &[Flag::HtmlWasm],
                &BuildConfigInputSet::new(),
            );
            let messages = match result {
                Err(messages) => messages,
                Ok(_) => panic!("Wasm must retain its reachable failure target gate"),
            };
            assert!(
                !messages.has_infrastructure_error(),
                "must diagnose before LIR: {messages:?}"
            );
            let diagnostics = messages.error_diagnostics().collect::<Vec<_>>();
            assert_eq!(diagnostics.len(), 1);
            assert_eq!(diagnostics[0].kind.code(), "MOTH-RULE-0064");
            assert!(matches!(
                diagnostics[0].payload,
                DiagnosticPayload::UnsupportedBackendFeature { reason, .. }
                    if reason == expected_reason
            ));
        }
    }
}

#[test]
fn synthetic_start_infallible_wasm_retains_plain_vec_abi() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    for directory_entry in [true, false] {
        let fixture = EntryFixture::new("value = \"plain-vector\"\n[:[value]]\n", directory_entry);
        let frontend = fixture.frontend();
        let module = frontend
            .successful_module_views()
            .next()
            .expect("entry must compile");
        assert_start_result(module, false);

        let built = fixture.build(&[Flag::HtmlWasm]);
        let mut saw_wasm = false;
        let mut saw_bootstrap = false;
        for output in &built.project.output_files {
            match output.file_kind() {
                FileKind::Wasm(bytes) => {
                    wasmparser::Validator::new()
                        .validate_all(bytes)
                        .expect("Wasm must validate");
                    saw_wasm = true;
                }
                FileKind::Js(script) if output.relative_output_path() == Path::new("page.js") => {
                    assert!(script.contains("instance.exports.moth_start()"));
                    assert!(script.contains("moth_vec_len"));
                    assert!(script.contains("moth_vec_get"));
                    assert!(!script.contains("moth_entry_result"));
                    saw_bootstrap = true;
                }
                _ => {}
            }
        }
        assert!(
            saw_wasm && saw_bootstrap,
            "must emit the existing Wasm/vector bootstrap"
        );
        let expectation = output_expectation("plain-vector");
        let case = success_test_case(BackendId::HtmlWasm, expectation.clone());
        let result = validate_success_result(&case, built, &expectation);
        assert!(result.passed, "{:?}", result.failure_reason);
    }
}

#[test]
fn synthetic_start_static_html_needs_no_runtime_error_branch() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let fixture = EntryFixture::new("#[:static-entry]\n", true);
    let frontend = fixture.frontend();
    let module = frontend
        .successful_module_views()
        .next()
        .expect("static root must compile");
    assert_start_result(module, false);
    let built = fixture.build(&[]);
    let html = built
        .project
        .output_files
        .iter()
        .find_map(|output| {
            if let FileKind::Html(html) = output.file_kind() {
                Some(html)
            } else {
                None
            }
        })
        .expect("static root must emit a page");
    assert!(html.contains("static-entry"));
    assert!(
        html.contains("if (typeof "),
        "static entry keeps its existing plain start call"
    );
    assert!(
        !html.contains("if (moth_result.tag"),
        "static page must not acquire an error branch"
    );
    assert!(
        !html.contains("catch (__moth_err)"),
        "static start must not acquire a fallible wrapper"
    );
    assert!(
        !html.contains("var moth_result ="),
        "static start must not acquire a result carrier"
    );
}

#[test]
fn synthetic_start_api_support_and_package_facade_have_no_start() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let fixture = EntryFixture::new("@support label\n[:[label]]\n", true);
    fs::create_dir(fixture.directory.path().join("src/support")).expect("should create support");
    fs::create_dir(fixture.directory.path().join("src/api")).expect("should create API module");
    for (path, source) in [
        (
            "src/support/+package.moth",
            "export:\n    label #= \"support\"\n;\n",
        ),
        ("src/api/@mod.moth", "export:\n    label #= \"api\"\n;\n"),
        ("+package.moth", "export:\n    label #= \"facade\"\n;\n"),
    ] {
        fs::write(fixture.directory.path().join(path), source).expect("should write API root");
    }
    let frontend = fixture.frontend();
    for relative_path in ["src/support/+package.moth", "+package.moth"] {
        let module = frontend
            .successful_module_views()
            .find(|module| module.metadata.entry_point.ends_with(relative_path))
            .expect("each API-only root must be compiled");
        assert!(
            start_function(module).is_none(),
            "{relative_path} must not synthesize start"
        );
    }

    // Declarations alone do not change a Normal root's semantic role or activate it.
    let normal_api = frontend
        .successful_module_views()
        .find(|module| module.metadata.entry_point.ends_with("src/api/@mod.moth"))
        .expect("declaration-only Normal root must be compiled");
    assert_start_result(normal_api, false);
}

#[test]
fn synthetic_start_failing_imported_provider_remains_dormant() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let fixture = EntryFixture::new("@provider label\n[:[label]]\n", true);
    fs::create_dir(fixture.directory.path().join("src/provider")).expect("should create provider");
    fs::write(
        fixture.directory.path().join("src/provider/@mod.moth"),
        "export:\n    label #= \"consumer-only\"\n;\n\
         load || -> String, Error!:\n    return! Error(\"provider-error\")\n;\n\
         io.line(\"provider-ran\")\n\
         value = load()!\n[:[value]]\n",
    )
    .expect("should write failing provider root");
    let frontend = fixture.frontend();
    let consumer = frontend
        .successful_module_views()
        .find(|module| {
            module.metadata.entry_point
                == fs::canonicalize(&fixture.root_source).expect("root must exist")
        })
        .expect("consumer must compile");
    assert_start_result(consumer, false);
    let provider = frontend
        .successful_module_views()
        .find(|module| module.metadata.entry_point.ends_with("provider/@mod.moth"))
        .expect("provider must compile its dormant body");
    assert_start_result(provider, true);

    let mut expectation = output_expectation("consumer-only");
    expectation.rendered_output.not_contains =
        vec!["provider-error".to_owned(), "provider-ran".to_owned()];
    let mut case = success_test_case(BackendId::Html, expectation);
    case.entry_path = fixture.entry.clone();
    case.fixture_root = fixture.directory.path().to_path_buf();
    let result = execute_test_case(&case);
    assert!(
        result.passed,
        "{:?} {:?}",
        result.failure_reason, result.messages
    );
}

#[test]
fn synthetic_start_expected_entry_code_from_linked_source_dependency_passes() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();

    // Project discovery links the support package; single-file entry coverage stays separate.
    for code in [0, u32::MAX] {
        let fixture = EntryFixture::new(
            "@support load\n\
             io.line(\"before-failure\")\n\
             [:staged-before-failure]\n\
             value = load()!\n\
             io.line(\"after-failure\")\n\
             [:after-failure-[value]]\n",
            true,
        );
        fs::create_dir(fixture.directory.path().join("src/support"))
            .expect("should create source dependency");
        fs::write(
            fixture.directory.path().join("src/support/+package.moth"),
            format!(
                "export:\n\
                     load || -> String, Error!:\n\
                         return! Error(message = \"application-secret\", code = {code})\n\
                     ;\n\
                 ;\n"
            ),
        )
        .expect("should write fallible source dependency");

        let mut expectation = output_expectation("before-failure");
        expectation.rendered_output.entry_error_code = Some(code);
        expectation.rendered_output.not_contains = vec![
            "staged-before-failure".to_owned(),
            "after-failure".to_owned(),
            "application-secret".to_owned(),
        ];
        let mut case = success_test_case(BackendId::Html, expectation);
        case.entry_path = fixture.entry.clone();
        case.fixture_root = fixture.directory.path().to_path_buf();
        let result = execute_test_case(&case);

        assert!(
            result.passed,
            "linked entry code {code}: {:?} {:?}",
            result.failure_reason, result.messages
        );
    }
}

#[test]
fn synthetic_start_canonical_postfix_case_executes() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let cases = super::super::fixture::load_canonical_case_specs(
        Path::new("tests/cases/result_postfix_in_start_code_success"),
        None,
    )
    .expect("canonical top-level propagation case must load");
    assert_eq!(cases.len(), 1);
    let result = execute_test_case(&cases[0]);
    assert!(
        result.passed,
        "{:?} {:?}",
        result.failure_reason, result.messages
    );
}

#[test]
fn synthetic_start_failure_publishes_no_staged_runtime_fragments() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    for directory_entry in [true, false] {
        for (source, code) in [
            (
                "load || -> String, Error!:\n    return! Error(message = \"application-secret\", code = 0)\n;\n\
             #[:static-survives]\n\
             io.line(\"before-failure\")\n\
             [:staged-before-failure]\n\
             value = load()!\n\
             io.line(\"after-failure\")\n\
             [:after-failure-[value]]\n",
                0,
            ),
            (
                "left ~= 1\nright ~= 0\n\
             #[:static-survives]\n\
             io.line(\"before-failure\")\n\
             [:staged-before-failure]\n\
             value = left // right\n\
             io.line(\"after-failure\")\n\
             [:after-failure-[value]]\n",
                300,
            ),
            (
                "left ~= 2147483647\nright ~= 1\n\
             #[:static-survives]\n\
             io.line(\"before-failure\")\n\
             [:staged-before-failure]\n\
             value = left + right\n\
             io.line(\"after-failure\")\n\
             [:after-failure-[value]]\n",
                301,
            ),
        ] {
            let fixture = EntryFixture::new(source, directory_entry);
            let mut built = fixture.build(&[]);
            let html = built
                .project
                .output_files
                .iter()
                .find_map(|output| {
                    if let FileKind::Html(html) = output.file_kind() {
                        Some(html)
                    } else {
                        None
                    }
                })
                .expect("fallible entry must emit HTML");
            assert!(
                html.contains("static-survives"),
                "static fragment remains independently published"
            );

            let rendered = execute_html_harness_for_test(&mut built)
                .expect("failed start must still produce a valid harness summary");
            assert_eq!(
                rendered.events(),
                &[
                    RuntimeEvent::Console {
                        text: "before-failure".to_owned()
                    },
                    RuntimeEvent::EntryFailure { code },
                ],
                "{source}",
            );
            assert_eq!(rendered.combined_output(), "before-failure", "{source}");
            assert!(
                rendered.slot_outputs().is_empty(),
                "failed start must publish no staged fragments"
            );
            assert!(
                rendered.runtime_error_message().is_none(),
                "entry failure is not an uncaught Error"
            );
            for forbidden in [
                "after-failure",
                "staged-before-failure",
                "application-secret",
            ] {
                assert!(
                    !rendered.combined_output().contains(forbidden),
                    "{source}: {forbidden}"
                );
            }

            let expectation = output_expectation("before-failure");
            let case = success_test_case(BackendId::Html, expectation.clone());
            let result = validate_success_result(&case, built, &expectation);
            assert!(
                !result.passed,
                "a failed start must not satisfy a success expectation"
            );
            assert_eq!(
                result.failure_kind,
                Some(FailureKind::EntryFailed),
                "{source}"
            );
            let reason = result
                .failure_reason
                .expect("entry failure must report a generic notice");
            assert_eq!(
                reason,
                format!("rendered_output: {}", ENTRY_FAILURE_NOTICE.trim_end())
            );
            assert!(!reason.contains("application-secret"), "{reason}");
        }
    }
}

#[test]
fn synthetic_start_dynamic_zero_range_step_is_entry_failure_without_body_output() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    for directory_entry in [true, false] {
        let fixture = EntryFixture::new(
            "step ~= 0\n\
             loop 0 to 3 by step |value|:\n\
                 io.line(\"range-body-ran\")\n\
             ;\n",
            directory_entry,
        );
        let mut built = fixture.build(&[]);
        let rendered = execute_html_harness_for_test(&mut built)
            .expect("zero range step must produce a valid entry failure summary");
        assert_eq!(
            rendered.events(),
            &[RuntimeEvent::EntryFailure { code: 306 }]
        );
        assert_eq!(
            rendered.combined_output(),
            "",
            "zero step must fail before the loop body"
        );
        assert!(
            rendered.slot_outputs().is_empty(),
            "zero step must publish no loop-body fragments"
        );
        assert!(
            rendered.runtime_error_message().is_none(),
            "zero step is an entry failure, not an uncaught Error"
        );

        let expectation = output_expectation("range-body-ran");
        let case = success_test_case(BackendId::Html, expectation.clone());
        let result = validate_success_result(&case, built, &expectation);
        assert!(
            !result.passed,
            "a zero-step start must not satisfy a success expectation"
        );
        assert_eq!(result.failure_kind, Some(FailureKind::EntryFailed));
    }
}

#[test]
fn synthetic_start_release_fallback_is_fixed_text_and_preserves_earlier_io() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let source = "load || -> String, Error!:\n\
         return! Error(message = \"application-secret\", code = 0)\n\
     ;\n\
     #[:static-survives]\n\
     io.line(\"before-failure\")\n\
     [:staged-before-failure]\n\
     value = load()!\n\
     io.line(\"after-failure\")\n";
    for release in [false, true] {
        let fixture = EntryFixture::new(source, true);
        let flags: &[Flag] = if release { &[Flag::Release] } else { &[] };
        let mut built = fixture.build(flags);
        let html = built
            .project
            .output_files
            .iter()
            .find_map(|output| {
                if let FileKind::Html(html) = output.file_kind() {
                    Some(html.clone())
                } else {
                    None
                }
            })
            .expect("fallible entry must emit HTML");
        assert!(
            html.contains("static-survives"),
            "static HTML remains in the release document"
        );
        let fallback_call = format!("document.createTextNode({RELEASE_ENTRY_FAILURE_NOTICE:?})");
        if release {
            assert!(
                html.contains(&fallback_call),
                "release profile must emit the fixed text node"
            );
            assert!(!html.contains("insertAdjacentHTML(\"beforeend\", moth_result"));
        } else {
            assert!(
                !html.contains(&fallback_call),
                "dev pages must not insert the release notice"
            );
            assert!(!html.contains(RELEASE_ENTRY_FAILURE_NOTICE));
        }
        let rendered = execute_html_harness_for_test(&mut built)
            .expect("release and dev failures must remain harness-observable");
        assert_eq!(
            rendered.events(),
            &[
                RuntimeEvent::Console {
                    text: "before-failure".to_owned()
                },
                RuntimeEvent::EntryFailure { code: 0 },
            ],
        );
        assert!(rendered.slot_outputs().is_empty());
        assert!(!rendered.combined_output().contains("application-secret"));
        assert!(!rendered.combined_output().contains("after-failure"));
    }
}

#[test]
fn synthetic_start_root_assertion_remains_an_unexpected_runtime_error() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    for directory_entry in [true, false] {
        let fixture = EntryFixture::new(
            "io.line(\"before-assertion\")\n[:staged-before-assertion]\n\
             assert(false, \"root invariant failed\")\n",
            directory_entry,
        );
        let mut built = fixture.build(&[]);
        let rendered = execute_html_harness_for_test(&mut built)
            .expect("root assertion must retain the uncaught Error protocol");
        assert_eq!(rendered.combined_output(), "before-assertion");
        assert!(rendered.slot_outputs().is_empty());
        assert!(matches!(
            rendered.events().last(),
            Some(RuntimeEvent::RuntimeError { message }) if message.contains("root invariant failed")
        ));

        let expectation = output_expectation("unreachable");
        let case = success_test_case(BackendId::Html, expectation.clone());
        let result = validate_success_result(&case, built, &expectation);
        assert!(!result.passed);
        assert_eq!(result.failure_kind, Some(FailureKind::HarnessFailed));
        assert!(
            result
                .failure_reason
                .is_some_and(|reason| reason.contains("root invariant failed"))
        );
    }
}

#[test]
fn synthetic_start_stack_overflow_is_a_host_fault_not_a_caught_failure() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    for directory_entry in [true, false] {
        let fixture = EntryFixture::new(
            "boom |depth Int| -> Int:\n\
             \x20   return boom(depth + 1)\n\
             ;\n\
             io.line(\"before-overflow\")\n\
             result = boom(0) catch:\n\
             \x20   io.line(\"handler-ran\")\n\
             \x20   then 0 - 99\n\
             ;\n",
            directory_entry,
        );
        let mut built = fixture.build(&[]);
        let harness_error = match execute_html_harness_for_test(&mut built) {
            Ok(rendered) => panic!(
                "stack exhaustion must not produce a classified render; events: {:?}",
                rendered.events()
            ),
            Err(error) => error,
        };
        assert!(
            harness_error
                .message
                .contains("Maximum call stack size exceeded"),
            "stack exhaustion must surface the host RangeError, got: {}",
            harness_error.message
        );

        let expectation = output_expectation("before-overflow");
        let case = success_test_case(BackendId::Html, expectation.clone());
        let result = validate_success_result(&case, built, &expectation);
        assert!(!result.passed);
        assert_eq!(result.failure_kind, Some(FailureKind::HarnessFailed));
        assert_ne!(result.failure_kind, Some(FailureKind::EntryFailed));
        assert!(
            result
                .failure_reason
                .is_some_and(|reason| !reason.contains("handler-ran")),
            "the catch handler must not observe stack exhaustion"
        );
    }
}
