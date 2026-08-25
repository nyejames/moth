use super::{TimerOutputMode, TimingSessionConfiguration};

#[test]
fn timer_mode_parser_maps_known_values_and_defaults() {
    assert_eq!(
        TimerOutputMode::parse(Some("silent")),
        TimerOutputMode::Silent
    );
    assert_eq!(
        TimerOutputMode::parse(Some("none")),
        TimerOutputMode::Silent
    );
    assert_eq!(TimerOutputMode::parse(Some("off")), TimerOutputMode::Silent);
    assert_eq!(
        TimerOutputMode::parse(Some("summary")),
        TimerOutputMode::Summary
    );
    assert_eq!(
        TimerOutputMode::parse(Some("bench")),
        TimerOutputMode::Bench
    );
    assert_eq!(
        TimerOutputMode::parse(Some("verbose")),
        TimerOutputMode::Verbose
    );
    assert_eq!(
        TimerOutputMode::parse(Some("full")),
        TimerOutputMode::Verbose
    );

    #[cfg(feature = "detailed_timers")]
    assert_eq!(TimerOutputMode::parse(None), TimerOutputMode::Verbose);
    #[cfg(not(feature = "detailed_timers"))]
    assert_eq!(TimerOutputMode::parse(None), TimerOutputMode::Summary);
}

#[test]
fn timer_only_command_channels_follow_the_mode_matrix() {
    let summary = TimingSessionConfiguration::for_test(TimerOutputMode::Summary);
    assert!(summary.channels().metrics());
    assert!(summary.channels().attribution());
    assert!(summary.channels().human_summary());
    assert!(!summary.channels().detailed());
    assert!(!summary.channels().bench_output());

    let verbose = TimingSessionConfiguration::for_test(TimerOutputMode::Verbose);
    assert!(verbose.channels().metrics());
    assert!(verbose.channels().attribution());
    assert!(verbose.channels().detailed());
    assert!(verbose.channels().bench_output());
    assert!(verbose.channels().human_summary());

    let bench = TimingSessionConfiguration::for_test(TimerOutputMode::Bench);
    assert!(bench.channels().metrics());
    assert!(bench.channels().detailed());
    assert!(bench.channels().bench_output());
    assert!(!bench.channels().attribution());
    assert!(!bench.channels().human_summary());

    let silent = TimingSessionConfiguration::for_test(TimerOutputMode::Silent);
    assert!(!silent.has_collection());
    assert!(!silent.channels().metrics());
    assert!(!silent.channels().counters());
}

#[test]
fn raw_benchmark_configuration_owns_metrics_without_attribution() {
    let raw = TimingSessionConfiguration::raw_benchmark(true, false);
    assert!(raw.channels().metrics());
    assert!(raw.channels().detailed());
    assert!(!raw.channels().attribution());
    assert!(raw.suppress_output());
    assert!(!raw.channels().bench_output());
}

#[cfg(feature = "benchmark_counters")]
#[test]
fn counter_modes_extend_the_session_channel_matrix() {
    use crate::timing::CounterOutputMode;

    let summary = TimingSessionConfiguration::for_test_with_counters(
        TimerOutputMode::Summary,
        CounterOutputMode::Summary,
    );
    assert!(summary.channels().metrics());
    assert!(summary.channels().counters());
    assert!(summary.channels().attribution());
    assert!(summary.channels().human_summary());
    assert_eq!(summary.counter_mode(), CounterOutputMode::Summary);

    let bench = TimingSessionConfiguration::for_test_with_counters(
        TimerOutputMode::Bench,
        CounterOutputMode::Summary,
    );
    assert!(bench.channels().metrics());
    assert!(bench.channels().counters());
    assert!(bench.channels().bench_output());
    assert!(bench.channels().human_summary());
    assert!(!bench.channels().attribution());

    let counter_only = TimingSessionConfiguration::for_test_with_counters(
        TimerOutputMode::Silent,
        CounterOutputMode::Summary,
    );
    assert!(!counter_only.channels().metrics());
    assert!(counter_only.channels().counters());
    assert!(counter_only.channels().human_summary());
    assert!(!counter_only.channels().attribution());

    let full = TimingSessionConfiguration::for_test_with_counters(
        TimerOutputMode::Silent,
        CounterOutputMode::Full,
    );
    assert!(!full.channels().metrics());
    assert!(full.channels().counters());
    assert!(full.channels().human_summary());
    assert_eq!(full.counter_mode(), CounterOutputMode::Full);
}
