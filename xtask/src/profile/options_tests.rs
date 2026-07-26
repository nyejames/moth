//! Tests for profile options parsing and filter mode thresholds.
//!
//! WHAT: Validates that `ProfileOptions`, `ProfileFilterMode`, and the
//! argument parser handle all documented CLI forms correctly, including
//! edge cases, error messages, and threshold values.
//!
//! WHY: The profiling command parser has no framework to validate arguments;
//! these tests ensure the hand-rolled parser is correct and produces clear
//! error messages for invalid input.

use super::*;

// ----------------------------
//  ProfileFilterMode parsing
// ----------------------------

#[test]
fn filter_parse_terse() {
    assert_eq!(
        ProfileFilterMode::parse("terse"),
        Some(ProfileFilterMode::Terse)
    );
}

#[test]
fn filter_parse_normal() {
    assert_eq!(
        ProfileFilterMode::parse("normal"),
        Some(ProfileFilterMode::Normal)
    );
}

#[test]
fn filter_parse_deep() {
    assert_eq!(
        ProfileFilterMode::parse("deep"),
        Some(ProfileFilterMode::Deep)
    );
}

#[test]
fn filter_parse_raw_index() {
    assert_eq!(
        ProfileFilterMode::parse("raw-index"),
        Some(ProfileFilterMode::RawIndex)
    );
}

#[test]
fn filter_parse_empty_defaults_to_terse() {
    assert_eq!(ProfileFilterMode::parse(""), Some(ProfileFilterMode::Terse));
}

#[test]
fn filter_parse_unknown_returns_none() {
    assert_eq!(ProfileFilterMode::parse("verbose"), None);
    assert_eq!(ProfileFilterMode::parse("terse-normal"), None);
}

// ----------------------------
//  ProfileFilterMode thresholds
// ----------------------------

#[test]
fn terse_hot_function_limit() {
    assert_eq!(ProfileFilterMode::Terse.hot_function_limit(), 8);
}

#[test]
fn normal_hot_function_limit() {
    assert_eq!(ProfileFilterMode::Normal.hot_function_limit(), 20);
}

#[test]
fn deep_hot_function_limit() {
    assert_eq!(ProfileFilterMode::Deep.hot_function_limit(), 50);
}

#[test]
fn raw_index_hot_function_limit_is_zero() {
    assert_eq!(ProfileFilterMode::RawIndex.hot_function_limit(), 0);
}

#[test]
fn terse_root_case_limit() {
    assert_eq!(ProfileFilterMode::Terse.root_case_limit(), 3);
}

#[test]
fn normal_root_case_limit() {
    assert_eq!(ProfileFilterMode::Normal.root_case_limit(), 8);
}

#[test]
fn deep_root_case_limit_is_unbounded() {
    assert_eq!(ProfileFilterMode::Deep.root_case_limit(), usize::MAX);
}

#[test]
fn terse_minimum_inclusive_pct() {
    assert_eq!(ProfileFilterMode::Terse.minimum_inclusive_pct(), 2.0);
}

#[test]
fn normal_minimum_inclusive_pct() {
    assert_eq!(ProfileFilterMode::Normal.minimum_inclusive_pct(), 1.0);
}

#[test]
fn deep_minimum_inclusive_pct() {
    assert_eq!(ProfileFilterMode::Deep.minimum_inclusive_pct(), 0.25);
}

#[test]
fn terse_minimum_self_pct() {
    assert_eq!(ProfileFilterMode::Terse.minimum_self_pct(), 1.0);
}

#[test]
fn normal_minimum_self_pct() {
    assert_eq!(ProfileFilterMode::Normal.minimum_self_pct(), 0.5);
}

#[test]
fn deep_minimum_self_pct() {
    assert_eq!(ProfileFilterMode::Deep.minimum_self_pct(), 0.25);
}

#[test]
fn terse_excludes_edges() {
    assert!(!ProfileFilterMode::Terse.include_edges());
}

#[test]
fn normal_excludes_edges() {
    assert!(!ProfileFilterMode::Normal.include_edges());
}

#[test]
fn deep_includes_edges() {
    assert!(ProfileFilterMode::Deep.include_edges());
}

#[test]
fn raw_index_excludes_edges() {
    assert!(!ProfileFilterMode::RawIndex.include_edges());
}

// ----------------------------
//  ProfileOptions defaults
// ----------------------------

#[test]
fn profile_options_default_is_terse() {
    let options = ProfileOptions::new();
    assert_eq!(options.filter, ProfileFilterMode::Terse);
    assert_eq!(options.case_filter, None);
    assert_eq!(options.samply_rate_hz, None);
    assert!(!options.presymbolicate);
}

// ----------------------------
//  parse_profile_args: basic forms
// ----------------------------

fn unwrap_options(result: ProfileParseResult) -> ProfileOptions {
    match result {
        ProfileParseResult::Options(opts) => opts,
        other => panic!("Expected Options, got: {:?}", format!("{:?}", other)),
    }
}

fn unwrap_error(result: ProfileParseResult) -> String {
    match result {
        ProfileParseResult::Error(msg) => msg,
        other => panic!("Expected Error, got: {:?}", format!("{:?}", other)),
    }
}

fn unwrap_help(result: ProfileParseResult) -> String {
    match result {
        ProfileParseResult::Help(msg) => msg,
        other => panic!("Expected Help, got: {:?}", format!("{:?}", other)),
    }
}

// Implement Debug for ProfileParseResult so the panic messages above work.
impl std::fmt::Debug for ProfileParseResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Help(_) => write!(f, "Help(...)"),
            Self::Options(opts) => write!(f, "Options({:?})", opts),
            Self::Error(msg) => write!(f, "Error({:?})", msg),
        }
    }
}

#[test]
fn parse_no_args_defaults_to_terse() {
    let options = unwrap_options(parse_profile_args(&[]));
    assert_eq!(options.filter, ProfileFilterMode::Terse);
    assert_eq!(options.case_filter, None);
    assert_eq!(options.samply_rate_hz, None);
    assert!(!options.presymbolicate);
}

#[test]
fn parse_positional_terse() {
    let options = unwrap_options(parse_profile_args(&["terse"]));
    assert_eq!(options.filter, ProfileFilterMode::Terse);
}

#[test]
fn parse_positional_normal() {
    let options = unwrap_options(parse_profile_args(&["normal"]));
    assert_eq!(options.filter, ProfileFilterMode::Normal);
}

#[test]
fn parse_positional_deep() {
    let options = unwrap_options(parse_profile_args(&["deep"]));
    assert_eq!(options.filter, ProfileFilterMode::Deep);
}

#[test]
fn parse_positional_raw_index() {
    let options = unwrap_options(parse_profile_args(&["raw-index"]));
    assert_eq!(options.filter, ProfileFilterMode::RawIndex);
}

// ----------------------------
//  parse_profile_args: --filter flag
// ----------------------------

#[test]
fn parse_filter_flag_terse() {
    let options = unwrap_options(parse_profile_args(&["--filter", "terse"]));
    assert_eq!(options.filter, ProfileFilterMode::Terse);
}

#[test]
fn parse_filter_flag_normal() {
    let options = unwrap_options(parse_profile_args(&["--filter", "normal"]));
    assert_eq!(options.filter, ProfileFilterMode::Normal);
}

#[test]
fn parse_filter_flag_deep() {
    let options = unwrap_options(parse_profile_args(&["--filter", "deep"]));
    assert_eq!(options.filter, ProfileFilterMode::Deep);
}

#[test]
fn parse_filter_flag_raw_index() {
    let options = unwrap_options(parse_profile_args(&["--filter", "raw-index"]));
    assert_eq!(options.filter, ProfileFilterMode::RawIndex);
}

#[test]
fn parse_filter_flag_missing_value() {
    let error = unwrap_error(parse_profile_args(&["--filter"]));
    assert!(error.contains("--filter requires a value"));
}

#[test]
fn parse_filter_flag_unknown_value() {
    let error = unwrap_error(parse_profile_args(&["--filter", "verbose"]));
    assert!(error.contains("Unknown filter 'verbose'"));
}

#[test]
fn parse_duplicate_filter_flag() {
    let error = unwrap_error(parse_profile_args(&[
        "--filter", "terse", "--filter", "normal",
    ]));
    assert!(error.contains("Duplicate --filter"));
}

// ----------------------------
//  parse_profile_args: --case flag
// ----------------------------

#[test]
fn parse_case_flag() {
    let options = unwrap_options(parse_profile_args(&["--case", "my_case"]));
    assert_eq!(options.case_filter, Some("my_case".to_string()));
}

#[test]
fn parse_case_flag_missing_value() {
    let error = unwrap_error(parse_profile_args(&["--case"]));
    assert!(error.contains("--case requires a case ID"));
}

#[test]
fn parse_duplicate_case_flag() {
    let error = unwrap_error(parse_profile_args(&["--case", "a", "--case", "b"]));
    assert!(error.contains("Duplicate --case"));
}

// ----------------------------
//  parse_profile_args: --rate flag
// ----------------------------

#[test]
fn parse_rate_flag() {
    let options = unwrap_options(parse_profile_args(&["--rate", "500"]));
    assert_eq!(options.samply_rate_hz, Some(500.0));
}

#[test]
fn parse_rate_flag_decimal() {
    let options = unwrap_options(parse_profile_args(&["--rate", "1000.5"]));
    assert_eq!(options.samply_rate_hz, Some(1000.5));
}

#[test]
fn parse_rate_flag_missing_value() {
    let error = unwrap_error(parse_profile_args(&["--rate"]));
    assert!(error.contains("--rate requires a positive number"));
}

#[test]
fn parse_rate_flag_zero_rejected() {
    let error = unwrap_error(parse_profile_args(&["--rate", "0"]));
    assert!(error.contains("Invalid sampling rate '0'"));
}

#[test]
fn parse_rate_flag_negative_rejected() {
    let error = unwrap_error(parse_profile_args(&["--rate", "-100"]));
    assert!(error.contains("Invalid sampling rate '-100'"));
}

#[test]
fn parse_rate_flag_infinite_rejected() {
    let error = unwrap_error(parse_profile_args(&["--rate", "inf"]));
    assert!(error.contains("Invalid sampling rate 'inf'"));
}

#[test]
fn parse_rate_flag_nan_rejected() {
    let error = unwrap_error(parse_profile_args(&["--rate", "NaN"]));
    assert!(error.contains("Invalid sampling rate 'NaN'"));
}

#[test]
fn parse_rate_flag_non_numeric_rejected() {
    let error = unwrap_error(parse_profile_args(&["--rate", "abc"]));
    assert!(error.contains("Invalid sampling rate 'abc'"));
}

#[test]
fn parse_duplicate_rate_flag() {
    let error = unwrap_error(parse_profile_args(&["--rate", "500", "--rate", "1000"]));
    assert!(error.contains("Duplicate --rate"));
}

// ----------------------------
//  parse_profile_args: --presymbolicate flag
// ----------------------------

#[test]
fn parse_presymbolicate_flag() {
    let options = unwrap_options(parse_profile_args(&["--presymbolicate"]));
    assert!(options.presymbolicate);
}

// ----------------------------
//  parse_profile_args: --help flag
// ----------------------------

#[test]
fn parse_help_flag() {
    let help = unwrap_help(parse_profile_args(&["--help"]));
    assert!(help.contains("Usage:"));
    assert!(help.contains("--filter"));
    assert!(help.contains("--case"));
    assert!(help.contains("--rate"));
    assert!(help.contains("--presymbolicate"));
}

// ----------------------------
//  parse_profile_args: combined flags
// ----------------------------

#[test]
fn parse_combined_filter_and_case() {
    let options = unwrap_options(parse_profile_args(&[
        "--case", "my_case", "--filter", "deep",
    ]));
    assert_eq!(options.filter, ProfileFilterMode::Deep);
    assert_eq!(options.case_filter, Some("my_case".to_string()));
}

#[test]
fn parse_combined_all_flags() {
    let options = unwrap_options(parse_profile_args(&[
        "--filter",
        "normal",
        "--case",
        "my_case",
        "--rate",
        "500",
        "--presymbolicate",
    ]));
    assert_eq!(options.filter, ProfileFilterMode::Normal);
    assert_eq!(options.case_filter, Some("my_case".to_string()));
    assert_eq!(options.samply_rate_hz, Some(500.0));
    assert!(options.presymbolicate);
}

#[test]
fn parse_positional_filter_with_flags() {
    let options = unwrap_options(parse_profile_args(&[
        "deep", "--case", "my_case", "--rate", "250",
    ]));
    assert_eq!(options.filter, ProfileFilterMode::Deep);
    assert_eq!(options.case_filter, Some("my_case".to_string()));
    assert_eq!(options.samply_rate_hz, Some(250.0));
}

// ----------------------------
//  parse_profile_args: error cases
// ----------------------------

#[test]
fn parse_unknown_flag_rejected() {
    let error = unwrap_error(parse_profile_args(&["--unknown"]));
    assert!(error.contains("Unknown argument '--unknown'"));
}

#[test]
fn parse_unknown_positional_rejected() {
    let error = unwrap_error(parse_profile_args(&["verbose"]));
    assert!(error.contains("Unknown filter 'verbose'"));
}

#[test]
fn parse_multiple_positional_filters_rejected() {
    let error = unwrap_error(parse_profile_args(&["terse", "normal"]));
    assert!(error.contains("Unexpected argument 'normal'"));
}
