//! Tests for profile observation logging.

use super::ProfileObservation;
use crate::bench_types::{BenchmarkCaseObservations, BenchmarkMetric};

#[test]
fn profile_observation_struct_fields() {
    let observation = ProfileObservation {
        case_name: "test_case".to_string(),
        group_name: "core".to_string(),
        command: "check".to_string(),
        command_args: vec!["foo.moth".to_string()],
        wall_ms: 500.0,
        observations: BenchmarkCaseObservations {
            stage_timings: vec![BenchmarkMetric {
                name: "ast_ms".to_string(),
                value: 300.0,
            }],
            counters: vec![],
        },
        stdout: "stdout content".to_string(),
        stderr: "stderr content".to_string(),
    };

    assert_eq!(observation.case_name, "test_case");
    assert_eq!(observation.group_name, "core");
    assert_eq!(observation.command, "check");
    assert_eq!(observation.command_args, vec!["foo.moth"]);
    assert_eq!(observation.wall_ms, 500.0);
    assert_eq!(observation.observations.stage_timings.len(), 1);
    assert_eq!(observation.stdout, "stdout content");
    assert_eq!(observation.stderr, "stderr content");
}
