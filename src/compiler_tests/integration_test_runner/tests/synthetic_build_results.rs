//! Synthetic build results and case specs shared by assertion self-tests.
//!
//! WHAT: builds `BuildResult` and `TestCaseSpec` values that satisfy production identity rules,
//!       plus the minimal valid documents the baselines accept.
//! WHY: several assertion families need the same success-shaped inputs. Keeping one owner stops
//!      each test file from inventing a slightly different "valid" document, which is how a
//!      baseline ends up proved against a document the compiler never emits.

use super::super::types::{GoldenExpectation, SuccessContract};
use super::super::{
    BackendId, ExpectedOutcome, SuccessExpectation, TestCaseSpec, WarningExpectation,
};
use crate::build_system::BuildProfile;
use crate::build_system::build::{BuildResult, FileKind, OutputFile, Project};
use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
use crate::build_system::output::{BuilderKind, CleanupPolicy, OutputOwner};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::Config;
use std::path::PathBuf;
/// The smallest document that satisfies the ordered HTML shell contract.
pub(super) const VALID_HTML: &str = "<!DOCTYPE html>\n<html lang=\"en\">\n  <head>\n  </head>\n  <body style=\"\">\n  </body>\n</html>\n";

/// The smallest HTML-Wasm document: the shell plus the page script include inside the body.
pub(super) const VALID_HTML_WASM: &str = "<!DOCTYPE html>\n<html lang=\"en\">\n  <head>\n  </head>\n  <body style=\"\">\n<script src=\"./page.js\"></script>\n  </body>\n</html>\n";

/// A `page.js` stand-in that uses exactly the runtime exports the real bootstrap uses.
pub(super) const VALID_PAGE_JS: &str = concat!(
    "async function __moth_instantiate_wasm(u, i) { return WebAssembly.instantiate(u, i); }\n",
    "const { instance } = await __moth_instantiate_wasm(\"./page.wasm\", {});\n",
    "const v = instance.exports.moth_start();\n",
    "const p = instance.exports.moth_str_ptr(v);\n",
    "const l = instance.exports.moth_str_len(v);\n",
    "new Uint8Array(instance.exports.memory.buffer, p, l);\n",
    "instance.exports.moth_release(v);\n",
);

pub(super) fn build_result_with_output_files(files: Vec<(PathBuf, FileKind)>) -> BuildResult {
    let output_files = files
        .into_iter()
        .map(|(path, kind)| OutputFile::new(path, kind))
        .collect();
    BuildResult {
        project: Project {
            output_files,
            entry_page_rel: Some(PathBuf::from("index.html")),
            cleanup_policy: CleanupPolicy::html(),
            warnings: Vec::new(),
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        },
        config: Config::new(PathBuf::from("main.moth")),
        warnings: Vec::new(),
        string_table: StringTable::new(),
        output_owner: OutputOwner {
            builder: BuilderKind::Html,
            profile: BuildProfile::Dev,
        },
        directory_output_plan: None,
    }
}

pub(super) fn build_result_with_index_html(html: &str) -> BuildResult {
    build_result_with_output_files(vec![(
        PathBuf::from("index.html"),
        FileKind::Html(html.to_owned()),
    )])
}

pub(super) fn acceptance_only_expectation() -> SuccessExpectation {
    SuccessExpectation {
        warnings: WarningExpectation::Forbid,
        success_contract: Some(SuccessContract::AcceptanceOnly),
        artifact_assertions: Vec::new(),
        golden: GoldenExpectation::default(),
        rendered_output: Default::default(),
        artifacts_must_not_exist: Vec::new(),
    }
}

pub(super) fn success_test_case(
    backend_id: BackendId,
    expectation: SuccessExpectation,
) -> TestCaseSpec {
    TestCaseSpec {
        display_name: "success-contract".to_string(),
        case_id: "success-contract".to_string(),
        manifest_relative_path: "success-contract".to_string(),
        fixture_root: PathBuf::from("."),
        tags: Vec::new(),
        contract: None,
        role: None,
        backend_id,
        entry_path: PathBuf::from("."),
        flags: Vec::new(),
        expected: ExpectedOutcome::Success(expectation),
    }
}
