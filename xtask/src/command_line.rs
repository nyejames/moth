//! Recorded POSIX shell command lines.
//!
//! WHAT: Owns one POSIX shell quoting rule for every command line that is
//! printed or written into an artifact after a child process ran, including
//! the environment assignments that configured the child, plus the fenced
//! Markdown form used by reports.
//! WHY: A recorded command must be copyable into a shell unchanged. Quoting
//! only whitespace-bearing arguments leaves quotes, globs, variables and
//! control characters active, and a non-UTF-8 argument has no exact shell
//! spelling at all, so its line is marked non-runnable instead of silently
//! naming a different path.

use std::ffi::OsStr;

/// Recorded placeholder for an executed argument that has no UTF-8 spelling.
///
/// Structured artifacts store arguments as JSON strings. An explicit marker
/// keeps a record from reading as lossy text that the child never received.
const NON_UTF8_ARGUMENT: &str = "<non-UTF-8 argument>";

/// Prefix for a command line that cannot be run verbatim.
const NON_RUNNABLE_PREFIX: &str = "non-runnable (non-UTF-8 argument): ";

/// How a recorded command's working directory is available when the line is replayed.
#[derive(Clone, Copy)]
pub(crate) enum RecordedWorkingDirectory<'a> {
    /// The directory exists independently of the run and remains at replay time.
    Existing(&'a OsStr),
    /// The directory belongs to the run and is removed with it, so a replay must recreate it
    /// before entering it.
    Ephemeral(&'a OsStr),
}

/// Render one executed command line as POSIX shell text.
///
/// A working directory is emitted as a leading `cd ... &&`. An ephemeral run directory is created
/// first with `mkdir -p`, because the removed directory would otherwise stop the replay before
/// the command started. Environment assignments follow the directory so a command whose
/// configuration comes from the caller stays replayable. An assignment keeps its `NAME=` prefix
/// unquoted, because only then does the shell treat it as an assignment rather than a command.
/// Arguments made only of shell-safe ASCII stay bare; every other argument is single-quoted with
/// each embedded quote written as `'\''`. When any component is not UTF-8, the lossy spelling is
/// still shown for diagnosis under an explicit non-runnable prefix.
pub(crate) fn posix_shell_command_line<'a>(
    working_directory: Option<RecordedWorkingDirectory<'_>>,
    environment: &[(&str, &str)],
    program: &OsStr,
    args: impl IntoIterator<Item = &'a OsStr>,
) -> String {
    let mut line = String::new();
    let mut runnable = true;

    if let Some(directory) = working_directory {
        let (directory, recreate) = match directory {
            RecordedWorkingDirectory::Existing(directory) => (directory, false),
            RecordedWorkingDirectory::Ephemeral(directory) => (directory, true),
        };

        if recreate {
            // The run directory is removed with the run, so recreate it before entering.
            line.push_str("mkdir -p");
            push_argument(&mut line, directory, &mut runnable);
            line.push_str(" && cd");
        } else {
            line.push_str("cd");
        }

        push_argument(&mut line, directory, &mut runnable);
        line.push_str(" &&");
    }

    for (name, value) in environment {
        push_environment_assignment(&mut line, name, value);
    }

    push_argument(&mut line, program, &mut runnable);
    for argument in args {
        push_argument(&mut line, argument, &mut runnable);
    }

    if runnable {
        line
    } else {
        let mut marked = String::with_capacity(NON_RUNNABLE_PREFIX.len() + line.len());
        marked.push_str(NON_RUNNABLE_PREFIX);
        marked.push_str(&line);
        marked
    }
}

/// Format a recorded command as a fenced Markdown code block.
///
/// The fence is longer than every backtick run in the command, so an argument
/// containing backticks cannot terminate the block.
pub(crate) fn markdown_command_block(command_line: &str) -> String {
    let fence_length = longest_backtick_run(command_line).saturating_add(1).max(3);
    let fence = "`".repeat(fence_length);
    format!("{fence}\n{command_line}\n{fence}")
}

/// Record one executed argument for a persisted history record.
///
/// Valid UTF-8 is retained exactly. A non-UTF-8 argument records the explicit
/// marker instead of its lossy spelling, so the record never reads as a path
/// the child did not receive.
pub(crate) fn recorded_argument(argument: &OsStr) -> String {
    match argument.to_str() {
        Some(text) => text.to_owned(),
        None => NON_UTF8_ARGUMENT.to_owned(),
    }
}

fn longest_backtick_run(text: &str) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for character in text.chars() {
        if character == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

fn push_argument(line: &mut String, argument: &OsStr, runnable: &mut bool) {
    if !line.is_empty() {
        line.push(' ');
    }

    match argument.to_str() {
        Some(text) => push_quoted_text(line, text),
        None => {
            *runnable = false;
            push_quoted_text(line, argument.to_string_lossy().as_ref());
        }
    }
}

/// Record one environment assignment as a shell word.
///
/// Callers supply a shell variable name. Only the value is quoted, because a shell recognises an
/// assignment only while the unquoted `NAME=` prefix leads the word.
fn push_environment_assignment(line: &mut String, name: &str, value: &str) {
    if !line.is_empty() {
        line.push(' ');
    }

    line.push_str(name);
    line.push('=');
    push_quoted_text(line, value);
}

fn push_quoted_text(line: &mut String, text: &str) {
    if is_safe_shell_text(text) {
        line.push_str(text);
        return;
    }

    line.push('\'');
    for character in text.chars() {
        if character == '\'' {
            line.push_str("'\\''");
        } else {
            line.push(character);
        }
    }
    line.push('\'');
}

/// Whether a POSIX shell passes this text through as one unchanged word.
fn is_safe_shell_text(text: &str) -> bool {
    !text.is_empty()
        && text.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'_' | b'-' | b'.' | b'/' | b':' | b'@' | b'%' | b'+' | b'=' | b','
                )
        })
}

#[cfg(test)]
#[path = "command_line/tests.rs"]
mod tests;
