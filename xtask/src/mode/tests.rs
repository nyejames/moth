use super::*;
use crate::profile::options::ProfileFilterMode;

// ----------------------------
//  parse_args: single-argument modes
// ----------------------------

fn unwrap_mode(result: ModeParseResult) -> BenchmarkMode {
    match result {
        ModeParseResult::Mode(mode) => mode,
        other => panic!("Expected Mode, got: {:?}", format!("{:?}", other)),
    }
}

fn unwrap_error(result: ModeParseResult) -> String {
    match result {
        ModeParseResult::Error(msg) => msg,
        other => panic!("Expected Error, got: {:?}", format!("{:?}", other)),
    }
}

impl std::fmt::Debug for ModeParseResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mode(mode) => write!(f, "Mode({:?})", mode),
            Self::ProfileHelp(_) => write!(f, "ProfileHelp(...)"),
            Self::Error(msg) => write!(f, "Error({:?})", msg),
        }
    }
}

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| s.to_string()).collect()
}

#[test]
fn parse_args_bench() {
    assert_eq!(
        unwrap_mode(BenchmarkMode::parse_args(&args(&["bench"]))),
        BenchmarkMode::Bench
    );
}

#[test]
fn parse_args_bench_check() {
    assert_eq!(
        unwrap_mode(BenchmarkMode::parse_args(&args(&["bench-check"]))),
        BenchmarkMode::BenchCheck
    );
}

#[test]
fn parse_args_bench_ci() {
    assert_eq!(
        unwrap_mode(BenchmarkMode::parse_args(&args(&["bench-ci"]))),
        BenchmarkMode::BenchCi
    );
}

#[test]
fn parse_args_bench_report() {
    assert_eq!(
        unwrap_mode(BenchmarkMode::parse_args(&args(&["bench-report"]))),
        BenchmarkMode::BenchReport
    );
}

#[test]
fn parse_args_bench_frontend() {
    assert_eq!(
        unwrap_mode(BenchmarkMode::parse_args(&args(&["bench-frontend"]))),
        BenchmarkMode::BenchFrontend
    );
}

#[test]
fn parse_args_bench_frontend_check() {
    assert_eq!(
        unwrap_mode(BenchmarkMode::parse_args(&args(&["bench-frontend-check"]))),
        BenchmarkMode::BenchFrontendCheck
    );
}

#[test]
fn parse_args_bench_data_layout_modes() {
    assert_eq!(
        unwrap_mode(BenchmarkMode::parse_args(&args(&["bench-data-layout"]))),
        BenchmarkMode::BenchDataLayout
    );
    assert_eq!(
        unwrap_mode(BenchmarkMode::parse_args(&args(&[
            "bench-data-layout-check"
        ]))),
        BenchmarkMode::BenchDataLayoutCheck
    );
}

#[test]
fn parse_args_single_mode_extra_args_rejected() {
    let error = unwrap_error(BenchmarkMode::parse_args(&args(&["bench", "extra"])));
    assert!(error.contains("does not accept additional arguments"));
}

#[test]
fn parse_args_empty_rejected() {
    let error = unwrap_error(BenchmarkMode::parse_args(&args(&[])));
    assert!(error.contains("No mode specified"));
}

#[test]
fn parse_args_unknown_mode_rejected() {
    let error = unwrap_error(BenchmarkMode::parse_args(&args(&["unknown-mode"])));
    assert!(error.contains("Unknown mode 'unknown-mode'"));
}

#[test]
fn top_level_usage_lists_bench_ci() {
    assert!(TOP_LEVEL_USAGE.contains("bench-ci"));
    assert!(TOP_LEVEL_USAGE.contains("bench-data-layout"));
    assert!(TOP_LEVEL_USAGE.contains("bench-data-layout-check"));
}

// ----------------------------
//  parse_args: bench-profile
// ----------------------------

#[test]
fn parse_args_bench_profile_default() {
    let mode = unwrap_mode(BenchmarkMode::parse_args(&args(&["bench-profile"])));
    match mode {
        BenchmarkMode::BenchProfile(options) => {
            assert_eq!(options.filter, ProfileFilterMode::Terse);
            assert_eq!(options.case_filter, None);
            assert_eq!(options.samply_rate_hz, None);
            assert!(!options.presymbolicate);
        }
        other => panic!("Expected BenchProfile, got {:?}", other),
    }
}

#[test]
fn parse_args_bench_profile_with_filter() {
    let mode = unwrap_mode(BenchmarkMode::parse_args(&args(&["bench-profile", "deep"])));
    match mode {
        BenchmarkMode::BenchProfile(options) => {
            assert_eq!(options.filter, ProfileFilterMode::Deep);
        }
        other => panic!("Expected BenchProfile, got {:?}", other),
    }
}

#[test]
fn parse_args_bench_profile_with_case() {
    let mode = unwrap_mode(BenchmarkMode::parse_args(&args(&[
        "bench-profile",
        "--case",
        "my_case",
    ])));
    match mode {
        BenchmarkMode::BenchProfile(options) => {
            assert_eq!(options.case_filter, Some("my_case".to_string()));
        }
        other => panic!("Expected BenchProfile, got {:?}", other),
    }
}

#[test]
fn parse_args_bench_profile_help() {
    let result = BenchmarkMode::parse_args(&args(&["bench-profile", "--help"]));
    match result {
        ModeParseResult::ProfileHelp(help) => {
            assert!(help.contains("Usage:"));
        }
        other => panic!("Expected ProfileHelp, got {:?}", format!("{:?}", other)),
    }
}

#[test]
fn parse_args_bench_profile_error() {
    let error = unwrap_error(BenchmarkMode::parse_args(&args(&[
        "bench-profile",
        "--unknown",
    ])));
    assert!(error.contains("Unknown argument"));
}

// ----------------------------
//  parse_args: stress
// ----------------------------

#[test]
fn parse_args_stress_defaults_to_the_owned_repeat_count() {
    assert_eq!(
        unwrap_mode(BenchmarkMode::parse_args(&args(&["stress"]))),
        BenchmarkMode::Stress {
            repeats: crate::stress::DEFAULT_STRESS_REPEATS
        }
    );
}

#[test]
fn parse_args_stress_accepts_an_explicit_repeat_count() {
    assert_eq!(
        unwrap_mode(BenchmarkMode::parse_args(&args(&[
            "stress",
            "--repeats",
            "7"
        ]))),
        BenchmarkMode::Stress { repeats: 7 }
    );
}

#[test]
fn parse_args_stress_rejects_a_non_positive_or_unparsable_repeat_count() {
    for value in ["0", "-1", "many", ""] {
        let error = unwrap_error(BenchmarkMode::parse_args(&args(&[
            "stress",
            "--repeats",
            value,
        ])));
        assert_eq!(
            error,
            format!("--repeats must be a positive integer, got '{value}'")
        );
    }
}

#[test]
fn parse_args_stress_rejects_a_missing_value_and_unknown_arguments() {
    assert_eq!(
        unwrap_error(BenchmarkMode::parse_args(&args(&["stress", "--repeats"]))),
        "--repeats requires a value."
    );
    assert_eq!(
        unwrap_error(BenchmarkMode::parse_args(&args(&["stress", "--forever"]))),
        "Mode 'stress' accepts only '--repeats <n>'."
    );
}

#[test]
fn top_level_usage_lists_stress() {
    assert!(TOP_LEVEL_USAGE.contains("stress"));
    assert!(TOP_LEVEL_USAGE.contains("--repeats <n>; default 3"));
}

// ----------------------------
//  parse_args: honesty-audit
// ----------------------------

/// The CI gate runs the audit without arguments, and must not modify the checkout.
#[test]
fn parse_args_honesty_audit_leaves_the_tracked_evidence_alone_by_default() {
    assert_eq!(
        unwrap_mode(BenchmarkMode::parse_args(&args(&["honesty-audit"]))),
        BenchmarkMode::HonestyAudit {
            update_evidence: false
        }
    );
}

#[test]
fn parse_args_honesty_audit_accepts_the_evidence_refresh_flag() {
    assert_eq!(
        unwrap_mode(BenchmarkMode::parse_args(&args(&[
            "honesty-audit",
            "--update-evidence"
        ]))),
        BenchmarkMode::HonestyAudit {
            update_evidence: true
        }
    );
}

#[test]
fn parse_args_honesty_audit_rejects_anything_else() {
    for rejected in [
        vec!["honesty-audit", "--update"],
        vec!["honesty-audit", "--update-evidence", "extra"],
        vec!["honesty-audit", "--update-evidence", "--update-evidence"],
    ] {
        let error = unwrap_error(BenchmarkMode::parse_args(&args(&rejected)));
        assert!(
            error.contains("--update-evidence"),
            "unexpected error for {rejected:?}: {error}"
        );
    }
}

#[test]
fn top_level_usage_lists_every_audit_mode() {
    for mode in [
        "honesty-audit",
        "source-audit",
        "first-party-deps",
        "feature-lane-check",
    ] {
        assert!(
            TOP_LEVEL_USAGE.contains(mode),
            "usage does not mention '{mode}', so nobody running `xtask` learns it exists"
        );
    }
}
