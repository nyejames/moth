//! Tests for recorded POSIX shell command-line rendering.

use super::*;
use std::ffi::OsString;

#[test]
fn plain_safe_ascii_arguments_stay_unquoted() {
    let line = posix_shell_command_line(
        None,
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
fn arguments_with_spaces_and_quotes_round_trip_through_a_shell() {
    let arguments = [
        OsString::from("check"),
        OsString::from("path with spaces"),
        OsString::from("it's"),
        OsString::from("a$b"),
        OsString::from(""),
    ];

    let line = posix_shell_command_line(
        None,
        OsStr::new("moth"),
        arguments.iter().map(OsString::as_os_str),
    );

    assert_eq!(line, "moth check 'path with spaces' 'it'\\''s' 'a$b' ''");
    assert_eq!(
        split_posix_words(&line),
        ["moth", "check", "path with spaces", "it's", "a$b", ""]
    );
}

#[test]
fn working_directory_is_rendered_as_a_cd_prefix() {
    let arguments = [
        OsString::from("check"),
        OsString::from("benchmarks/fixture"),
    ];

    let line = posix_shell_command_line(
        Some(OsStr::new("/repo with space")),
        OsStr::new("/repo with space/target/profiling/moth"),
        arguments.iter().map(OsString::as_os_str),
    );

    assert_eq!(
        line,
        "cd '/repo with space' && '/repo with space/target/profiling/moth' check benchmarks/fixture"
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
        Some(working_directory.as_os_str()),
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

/// Split a rendered line the way a POSIX shell would, for quoting round-trips.
fn split_posix_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut characters = line.chars();

    while let Some(character) = characters.next() {
        match character {
            ' ' => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                for quoted in characters.by_ref() {
                    if quoted == '\'' {
                        break;
                    }
                    current.push(quoted);
                }
            }
            '\\' => {
                in_word = true;
                if let Some(escaped) = characters.next() {
                    current.push(escaped);
                }
            }
            other => {
                in_word = true;
                current.push(other);
            }
        }
    }

    if in_word {
        words.push(current);
    }
    words
}
