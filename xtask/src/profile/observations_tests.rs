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

#[test]
fn relative_entry_command_names_binary_and_working_directory() {
    let observation = test_observation(vec![OsString::from("benchmarks/fixture")]);

    assert_eq!(
        observation.posix_shell_command_line(),
        "cd /repo && /repo/target/profiling/moth check benchmarks/fixture"
    );
}

#[test]
fn absolute_entry_command_omits_the_working_directory() {
    let observation = test_observation(vec![OsString::from("/repo/benchmarks/fixture.moth")]);

    assert_eq!(
        observation.posix_shell_command_line(),
        "/repo/target/profiling/moth check /repo/benchmarks/fixture.moth"
    );
}
