//! Boundary numeric-profile threading tests.
//!
//! WHAT: drives the real build bootstrap and frontend entry points with a test builder that selects
//!       each of the four `NumericProfile` combinations, then asserts profile-sensitive outcomes:
//!       a boundary `Int` config constant folds only under `Int64`, and boundary `Int`/`Float`
//!       module literals report the out-of-range and non-finite diagnostics exactly under the
//!       profiles that reject those spellings.
//! WHY: the profile is one boundary-wide fact selected by the builder. These tests compile source
//!      whose result depends on the profile through the real builder path, so any stage after the
//!      builder handoff that falls back to `STANDARD` fails here instead of silently typing numbers
//!      under widths no builder selected.

use super::*;
use crate::build_system::build::{
    ProjectBuilder, ProjectCompilation, bootstrap_project_build, build_project,
};
use crate::build_system::create_project_modules::{
    FrontendCompilationMode, ProjectFrontendCompilation, compile_project_frontend_with_inputs,
};
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_messages::{DiagnosticPayload, NumberLiteralErrorReason};
use crate::compiler_frontend::datatypes::numeric_profile::{
    FloatPrecision, IntWidth, NumericProfile,
};
use crate::projects::settings::CONFIG_FILE_NAME;
use std::fs;
use std::path::PathBuf;

/// Builder double that selects one boundary numeric profile and delegates everything else.
struct ProfileSelectingBuilder {
    numeric_profile: NumericProfile,
}

impl BackendBuilder for ProfileSelectingBuilder {
    fn numeric_profile(&self) -> NumericProfile {
        self.numeric_profile
    }

    fn build_backend(
        &self,
        project_compilation: ProjectCompilation,
        _config: &Config,
        _build_profile: BuildProfile,
        _flags: &[Flag],
        _string_table: &mut StringTable,
    ) -> Result<Project, CompilerMessages> {
        // WHAT: the double only builds when the compilation carries the profile it selected.
        // WHY: the backend is the last consumer of the boundary profile, so a handoff that
        //      re-selected a default instead of the settled value surfaces here.
        assert_eq!(
            project_compilation.numeric_profile(),
            self.numeric_profile,
            "the backend must receive the builder-selected profile on the project compilation",
        );
        Ok(Project {
            output_files: Vec::new(),
            entry_page_rel: None,
            cleanup_policy: CleanupPolicy::generic(Vec::<&str>::new()),
            warnings: Vec::new(),
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        })
    }

    fn validate_project_config(
        &self,
        _config: &Config,
        _string_table: &mut StringTable,
    ) -> Result<(), ProjectConfigError> {
        Ok(())
    }

    fn frontend_style_directives(&self) -> Vec<StyleDirectiveSpec> {
        Vec::new()
    }

    fn frontend_surface(&self) -> BuilderSurface {
        BuilderSurface::with_mandatory_core()
    }
}

/// Every profile combination a builder can select.
///
/// Each row is distinguished by its own boundary behaviour below: the `Int32` rows reject the
/// boundary `Int` literal and the `Float32` rows reject the boundary `Float` literal, so no two
/// rows share an outcome.
const PROFILES: [NumericProfile; 4] = [
    NumericProfile {
        int_width: IntWidth::Bits32,
        float_precision: FloatPrecision::Bits64,
    },
    NumericProfile {
        int_width: IntWidth::Bits32,
        float_precision: FloatPrecision::Bits32,
    },
    NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: FloatPrecision::Bits64,
    },
    NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: FloatPrecision::Bits32,
    },
];

/// Valid project config; the module lane compiles boundary literals against this config so a
/// config diagnostic can never mask a module outcome.
fn valid_project_config_source() -> String {
    "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n".to_owned()
}

/// Project config with a private `Int` constant at the `Int32` boundary.
///
/// WHY: unregistered top-level constants stay folded as private helpers without becoming
///      settings, so this constant proves the config service folds under the builder-selected
///      profile: `Int32` rows reject it while `Int64` rows fold it.
fn boundary_int_config_source() -> String {
    "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\nlimit #= 3_000_000_000\n"
        .to_owned()
}

/// Module source holding only the `Int` literal above the `Int32` maximum.
///
/// WHY: one source per boundary literal. A module reports only its first failing declaration, so a
///      single source holding both literals could never show both dimensions at once.
const INT_BOUNDARY_MODULE_SOURCE: &str = "wide = 3_000_000_000\n";

/// Module source holding only the `Float` literal above the `Float32` maximum finite value.
const FLOAT_BOUNDARY_MODULE_SOURCE: &str = "huge = 1e39\n";

/// True when the retained module diagnostics report an invalid-number literal `reason`.
fn has_invalid_number_reason(
    frontend: &ProjectFrontendCompilation,
    reason: NumberLiteralErrorReason,
) -> bool {
    frontend
        .project
        .diagnosed
        .iter()
        .flat_map(|module| module.diagnostics.diagnostics())
        .any(|diagnostic| {
            matches!(
                &diagnostic.payload,
                DiagnosticPayload::InvalidNumberLiteral {
                    reason: actual,
                    ..
                } if *actual == reason
            )
        })
}

/// Bootstrap one entry root and canonically compile it with the profile the bootstrap carries.
///
/// WHAT: returns the profile the bootstrap settled beside the retained frontend outcome.
/// WHY: every module-lane assertion needs the profile the production bootstrap produced rather
///      than the profile this test passed in, so a bootstrap that re-selected `STANDARD` is
///      visible in the assertion message instead of hiding behind the compiled source.
fn compile_entry_root(
    profile: NumericProfile,
    entry_path: PathBuf,
) -> (NumericProfile, ProjectFrontendCompilation) {
    let builder = ProjectBuilder::new(Box::new(ProfileSelectingBuilder {
        numeric_profile: profile,
    }));
    let mut bootstrap = bootstrap_project_build(&builder, entry_path, &BuildConfigInputSet::new())
        .expect("the entry root should bootstrap under every profile");
    let mut project_source_files = None;
    let frontend = compile_project_frontend_with_inputs(
        &mut bootstrap.config,
        BuildProfile::Dev,
        bootstrap.numeric_profile,
        None,
        &bootstrap.style_directives,
        &mut bootstrap.frontend_surface,
        &mut bootstrap.string_table,
        &mut project_source_files,
        &BuildConfigInputSet::new(),
        FrontendCompilationMode::Canonical,
    )
    .expect("frontend compilation should retain its outcome");
    (bootstrap.numeric_profile, frontend)
}

/// Assert one boundary literal's outcome: `reason` is reported exactly when the profile rejects
/// that literal's spelling.
fn assert_literal_boundary_outcome(
    label: &str,
    frontend: &ProjectFrontendCompilation,
    reason: NumberLiteralErrorReason,
    expected: bool,
) {
    assert_eq!(
        has_invalid_number_reason(frontend, reason),
        expected,
        "{label} should {}report {reason:?}",
        if expected { "" } else { "not " },
    );
}

#[test]
fn builder_profile_reaches_config_and_directory_module_services() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();

    for profile in PROFILES {
        let label = format!("directory {profile}");

        // Config lane: the boundary `Int` config constant folds only under `Int64`.
        {
            let temporary_directory =
                tempfile::tempdir().expect("should create temporary directory");
            let project_root = temporary_directory.path().to_path_buf();
            fs::create_dir_all(project_root.join("src"))
                .expect("should create project source root");
            fs::write(
                project_root.join(CONFIG_FILE_NAME),
                boundary_int_config_source(),
            )
            .expect("should write config");
            fs::write(project_root.join("src").join("@mod.moth"), "value = 1\n")
                .expect("should write module");

            let builder = ProjectBuilder::new(Box::new(ProfileSelectingBuilder {
                numeric_profile: profile,
            }));
            match bootstrap_project_build(&builder, project_root, &BuildConfigInputSet::new()) {
                Ok(_) => assert_eq!(
                    profile.int_width,
                    IntWidth::Bits64,
                    "{label} boundary Int config constant should fold only under Int64",
                ),
                Err(messages) => {
                    assert_eq!(
                        profile.int_width,
                        IntWidth::Bits32,
                        "{label} boundary Int config constant should diagnose under Int32",
                    );
                    assert!(
                        messages.diagnostics().any(|diagnostic| matches!(
                            &diagnostic.payload,
                            DiagnosticPayload::InvalidNumberLiteral {
                                reason: NumberLiteralErrorReason::OutsideIntRange,
                                ..
                            }
                        )),
                        "{label} Int32 config lane should report the out-of-range literal",
                    );
                }
            }
        }

        // Module lane: one module per boundary literal, so both dimensions of the row are
        // observed independently through canonical directory compilation.
        {
            let temporary_directory =
                tempfile::tempdir().expect("should create temporary directory");
            let project_root = temporary_directory.path().to_path_buf();
            let wide_root = project_root.join("src").join("wide");
            let huge_root = project_root.join("src").join("huge");
            fs::create_dir_all(&wide_root).expect("should create the Int boundary module root");
            fs::create_dir_all(&huge_root).expect("should create the Float boundary module root");
            fs::write(
                project_root.join(CONFIG_FILE_NAME),
                valid_project_config_source(),
            )
            .expect("should write config");
            fs::write(wide_root.join("@mod.moth"), INT_BOUNDARY_MODULE_SOURCE)
                .expect("should write the Int boundary module");
            fs::write(huge_root.join("@mod.moth"), FLOAT_BOUNDARY_MODULE_SOURCE)
                .expect("should write the Float boundary module");

            let (bootstrap_profile, frontend) = compile_entry_root(profile, project_root);
            assert_eq!(
                bootstrap_profile, profile,
                "{label} bootstrap should carry the builder-selected profile",
            );
            let expects_int_error = profile.int_width == IntWidth::Bits32;
            let expects_float_error = profile.float_precision == FloatPrecision::Bits32;
            assert_literal_boundary_outcome(
                &format!("{label} Int module"),
                &frontend,
                NumberLiteralErrorReason::OutsideIntRange,
                expects_int_error,
            );
            assert_literal_boundary_outcome(
                &format!("{label} Float module"),
                &frontend,
                NumberLiteralErrorReason::NonFiniteFloat,
                expects_float_error,
            );
            assert_eq!(
                frontend.has_diagnosed_or_blocked(),
                expects_int_error || expects_float_error,
                "{label} boundary modules should diagnose exactly under the profiles that reject them",
            );
        }
    }
}

#[test]
fn builder_profile_reaches_single_file_module_service() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();

    for profile in PROFILES {
        let label = format!("single file {profile}");
        for (literal_label, source, reason, expected) in [
            (
                "Int literal",
                INT_BOUNDARY_MODULE_SOURCE,
                NumberLiteralErrorReason::OutsideIntRange,
                profile.int_width == IntWidth::Bits32,
            ),
            (
                "Float literal",
                FLOAT_BOUNDARY_MODULE_SOURCE,
                NumberLiteralErrorReason::NonFiniteFloat,
                profile.float_precision == FloatPrecision::Bits32,
            ),
        ] {
            let temporary_directory =
                tempfile::tempdir().expect("should create temporary directory");
            let entry_path = temporary_directory.path().join("entry.moth");
            fs::write(&entry_path, source).expect("should write entry source");

            let (bootstrap_profile, frontend) = compile_entry_root(profile, entry_path);
            assert_eq!(
                bootstrap_profile, profile,
                "{label} bootstrap should carry the builder-selected profile",
            );
            assert_literal_boundary_outcome(
                &format!("{label} {literal_label}"),
                &frontend,
                reason,
                expected,
            );
            assert_eq!(
                frontend.has_diagnosed_or_blocked(),
                expected,
                "{label} {literal_label} should diagnose exactly when the profile rejects it",
            );
        }
    }
}

#[test]
fn builder_profile_reaches_the_backend_through_project_compilation() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();

    for profile in PROFILES {
        let temporary_directory = tempfile::tempdir().expect("should create temporary directory");
        let project_root = temporary_directory.path().to_path_buf();
        let source_root = project_root.join("src");
        fs::create_dir_all(&source_root).expect("should create project source root");
        fs::write(
            project_root.join(CONFIG_FILE_NAME),
            valid_project_config_source(),
        )
        .expect("should write config");
        fs::write(source_root.join("@mod.moth"), "value = 1\n").expect("should write module");

        let builder = ProjectBuilder::new(Box::new(ProfileSelectingBuilder {
            numeric_profile: profile,
        }));

        // `build_project` is the production handoff under test: it assembles the compilation the
        // builder receives, and the double asserts the profile it finds there is the row's.
        let build_result = build_project(
            &builder,
            &project_root.to_string_lossy(),
            &[],
            &BuildConfigInputSet::new(),
        );
        assert!(
            build_result.is_ok(),
            "directory project compiled under {profile} should build to completion",
        );
    }
}
