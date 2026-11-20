//! Tests for profile observation logging.

use super::ProfileObservation;
use crate::bench_types::{BenchmarkCaseObservations, BenchmarkMetric};
use std::ffi::OsString;
use std::path::PathBuf;

fn test_observation(command_args: Vec<OsString>) -> ProfileObservation {
    ProfileObservation {
        case_id: "test_case".to_string(),
        group_name: "core".to_string(),
        command: "check".to_string(),
        command_args,
        moth_path: PathBuf::from("/repo/target/profiling/moth"),
        current_directory: PathBuf::from("/repo"),
        wall_ms: 500.0,
        observations: BenchmarkCaseObservations {
            timing_schema_version: 2,
            stage_timings: vec![BenchmarkMetric {
                name: "frontend.ast.total".to_string(),
                value: 300.0,
            }],
            counters: vec![],
        },
        stdout: "stdout content".to_string(),
        stderr: "stderr content".to_string(),
    }
}

#[test]
fn profile_observation_struct_fields() {
    let observation = test_observation(vec![OsString::from("foo.moth")]);

    assert_eq!(observation.case_id, "test_case");
    assert_eq!(observation.group_name, "core");
    assert_eq!(observation.command, "check");
    assert_eq!(observation.command_args, vec![OsString::from("foo.moth")]);
    assert_eq!(observation.wall_ms, 500.0);
    assert_eq!(observation.observations.stage_timings.len(), 1);
    assert_eq!(observation.stdout, "stdout content");
    assert_eq!(observation.stderr, "stderr content");
}

// File-entry builds run from a generated case directory that the run then removes, so the
// replay must recreate it before entering it and still name the benchmark environment.
#[test]
fn file_entry_build_replay_records_isolated_directory_and_environment() {
    let root = if cfg!(windows) { "C:/repo" } else { "/repo" };
    let mut observation = test_observation(vec![OsString::from(format!(
        "{root}/benchmarks/fixture.moth"
    ))]);
    observation.command = "build".to_string();
    observation.moth_path = PathBuf::from(format!("{root}/target/profiling/moth"));
    observation.current_directory =
        PathBuf::from(format!("{root}/target/benchmark-work/speed_test_build"));

    assert_eq!(
        observation.posix_shell_command_line(),
        format!(
            "mkdir -p {root}/target/benchmark-work/speed_test_build && \
             cd {root}/target/benchmark-work/speed_test_build && \
             MOTH_TIMERS=bench MOTH_COUNTERS=off MOTH_BENCH_STATUS=1 \
             {root}/target/profiling/moth build {root}/benchmarks/fixture.moth"
        )
    );
}

// The replay must work after the run removed its case directory. A shell stub stands in for the
// compiler through the same renderer, so this proves the directory is created and entered without
// requiring a built moth binary.
#[cfg(unix)]
#[test]
fn replayed_file_entry_build_creates_and_enters_the_removed_working_directory() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = tempfile::tempdir().expect("temporary directory should exist");
    let stub = temp_dir.path().join("replay-stub");
    std::fs::write(&stub, "#!/bin/sh\nprintf '%s' \"$PWD\"\n")
        .expect("replay stub should be writable");
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
        .expect("replay stub should be executable");

    let working_directory = temp_dir.path().join("benchmark-work/speed_test_build");
    let mut observation = test_observation(vec![OsString::from("fixture.moth")]);
    observation.command = "build".to_string();
    observation.moth_path = stub;
    observation.current_directory = working_directory.clone();
    assert!(
        !working_directory.exists(),
        "the replay directory must start absent, as it is after a profiling run"
    );

    let line = observation.posix_shell_command_line();
    let output = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(&line)
        .output()
        .expect("/bin/sh should run");
    assert!(output.status.success(), "replay failed: {line}");

    let reported = String::from_utf8(output.stdout).expect("stub output should be UTF-8");
    assert_eq!(
        std::fs::canonicalize(reported.trim())
            .expect("reported directory should canonicalise after the replay"),
        std::fs::canonicalize(&working_directory)
            .expect("replay should create the working directory"),
    );
}
