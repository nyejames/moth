use super::*;
use crate::benchmark_workspace::BenchmarkExecutionWorkspace;
use std::fs::{self, File};
use tempfile::tempdir;
fn write_manifest(repository_root: &Path, contents: &str) -> PathBuf {
    let manifest_path = repository_root.join("manifest.toml");
    fs::write(&manifest_path, contents).expect("manifest should be writable");
    manifest_path
}

fn create_entry(repository_root: &Path, relative_path: &str) {
    let path = repository_root.join(relative_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("entry parent should be creatable");
    }
    File::create(path).expect("entry should be creatable");
}

fn create_directory_entry(repository_root: &Path, relative_path: &str) {
    let path = repository_root.join(relative_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("entry parent should be creatable");
    }
    fs::create_dir_all(path).expect("directory entry should be creatable");
}

fn minimal_manifest(entry: &str, case_id: &str) -> String {
    format!(
        r#"schema = 3

[[workload]]
id = "workload"
entry = "{entry}"
fingerprint_mode = "full_tree"
fingerprint_roots = ["{entry}"]
fingerprint_excludes = []

[[case]]
id = "{case_id}"
workload = "workload"
group = "core"
quick = false
expectation = "clean"

[case.runner]
kind = "cli"
command = "check"
args = []
"#
    )
}

fn two_workload_manifest(first_entry: &str, second_entry: &str) -> String {
    format!(
        r#"schema = 3

[[workload]]
id = "first_workload"
entry = "{first_entry}"
fingerprint_mode = "full_tree"
fingerprint_roots = ["{first_entry}"]
fingerprint_excludes = []

[[workload]]
id = "second_workload"
entry = "{second_entry}"
fingerprint_mode = "full_tree"
fingerprint_roots = ["{second_entry}"]
fingerprint_excludes = []

[[case]]
id = "first_case"
workload = "first_workload"
group = "core"
quick = false
expectation = "clean"
[case.runner]
kind = "cli"
command = "check"
args = []

[[case]]
id = "second_case"
workload = "second_workload"
group = "core"
quick = false
expectation = "clean"
[case.runner]
kind = "cli"
command = "build"
args = []
"#
    )
}

fn directory_build_manifest(entry: &str, roots: &[&str], excludes: &[&str]) -> String {
    let roots_str = roots
        .iter()
        .map(|root| format!("\"{root}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let excludes_str = excludes
        .iter()
        .map(|exclude| format!("\"{exclude}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r#"schema = 3

[[workload]]
id = "workload"
entry = "{entry}"
fingerprint_mode = "full_tree"
fingerprint_roots = ["{entry}"]
fingerprint_excludes = [{excludes_str}]
generated_output_roots = [{roots_str}]

[[case]]
id = "build_case"
workload = "workload"
group = "core"
quick = false
expectation = "clean"

[case.runner]
kind = "cli"
command = "build"
args = []
"#
    )
}

#[test]
fn directory_build_workload_with_declared_roots_loads() {
    let directory = tempdir().expect("temporary repository should exist");
    create_directory_entry(directory.path(), "project");

    let path = write_manifest(
        directory.path(),
        &directory_build_manifest(
            "project",
            &["dev", "release"],
            &["project/dev", "project/release"],
        ),
    );
    let manifest = load_manifest_at(&path, directory.path()).expect("manifest should load");

    assert_eq!(
        manifest.workloads[0].generated_output_roots,
        vec![PathBuf::from("dev"), PathBuf::from("release")]
    );
}

#[test]
fn unknown_group_fails_manifest_loading() {
    let directory = tempdir().expect("temporary repository should exist");
    create_entry(directory.path(), "fixture.moth");

    let contents = r#"schema = 3

[[workload]]
id = "workload"
entry = "fixture.moth"
fingerprint_mode = "full_tree"
fingerprint_roots = ["fixture.moth"]
fingerprint_excludes = []

[[case]]
id = "case"
workload = "workload"
group = "bogus"
quick = false
expectation = "clean"

[case.runner]
kind = "cli"
command = "check"
args = []
"#;
    let path = write_manifest(directory.path(), contents);
    let error = load_manifest_at(&path, directory.path())
        .expect_err("an unknown group must fail manifest loading");
    assert!(error.to_string().contains("unknown group 'bogus'"));
}

#[test]
fn file_workload_with_generated_roots_fails() {
    let directory = tempdir().expect("temporary repository should exist");
    create_entry(directory.path(), "fixture.moth");

    let path = write_manifest(
        directory.path(),
        &directory_build_manifest("fixture.moth", &["dev"], &[]),
    );
    let error = load_manifest_at(&path, directory.path())
        .expect_err("file workloads must not declare generated output roots");
    assert!(error.to_string().contains("file workloads may not declare"));
}

#[test]
fn directory_build_workload_without_roots_fails() {
    let directory = tempdir().expect("temporary repository should exist");
    create_directory_entry(directory.path(), "project");

    let path = write_manifest(
        directory.path(),
        &directory_build_manifest("project", &[], &[]),
    );
    let error = load_manifest_at(&path, directory.path())
        .expect_err("directory build workloads must declare at least one root");
    assert!(
        error
            .to_string()
            .contains("must declare at least one generated output root")
    );
}

#[test]
fn generated_root_without_matching_exclude_fails() {
    let directory = tempdir().expect("temporary repository should exist");
    create_directory_entry(directory.path(), "project");

    let path = write_manifest(
        directory.path(),
        &directory_build_manifest("project", &["dev"], &[]),
    );
    let error = load_manifest_at(&path, directory.path())
        .expect_err("a generated root must be covered by an explicit fingerprint exclude");
    assert!(error.to_string().contains("explicit fingerprint exclude"));
}

#[test]
fn duplicate_generated_roots_fail() {
    let directory = tempdir().expect("temporary repository should exist");
    create_directory_entry(directory.path(), "project");

    let path = write_manifest(
        directory.path(),
        &directory_build_manifest("project", &["dev", "dev"], &["project/dev"]),
    );
    let error =
        load_manifest_at(&path, directory.path()).expect_err("duplicate generated roots must fail");
    assert!(
        error
            .to_string()
            .contains("duplicate generated output root")
    );
}

#[test]
fn ascii_case_colliding_generated_roots_fail() {
    let directory = tempdir().expect("temporary repository should exist");
    create_directory_entry(directory.path(), "project");

    let path = write_manifest(
        directory.path(),
        &directory_build_manifest("project", &["dev", "DEV"], &["project/dev"]),
    );
    let error = load_manifest_at(&path, directory.path())
        .expect_err("ASCII-case-colliding generated roots must fail");
    assert!(error.to_string().contains("only by ASCII case"));
}

#[test]
fn overlapping_generated_roots_fail() {
    let directory = tempdir().expect("temporary repository should exist");
    create_directory_entry(directory.path(), "project");

    let path = write_manifest(
        directory.path(),
        &directory_build_manifest(
            "project",
            &["dev", "dev/sub"],
            &["project/dev", "project/dev/sub"],
        ),
    );
    let error = load_manifest_at(&path, directory.path())
        .expect_err("overlapping generated roots must fail");
    assert!(error.to_string().contains("must not overlap"));
}

#[test]
fn case_insensitive_duplicate_generated_roots_fail() {
    let directory = tempdir().expect("temporary repository should exist");
    create_directory_entry(directory.path(), "project");

    let path = write_manifest(
        directory.path(),
        &directory_build_manifest("project", &["Dev", "dev"], &["project/Dev"]),
    );
    let error = load_manifest_at(&path, directory.path())
        .expect_err("ASCII-case duplicate generated roots must fail");
    assert!(error.to_string().contains("only by ASCII case"));
}

#[test]
fn case_insensitive_ancestor_overlap_generated_roots_fail() {
    let directory = tempdir().expect("temporary repository should exist");
    create_directory_entry(directory.path(), "project");

    let path = write_manifest(
        directory.path(),
        &directory_build_manifest(
            "project",
            &["Dev", "dev/assets"],
            &["project/Dev", "project/dev/assets"],
        ),
    );
    let error = load_manifest_at(&path, directory.path())
        .expect_err("case-insensitive overlapping generated roots must fail");
    assert!(error.to_string().contains("must not overlap"));
}

#[test]
fn case_insensitive_ancestor_overlap_with_suffix_fails() {
    let directory = tempdir().expect("temporary repository should exist");
    create_directory_entry(directory.path(), "project");

    let path = write_manifest(
        directory.path(),
        &directory_build_manifest(
            "project",
            &["output/assets", "OUTPUT"],
            &["project/output/assets", "project/OUTPUT"],
        ),
    );
    let error = load_manifest_at(&path, directory.path())
        .expect_err("case-insensitive ancestor overlap must fail");
    assert!(error.to_string().contains("must not overlap"));
}

#[test]
fn non_overlapping_case_distinct_generated_roots_load() {
    let directory = tempdir().expect("temporary repository should exist");
    create_directory_entry(directory.path(), "project");

    let path = write_manifest(
        directory.path(),
        &directory_build_manifest(
            "project",
            &["dev", "release"],
            &["project/dev", "project/release"],
        ),
    );
    let manifest =
        load_manifest_at(&path, directory.path()).expect("distinct generated roots should load");
    assert_eq!(
        manifest.workloads[0].generated_output_roots,
        vec![PathBuf::from("dev"), PathBuf::from("release")]
    );
}

#[test]
#[cfg(unix)]
fn symlink_generated_root_fails() {
    let directory = tempdir().expect("temporary repository should exist");
    let entry_path = directory.path().join("project");
    std::fs::create_dir_all(&entry_path).expect("project directory should be creatable");
    std::fs::create_dir_all(entry_path.join("real-dev"))
        .expect("real output directory should be creatable");
    std::os::unix::fs::symlink(entry_path.join("real-dev"), entry_path.join("dev"))
        .expect("symlink should be creatable");

    let path = write_manifest(
        directory.path(),
        &directory_build_manifest("project", &["dev"], &["project/dev"]),
    );
    let error =
        load_manifest_at(&path, directory.path()).expect_err("a symlink generated root must fail");
    assert!(error.to_string().contains("must not be a symlink"));
}

#[test]
fn valid_manifest_preserves_source_order_and_resolves_workloads() {
    let directory = tempdir().expect("temporary repository should exist");
    create_entry(directory.path(), "first.moth");
    create_entry(directory.path(), "second.moth");
    let contents = r#"schema = 3

[[workload]]
id = "first_workload"
entry = "first.moth"
fingerprint_mode = "full_tree"
fingerprint_roots = ["first.moth"]
fingerprint_excludes = []

[[workload]]
id = "second_workload"
entry = "second.moth"
fingerprint_mode = "full_tree"
fingerprint_roots = ["second.moth"]
fingerprint_excludes = []

[[case]]
id = "first_case"
workload = "first_workload"
group = "core"
quick = false
expectation = "clean"
[case.runner]
kind = "cli"
command = "check"
args = []

[[case]]
id = "second_case"
workload = "second_workload"
group = "docs"
quick = true
expectation = "clean"
[case.runner]
kind = "frontend"
profile = "dev"
"#;
    let path = write_manifest(directory.path(), contents);
    let manifest = load_manifest_at(&path, directory.path()).expect("manifest should load");

    assert_eq!(
        manifest
            .cases
            .iter()
            .map(|case| case.id.as_str())
            .collect::<Vec<_>>(),
        ["first_case", "second_case"]
    );
    assert_eq!(manifest.cases[0].workload_index, 0);
    assert_eq!(manifest.cases[1].workload_index, 1);

    let canonical_root =
        fs::canonicalize(directory.path()).expect("repository root should canonicalise");
    let workspace = BenchmarkExecutionWorkspace::create(&canonical_root)
        .expect("workspace should be creatable");
    let cli_invocation = workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect("CLI invocation should resolve");
    assert!(cli_invocation.args[0].ends_with("first.moth"));
    assert!(cli_invocation.current_directory.ends_with("first_case"));
    assert!(
        cli_invocation
            .current_directory
            .starts_with(&canonical_root)
    );
    assert_eq!(
        manifest
            .frontend_invocation(&manifest.cases[1])
            .expect("frontend invocation")
            .entry,
        fs::canonicalize(directory.path())
            .expect("repository root should canonicalise")
            .join("second.moth")
    );
    assert_eq!(
        manifest.repository_root,
        fs::canonicalize(directory.path()).expect("repository root should canonicalise")
    );
}

#[test]
fn nested_start_directory_resolves_all_invocations_from_repository_root() {
    let directory = tempdir().expect("temporary repository should exist");
    create_entry(directory.path(), "fixture.moth");
    fs::create_dir_all(directory.path().join("tools/nested"))
        .expect("nested directory should be creatable");
    let contents = format!(
        "{}\n[[case]]\nid = \"frontend_case\"\nworkload = \"workload\"\ngroup = \"core\"\nquick = false\nexpectation = \"clean\"\n[case.runner]\nkind = \"frontend\"\nprofile = \"dev\"\n",
        minimal_manifest("fixture.moth", "cli_case")
    );
    let manifest_directory = directory.path().join("benchmarks");
    fs::create_dir(&manifest_directory).expect("manifest directory should be creatable");
    fs::write(manifest_directory.join("manifest.toml"), contents)
        .expect("manifest should be writable");

    let manifest = load_benchmark_manifest_from(&directory.path().join("tools/nested"))
        .expect("nested invocation should find the repository manifest");
    let canonical_root =
        fs::canonicalize(directory.path()).expect("repository root should canonicalise");
    let workspace = BenchmarkExecutionWorkspace::create(&canonical_root)
        .expect("workspace should be creatable");
    let cli_invocation = workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect("CLI invocation should resolve");
    let frontend_invocation = manifest
        .frontend_invocation(&manifest.cases[1])
        .expect("frontend invocation should resolve");

    assert_eq!(manifest.repository_root, canonical_root);
    assert!(cli_invocation.current_directory.ends_with("cli_case"));
    assert!(
        cli_invocation
            .current_directory
            .starts_with(&canonical_root)
    );
    assert!(cli_invocation.args[0].ends_with("fixture.moth"));
    assert_eq!(
        frontend_invocation.entry,
        canonical_root.join("fixture.moth")
    );
}

struct ExpectedWorkload {
    id: &'static str,
    entry: &'static str,
    entry_kind: BenchmarkEntryKind,
    fingerprint_mode: BenchmarkFingerprintMode,
    roots: &'static [&'static str],
    excludes: &'static [&'static str],
    generated_roots: &'static [&'static str],
}

#[derive(Clone, Copy)]
enum ExpectedRunner {
    Cli(CliBenchmarkCommand, &'static [&'static str]),
    Frontend(FrontendBenchmarkProfile),
}

struct ExpectedCase {
    id: &'static str,
    workload_id: &'static str,
    group: &'static str,
    quick: bool,
    runner: ExpectedRunner,
}

#[test]
fn repository_manifest_has_complete_ordered_authority() {
    const CHECK: ExpectedRunner = ExpectedRunner::Cli(CliBenchmarkCommand::Check, &[]);
    const BUILD: ExpectedRunner = ExpectedRunner::Cli(CliBenchmarkCommand::Build, &[]);
    const FRONTEND: ExpectedRunner = ExpectedRunner::Frontend(FrontendBenchmarkProfile::Dev);

    let expected_workloads = [
        ExpectedWorkload {
            id: "root_single_file",
            entry: "benchmark-root-single-file.moth",
            entry_kind: BenchmarkEntryKind::File,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmark-root-single-file.moth"],
            excludes: &[],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "speed_test",
            entry: "benchmarks/speed-test.moth",
            entry_kind: BenchmarkEntryKind::File,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/speed-test.moth"],
            excludes: &[],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "docs",
            entry: "docs",
            entry_kind: BenchmarkEntryKind::Directory,
            fingerprint_mode: BenchmarkFingerprintMode::Partitioned,
            roots: &["docs/config.moth", "docs/src"],
            excludes: &[],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "template_stress",
            entry: "benchmarks/template-stress.moth",
            entry_kind: BenchmarkEntryKind::File,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/template-stress.moth"],
            excludes: &[],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "code_highlighter_stress",
            entry: "benchmarks/code-highlighter-stress.moth",
            entry_kind: BenchmarkEntryKind::File,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/code-highlighter-stress.moth"],
            excludes: &[],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "type_stress",
            entry: "benchmarks/type-stress.moth",
            entry_kind: BenchmarkEntryKind::File,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/type-stress.moth"],
            excludes: &[],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "fold_stress",
            entry: "benchmarks/fold-stress.moth",
            entry_kind: BenchmarkEntryKind::File,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/fold-stress.moth"],
            excludes: &[],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "pattern_stress",
            entry: "benchmarks/pattern-stress.moth",
            entry_kind: BenchmarkEntryKind::File,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/pattern-stress.moth"],
            excludes: &[],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "collection_stress",
            entry: "benchmarks/collection-stress.moth",
            entry_kind: BenchmarkEntryKind::File,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/collection-stress.moth"],
            excludes: &[],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "environment_stress",
            entry: "benchmarks/environment-stress.moth",
            entry_kind: BenchmarkEntryKind::File,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/environment-stress.moth"],
            excludes: &[],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "one_module_kitchen_sink",
            entry: "benchmarks/adversarial/one-module-kitchen-sink.moth",
            entry_kind: BenchmarkEntryKind::File,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/adversarial/one-module-kitchen-sink.moth"],
            excludes: &[],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "deep_scope_churn",
            entry: "benchmarks/adversarial/deep-scope-churn.moth",
            entry_kind: BenchmarkEntryKind::File,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/adversarial/deep-scope-churn.moth"],
            excludes: &[],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "template_render_plan_churn",
            entry: "benchmarks/adversarial/template-render-plan-churn.moth",
            entry_kind: BenchmarkEntryKind::File,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/adversarial/template-render-plan-churn.moth"],
            excludes: &[],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "constant_dag_churn",
            entry: "benchmarks/adversarial/constant-dag-churn.moth",
            entry_kind: BenchmarkEntryKind::File,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/adversarial/constant-dag-churn.moth"],
            excludes: &[],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "expression_rpn_churn",
            entry: "benchmarks/adversarial/expression-rpn-churn.moth",
            entry_kind: BenchmarkEntryKind::File,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/adversarial/expression-rpn-churn.moth"],
            excludes: &[],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "generic_trait_churn",
            entry: "benchmarks/adversarial/generic-trait-churn.moth",
            entry_kind: BenchmarkEntryKind::File,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/adversarial/generic-trait-churn.moth"],
            excludes: &[],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "collection_map_borrow_churn",
            entry: "benchmarks/adversarial/collection-map-borrow-churn.moth",
            entry_kind: BenchmarkEntryKind::File,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/adversarial/collection-map-borrow-churn.moth"],
            excludes: &[],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "module_graph",
            entry: "benchmarks/module-graph",
            entry_kind: BenchmarkEntryKind::Directory,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/module-graph"],
            excludes: &[
                "benchmarks/module-graph/dev",
                "benchmarks/module-graph/release",
            ],
            generated_roots: &["dev", "release"],
        },
        ExpectedWorkload {
            id: "import_fanout",
            entry: "benchmarks/import-fanout",
            entry_kind: BenchmarkEntryKind::Directory,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/import-fanout"],
            excludes: &[
                "benchmarks/import-fanout/dev",
                "benchmarks/import-fanout/release",
            ],
            generated_roots: &["dev", "release"],
        },
        ExpectedWorkload {
            id: "external_js_imports",
            entry: "benchmarks/external-js-imports",
            entry_kind: BenchmarkEntryKind::Directory,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/external-js-imports"],
            excludes: &[
                "benchmarks/external-js-imports/dev",
                "benchmarks/external-js-imports/release",
            ],
            generated_roots: &["dev", "release"],
        },
        ExpectedWorkload {
            id: "module_root_stress",
            entry: "benchmarks/module-root-stress",
            entry_kind: BenchmarkEntryKind::Directory,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/module-root-stress"],
            excludes: &[
                "benchmarks/module-root-stress/dev",
                "benchmarks/module-root-stress/release",
            ],
            generated_roots: &["dev", "release"],
        },
        ExpectedWorkload {
            id: "import_external_churn",
            entry: "benchmarks/adversarial/import-external-churn",
            entry_kind: BenchmarkEntryKind::Directory,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/adversarial/import-external-churn"],
            excludes: &[
                "benchmarks/adversarial/import-external-churn/dev",
                "benchmarks/adversarial/import-external-churn/release",
            ],
            generated_roots: &["dev", "release"],
        },
        ExpectedWorkload {
            id: "borrow_stress",
            entry: "benchmarks/borrow-stress.moth",
            entry_kind: BenchmarkEntryKind::File,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/borrow-stress.moth"],
            excludes: &[],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "module_root_role_mix",
            entry: "benchmarks/module-root-role-mix",
            entry_kind: BenchmarkEntryKind::Directory,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/module-root-role-mix"],
            excludes: &[
                "benchmarks/module-root-role-mix/scratch",
                "benchmarks/module-root-role-mix/generated",
            ],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "tiny_one_file",
            entry: "benchmarks/parallelism/tiny-one-file",
            entry_kind: BenchmarkEntryKind::Directory,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/parallelism/tiny-one-file"],
            excludes: &[
                "benchmarks/parallelism/tiny-one-file/dev",
                "benchmarks/parallelism/tiny-one-file/release",
            ],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "tiny_two_files",
            entry: "benchmarks/parallelism/tiny-two-files",
            entry_kind: BenchmarkEntryKind::Directory,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/parallelism/tiny-two-files"],
            excludes: &[
                "benchmarks/parallelism/tiny-two-files/dev",
                "benchmarks/parallelism/tiny-two-files/release",
            ],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "tiny_seven_files",
            entry: "benchmarks/parallelism/tiny-seven-files",
            entry_kind: BenchmarkEntryKind::Directory,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/parallelism/tiny-seven-files"],
            excludes: &[
                "benchmarks/parallelism/tiny-seven-files/dev",
                "benchmarks/parallelism/tiny-seven-files/release",
            ],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "tiny_eight_files",
            entry: "benchmarks/parallelism/tiny-eight-files",
            entry_kind: BenchmarkEntryKind::Directory,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/parallelism/tiny-eight-files"],
            excludes: &[
                "benchmarks/parallelism/tiny-eight-files/dev",
                "benchmarks/parallelism/tiny-eight-files/release",
            ],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "many_tiny_files",
            entry: "benchmarks/parallelism/many-tiny-files",
            entry_kind: BenchmarkEntryKind::Directory,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/parallelism/many-tiny-files"],
            excludes: &[
                "benchmarks/parallelism/many-tiny-files/dev",
                "benchmarks/parallelism/many-tiny-files/release",
            ],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "many_medium_files",
            entry: "benchmarks/parallelism/many-medium-files",
            entry_kind: BenchmarkEntryKind::Directory,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/parallelism/many-medium-files"],
            excludes: &[
                "benchmarks/parallelism/many-medium-files/dev",
                "benchmarks/parallelism/many-medium-files/release",
            ],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "many_markdown_assets",
            entry: "benchmarks/parallelism/many-markdown-assets",
            entry_kind: BenchmarkEntryKind::Directory,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/parallelism/many-markdown-assets"],
            excludes: &[
                "benchmarks/parallelism/many-markdown-assets/dev",
                "benchmarks/parallelism/many-markdown-assets/release",
            ],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "many_modules_one_file_each",
            entry: "benchmarks/parallelism/many-modules-one-file-each",
            entry_kind: BenchmarkEntryKind::Directory,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/parallelism/many-modules-one-file-each"],
            excludes: &[
                "benchmarks/parallelism/many-modules-one-file-each/dev",
                "benchmarks/parallelism/many-modules-one-file-each/release",
            ],
            generated_roots: &[],
        },
        ExpectedWorkload {
            id: "few_modules_many_files_each",
            entry: "benchmarks/parallelism/few-modules-many-files-each",
            entry_kind: BenchmarkEntryKind::Directory,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            roots: &["benchmarks/parallelism/few-modules-many-files-each"],
            excludes: &[
                "benchmarks/parallelism/few-modules-many-files-each/dev",
                "benchmarks/parallelism/few-modules-many-files-each/release",
            ],
            generated_roots: &[],
        },
    ];
    let expected_cases = [
        ExpectedCase {
            id: "root_single_file_check",
            workload_id: "root_single_file",
            group: "core",
            quick: true,
            runner: CHECK,
        },
        ExpectedCase {
            id: "speed_test_check",
            workload_id: "speed_test",
            group: "core",
            quick: false,
            runner: CHECK,
        },
        ExpectedCase {
            id: "speed_test_build",
            workload_id: "speed_test",
            group: "core",
            quick: true,
            runner: BUILD,
        },
        ExpectedCase {
            id: "docs_check",
            workload_id: "docs",
            group: "docs",
            quick: true,
            runner: CHECK,
        },
        ExpectedCase {
            id: "template_stress_check",
            workload_id: "template_stress",
            group: "stress",
            quick: false,
            runner: CHECK,
        },
        ExpectedCase {
            id: "code_highlighter_stress_check",
            workload_id: "code_highlighter_stress",
            group: "stress",
            quick: false,
            runner: CHECK,
        },
        ExpectedCase {
            id: "type_stress_check",
            workload_id: "type_stress",
            group: "stress",
            quick: false,
            runner: CHECK,
        },
        ExpectedCase {
            id: "fold_stress_check",
            workload_id: "fold_stress",
            group: "stress",
            quick: false,
            runner: CHECK,
        },
        ExpectedCase {
            id: "pattern_stress_check",
            workload_id: "pattern_stress",
            group: "stress",
            quick: false,
            runner: CHECK,
        },
        ExpectedCase {
            id: "collection_stress_check",
            workload_id: "collection_stress",
            group: "stress",
            quick: false,
            runner: CHECK,
        },
        ExpectedCase {
            id: "environment_stress_check",
            workload_id: "environment_stress",
            group: "stress",
            quick: false,
            runner: CHECK,
        },
        ExpectedCase {
            id: "one_module_kitchen_sink_check",
            workload_id: "one_module_kitchen_sink",
            group: "stress",
            quick: true,
            runner: CHECK,
        },
        ExpectedCase {
            id: "deep_scope_churn_check",
            workload_id: "deep_scope_churn",
            group: "stress",
            quick: false,
            runner: CHECK,
        },
        ExpectedCase {
            id: "template_render_plan_churn_check",
            workload_id: "template_render_plan_churn",
            group: "stress",
            quick: false,
            runner: CHECK,
        },
        ExpectedCase {
            id: "constant_dag_churn_check",
            workload_id: "constant_dag_churn",
            group: "stress",
            quick: false,
            runner: CHECK,
        },
        ExpectedCase {
            id: "expression_rpn_churn_check",
            workload_id: "expression_rpn_churn",
            group: "stress",
            quick: false,
            runner: CHECK,
        },
        ExpectedCase {
            id: "generic_trait_churn_check",
            workload_id: "generic_trait_churn",
            group: "stress",
            quick: false,
            runner: CHECK,
        },
        ExpectedCase {
            id: "collection_map_borrow_churn_check",
            workload_id: "collection_map_borrow_churn",
            group: "stress",
            quick: false,
            runner: CHECK,
        },
        ExpectedCase {
            id: "module_graph_check",
            workload_id: "module_graph",
            group: "module",
            quick: false,
            runner: CHECK,
        },
        ExpectedCase {
            id: "module_graph_build",
            workload_id: "module_graph",
            group: "module",
            quick: true,
            runner: BUILD,
        },
        ExpectedCase {
            id: "import_fanout_check",
            workload_id: "import_fanout",
            group: "module",
            quick: false,
            runner: CHECK,
        },
        ExpectedCase {
            id: "import_fanout_build",
            workload_id: "import_fanout",
            group: "module",
            quick: false,
            runner: BUILD,
        },
        ExpectedCase {
            id: "external_js_imports_check",
            workload_id: "external_js_imports",
            group: "module",
            quick: false,
            runner: CHECK,
        },
        ExpectedCase {
            id: "external_js_imports_build",
            workload_id: "external_js_imports",
            group: "module",
            quick: true,
            runner: BUILD,
        },
        ExpectedCase {
            id: "module_root_stress_check",
            workload_id: "module_root_stress",
            group: "module",
            quick: false,
            runner: CHECK,
        },
        ExpectedCase {
            id: "module_root_stress_build",
            workload_id: "module_root_stress",
            group: "module",
            quick: false,
            runner: BUILD,
        },
        ExpectedCase {
            id: "import_external_churn_check",
            workload_id: "import_external_churn",
            group: "module",
            quick: false,
            runner: CHECK,
        },
        ExpectedCase {
            id: "import_external_churn_build",
            workload_id: "import_external_churn",
            group: "module",
            quick: true,
            runner: BUILD,
        },
        ExpectedCase {
            id: "borrow_stress_check",
            workload_id: "borrow_stress",
            group: "borrow",
            quick: true,
            runner: CHECK,
        },
        ExpectedCase {
            id: "type_stress_frontend",
            workload_id: "type_stress",
            group: "core",
            quick: false,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "docs_frontend",
            workload_id: "docs",
            group: "docs",
            quick: true,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "template_stress_frontend",
            workload_id: "template_stress",
            group: "stress",
            quick: true,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "code_highlighter_stress_frontend",
            workload_id: "code_highlighter_stress",
            group: "stress",
            quick: false,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "fold_stress_frontend",
            workload_id: "fold_stress",
            group: "stress",
            quick: true,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "pattern_stress_frontend",
            workload_id: "pattern_stress",
            group: "stress",
            quick: false,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "collection_stress_frontend",
            workload_id: "collection_stress",
            group: "stress",
            quick: false,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "environment_stress_frontend",
            workload_id: "environment_stress",
            group: "stress",
            quick: false,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "one_module_kitchen_sink_frontend",
            workload_id: "one_module_kitchen_sink",
            group: "stress",
            quick: false,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "deep_scope_churn_frontend",
            workload_id: "deep_scope_churn",
            group: "stress",
            quick: false,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "template_render_plan_churn_frontend",
            workload_id: "template_render_plan_churn",
            group: "stress",
            quick: false,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "constant_dag_churn_frontend",
            workload_id: "constant_dag_churn",
            group: "stress",
            quick: false,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "expression_rpn_churn_frontend",
            workload_id: "expression_rpn_churn",
            group: "stress",
            quick: true,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "generic_trait_churn_frontend",
            workload_id: "generic_trait_churn",
            group: "stress",
            quick: false,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "collection_map_borrow_churn_frontend",
            workload_id: "collection_map_borrow_churn",
            group: "stress",
            quick: true,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "module_graph_frontend",
            workload_id: "module_graph",
            group: "module",
            quick: true,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "import_fanout_frontend",
            workload_id: "import_fanout",
            group: "module",
            quick: false,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "module_root_stress_frontend",
            workload_id: "module_root_stress",
            group: "module",
            quick: false,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "external_js_imports_frontend",
            workload_id: "external_js_imports",
            group: "module",
            quick: false,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "import_external_churn_frontend",
            workload_id: "import_external_churn",
            group: "module",
            quick: true,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "module_root_role_mix_frontend",
            workload_id: "module_root_role_mix",
            group: "module",
            quick: false,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "tiny_one_file_frontend",
            workload_id: "tiny_one_file",
            group: "parallelism",
            quick: false,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "tiny_two_files_frontend",
            workload_id: "tiny_two_files",
            group: "parallelism",
            quick: false,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "tiny_seven_files_frontend",
            workload_id: "tiny_seven_files",
            group: "parallelism",
            quick: false,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "tiny_eight_files_frontend",
            workload_id: "tiny_eight_files",
            group: "parallelism",
            quick: false,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "many_tiny_files_frontend",
            workload_id: "many_tiny_files",
            group: "parallelism",
            quick: false,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "many_medium_files_frontend",
            workload_id: "many_medium_files",
            group: "parallelism",
            quick: true,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "many_markdown_assets_frontend",
            workload_id: "many_markdown_assets",
            group: "parallelism",
            quick: false,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "many_modules_one_file_each_frontend",
            workload_id: "many_modules_one_file_each",
            group: "parallelism",
            quick: true,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "few_modules_many_files_each_frontend",
            workload_id: "few_modules_many_files_each",
            group: "parallelism",
            quick: false,
            runner: FRONTEND,
        },
        ExpectedCase {
            id: "borrow_stress_frontend",
            workload_id: "borrow_stress",
            group: "borrow",
            quick: true,
            runner: FRONTEND,
        },
    ];

    let manifest = load_benchmark_manifest().expect("repository manifest should load");

    assert_eq!(manifest.workloads.len(), expected_workloads.len());
    for (actual, expected) in manifest.workloads.iter().zip(expected_workloads) {
        assert_eq!(actual.id, expected.id);
        assert_eq!(actual.entry, PathBuf::from(expected.entry));
        assert_eq!(actual.entry_kind, expected.entry_kind);
        assert_eq!(actual.fingerprint_mode, expected.fingerprint_mode);
        assert_eq!(
            actual.fingerprint_roots,
            expected.roots.iter().map(PathBuf::from).collect::<Vec<_>>()
        );
        assert_eq!(
            actual.fingerprint_excludes,
            expected
                .excludes
                .iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            actual.generated_output_roots,
            expected
                .generated_roots
                .iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>()
        );
    }

    assert_eq!(manifest.cases.len(), expected_cases.len());
    for (actual, expected) in manifest.cases.iter().zip(expected_cases) {
        let workload = manifest
            .workload_for(actual)
            .expect("case workload relationship should be valid");

        assert_eq!(actual.id, expected.id);
        assert_eq!(workload.id, expected.workload_id);
        assert_eq!(actual.group_name.persistence_spelling(), expected.group);
        assert_eq!(actual.quick, expected.quick);
        assert_eq!(actual.expectation, BenchmarkExpectation::Clean);

        match (&actual.runner, expected.runner) {
            (
                BenchmarkRunner::Cli { command, args },
                ExpectedRunner::Cli(expected_command, expected_args),
            ) => {
                assert_eq!(*command, expected_command);
                assert_eq!(
                    args,
                    &expected_args
                        .iter()
                        .map(|argument| (*argument).to_owned())
                        .collect::<Vec<_>>()
                );
            }
            (BenchmarkRunner::Frontend { profile }, ExpectedRunner::Frontend(expected_profile)) => {
                assert_eq!(*profile, expected_profile);
            }
            _ => panic!("case '{}' has the wrong runner kind", actual.id),
        }
    }
}

#[test]
fn duplicate_workload_id_fails() {
    let directory = tempdir().expect("temporary repository should exist");
    create_entry(directory.path(), "first.moth");
    let contents = minimal_manifest("first.moth", "first_case").replace(
            "[[case]]",
            "[[workload]]\nid = \"workload\"\nentry = \"first.moth\"\nfingerprint_roots = [\"first.moth\"]\nfingerprint_excludes = []\n\n[[case]]",
        );
    let path = write_manifest(directory.path(), &contents);
    assert!(load_manifest_at(&path, directory.path()).is_err());
}

#[test]
fn duplicate_case_id_fails() {
    let directory = tempdir().expect("temporary repository should exist");
    create_entry(directory.path(), "first.moth");
    let contents = format!(
        "{}\n[[case]]\nid = \"first_case\"\nworkload = \"workload\"\ngroup = \"core\"\nquick = false\nexpectation = \"clean\"\n[case.runner]\nkind = \"cli\"\ncommand = \"build\"\nargs = []\n",
        minimal_manifest("first.moth", "first_case")
    );
    let path = write_manifest(directory.path(), &contents);
    assert!(load_manifest_at(&path, directory.path()).is_err());
}

#[test]
fn unknown_workload_and_duplicate_invocation_fail() {
    let directory = tempdir().expect("temporary repository should exist");
    create_entry(directory.path(), "first.moth");
    let unknown = minimal_manifest("first.moth", "first_case")
        .replace("workload = \"workload\"", "workload = \"missing\"");
    let unknown_path = write_manifest(directory.path(), &unknown);
    assert!(load_manifest_at(&unknown_path, directory.path()).is_err());

    let duplicate = format!(
        "{}\n[[case]]\nid = \"second_case\"\nworkload = \"workload\"\ngroup = \"core\"\nquick = false\nexpectation = \"clean\"\n[case.runner]\nkind = \"cli\"\ncommand = \"check\"\nargs = []\n",
        minimal_manifest("first.moth", "first_case")
    );
    let duplicate_path = write_manifest(directory.path(), &duplicate);
    assert!(load_manifest_at(&duplicate_path, directory.path()).is_err());
}

#[test]
fn invalid_ids_and_runner_values_fail() {
    let directory = tempdir().expect("temporary repository should exist");
    create_entry(directory.path(), "first.moth");
    let invalid_id = minimal_manifest("first.moth", "First_case");
    let invalid_id_path = write_manifest(directory.path(), &invalid_id);
    assert!(load_manifest_at(&invalid_id_path, directory.path()).is_err());

    let invalid_command = minimal_manifest("first.moth", "first_case")
        .replace("command = \"check\"", "command = \"run\"");
    let invalid_command_path = write_manifest(directory.path(), &invalid_command);
    assert!(load_manifest_at(&invalid_command_path, directory.path()).is_err());

    let invalid_profile = minimal_manifest("first.moth", "first_case").replace(
        "kind = \"cli\"\ncommand = \"check\"\nargs = []",
        "kind = \"frontend\"\nprofile = \"release\"",
    );
    let invalid_profile_path = write_manifest(directory.path(), &invalid_profile);
    assert!(load_manifest_at(&invalid_profile_path, directory.path()).is_err());

    let invalid_kind = minimal_manifest("first.moth", "first_case")
        .replace("kind = \"cli\"", "kind = \"unknown\"");
    let invalid_kind_path = write_manifest(directory.path(), &invalid_kind);
    assert!(load_manifest_at(&invalid_kind_path, directory.path()).is_err());
}

#[test]
fn unknown_field_and_missing_required_field_fail() {
    let directory = tempdir().expect("temporary repository should exist");
    create_entry(directory.path(), "first.moth");
    let unknown = format!(
        "{}\nextra = true\n",
        minimal_manifest("first.moth", "first_case")
    );
    let unknown_path = write_manifest(directory.path(), &unknown);
    assert!(load_manifest_at(&unknown_path, directory.path()).is_err());

    let missing = minimal_manifest("first.moth", "first_case").replace("quick = false\n", "");
    let missing_path = write_manifest(directory.path(), &missing);
    assert!(load_manifest_at(&missing_path, directory.path()).is_err());
}

#[test]
fn unused_workload_fails() {
    let directory = tempdir().expect("temporary repository should exist");
    create_entry(directory.path(), "first.moth");
    create_entry(directory.path(), "second.moth");
    let contents = minimal_manifest("first.moth", "first_case").replace(
            "[[case]]",
            "[[workload]]\nid = \"unused\"\nentry = \"second.moth\"\nfingerprint_roots = [\"second.moth\"]\nfingerprint_excludes = []\n\n[[case]]",
        );
    let path = write_manifest(directory.path(), &contents);
    assert!(load_manifest_at(&path, directory.path()).is_err());
}

#[test]
fn absolute_parent_and_current_paths_fail() {
    let directory = tempdir().expect("temporary repository should exist");
    create_entry(directory.path(), "first.moth");
    for path in ["/tmp/first.moth", "../first.moth", "./first.moth"] {
        let contents = minimal_manifest(path, "first_case");
        let manifest_path = write_manifest(directory.path(), &contents);
        assert!(
            load_manifest_at(&manifest_path, directory.path()).is_err(),
            "{path} should fail"
        );
    }
}

#[test]
fn raw_interior_and_trailing_current_directory_components_fail() {
    let manifest_path = Path::new("manifest.toml");
    for raw_path in [
        "benchmarks/./speed-test.moth",
        "benchmarks/speed-test.moth/.",
        r"benchmarks\.\speed-test.moth",
        r"benchmarks\speed-test.moth\.",
    ] {
        let error = validate_relative_path(
            manifest_path,
            "workload 'workload'".to_owned(),
            "entry",
            raw_path,
        )
        .expect_err("raw current-directory component should fail");

        assert!(
            error.to_string().contains("may not contain '.' or '..'"),
            "{raw_path} should fail as a raw navigation component"
        );
    }

    assert_eq!(
        validate_relative_path(
            manifest_path,
            "workload 'workload'".to_owned(),
            "entry",
            r"benchmarks\speed-test.moth",
        )
        .expect("Windows separators without navigation should remain valid"),
        PathBuf::from("benchmarks/speed-test.moth")
    );
}

#[test]
fn duplicate_workload_entry_repository_path_fails() {
    let directory = tempdir().expect("temporary repository should exist");
    create_entry(directory.path(), "entry.moth");
    let contents = two_workload_manifest("entry.moth", "entry.moth");
    let path = write_manifest(directory.path(), &contents);
    let error = load_manifest_at(&path, directory.path()).expect_err("duplicate entry should fail");

    assert!(
        error
            .to_string()
            .contains("same repository path as workload 'first_workload'")
    );
}

#[cfg(unix)]
#[test]
fn in_repository_symlink_alias_workload_entry_fails() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().expect("temporary repository should exist");
    create_entry(directory.path(), "entry.moth");
    symlink(
        directory.path().join("entry.moth"),
        directory.path().join("entry-alias.moth"),
    )
    .expect("symlink should be creatable");
    let contents = two_workload_manifest("entry.moth", "entry-alias.moth");
    let path = write_manifest(directory.path(), &contents);
    let error =
        load_manifest_at(&path, directory.path()).expect_err("symlink entry alias should fail");

    assert!(
        error
            .to_string()
            .contains("same repository path as workload 'first_workload'")
    );
}

#[test]
fn empty_inventories_fail() {
    let directory = tempdir().expect("temporary repository should exist");
    let path = write_manifest(directory.path(), "schema = 1\nworkload = []\ncase = []\n");
    assert!(load_manifest_at(&path, directory.path()).is_err());
}

#[test]
fn uncovered_entry_and_invalid_exclude_fail() {
    let directory = tempdir().expect("temporary repository should exist");
    create_entry(directory.path(), "entry.moth");
    create_entry(directory.path(), "other.moth");
    let uncovered = minimal_manifest("entry.moth", "first_case").replace(
        "fingerprint_roots = [\"entry.moth\"]",
        "fingerprint_roots = [\"other.moth\"]",
    );
    let uncovered_path = write_manifest(directory.path(), &uncovered);
    assert!(load_manifest_at(&uncovered_path, directory.path()).is_err());

    let project_directory = tempdir().expect("temporary repository should exist");
    create_entry(project_directory.path(), "project/main.moth");
    let invalid_exclude = minimal_manifest("project/main.moth", "first_case").replace(
        "fingerprint_excludes = []",
        "fingerprint_excludes = [\"other\"]",
    );
    let exclude_path = write_manifest(project_directory.path(), &invalid_exclude);
    assert!(load_manifest_at(&exclude_path, project_directory.path()).is_err());
}

#[cfg(unix)]
#[test]
fn canonical_entry_escape_fails() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().expect("temporary repository should exist");
    let outside = tempdir().expect("outside directory should exist");
    create_entry(outside.path(), "outside.moth");
    symlink(
        outside.path().join("outside.moth"),
        directory.path().join("entry.moth"),
    )
    .expect("symlink should be creatable");
    let path = write_manifest(
        directory.path(),
        &minimal_manifest("entry.moth", "first_case"),
    );
    assert!(load_manifest_at(&path, directory.path()).is_err());
}

#[test]
fn missing_custom_named_exclude_under_directory_root_loads() {
    let directory = tempdir().expect("temporary repository should exist");
    create_entry(directory.path(), "project/main.moth");
    let contents = minimal_manifest("project/main.moth", "first_case")
        .replace(r#"entry = "project/main.moth""#, r#"entry = "project""#)
        .replace(
            "fingerprint_roots = [\"project/main.moth\"]",
            "fingerprint_roots = [\"project\"]",
        )
        .replace(
            "fingerprint_excludes = []",
            "fingerprint_excludes = [\"project/custom-output\"]",
        );
    let path = write_manifest(directory.path(), &contents);
    let manifest = load_manifest_at(&path, directory.path())
        .expect("custom missing exclusion should be structurally valid");

    assert_eq!(
        manifest.workloads[0].fingerprint_excludes,
        [PathBuf::from("project/custom-output")]
    );
}

#[test]
fn exclude_equal_to_root_or_below_file_root_fails() {
    let directory = tempdir().expect("temporary repository should exist");
    create_entry(directory.path(), "project/main.moth");
    let equal_root = minimal_manifest("project/main.moth", "first_case")
        .replace(r#"entry = "project/main.moth""#, r#"entry = "project""#)
        .replace(
            "fingerprint_roots = [\"project/main.moth\"]",
            "fingerprint_roots = [\"project\"]",
        )
        .replace(
            "fingerprint_excludes = []",
            "fingerprint_excludes = [\"project\"]",
        );
    let equal_root_path = write_manifest(directory.path(), &equal_root);
    let equal_root_error = load_manifest_at(&equal_root_path, directory.path())
        .expect_err("an exclusion equal to its root should fail");
    assert!(
        equal_root_error
            .to_string()
            .contains("is equal to a declared root")
    );

    let below_file_root = minimal_manifest("project/main.moth", "first_case").replace(
        "fingerprint_excludes = []",
        "fingerprint_excludes = [\"project/main.moth/generated\"]",
    );
    let below_file_path = write_manifest(directory.path(), &below_file_root);
    let below_file_error = load_manifest_at(&below_file_path, directory.path())
        .expect_err("an exclusion below a file root should fail");
    assert!(
        below_file_error
            .to_string()
            .contains("must be a strict descendant of a declared directory root")
    );
}

#[cfg(unix)]
#[test]
fn missing_exclude_through_symlink_outside_root_fails() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().expect("temporary repository should exist");
    let outside = tempdir().expect("outside directory should exist");
    create_entry(directory.path(), "project/main.moth");
    symlink(outside.path(), directory.path().join("project/output-link"))
        .expect("symlink should be creatable");
    let contents = minimal_manifest("project/main.moth", "first_case")
        .replace(r#"entry = "project/main.moth""#, r#"entry = "project""#)
        .replace(
            "fingerprint_roots = [\"project/main.moth\"]",
            "fingerprint_roots = [\"project\"]",
        )
        .replace(
            "fingerprint_excludes = []",
            "fingerprint_excludes = [\"project/output-link/custom-name\"]",
        );
    let path = write_manifest(directory.path(), &contents);
    let error = load_manifest_at(&path, directory.path())
        .expect_err("a missing exclusion escaping through a symlink should fail");

    assert!(
        error
            .to_string()
            .contains("escapes its declared directory root")
    );
}

#[test]
fn file_entry_retains_file_kind() {
    let directory = tempdir().expect("temporary repository should exist");
    create_entry(directory.path(), "entry.moth");
    let path = write_manifest(
        directory.path(),
        &minimal_manifest("entry.moth", "first_case"),
    );
    let manifest = load_manifest_at(&path, directory.path()).expect("manifest should load");

    assert_eq!(manifest.workloads[0].entry_kind, BenchmarkEntryKind::File);
}

#[test]
fn directory_entry_retains_directory_kind() {
    let directory = tempdir().expect("temporary repository should exist");
    fs::create_dir_all(directory.path().join("project"))
        .expect("project directory should be creatable");
    create_entry(directory.path(), "project/main.moth");
    let contents = minimal_manifest("project/main.moth", "first_case")
        .replace(r#"entry = "project/main.moth""#, r#"entry = "project""#)
        .replace(
            "fingerprint_roots = [\"project/main.moth\"]",
            "fingerprint_roots = [\"project\"]",
        );
    let path = write_manifest(directory.path(), &contents);
    let manifest = load_manifest_at(&path, directory.path()).expect("manifest should load");

    assert_eq!(
        manifest.workloads[0].entry_kind,
        BenchmarkEntryKind::Directory
    );
}

#[test]
fn workload_path_io_error_renders_complete_context() {
    let directory = tempdir().expect("temporary repository should exist");
    let contents = minimal_manifest("missing.moth", "first_case");
    let manifest_path = write_manifest(directory.path(), &contents);
    let error = load_manifest_at(&manifest_path, directory.path())
        .expect_err("missing workload entry should fail");

    let BenchmarkManifestError::WorkloadPath {
        manifest_path: error_manifest_path,
        workload_id,
        field,
        authored_path,
        source,
    } = &error
    else {
        panic!("expected typed workload path error, got {error}");
    };

    assert_eq!(error_manifest_path, &manifest_path);
    assert_eq!(workload_id, "workload");
    assert_eq!(*field, "entry");
    assert_eq!(authored_path, "missing.moth");
    assert_eq!(source.kind(), io::ErrorKind::NotFound);

    let rendered = error.to_string();
    assert!(rendered.contains(&manifest_path.display().to_string()));
    assert!(rendered.contains("workload 'workload'"));
    assert!(rendered.contains("entry 'missing.moth'"));
    assert!(rendered.contains(&source.to_string()));
}

// ------------------------
//  Fingerprint boundary mode tests
// ------------------------

fn full_tree_manifest(entry: &str, case_id: &str) -> String {
    format!(
        r#"schema = 3

[[workload]]
id = "workload"
entry = "{entry}"
fingerprint_mode = "full_tree"
fingerprint_roots = ["{entry}"]
fingerprint_excludes = []

[[case]]
id = "{case_id}"
workload = "workload"
group = "core"
quick = false
expectation = "clean"

[case.runner]
kind = "cli"
command = "check"
args = []
"#
    )
}

fn partitioned_manifest(entry: &str, roots: &[&str], case_id: &str) -> String {
    let roots_str = roots
        .iter()
        .map(|r| format!("\"{r}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r#"schema = 3

[[workload]]
id = "workload"
entry = "{entry}"
fingerprint_mode = "partitioned"
fingerprint_roots = [{roots_str}]
fingerprint_excludes = []

[[case]]
id = "{case_id}"
workload = "workload"
group = "core"
quick = false
expectation = "clean"

[case.runner]
kind = "cli"
command = "check"
args = []
"#
    )
}

#[test]
fn file_full_tree_exact_root_accepted() {
    let directory = tempdir().expect("temporary repository should exist");
    create_entry(directory.path(), "fixture.moth");
    let path = write_manifest(
        directory.path(),
        &full_tree_manifest("fixture.moth", "case"),
    );
    load_manifest_at(&path, directory.path()).expect("file full-tree should load");
}

#[test]
fn file_full_tree_extra_root_rejected() {
    let directory = tempdir().expect("temporary repository should exist");
    create_entry(directory.path(), "fixture.moth");
    create_entry(directory.path(), "other.moth");
    let contents = full_tree_manifest("fixture.moth", "case").replace(
        "fingerprint_roots = [\"fixture.moth\"]",
        "fingerprint_roots = [\"fixture.moth\", \"other.moth\"]",
    );
    let path = write_manifest(directory.path(), &contents);
    load_manifest_at(&path, directory.path())
        .expect_err("file full-tree with extra root should fail");
}

#[test]
fn file_full_tree_exclude_rejected() {
    let directory = tempdir().expect("temporary repository should exist");
    create_entry(directory.path(), "fixture.moth");
    let contents = full_tree_manifest("fixture.moth", "case").replace(
        "fingerprint_excludes = []",
        "fingerprint_excludes = [\"fixture.moth\"]",
    );
    let path = write_manifest(directory.path(), &contents);
    load_manifest_at(&path, directory.path())
        .expect_err("file full-tree with exclude equal to root should fail");
}

#[test]
fn directory_full_tree_exact_entry_root_accepted() {
    let directory = tempdir().expect("temporary repository should exist");
    fs::create_dir_all(directory.path().join("project")).expect("directory should be creatable");
    create_entry(directory.path(), "project/main.moth");
    let path = write_manifest(directory.path(), &full_tree_manifest("project", "case"));
    load_manifest_at(&path, directory.path()).expect("directory full-tree should load");
}

#[test]
fn directory_full_tree_nested_only_root_rejected() {
    let directory = tempdir().expect("temporary repository should exist");
    fs::create_dir_all(directory.path().join("project")).expect("directory should be creatable");
    create_entry(directory.path(), "project/main.moth");
    let contents = full_tree_manifest("project", "case").replace(
        "fingerprint_roots = [\"project\"]",
        "fingerprint_roots = [\"project/main.moth\"]",
    );
    let path = write_manifest(directory.path(), &contents);
    load_manifest_at(&path, directory.path())
        .expect_err("directory full-tree with nested-only root should fail");
}

#[test]
fn partitioned_file_entry_rejected() {
    let directory = tempdir().expect("temporary repository should exist");
    create_entry(directory.path(), "fixture.moth");
    let path = write_manifest(
        directory.path(),
        &partitioned_manifest("fixture.moth", &["fixture.moth"], "case"),
    );
    load_manifest_at(&path, directory.path()).expect_err("partitioned file entry should fail");
}

#[test]
fn partitioned_disjoint_roots_accepted() {
    let directory = tempdir().expect("temporary repository should exist");
    fs::create_dir_all(directory.path().join("project")).expect("directory should be creatable");
    create_entry(directory.path(), "project/config.moth");
    create_entry(directory.path(), "project/src/main.moth");
    let path = write_manifest(
        directory.path(),
        &partitioned_manifest("project", &["project/config.moth", "project/src"], "case"),
    );
    load_manifest_at(&path, directory.path()).expect("partitioned disjoint roots should load");
}

#[test]
fn partitioned_root_outside_entry_rejected() {
    let directory = tempdir().expect("temporary repository should exist");
    create_entry(directory.path(), "outside.moth");
    fs::create_dir_all(directory.path().join("project")).expect("directory should be creatable");
    let path = write_manifest(
        directory.path(),
        &partitioned_manifest("project", &["outside.moth"], "case"),
    );
    load_manifest_at(&path, directory.path())
        .expect_err("partitioned root outside entry should fail");
}

#[test]
fn partitioned_duplicate_roots_rejected() {
    let directory = tempdir().expect("temporary repository should exist");
    fs::create_dir_all(directory.path().join("project")).expect("directory should be creatable");
    create_entry(directory.path(), "project/main.moth");
    let path = write_manifest(
        directory.path(),
        &partitioned_manifest(
            "project",
            &["project/main.moth", "project/main.moth"],
            "case",
        ),
    );
    load_manifest_at(&path, directory.path()).expect_err("partitioned duplicate roots should fail");
}

#[test]
fn partitioned_ancestor_descendant_roots_rejected() {
    let directory = tempdir().expect("temporary repository should exist");
    fs::create_dir_all(directory.path().join("project/src/nested"))
        .expect("directory should be creatable");
    create_entry(directory.path(), "project/src/main.moth");
    let path = write_manifest(
        directory.path(),
        &partitioned_manifest("project", &["project/src", "project/src/nested"], "case"),
    );
    load_manifest_at(&path, directory.path())
        .expect_err("partitioned ancestor/descendant roots should fail");
}

#[test]
fn exclude_containing_another_root_rejected() {
    let directory = tempdir().expect("temporary repository should exist");
    fs::create_dir_all(directory.path().join("project/src"))
        .expect("directory should be creatable");
    create_entry(directory.path(), "project/config.moth");
    create_entry(directory.path(), "project/src/main.moth");
    let contents = partitioned_manifest("project", &["project/config.moth", "project/src"], "case")
        .replace(
            "fingerprint_excludes = []",
            "fingerprint_excludes = [\"project/src\"]",
        );
    let path = write_manifest(directory.path(), &contents);
    load_manifest_at(&path, directory.path())
        .expect_err("exclude containing another root should fail");
}

#[test]
fn exclude_under_no_directory_root_rejected() {
    let directory = tempdir().expect("temporary repository should exist");
    create_entry(directory.path(), "fixture.moth");
    let contents = full_tree_manifest("fixture.moth", "case").replace(
        "fingerprint_excludes = []",
        "fingerprint_excludes = [\"other\"]",
    );
    let path = write_manifest(directory.path(), &contents);
    load_manifest_at(&path, directory.path())
        .expect_err("exclude under no directory root should fail");
}

#[test]
fn non_existent_generated_descendant_exclude_accepted() {
    let directory = tempdir().expect("temporary repository should exist");
    fs::create_dir_all(directory.path().join("project")).expect("directory should be creatable");
    create_entry(directory.path(), "project/main.moth");
    let contents = full_tree_manifest("project", "case").replace(
        "fingerprint_excludes = []",
        "fingerprint_excludes = [\"project/dev\"]",
    );
    let path = write_manifest(directory.path(), &contents);
    load_manifest_at(&path, directory.path())
        .expect("non-existent generated descendant exclude should be accepted");
}
