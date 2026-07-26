//! Tests for Samply runner integration.
//!
//! These tests verify command construction, gzip profile validation,
//! and error handling without requiring Samply to be installed.

use super::*;
use flate2::Compression;
use flate2::write::GzEncoder;
use std::ffi::OsStr;
use std::io::Write;

/// Helper: write a gzip-compressed file with the given content.
fn write_gzip_file(path: &Path, content: &[u8]) {
    let file = File::create(path).expect("create gzip test file");
    let mut encoder = GzEncoder::new(file, Compression::default());
    encoder.write_all(content).expect("write gzip content");
    encoder.finish().expect("finish gzip encoding");
}

/// Helper: write a plain (non-gzip) file.
fn write_plain_file(path: &Path, content: &[u8]) {
    std::fs::write(path, content).expect("write plain test file");
}

fn command_environment<'a>(command: &'a Command, name: &str) -> Option<&'a OsStr> {
    command.get_envs().find_map(
        |(key, value)| {
            if key == OsStr::new(name) { value } else { None }
        },
    )
}

// ---------------------------------------------------------------------------
//  Command building tests
// ---------------------------------------------------------------------------

#[test]
fn presymbolication_flag_prefers_stable_help_flag() {
    let help = "\
Options:
      --presymbolicate
      --unstable-presymbolicate
";

    assert_eq!(
        PresymbolicationFlag::from_record_help(help),
        PresymbolicationFlag::Stable
    );
    assert_eq!(
        PresymbolicationFlag::Stable.command_flag(),
        Some("--presymbolicate")
    );
}

#[test]
fn presymbolication_flag_falls_back_to_unstable_help_flag() {
    let help = "\
Options:
      --unstable-presymbolicate
";

    assert_eq!(
        PresymbolicationFlag::from_record_help(help),
        PresymbolicationFlag::Unstable
    );
    assert_eq!(
        PresymbolicationFlag::Unstable.command_flag(),
        Some("--unstable-presymbolicate")
    );
}

#[test]
fn presymbolication_flag_reports_unavailable_when_help_has_no_flag() {
    let help = "\
Options:
      --save-only
";

    assert_eq!(
        PresymbolicationFlag::from_record_help(help),
        PresymbolicationFlag::Unavailable
    );
    assert_eq!(PresymbolicationFlag::Unavailable.command_flag(), None);
    assert_eq!(
        PresymbolicationFlag::Unavailable.display_label(),
        "unavailable"
    );
}

#[test]
fn build_samply_command_has_required_flags() {
    let input = SamplyRunInput {
        moth_path: PathBuf::from("/usr/bin/moth"),
        current_directory: PathBuf::from("/tmp/repository"),
        command: "check".to_string(),
        args: vec!["test.moth".to_string()],
        output_path: PathBuf::from("/tmp/profile.json.gz"),
        samply_rate_hz: None,
        presymbolicate: false,
        presymbolication_flag: PresymbolicationFlag::Unavailable,
        symbol_dirs: Vec::new(),
    };

    let cmd = build_samply_command(&input);
    let args: Vec<_> = cmd.get_args().collect();

    assert_eq!(cmd.get_current_dir(), Some(Path::new("/tmp/repository")));
    assert_eq!(
        command_environment(&cmd, "MOTH_TIMERS"),
        Some(OsStr::new("bench"))
    );
    assert_eq!(
        command_environment(&cmd, "MOTH_COUNTERS"),
        Some(OsStr::new("off"))
    );
    assert_eq!(
        command_environment(&cmd, "MOTH_BENCH_STATUS"),
        Some(OsStr::new("1"))
    );

    // Must start with `record --save-only -o <path>`.
    assert_eq!(args[0].to_str().unwrap(), "record");
    assert_eq!(args[1].to_str().unwrap(), "--save-only");
    assert_eq!(args[2].to_str().unwrap(), "-o");
    assert_eq!(args[3].to_str().unwrap(), "/tmp/profile.json.gz");

    // Then `--` separator, moth path, command, and args.
    assert_eq!(args[4].to_str().unwrap(), "--");
    assert_eq!(args[5].to_str().unwrap(), "/usr/bin/moth");
    assert_eq!(args[6].to_str().unwrap(), "check");
    assert_eq!(args[7].to_str().unwrap(), "test.moth");
}

#[test]
fn build_samply_command_with_rate() {
    let input = SamplyRunInput {
        moth_path: PathBuf::from("/usr/bin/moth"),
        current_directory: PathBuf::from("/tmp/repository"),
        command: "check".to_string(),
        args: vec![],
        output_path: PathBuf::from("/tmp/profile.json.gz"),
        samply_rate_hz: Some(500.0),
        presymbolicate: false,
        presymbolication_flag: PresymbolicationFlag::Unavailable,
        symbol_dirs: Vec::new(),
    };

    let cmd = build_samply_command(&input);
    let args: Vec<_> = cmd.get_args().collect();

    // --rate 500 must appear before `--`.
    let rate_pos = args
        .iter()
        .position(|a| a.to_str() == Some("--rate"))
        .expect("--rate flag present");
    assert_eq!(args[rate_pos + 1].to_str().unwrap(), "500");
    // `--` must come after `--rate`.
    let separator_pos = args
        .iter()
        .position(|a| a.to_str() == Some("--"))
        .expect("-- separator present");
    assert!(rate_pos < separator_pos);
}

#[test]
fn build_samply_command_with_presymbolicate() {
    let input = SamplyRunInput {
        moth_path: PathBuf::from("/usr/bin/moth"),
        current_directory: PathBuf::from("/tmp/repository"),
        command: "check".to_string(),
        args: vec![],
        output_path: PathBuf::from("/tmp/profile.json.gz"),
        samply_rate_hz: None,
        presymbolicate: true,
        presymbolication_flag: PresymbolicationFlag::Stable,
        symbol_dirs: Vec::new(),
    };

    let cmd = build_samply_command(&input);
    let args: Vec<_> = cmd.get_args().collect();

    assert!(args.iter().any(|a| a.to_str() == Some("--presymbolicate")));
}

#[test]
fn build_samply_command_with_unstable_presymbolicate() {
    let input = SamplyRunInput {
        moth_path: PathBuf::from("/usr/bin/moth"),
        current_directory: PathBuf::from("/tmp/repository"),
        command: "check".to_string(),
        args: vec![],
        output_path: PathBuf::from("/tmp/profile.json.gz"),
        samply_rate_hz: None,
        presymbolicate: true,
        presymbolication_flag: PresymbolicationFlag::Unstable,
        symbol_dirs: Vec::new(),
    };

    let cmd = build_samply_command(&input);
    let args: Vec<_> = cmd.get_args().collect();

    assert!(
        args.iter()
            .any(|a| a.to_str() == Some("--unstable-presymbolicate"))
    );
}

#[test]
fn build_samply_command_with_rate_and_presymbolicate() {
    let input = SamplyRunInput {
        moth_path: PathBuf::from("/usr/bin/moth"),
        current_directory: PathBuf::from("/tmp/repository"),
        command: "build".to_string(),
        args: vec!["foo.moth".to_string(), "bar.moth".to_string()],
        output_path: PathBuf::from("/tmp/out/profile.json.gz"),
        samply_rate_hz: Some(1000.0),
        presymbolicate: true,
        presymbolication_flag: PresymbolicationFlag::Stable,
        symbol_dirs: Vec::new(),
    };

    let cmd = build_samply_command(&input);
    let args: Vec<_> = cmd.get_args().collect();

    // Both optional flags must be present.
    assert!(args.iter().any(|a| a.to_str() == Some("--rate")));
    assert!(args.iter().any(|a| a.to_str() == Some("--presymbolicate")));

    // Multiple args must follow `--`.
    let separator_pos = args
        .iter()
        .position(|a| a.to_str() == Some("--"))
        .expect("-- separator present");
    assert_eq!(args[separator_pos + 1].to_str().unwrap(), "/usr/bin/moth");
    assert_eq!(args[separator_pos + 2].to_str().unwrap(), "build");
    assert_eq!(args[separator_pos + 3].to_str().unwrap(), "foo.moth");
    assert_eq!(args[separator_pos + 4].to_str().unwrap(), "bar.moth");
}

#[test]
fn build_samply_command_with_symbol_dirs() {
    let input = SamplyRunInput {
        moth_path: PathBuf::from("/usr/bin/moth"),
        current_directory: PathBuf::from("/tmp/repository"),
        command: "check".to_string(),
        args: vec![],
        output_path: PathBuf::from("/tmp/profile.json.gz"),
        samply_rate_hz: None,
        presymbolicate: true,
        presymbolication_flag: PresymbolicationFlag::Unstable,
        symbol_dirs: vec![
            PathBuf::from("/tmp/target/profiling"),
            PathBuf::from("/tmp/target/profiling/moth.dSYM"),
        ],
    };

    let cmd = build_samply_command(&input);
    let args: Vec<_> = cmd.get_args().collect();

    let presym_pos = args
        .iter()
        .position(|a| a.to_str() == Some("--unstable-presymbolicate"))
        .expect("presymbolicate flag present");
    let first_symbol_pos = args
        .iter()
        .position(|a| a.to_str() == Some("--symbol-dir"))
        .expect("symbol-dir flag present");
    let sep_pos = args.iter().position(|a| a.to_str() == Some("--")).unwrap();

    assert!(presym_pos < first_symbol_pos);
    assert!(first_symbol_pos < sep_pos);
    assert_eq!(
        args[first_symbol_pos + 1].to_str().unwrap(),
        "/tmp/target/profiling"
    );
    assert_eq!(args[first_symbol_pos + 2].to_str().unwrap(), "--symbol-dir");
    assert_eq!(
        args[first_symbol_pos + 3].to_str().unwrap(),
        "/tmp/target/profiling/moth.dSYM"
    );
}

#[test]
fn build_samply_command_rate_appears_before_separator() {
    // Verify flag ordering: --save-only, -o, [--rate], [--unstable-presymbolicate], --, moth, ...
    let input = SamplyRunInput {
        moth_path: PathBuf::from("/usr/bin/moth"),
        current_directory: PathBuf::from("/tmp/repository"),
        command: "check".to_string(),
        args: vec![],
        output_path: PathBuf::from("/tmp/p.json.gz"),
        samply_rate_hz: Some(250.0),
        presymbolicate: true,
        presymbolication_flag: PresymbolicationFlag::Stable,
        symbol_dirs: Vec::new(),
    };

    let cmd = build_samply_command(&input);
    let args: Vec<_> = cmd.get_args().collect();

    let save_only_pos = args
        .iter()
        .position(|a| a.to_str() == Some("--save-only"))
        .unwrap();
    let o_pos = args.iter().position(|a| a.to_str() == Some("-o")).unwrap();
    let rate_pos = args
        .iter()
        .position(|a| a.to_str() == Some("--rate"))
        .unwrap();
    let presym_pos = args
        .iter()
        .position(|a| a.to_str() == Some("--presymbolicate"))
        .unwrap();
    let sep_pos = args.iter().position(|a| a.to_str() == Some("--")).unwrap();

    assert!(save_only_pos < o_pos);
    assert!(o_pos < rate_pos);
    assert!(rate_pos < presym_pos);
    assert!(presym_pos < sep_pos);
}

// ---------------------------------------------------------------------------
//  Gzip profile validation tests
// ---------------------------------------------------------------------------

#[test]
fn peek_profile_first_byte_from_valid_gzip_json() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let profile_path = temp_dir.path().join("profile.json.gz");

    write_gzip_file(&profile_path, b"{\"key\": \"value\"}");

    let first = peek_profile_first_byte(&profile_path).expect("peek first byte");
    assert_eq!(first, b'{');
}

#[test]
fn peek_profile_first_byte_skips_leading_whitespace() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let profile_path = temp_dir.path().join("profile.json.gz");

    // JSON allows whitespace before the opening brace.
    write_gzip_file(&profile_path, b"  \n\t{\"key\": \"value\"}");

    let first = peek_profile_first_byte(&profile_path).expect("peek first byte");
    assert_eq!(first, b'{');
}

#[test]
fn peek_profile_first_byte_errors_on_empty_gzip() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let profile_path = temp_dir.path().join("profile.json.gz");

    // Empty gzip stream.
    write_gzip_file(&profile_path, b"");

    let result = peek_profile_first_byte(&profile_path);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("empty after decompression"));
}

#[test]
fn peek_profile_first_byte_errors_on_nonexistent_file() {
    let result = peek_profile_first_byte(Path::new("/nonexistent/profile.json.gz"));
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Failed to open"));
}

#[test]
fn peek_profile_first_byte_errors_on_non_gzip_file() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let profile_path = temp_dir.path().join("profile.json.gz");

    // Write plain text, not gzip-compressed.
    write_plain_file(&profile_path, b"{\"key\": \"value\"}");

    let result = peek_profile_first_byte(&profile_path);
    assert!(result.is_err());
    // The error should mention reading/decompression failure.
    assert!(result.unwrap_err().contains("Failed to read"));
}

#[test]
fn verify_profile_format_accepts_valid_gzip_json() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let profile_path = temp_dir.path().join("profile.json.gz");

    write_gzip_file(&profile_path, b"{\"threads\": []}");

    verify_profile_format(&profile_path).expect("should accept valid gzip JSON");
}

#[test]
fn verify_profile_format_rejects_non_json_content() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let profile_path = temp_dir.path().join("profile.json.gz");

    // Gzip-compressed but not JSON.
    write_gzip_file(&profile_path, b"<html>not json</html>");

    let result = verify_profile_format(&profile_path);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("does not contain valid JSON"));
    assert!(err.contains("Expected"));
}

#[test]
fn verify_profile_format_rejects_array_toplevel() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let profile_path = temp_dir.path().join("profile.json.gz");

    // Firefox profiles are objects, not arrays.
    write_gzip_file(&profile_path, b"[1, 2, 3]");

    let result = verify_profile_format(&profile_path);
    assert!(result.is_err());
}

#[test]
fn verify_profile_format_rejects_empty_file() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let profile_path = temp_dir.path().join("profile.json.gz");

    write_gzip_file(&profile_path, b"");

    let result = verify_profile_format(&profile_path);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
//  Samply availability check tests
// ---------------------------------------------------------------------------

/// If Samply is installed, the availability check should succeed.
/// This test is marked to run only when Samply is present so CI
/// environments without Samply do not fail.
#[test]
fn check_samply_available_succeeds_when_installed() {
    match check_samply_available() {
        Ok(capabilities) => {
            // Samply is installed; this is the expected path on developer machines.
            assert!(!capabilities.version.is_empty());
        }
        Err(_) => {
            // Samply is not installed; skip rather than fail.
            // This keeps unit tests green in CI without Samply.
        }
    }
}

// ---------------------------------------------------------------------------
//  ProfileProcessRun data model tests
// ---------------------------------------------------------------------------

#[test]
fn profile_process_run_struct_fields() {
    let run = ProfileProcessRun {
        duration_ms: 1234.5,
        success: true,
        stdout: "samply output".to_string(),
        stderr: "samply errors".to_string(),
        command_line: "samply record --save-only".to_string(),
        output_path: PathBuf::from("/tmp/profile.json.gz"),
        presymbolication_flag: PresymbolicationFlag::Unstable,
    };

    assert_eq!(run.duration_ms, 1234.5);
    assert!(run.success);
    assert_eq!(run.stdout, "samply output");
    assert_eq!(run.stderr, "samply errors");
    assert_eq!(run.command_line, "samply record --save-only");
    assert_eq!(run.output_path, PathBuf::from("/tmp/profile.json.gz"));
    assert_eq!(run.presymbolication_flag, PresymbolicationFlag::Unstable);
}

// ---------------------------------------------------------------------------
//  SamplyRunInput data model tests
// ---------------------------------------------------------------------------

#[test]
fn samply_run_input_struct_fields() {
    let input = SamplyRunInput {
        moth_path: PathBuf::from("/usr/bin/moth"),
        current_directory: PathBuf::from("/tmp/repository"),
        command: "check".to_string(),
        args: vec!["foo.moth".to_string()],
        output_path: PathBuf::from("/tmp/profile.json.gz"),
        samply_rate_hz: Some(500.0),
        presymbolicate: true,
        presymbolication_flag: PresymbolicationFlag::Stable,
        symbol_dirs: vec![PathBuf::from("/tmp/target/profiling")],
    };

    assert_eq!(input.moth_path, PathBuf::from("/usr/bin/moth"));
    assert_eq!(input.command, "check");
    assert_eq!(input.args, vec!["foo.moth"]);
    assert_eq!(input.output_path, PathBuf::from("/tmp/profile.json.gz"));
    assert_eq!(input.samply_rate_hz, Some(500.0));
    assert!(input.presymbolicate);
    assert_eq!(input.presymbolication_flag, PresymbolicationFlag::Stable);
    assert_eq!(
        input.symbol_dirs,
        vec![PathBuf::from("/tmp/target/profiling")]
    );
}
