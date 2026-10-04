//! Integration execution regressions over real source and the existing build/runtime harness.
//!
//! Owns panic bookkeeping and the synthetic-start contract across frontend, target validation
//! and emitted projects. Backend-local bootstrap shape tests remain with the HTML builder.

use super::super::FailureKind;
use super::super::execution::panic_case_result;
use super::super::assertions::validate_success_result;
use super::super::execution::execute_test_case;
use super::super::types::{GoldenExpectation, RenderedOutputExpectation};
use super::super::{BackendId, SuccessExpectation, WarningExpectation};
use super::synthetic_build_results::success_test_case;
use crate::build_system::BuildProfile;
use crate::build_system::build::{BuildResult, FileKind, ProjectBuilder, build_project};
use crate::build_system::create_project_modules::{
    ProjectFrontendCompilation, compile_project_frontend,
};
use crate::builder_surface::BuilderSurface;
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidControlFlowStatementReason, TypeMismatchContext,
    UnsupportedBackendFeatureReason,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::hir::utils::terminator_targets;
use crate::compiler_frontend::module_compilation::Module;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_tests::test_support::frontend_test_style_directives;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
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
        ).expect("should write directory config");
        // The existing runtime harness owns index.html plus root page.js/page.wasm.
        // @page is the single-file homepage convention; selection still dispatches by file path.
        let root_source = directory.path().join("src/@page.moth");
        fs::write(&root_source, source).expect("should write authored root");
        let entry = if directory_entry {
            directory.path().to_path_buf()
        } else {
            root_source.clone()
        };
        Self { directory, entry, root_source }
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
        ).expect("entry frontend should complete without infrastructure failure");
        if frontend.has_diagnosed_or_blocked() {
            let source_database = frontend.project_source_database.take();
            let messages = frontend.into_render_messages_with_frozen_identity(
                &mut string_table,
                source_database,
                None,
            ).expect("frontend diagnostics must retain their frozen source identity");
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

    fn build(&self, flags: &[Flag]) -> BuildResult {
        build_project(
            &ProjectBuilder::new(Box::new(HtmlProjectBuilder::new())),
            self.entry.to_str().expect("fixture entry should be UTF-8"),
            flags,
            &BuildConfigInputSet::new(),
        ).expect("entry project should build")
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
    assert_eq!(types.collection_element_type(success_type), Some(builtin_type_ids::STRING));

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
                assert!(fallible, "narrowed start must return a plain fragment collection");
                reaches_success = true;
            }
            HirTerminator::Return(_) => {
                assert!(!fallible, "fallible start must return through the success channel");
                reaches_success = true;
            }
            _ => {}
        }
        pending.extend(terminator_targets(terminator));
    }
    assert_eq!(reaches_error, fallible);
    assert!(reaches_success, "start must retain the fragment-collection success return");
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
fn synthetic_start_builtin_propagation_and_arithmetic_execute_directory_and_single_file() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    for directory_entry in [true, false] {
        for (source, expected) in [
            (
                "load || -> String, Error!:\n    return \"loaded\"\n;\n\
                 value = load()!\n[:[value]]\n",
                "loaded",
            ),
            ("left ~= 6\nright ~= 7\nvalue = left * right\n[:[value]]\n", "42"),
            (
                "product |left Int, right Int| -> Int:\n    return left * right\n;\n\
                 value = product(6, 7)\n[:[value]]\n",
                "42",
            ),
        ] {
            let fixture = EntryFixture::new(source, directory_entry);
            let frontend = fixture.frontend();
            let module = frontend.successful_module_views().next().expect("entry must compile");
            assert_start_result(module, true);

            let expectation = output_expectation(expected);
            let mut case = success_test_case(BackendId::Html, expectation);
            case.entry_path = fixture.entry.clone();
            case.fixture_root = fixture.directory.path().to_path_buf();
            let result = execute_test_case(&case);
            assert!(result.passed, "{}: {:?} {:?}", source, result.failure_reason, result.messages);
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
            ("left ~= 1\nright ~= 0\nvalue = left // right catch then 17\n[:[value]]\n", "17"),
        ] {
            let fixture = EntryFixture::new(source, directory_entry);
            let frontend = fixture.frontend();
            let module = frontend.successful_module_views().next().expect("entry must compile");
            assert_start_result(module, false);

            let built = fixture.build(&[]);
            let html = built.project.output_files.iter().find_map(|output| {
                if let FileKind::Html(html) = output.file_kind() { Some(html) } else { None }
            }).expect("entry must emit HTML");
            assert!(!html.contains("moth_entry_result"), "infallible entry needs no result branch");
            assert!(!html.contains("if (moth_result.tag"), "narrowed entry needs no error branch");
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
        let result = build_project(
            &ProjectBuilder::new(Box::new(HtmlProjectBuilder::new())),
            fixture.entry.to_str().expect("fixture entry should be UTF-8"),
            &[],
            &BuildConfigInputSet::new(),
        );
        let messages = match result {
            Err(messages) => messages,
            Ok(_) => panic!("custom error cannot propagate through builtin start Error!"),
        };
        assert!(!messages.has_infrastructure_error(), "{messages:?}");
        let diagnostics = messages.error_diagnostics().collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].kind.code(), "MOTH-TYPE-0001");
        assert!(matches!(
            diagnostics[0].payload,
            DiagnosticPayload::TypeMismatch { context: TypeMismatchContext::ErrorReturn, .. }
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
                fixture.entry.to_str().expect("fixture entry should be UTF-8"),
                &[],
                &BuildConfigInputSet::new(),
            );
            let messages = match result {
                Err(messages) => messages,
                Ok(_) => panic!("authored return must not become a synthetic start return: {source}"),
            };
            assert!(!messages.has_infrastructure_error(), "{source}: {messages:?}");
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
    assert!(!frontend.has_diagnosed_or_blocked(), "nested authored return must stay legal");
    let expectation = output_expectation("nested");
    let mut case = success_test_case(BackendId::Html, expectation);
    case.entry_path = fixture.entry.clone();
    case.fixture_root = fixture.directory.path().to_path_buf();
    let result = execute_test_case(&case);
    assert!(result.passed, "{:?} {:?}", result.failure_reason, result.messages);
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
                fixture.entry.to_str().expect("fixture entry should be UTF-8"),
                &[Flag::HtmlWasm],
                &BuildConfigInputSet::new(),
            );
            let messages = match result {
                Err(messages) => messages,
                Ok(_) => panic!("Wasm must retain its reachable failure target gate"),
            };
            assert!(!messages.has_infrastructure_error(), "must diagnose before LIR: {messages:?}");
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
        let module = frontend.successful_module_views().next().expect("entry must compile");
        assert_start_result(module, false);

        let built = fixture.build(&[Flag::HtmlWasm]);
        let mut saw_wasm = false;
        let mut saw_bootstrap = false;
        for output in &built.project.output_files {
            match output.file_kind() {
                FileKind::Wasm(bytes) => {
                    wasmparser::Validator::new().validate_all(bytes).expect("Wasm must validate");
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
        assert!(saw_wasm && saw_bootstrap, "must emit the existing Wasm/vector bootstrap");
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
    let module = frontend.successful_module_views().next().expect("static root must compile");
    assert_start_result(module, false);
    let built = fixture.build(&[]);
    let html = built.project.output_files.iter().find_map(|output| {
        if let FileKind::Html(html) = output.file_kind() { Some(html) } else { None }
    }).expect("static root must emit a page");
    assert!(html.contains("static-entry"));
    assert!(html.contains("if (typeof "), "static entry keeps its existing plain start call");
    assert!(!html.contains("if (moth_result.tag"), "static page must not acquire an error branch");
    assert!(!html.contains("catch (__moth_err)"), "static start must not acquire a fallible wrapper");
    assert!(!html.contains("var moth_result ="), "static start must not acquire a result carrier");
}

#[test]
fn synthetic_start_api_support_and_package_facade_have_no_start() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let fixture = EntryFixture::new("@support label\n[:[label]]\n", true);
    fs::create_dir(fixture.directory.path().join("src/support")).expect("should create support");
    fs::create_dir(fixture.directory.path().join("src/api")).expect("should create API module");
    for (path, source) in [
        ("src/support/+package.moth", "export:\n    label #= \"support\"\n;\n"),
        ("src/api/@mod.moth", "export:\n    label #= \"api\"\n;\n"),
        ("+package.moth", "export:\n    label #= \"facade\"\n;\n"),
    ] {
        fs::write(fixture.directory.path().join(path), source).expect("should write API root");
    }
    let frontend = fixture.frontend();
    for relative_path in ["src/support/+package.moth", "+package.moth"] {
        let module = frontend.successful_module_views().find(|module| {
            module.metadata.entry_point.ends_with(relative_path)
        }).expect("each API-only root must be compiled");
        assert!(start_function(module).is_none(), "{relative_path} must not synthesize start");
    }

    // Declarations alone do not change a Normal root's semantic role or activate it.
    let normal_api = frontend.successful_module_views().find(|module| {
        module.metadata.entry_point.ends_with("src/api/@mod.moth")
    }).expect("declaration-only Normal root must be compiled");
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
    ).expect("should write failing provider root");
    let frontend = fixture.frontend();
    let consumer = frontend.successful_module_views().find(|module| {
        module.metadata.entry_point == fs::canonicalize(&fixture.root_source).expect("root must exist")
    }).expect("consumer must compile");
    assert_start_result(consumer, false);
    let provider = frontend.successful_module_views().find(|module| {
        module.metadata.entry_point.ends_with("provider/@mod.moth")
    }).expect("provider must compile its dormant body");
    assert_start_result(provider, true);

    let mut expectation = output_expectation("consumer-only");
    expectation.rendered_output.not_contains = vec!["provider-error".to_owned(), "provider-ran".to_owned()];
    let mut case = success_test_case(BackendId::Html, expectation);
    case.entry_path = fixture.entry.clone();
    case.fixture_root = fixture.directory.path().to_path_buf();
    let result = execute_test_case(&case);
    assert!(result.passed, "{:?} {:?}", result.failure_reason, result.messages);
}

#[test]
fn synthetic_start_canonical_postfix_case_executes() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let cases = super::super::fixture::load_canonical_case_specs(
        Path::new("tests/cases/result_postfix_in_start_code_success"),
        None,
    ).expect("canonical top-level propagation case must load");
    assert_eq!(cases.len(), 1);
    let result = execute_test_case(&cases[0]);
    assert!(result.passed, "{:?} {:?}", result.failure_reason, result.messages);
}

#[test]
fn synthetic_start_failure_publishes_no_staged_runtime_fragments() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    for directory_entry in [true, false] {
        for source in [
            "load || -> String, Error!:\n    return! Error(\"private-failure-message\")\n;\n\
             #[:static-survives]\n\
             io.line(\"before-failure\")\n\
             [:staged-before-failure]\n\
             value = load()!\n\
             io.line(\"after-failure\")\n\
             [:after-failure-[value]]\n",
            "left ~= 1\nright ~= 0\n\
             #[:static-survives]\n\
             io.line(\"before-failure\")\n\
             [:staged-before-failure]\n\
             value = left // right\n\
             io.line(\"after-failure\")\n\
             [:after-failure-[value]]\n",
        ] {
            let fixture = EntryFixture::new(source, directory_entry);
            let built = fixture.build(&[]);
            let html = built.project.output_files.iter().find_map(|output| {
                if let FileKind::Html(html) = output.file_kind() { Some(html) } else { None }
            }).expect("fallible entry must emit HTML");
            assert!(html.contains("static-survives"), "static fragment remains independently published");

            let mut expectation = output_expectation("before-failure");
            expectation.rendered_output = RenderedOutputExpectation {
                exact: Some("before-failure".to_owned()),
                ..Default::default()
            };
            let case = success_test_case(BackendId::Html, expectation.clone());
            let result = validate_success_result(&case, built, &expectation);
            assert!(result.passed, "{source}: {:?}", result.failure_reason);
        }
    }
}
