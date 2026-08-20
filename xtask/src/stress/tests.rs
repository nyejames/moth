use super::*;

#[test]
fn lane_matrix_covers_every_thread_count_for_both_suites() {
    let lanes = stress_lanes(2);

    // Two suites, three thread counts, two iterations each.
    assert_eq!(lanes.len(), 12);
    assert_eq!(
        lanes
            .iter()
            .filter(|lane| lane.suite == StressSuite::Unit)
            .count(),
        6
    );
    for suite in [StressSuite::Unit, StressSuite::Integration] {
        for threads in [Some(1), None, Some(HIGH_THREAD_COUNT)] {
            let iterations: Vec<u32> = lanes
                .iter()
                .filter(|lane| lane.suite == suite && lane.threads == threads)
                .map(|lane| lane.iteration)
                .collect();
            assert_eq!(
                iterations,
                vec![1, 2],
                "{} threads={threads:?} must repeat in order",
                suite.label()
            );
        }
    }
}

#[test]
fn a_single_repeat_still_runs_every_thread_count() {
    let lanes = stress_lanes(1);

    assert_eq!(lanes.len(), 6);
    assert!(lanes.iter().all(|lane| lane.iteration == 1));
}

#[test]
fn zero_repeats_is_rejected_before_any_lane_runs() {
    let error = run_stress_matrix(0).expect_err("zero repeats must be rejected");

    assert_eq!(error, "stress repeats must be greater than 0");
}

#[test]
fn lane_labels_name_the_suite_thread_count_and_position() {
    let default_threads = StressLane {
        suite: StressSuite::Integration,
        threads: None,
        iteration: 2,
        repeats: 3,
    };
    assert_eq!(
        default_threads.to_string(),
        "integration threads=default run 2/3"
    );

    let single_thread = StressLane {
        suite: StressSuite::Unit,
        threads: Some(1),
        iteration: 1,
        repeats: 1,
    };
    assert_eq!(single_thread.to_string(), "unit threads=1 run 1/1");
}

#[test]
fn failure_reasons_distinguish_launch_from_exit() {
    assert_eq!(
        StressFailure::Launch("no such file".to_string()).to_string(),
        "could not start: no such file"
    );
    assert_eq!(StressFailure::Exit(Some(101)).to_string(), "exit code 101");
    assert_eq!(
        StressFailure::Exit(None).to_string(),
        "terminated without an exit code"
    );
}

#[test]
fn unit_lanes_pass_the_thread_count_through_to_the_test_harness() {
    let single = unit_suite_command(Some(4));
    let arguments: Vec<&str> = single
        .get_args()
        .map(|argument| argument.to_str().expect("arguments are ASCII"))
        .collect();
    assert_eq!(
        arguments,
        vec![
            "test",
            "--workspace",
            "--quiet",
            "--",
            "--format",
            "terse",
            "--test-threads=4"
        ]
    );

    let default = unit_suite_command(None);
    assert!(
        !default
            .get_args()
            .any(|argument| argument.to_string_lossy().starts_with("--test-threads")),
        "default parallelism must not pin a thread count"
    );
}

#[test]
fn integration_lanes_set_or_clear_the_runner_thread_variable() {
    let pinned = integration_suite_command(Some(8));
    let pinned_threads: Vec<Option<String>> = pinned
        .get_envs()
        .filter(|(key, _)| *key == "MOTH_TEST_THREADS")
        .map(|(_, value)| value.map(|value| value.to_string_lossy().into_owned()))
        .collect();
    assert_eq!(pinned_threads, vec![Some("8".to_string())]);

    let default = integration_suite_command(None);
    let cleared: Vec<Option<String>> = default
        .get_envs()
        .filter(|(key, _)| *key == "MOTH_TEST_THREADS")
        .map(|(_, value)| value.map(|value| value.to_string_lossy().into_owned()))
        .collect();
    assert_eq!(
        cleared,
        vec![None],
        "a default-parallelism lane must clear an inherited thread count"
    );
}
