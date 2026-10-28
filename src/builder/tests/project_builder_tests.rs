//! Project builder driver tests.
//!
//! WHAT: runs `build_project` against real project folders with a recording test builder and checks
//! which trait method runs, how often, and which error reaches the caller.
//! WHY: the driver alone decides the order of definition, compilation and `build`, and which
//! failure wins, so those rules are pinned where the driver lives.

use crate::builder::{
    BuildArtifacts, BuildContext, BuildRequest, BuilderDefinition, CompiledProject, Error,
    ErrorKind, ProjectBuilder, build_project,
};
use crate::compiler_frontend::compiler_errors::{InfrastructureErrors, InfrastructureFailure};
use crate::compiler_frontend::compiler_messages::CompilerStage;
use std::cell::Cell;
#[cfg(unix)]
use std::ffi::OsString;
use std::fs;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
use std::path::Path;

/// Declares only the mandatory `project` section because the test builder adds no config sections.
const CONFIG_SOURCE: &str = "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\n";

/// The smallest page that compiles without diagnostics.
const PAGE_SOURCE: &str = "#[:<h1>About</h1>]\n";

/// A record literal that is never closed, so the frontend rejects the project before any builder
/// runs.
const UNTERMINATED_RECORD_PAGE_SOURCE: &str = "stats #= (a = 1\n";

/// Repeated literal patterns that make later arms unreachable. The frontend reports three warnings
/// and the project still builds.
const DUPLICATE_LITERAL_PAGE_SOURCE: &str = r#"value ~= "hello"
result ~= "unset"

if value is:
    "hello" => result = "one"
    "hello" => result = "two"
    "hello" => result = "three"
    "hello" => result = "four"
    else => result = "other"
;

[:pattern_unreachable_after_duplicate_literal_warning result=[result]]
"#;

const INVALID_DEFINITION_MESSAGE: &str = "The test definition is invalid.";
const BUILD_FAILURE_MESSAGE: &str = "The test builder failed.";

/// What the recording builder's `build` does once the driver reaches it.
enum BuildBehaviour {
    /// Returns empty artifacts.
    Succeed,
    /// Returns a builder-authored error that the driver must hand back unchanged.
    Fail,
}

/// Test builder that counts how often the driver calls each trait method.
struct RecordingBuilder {
    definition_failure: Option<&'static str>,
    build_behaviour: BuildBehaviour,
    definition_calls: Cell<usize>,
    build_calls: Cell<usize>,
}

impl RecordingBuilder {
    fn new(build_behaviour: BuildBehaviour) -> Self {
        Self {
            definition_failure: None,
            build_behaviour,
            definition_calls: Cell::new(0),
            build_calls: Cell::new(0),
        }
    }
}

impl ProjectBuilder for RecordingBuilder {
    fn definition(&self) -> Result<BuilderDefinition, Error> {
        self.definition_calls.set(self.definition_calls.get() + 1);
        match self.definition_failure {
            Some(message) => Err(Error::invalid_definition(message)),
            None => BuilderDefinition::new("game").finish(),
        }
    }

    fn build(
        &self,
        _project: &CompiledProject,
        _context: &mut BuildContext,
    ) -> Result<BuildArtifacts, Error> {
        self.build_calls.set(self.build_calls.get() + 1);
        match self.build_behaviour {
            BuildBehaviour::Succeed => Ok(BuildArtifacts::new()),
            BuildBehaviour::Fail => Err(Error::build(BUILD_FAILURE_MESSAGE)),
        }
    }
}

fn project_temp_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("should create project directory")
}

/// Writes a project whose `src/@page.moth` holds `page_source`.
fn write_project(project_root: &Path, page_source: &str) {
    let src = project_root.join("src");
    fs::create_dir_all(&src).expect("should create src");
    fs::write(project_root.join("config.moth"), CONFIG_SOURCE).expect("should write config");
    fs::write(src.join("@page.moth"), page_source).expect("should write @page.moth source");
}

#[test]
fn definition_error_is_returned_without_building() {
    let temp_dir = project_temp_dir();
    write_project(temp_dir.path(), PAGE_SOURCE);
    let builder = RecordingBuilder {
        definition_failure: Some(INVALID_DEFINITION_MESSAGE),
        ..RecordingBuilder::new(BuildBehaviour::Succeed)
    };

    let error = build_project(&builder, BuildRequest::new(temp_dir.path()))
        .expect_err("invalid definition should fail the build");

    assert_eq!(error, Error::invalid_definition(INVALID_DEFINITION_MESSAGE));
    assert_eq!(builder.definition_calls.get(), 1);
    assert_eq!(builder.build_calls.get(), 0);
}

#[test]
fn successful_build_returns_project_without_warnings() {
    let temp_dir = project_temp_dir();
    write_project(temp_dir.path(), PAGE_SOURCE);
    let builder = RecordingBuilder::new(BuildBehaviour::Succeed);

    let built = build_project(&builder, BuildRequest::new(temp_dir.path()))
        .expect("valid project should build");

    assert!(built.warnings().is_empty());
    assert_eq!(builder.definition_calls.get(), 1);
    assert_eq!(builder.build_calls.get(), 1);
}

#[test]
fn missing_project_root_returns_infrastructure_error_without_building() {
    let temp_dir = project_temp_dir();
    let project_root = temp_dir.path().join("missing-project");
    let builder = RecordingBuilder::new(BuildBehaviour::Succeed);

    let error = build_project(&builder, BuildRequest::new(&project_root))
        .expect_err("missing project root should fail the build");

    assert_eq!(error.kind(), ErrorKind::Infrastructure);
    assert_eq!(builder.build_calls.get(), 0);
}

#[test]
fn source_diagnostics_return_compilation_error_without_building() {
    let temp_dir = project_temp_dir();
    write_project(temp_dir.path(), UNTERMINATED_RECORD_PAGE_SOURCE);
    let builder = RecordingBuilder::new(BuildBehaviour::Succeed);

    let error = build_project(&builder, BuildRequest::new(temp_dir.path()))
        .expect_err("source with diagnostics should fail the build");

    assert_eq!(error.kind(), ErrorKind::Compilation);
    assert!(
        error
            .messages()
            .iter()
            .any(|message| { message.split('|').nth(2) == Some("@page.moth") }),
        "diagnosed source must retain its authored path"
    );
    assert_eq!(builder.build_calls.get(), 0);
}

#[test]
fn builder_error_is_returned_unchanged() {
    let temp_dir = project_temp_dir();
    write_project(temp_dir.path(), PAGE_SOURCE);
    let builder = RecordingBuilder::new(BuildBehaviour::Fail);

    let error = build_project(&builder, BuildRequest::new(temp_dir.path()))
        .expect_err("builder failure should fail the build");

    assert_eq!(error, Error::build(BUILD_FAILURE_MESSAGE));
    assert_eq!(builder.build_calls.get(), 1);
}

#[test]
fn warnings_are_returned_as_rendered_lines() {
    let temp_dir = project_temp_dir();
    write_project(temp_dir.path(), DUPLICATE_LITERAL_PAGE_SOURCE);
    let builder = RecordingBuilder::new(BuildBehaviour::Succeed);

    let built = build_project(&builder, BuildRequest::new(temp_dir.path()))
        .expect("warning-only project should build");

    assert_eq!(
        built.warnings().len(),
        3,
        "each warning should render as one terse line"
    );
    assert!(
        built.warnings().iter().all(|warning| {
            warning.starts_with("W|MOTH-RULE-0022|")
                && warning.split('|').nth(2) == Some("@page.moth")
        }),
        "frozen warnings must retain their stable code and authored source path"
    );
    assert_eq!(builder.build_calls.get(), 1);
}

#[cfg(unix)]
#[test]
fn non_utf8_project_root_returns_infrastructure_error_without_building() {
    let project_root = OsString::from_vec(b"safe-\xFF-root".to_vec());
    let builder = RecordingBuilder::new(BuildBehaviour::Succeed);

    let error = build_project(&builder, BuildRequest::new(project_root))
        .expect_err("non-UTF-8 project root should fail the build");

    assert_eq!(error.kind(), ErrorKind::Infrastructure);
    assert!(
        error.to_string().contains("safe-"),
        "error should name the rejected root"
    );
    assert_eq!(builder.build_calls.get(), 0);
}

#[test]
fn infrastructure_error_kind_uses_modern_failure_classification() {
    let compiler_detail = "typed internal failure detail";
    let compiler_error = Error::from_infrastructure_errors(
        InfrastructureFailure::internal(CompilerStage::ProjectBuildAssembly, compiler_detail)
            .into(),
    );
    assert_eq!(compiler_error.kind(), ErrorKind::CompilerBug);
    assert!(
        compiler_error.messages()[0].contains(compiler_detail),
        "terse compiler-bug reporting should retain the infrastructure cause"
    );

    let operational_detail = "typed filesystem failure detail";
    let operational_error = Error::from_infrastructure_errors(
        InfrastructureFailure::filesystem(
            CompilerStage::ProjectBuildAssembly,
            Path::new("project"),
            operational_detail,
        )
        .into(),
    );
    assert_eq!(operational_error.kind(), ErrorKind::Infrastructure);
    assert!(
        operational_error.messages()[0].contains(operational_detail),
        "terse operational reporting should retain the infrastructure cause"
    );

    let mut mixed_errors = InfrastructureErrors::new(InfrastructureFailure::filesystem(
        CompilerStage::ProjectBuildAssembly,
        Path::new("project"),
        operational_detail,
    ));
    mixed_errors.push(InfrastructureFailure::internal(
        CompilerStage::ProjectBuildAssembly,
        compiler_detail,
    ));
    let mixed_error = Error::from_infrastructure_errors(mixed_errors);
    assert_eq!(mixed_error.kind(), ErrorKind::CompilerBug);
    assert_eq!(mixed_error.messages().len(), 1);
    let record = &mixed_error.messages()[0];
    let operational_position = record
        .find(operational_detail)
        .expect("the operational primary cause must survive SDK conversion");
    let compiler_position = record
        .find(compiler_detail)
        .expect("the additional compiler cause must survive SDK conversion");
    assert!(operational_position < compiler_position);
}
