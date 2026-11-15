//! Acceptance test: a crate outside the compiler runs the whole builder workflow.
//!
//! WHAT: compiles the committed Moth project fixture with `GameBuilder`, publishes its artefact and
//! observes the typed native-Wasm capability error.
//! WHY: the milestone is complete only when a real external crate runs the workflow end to end.
//! Each step asserts on an observable result, so broken package registration, configuration,
//! compilation, capability reporting or output publication fails here.

use moth::builder::{BuildRequest, ErrorKind, build_project};
use moth_builder_fixture::{GameBuilder, STATUS_ARTEFACT};
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The Moth source package directory the fixture builder installs.
const SOURCE_PACKAGE_DIR: &str = "moth/game";

/// The project fixture directory, copied out so publication never writes into this crate.
const PROJECT_FIXTURE_DIR: &str = "moth/project";

/// The project fixture files the build reads. A facade-only project is a valid SDK target.
const PROJECT_FILES: &[&str] = &["@config.moth", "@package.moth"];

/// The exact artefact the fixture publishes while native Wasm compilation is unavailable.
const EXPECTED_STATUS_ARTEFACT: &str = concat!(
    "native_wasm = unavailable\n",
    "title = native handoff\n",
    "cause = Native Wasm compilation of a project package is not available in this compiler yet.\n",
);

fn fixture_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// A unique per-invocation copy of the committed project fixture.
///
/// WHAT: copies the fixture files under Cargo's target temporary directory and removes the copy
/// when the value drops, including on panic.
/// WHY: two test processes can share one target directory, so a fixed path would race. The
/// process id, clock value and in-process counter keep each copy distinct without a new
/// dependency.
struct ProjectFixture {
    root: PathBuf,
}

impl ProjectFixture {
    fn create() -> Self {
        static NEXT_COPY: AtomicU64 = AtomicU64::new(0);
        let unique = NEXT_COPY.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock should be after the Unix epoch")
            .as_nanos();
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "native-handoff-project-{}-{nanos}-{unique}",
            process::id()
        ));
        // Construct the guard before any fallible filesystem work, so a failed copy still removes
        // the partial directory.
        let fixture = Self { root };
        fs::create_dir_all(fixture.path()).expect("should create the fixture project copy");

        for file in PROJECT_FILES {
            fs::copy(
                fixture_path(PROJECT_FIXTURE_DIR).join(file),
                fixture.path().join(file),
            )
            .unwrap_or_else(|error| panic!("should copy fixture file '{file}': {error}"));
        }

        fixture
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for ProjectFixture {
    fn drop(&mut self) {
        // A failed cleanup must not turn a test result into a double panic.
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn external_builder_compiles_the_fixture_publishes_an_artefact_and_reports_the_missing_wasm_capability()
 {
    let project_root = ProjectFixture::create();
    let builder = GameBuilder::new(fixture_path(SOURCE_PACKAGE_DIR));

    let built = build_project(&builder, BuildRequest::new(project_root.path()))
        .expect("the fixture project should compile through the public SDK");
    let outputs = built
        .write_outputs()
        .expect("the fixture builder's artefact should publish");

    let observation = builder.observation();
    assert_eq!(
        observation.title.as_deref(),
        Some("native handoff"),
        "the builder must read the project's authored section value"
    );
    assert_eq!(
        observation.debug_overlay,
        Some(true),
        "the builder must read the project's authored flag"
    );

    let error = observation
        .wasm_error
        .expect("the builder must reach the native Wasm request");
    assert_eq!(error.kind(), ErrorKind::UnsupportedCapability);
    let rendered = error.to_string();
    assert!(
        rendered.contains("Wasm compilation of a project package"),
        "the capability error must name the missing capability: {rendered}"
    );
    assert!(
        rendered.contains("not available in this compiler yet"),
        "the capability error must state the capability is unavailable: {rendered}"
    );

    assert_eq!(outputs.output_root(), project_root.path().join("_dev"));
    assert_eq!(outputs.written_paths(), &[PathBuf::from(STATUS_ARTEFACT)]);
    assert!(
        outputs.warnings().is_empty(),
        "the fixture project should compile without warnings: {:?}",
        outputs.warnings()
    );
    let published = fs::read(project_root.path().join("_dev").join(STATUS_ARTEFACT))
        .expect("the published artefact should exist");
    assert_eq!(
        published,
        EXPECTED_STATUS_ARTEFACT.as_bytes(),
        "the published artefact must record the capability gap, the rendered cause and the \
         project's authored title"
    );
}
