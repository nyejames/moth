//! Tests for recorded POSIX shell command-line rendering.

use super::*;
use std::ffi::OsString;

#[test]
fn plain_safe_ascii_arguments_stay_unquoted() {
    let line = posix_shell_command_line(
        None,
        &[],
        OsStr::new("moth"),
        [
            OsStr::new("check"),
            OsStr::new("/repo/benchmarks/speed-test.moth"),
            OsStr::new("--terse"),
        ],
    );

    assert_eq!(line, "moth check /repo/benchmarks/speed-test.moth --terse");
}

#[test]
fn arguments_with_spaces_and_quotes_use_shell_quoting() {
    let arguments = [
        OsString::from("check"),
        OsString::from("path with spaces"),
        OsString::from("it's"),
        OsString::from("a$b"),
        OsString::from(""),
    ];

    let line = posix_shell_command_line(
        None,
        &[],
        OsStr::new("moth"),
        arguments.iter().map(OsString::as_os_str),
    );

    assert_eq!(line, "moth check 'path with spaces' 'it'\\''s' 'a$b' ''");
}

#[test]
fn working_directory_is_rendered_as_a_cd_prefix() {
    let arguments = [
        OsString::from("check"),
        OsString::from("benchmarks/fixture"),
    ];

    let line = posix_shell_command_line(
        Some(RecordedWorkingDirectory::Existing(OsStr::new(
            "/repo with space",
        ))),
        &[],
        OsStr::new("/repo with space/target/profiling/moth"),
        arguments.iter().map(OsString::as_os_str),
    );

    assert_eq!(
        line,
        "cd '/repo with space' && '/repo with space/target/profiling/moth' check benchmarks/fixture"
    );
}

#[test]
fn environment_assignments_follow_the_working_directory() {
    let line = posix_shell_command_line(
        Some(RecordedWorkingDirectory::Existing(OsStr::new("/repo"))),
        &[("MOTH_TIMERS", "bench"), ("MOTH_COUNTERS", "off")],
        OsStr::new("moth"),
        [OsStr::new("check")],
    );

    assert_eq!(
        line,
        "cd /repo && MOTH_TIMERS=bench MOTH_COUNTERS=off moth check"
    );
}

#[test]
fn environment_assignments_quote_only_their_values() {
    let line = posix_shell_command_line(
        None,
        &[("MOTH_GREETING", "hello world"), ("MOTH_QUOTE", "it's")],
        OsStr::new("moth"),
        [OsStr::new("check")],
    );

    assert_eq!(
        line,
        "MOTH_GREETING='hello world' MOTH_QUOTE='it'\\''s' moth check"
    );
}

#[test]
#[cfg(unix)]
fn environment_assignments_run_under_bin_sh() {
    use std::process::Command;

    let line = posix_shell_command_line(
        None,
        &[("MOTH_GREETING", "hello world")],
        OsStr::new("printenv"),
        [OsStr::new("MOTH_GREETING")],
    );
    let output = Command::new("/bin/sh")
        .arg("-c")
        .arg(&line)
        .output()
        .expect("/bin/sh should run");
    assert!(output.status.success(), "rendered line failed: {line}");
    assert_eq!(
        String::from_utf8(output.stdout).expect("printenv output should be UTF-8"),
        "hello world\n"
    );
}

#[test]
fn ephemeral_working_directory_is_recreated_before_entering() {
    let line = posix_shell_command_line(
        Some(RecordedWorkingDirectory::Ephemeral(OsStr::new(
            "/repo/target/benchmark-work/run/speed_test_build",
        ))),
        &[("MOTH_TIMERS", "bench")],
        OsStr::new("moth"),
        [OsStr::new("build")],
    );

    assert_eq!(
        line,
        "mkdir -p /repo/target/benchmark-work/run/speed_test_build && \
         cd /repo/target/benchmark-work/run/speed_test_build && \
         MOTH_TIMERS=bench moth build"
    );
}

#[test]
fn markdown_block_fence_outgrows_every_backtick_run() {
    let command = "moth check 'a`b' --x '```'";

    assert_eq!(
        markdown_command_block(command),
        "````\nmoth check 'a`b' --x '```'\n````"
    );
}

#[test]
#[cfg(unix)]
fn non_utf8_arguments_are_marked_in_records_and_labelled_non_runnable() {
    use std::os::unix::ffi::OsStringExt;

    let argument = OsString::from_vec(b"bad/\xff.moth".to_vec());

    let line = posix_shell_command_line(
        None,
        &[],
        OsStr::new("moth"),
        std::iter::once(argument.as_os_str()),
    );
    assert_eq!(
        line,
        format!("{NON_RUNNABLE_PREFIX}moth 'bad/\u{FFFD}.moth'")
    );

    assert_eq!(recorded_argument(OsStr::new("plain.moth")), "plain.moth");
    assert_eq!(recorded_argument(argument.as_os_str()), NON_UTF8_ARGUMENT);
}

#[test]
#[cfg(unix)]
fn rendered_lines_run_under_bin_sh() {
    use std::process::Command;

    let arguments = [
        OsString::from("path with spaces"),
        OsString::from("it's"),
        OsString::from("a$b"),
        OsString::new(),
    ];
    let line = posix_shell_command_line(
        None,
        &[],
        OsStr::new("printf"),
        std::iter::once(OsStr::new("%s\n")).chain(arguments.iter().map(OsString::as_os_str)),
    );
    let output = Command::new("/bin/sh")
        .arg("-c")
        .arg(&line)
        .output()
        .expect("/bin/sh should run");
    assert!(output.status.success(), "rendered line failed: {line}");
    assert_eq!(
        String::from_utf8(output.stdout).expect("printf output should be UTF-8"),
        "path with spaces\nit's\na$b\n\n"
    );

    let temp = tempfile::tempdir().expect("temporary directory should exist");
    let working_directory = temp.path().join("dir with space");
    std::fs::create_dir(&working_directory).expect("working directory should be creatable");
    let line = posix_shell_command_line(
        Some(RecordedWorkingDirectory::Existing(
            working_directory.as_os_str(),
        )),
        &[],
        OsStr::new("pwd"),
        std::iter::empty(),
    );
    let output = Command::new("/bin/sh")
        .arg("-c")
        .arg(&line)
        .output()
        .expect("/bin/sh should run");
    assert!(output.status.success(), "rendered line failed: {line}");
    let reported = String::from_utf8(output.stdout).expect("pwd output should be UTF-8");
    assert_eq!(
        std::fs::canonicalize(reported.trim()).expect("reported directory should canonicalise"),
        std::fs::canonicalize(&working_directory).expect("working directory should canonicalise")
    );
}
