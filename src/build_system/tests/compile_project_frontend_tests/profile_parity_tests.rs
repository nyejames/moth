//! Debug/release semantic parity through the full build service.
//!
//! WHAT: builds one project root twice through `build_project`, without and with `Flag::Release`,
//!       and compares the semantic facts the frontend hands to the backend: each module and
//!       generated sidecar's HIR (CFG, operations, numeric modes, failure and constraint side
//!       tables, call summaries, provenance), type environment, borrow report, numeric proofs,
//!       link facts, resources and entry metadata including folded const fragments; project facade
//!       and source-package facade interfaces; the text of every interned string; and successful
//!       build warnings. A diagnosed project compares every rendered diagnostic field instead.
//! WHY: the frontend takes no build profile, so profile independence is structural; `build_project`
//!      is the one place a profile exists before backend lowering. Comparing the backend handoff
//!      catches any future path that lets the profile reach semantic compilation.

use super::entry_api_stability_tests::interface_fingerprint_content;
use super::package_materialisation_tests::RegisteredPackagesHtmlBuilder;
use super::write_project_fixture;
use crate::build_system::BuildProfile;
use crate::build_system::backend_build_context::BackendBuildContext;
use crate::build_system::build::{
    BackendBuilder, BackendInvocationFailure, Project, ProjectCompilation, build_project,
};
use crate::builder_surface::BuilderSurface;
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_messages::DiagnosticDiscoveryStops;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilationOutcome;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet;
use crate::compiler_frontend::compiler_messages::render::{DiagnosticRenderContext, terminal};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::{Config, ProjectConfigError};
use std::cell::RefCell;
use std::convert::Infallible;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// HTML builder that records the semantic handoff before delegating backend lowering.
struct SemanticCaptureBuilder {
    html: RegisteredPackagesHtmlBuilder,
    snapshot: RefCell<Option<String>>,
}

impl BackendBuilder for SemanticCaptureBuilder {
    type Error = Infallible;

    fn frontend_surface(&self) -> BuilderSurface {
        self.html.frontend_surface()
    }

    fn validate_project_config(
        &self,
        config: &Config,
        strings: &mut DiagnosticStringBuilder<'_>,
    ) -> Result<(), ProjectConfigError> {
        self.html.validate_project_config(config, strings)
    }

    fn build_backend(
        &self,
        compilation: ProjectCompilation,
        config: &Config,
        build_profile: BuildProfile,
        flags: &[Flag],
        context: &mut BackendBuildContext<'_>,
    ) -> Result<Project, BackendInvocationFailure<Infallible>> {
        *self.snapshot.borrow_mut() = Some(semantic_snapshot(&compilation, context.strings()));
        self.html
            .build_backend(compilation, config, build_profile, flags, context)
    }
}

/// Pretty `Debug` keeps one fact per line, so the first differing line names the divergence.
///
/// The builder-provided external package registry is omitted: it is builder input, not a frontend
/// product.
fn semantic_snapshot(compilation: &ProjectCompilation, strings: &StringTable) -> String {
    let mut snapshot = String::new();
    for (package, artefact) in compilation.facade_artefacts() {
        let _ = writeln!(snapshot, "interface_owner: {package:?}");
        for (role, interface) in [
            ("normal", &artefact.normal_interface),
            ("support", &artefact.support_interface),
        ] {
            let interface = interface.as_ref().map(interface_fingerprint_content);
            let _ = writeln!(snapshot, "{role}_interface: {interface:#?}");
        }
    }
    for module in compilation.modules() {
        let executable = &module.executable;
        if !executable.hir.function_ids_by_generated.is_empty() {
            let _ = writeln!(
                snapshot,
                "generated: {:#?}",
                executable.hir.function_ids_by_generated
            );
        }
        let _ = writeln!(
            snapshot,
            "hir: {:#?}\ntypes: {:#?}\nborrows: {:#?}\nnumeric_proofs: {:#?}\nresources: {:#?}",
            executable.hir,
            executable.type_environment,
            executable.borrow_analysis,
            executable.numeric_proofs,
            executable.resource_table
        );
        let _ = writeln!(
            snapshot,
            "link_facts: {:#?}\nexternal_imports: {:#?}",
            module.link_facts.functions, module.link_facts.external_import_candidates
        );
        if let Some(entry) = &module.metadata.entry {
            let _ = writeln!(
                snapshot,
                "entry_point: {:?}\nroot_activity: {:#?}",
                entry.entry_point, entry.root_activity
            );
            for fragment in &entry.const_top_level_fragments {
                let _ = writeln!(
                    snapshot,
                    "const_fragment: {} {:?} {:?}\n{:#?}",
                    fragment.runtime_insertion_index,
                    fragment.destination,
                    fragment.span,
                    fragment.value
                );
            }
        }
    }
    for (_, text) in strings.iter() {
        let _ = writeln!(snapshot, "string: {text:?}");
    }
    snapshot
}

enum SemanticOutcome {
    Built(String),
    Diagnosed(String),
}

fn build_semantics(
    root: &Path,
    flags: &[Flag],
    packages: Vec<(String, PathBuf)>,
) -> SemanticOutcome {
    let builder = SemanticCaptureBuilder {
        html: RegisteredPackagesHtmlBuilder { packages },
        snapshot: RefCell::new(None),
    };
    let mut discovery_stops = DiagnosticDiscoveryStops::NONE;
    let build = build_project(
        &builder,
        root.to_str().expect("temporary path should be UTF-8"),
        flags,
        &BuildConfigInputSet::new(),
        &mut discovery_stops,
    )
    .unwrap_or_else(|never| match never {});
    let build_result = match build {
        Ok(CompilationOutcome::Success { value, .. }) => value,
        Ok(CompilationOutcome::Diagnosed(reports)) => {
            return SemanticOutcome::Diagnosed(rendered_diagnostics(&reports, root));
        }
        Err(errors) => panic!("the parity fixture must not fail infrastructure: {errors:?}"),
    };
    let mut snapshot = builder
        .snapshot
        .into_inner()
        .expect("a successful build must reach the backend");
    match build_result.finish_output(Ok(())) {
        Ok(CompilationOutcome::Success { warnings, .. }) => {
            if let Some(warnings) = warnings {
                snapshot.push_str("warnings:\n");
                snapshot.push_str(&rendered_diagnostics(&warnings, root));
            }
        }
        Ok(CompilationOutcome::Diagnosed(reports)) => {
            panic!(
                "finishing a successful build must not diagnose:\n{}",
                rendered_diagnostics(&reports, root)
            )
        }
        Err(errors) => panic!("finishing a successful build must not fail: {errors:?}"),
    }
    SemanticOutcome::Built(snapshot)
}

/// Render each record's code, reason, severity, title, root-relative locations and labels,
/// message and guidance.
fn rendered_diagnostics(reports: &DiagnosticReportSet, root: &Path) -> String {
    let mut rendered = String::new();
    for (report, view) in reports.iter() {
        let context = DiagnosticRenderContext::for_report(report);
        let descriptor = view.descriptor();
        let _ = writeln!(
            rendered,
            "code: {} | reason: {:?} | severity: {:?} | title: {}",
            descriptor.code,
            view.reason_key(),
            view.severity(),
            descriptor.title
        );
        for line in terminal::format_terminal_locations_for_test(view, context, root)
            .into_iter()
            .chain(terminal::format_guidance_view(view, context))
        {
            let _ = writeln!(rendered, "  {line}");
        }
    }
    rendered
}

/// Report the first differing line with its enclosing context rather than two multi-megabyte
/// snapshots.
fn assert_same_semantics(expected: &str, actual: &str) {
    let expected_lines = expected.lines().collect::<Vec<_>>();
    let actual_lines = actual.lines().collect::<Vec<_>>();
    let first_difference = (0..expected_lines.len().max(actual_lines.len()))
        .find(|&index| expected_lines.get(index) != actual_lines.get(index));
    if let Some(index) = first_difference {
        let context =
            expected_lines[index.saturating_sub(8)..index.min(expected_lines.len())].join("\n");
        panic!(
            "frontend semantics diverge at snapshot line {}:\n{context}\n  expected: {}\n  actual:   {}",
            index + 1,
            expected_lines.get(index).unwrap_or(&"<end of snapshot>"),
            actual_lines.get(index).unwrap_or(&"<end of snapshot>")
        );
    }
}

const CONFIG: &str = "project #= (\n    name = \"profile_parity\",\n)\nhtml #= ()\n";

/// Required const calls, a folded const fragment, static selection, authored-dead validation,
/// constraint evidence beside ordinary calls, imported and trusted Core calls and a generated
/// generic sidecar.
const ACCEPTED_ENTRY: &str = r#"@core/text length
@core/random random_int
@helpers scaled, show_value
@builder/test/parity from_package

LIMIT #= 4

doubled #|value I32| -> I32:
    return value * 2
;

divisor || -> I32:
    return 2
;

$infallible
literal_half |value I32| -> I32:
    return value // 2
;

guarded |value I32| -> I32:
    if false:
        assert(false)
    ;
    q I32 = value // divisor() catch then 0
    return q
;

folded #= doubled(LIMIT)
[: limit=[LIMIT] folded=[folded]]
x ~= scaled(3)
if folded is 8:
    ~x = x + 1
;
if x is:
    10 => ~x = x + 1
    else => ~x = 0
;
message String = show_value(divisor())
count = random_int(1, 3)
size = length(message)
package_value = from_package(divisor())
~[:folded=[folded] x=[x] count=[count] size=[size] half=[literal_half(10)] guarded=[guarded(6)] message=[message] package=[package_value]]
"#;

const HELPERS: &str = r#"$export
scaled |value I32| -> I32:
    return value * 3 catch then 0
;

$export
show_value type T is CASTABLE_TO_STRING |value T| -> String:
    return cast value
;
"#;

const SOURCE_PACKAGE: &str = r#"identity type T |value T| -> T:
    return value
;

$export
from_package |value I32| -> I32:
    if value is 0:
        return 1
    ;
    return identity(value)
;
"#;

/// Ordinary call results must not discharge `$infallible` evidence in either profile.
const REJECTED_ENTRY: &str = r#"divisor || -> I32:
    return 2
;

$infallible
probe_local |value I32| -> I32:
    d I32 = divisor()
    result I32 = value // d catch:
        assert(false)
    ;
    return result
;

$infallible
probe_direct |value I32| -> I32:
    return value // divisor()
;

~[:[probe_local(4)] [probe_direct(6)]]
"#;

fn built_snapshot(root: &Path, flags: &[Flag], package: &Path) -> String {
    let packages = vec![("builder/test/parity".to_owned(), package.to_path_buf())];
    match build_semantics(root, flags, packages) {
        SemanticOutcome::Built(snapshot) => snapshot,
        SemanticOutcome::Diagnosed(reports) => {
            panic!("the accepted parity fixture must build with {flags:?}:\n{reports}")
        }
    }
}

fn diagnosed_reports(root: &Path, flags: &[Flag]) -> String {
    match build_semantics(root, flags, Vec::new()) {
        SemanticOutcome::Diagnosed(reports) => reports,
        SemanticOutcome::Built(_) => {
            panic!("ordinary call results must not prove `$infallible` with {flags:?}")
        }
    }
}

#[test]
fn accepted_project_hands_identical_semantics_to_debug_and_release_backends() {
    let project = write_project_fixture(&[
        ("@config.moth", CONFIG),
        ("@entry.moth", ACCEPTED_ENTRY),
        ("helpers/@mod.moth", HELPERS),
    ]);

    let package = write_project_fixture(&[("@package.moth", SOURCE_PACKAGE)]);
    let dev = built_snapshot(project.path(), &[], package.path());
    let release = built_snapshot(project.path(), &[Flag::Release], package.path());
    assert_same_semantics(&dev, &release);

    // Capture the project and package's generated bodies, the source-package facade, and exact
    // interned scope names rather than normalizing away differences between builds.
    assert_eq!(
        dev.lines()
            .filter(|line| line.starts_with("generated: "))
            .count(),
        2,
        "expected one generated module in the project and one in the source package"
    );
    assert!(dev.contains(
        "interface_owner: Some(\"builder/test/parity\")\nnormal_interface: None\nsupport_interface: Some("
    ));
    assert!(dev.contains("const_fragment: "));
    assert!(dev.contains("string: \"$scope_"));

    let root = project.path();
    let package = package.path();
    let started = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let parallel = scope.spawn(|| {
            started.wait();
            built_snapshot(root, &[Flag::Release], package)
        });
        started.wait();
        let repeated = built_snapshot(root, &[], package);
        assert_same_semantics(&dev, &repeated);
        assert_same_semantics(
            &dev,
            &parallel.join().expect("parallel compilation panicked"),
        );
    });
}

#[test]
fn rejected_project_reports_identical_diagnostics_in_debug_and_release() {
    let project =
        write_project_fixture(&[("@config.moth", CONFIG), ("@entry.moth", REJECTED_ENTRY)]);

    let dev = diagnosed_reports(project.path(), &[]);
    let release = diagnosed_reports(project.path(), &[Flag::Release]);

    // `probe_local` traps through its handler's assertion; `probe_direct` lets the division's
    // recoverable failure escape.
    let records = dev
        .lines()
        .filter(|line| line.starts_with("code: "))
        .collect::<Vec<_>>();
    let reason_record = |reason: &str| {
        format!(
            "code: MOTH-RULE-0091 | reason: Some(\"invalid_source_constraint.{reason}\") | severity: Error | title: Source constraint not satisfied"
        )
    };
    assert_eq!(
        records,
        [
            reason_record("trap_assertion"),
            reason_record("escaping_error")
        ]
    );
    assert_same_semantics(&dev, &release);
}
