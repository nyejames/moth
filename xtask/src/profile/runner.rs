//! Samply runner integration for the profiling workflow
//!
//! WHAT: Spawns Samply to record profiling data for benchmark cases,
//! verifies the output exists and is valid gzip-compressed JSON, and
//! provides a compatibility check that the profile can be parsed.
//!
//! WHY: Samply is invoked as an external command (not a Rust dependency)
//! to capture stack samples while the Moth compiler runs benchmark
//! cases. Separating the runner from the orchestrator keeps command
//! construction testable without requiring Samply to be installed.
//!
//! # What this module owns
//! - `check_samply_available()` for availability checks
//! - `SamplyRunInput` and `ProfileProcessRun` data types
//! - `build_samply_command()` for testable command construction
//! - `run_samply()` for spawning Samply and capturing output
//! - `peek_profile_first_byte()` for gzip-aware profile validation
//! - `verify_profile_format()` for the first-run parser compatibility check
//!
//! # What this module does NOT own
//! - Profiling build helpers (see `compiler_binary.rs`)
//! - Observation passes or timer parsing (see `observations.rs`)
//! - Artifact directory layout (see `artifacts.rs`)
//! - Profile JSON parsing or hotspot extraction (Phase 4)

use flate2::read::GzDecoder;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

#[derive(Debug, Clone)]
pub(crate) struct SamplyRecordCapabilities {
    /// `samply --version` stdout, trimmed for diagnostics.
    pub(crate) version: String,
    /// The presymbolication flag supported by this Samply install.
    pub(crate) presymbolication_flag: PresymbolicationFlag,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PresymbolicationFlag {
    Stable,
    Unstable,
    Unavailable,
}

impl PresymbolicationFlag {
    pub(crate) fn from_record_help(help: &str) -> Self {
        if help.contains("--presymbolicate") {
            Self::Stable
        } else if help.contains("--unstable-presymbolicate") {
            Self::Unstable
        } else {
            Self::Unavailable
        }
    }

    pub(crate) fn command_flag(self) -> Option<&'static str> {
        match self {
            Self::Stable => Some("--presymbolicate"),
            Self::Unstable => Some("--unstable-presymbolicate"),
            Self::Unavailable => None,
        }
    }

    pub(crate) fn display_label(self) -> &'static str {
        self.command_flag().unwrap_or("unavailable")
    }
}

/// Input configuration for a single Samply profiling run.
///
/// WHAT: Encapsulates the command, arguments, output path, and Samply
/// options needed to record one benchmark case.
///
/// WHY: A named struct makes the runner's inputs explicit and testable
/// without threading many arguments through function signatures.
pub(crate) struct SamplyRunInput {
    /// Path to the built moth binary to profile.
    pub(crate) moth_path: PathBuf,
    /// The moth subcommand (e.g., "check", "build").
    pub(crate) command: String,
    /// Arguments to the subcommand.
    pub(crate) args: Vec<String>,
    /// Output path for the Samply profile (typically `profile.json.gz`).
    pub(crate) output_path: PathBuf,
    /// Optional sampling rate in Hz; `None` uses Samply's default.
    pub(crate) samply_rate_hz: Option<f64>,
    /// Whether to pass `--presymbolicate` to Samply.
    pub(crate) presymbolicate: bool,
    /// Version-specific Samply flag to use when presymbolication is requested.
    pub(crate) presymbolication_flag: PresymbolicationFlag,
    /// Symbol directories to pass to Samply in deterministic order.
    pub(crate) symbol_dirs: Vec<PathBuf>,
}

/// Result of executing a Samply profiling run.
///
/// WHAT: Captures the duration, success status, output, and the path
/// where Samply wrote the profile file.
///
/// WHY: The orchestrator needs timing and success data to report progress
/// and decide whether to continue with the next case.
pub(crate) struct ProfileProcessRun {
    /// Wall-clock duration of the Samply process in milliseconds.
    pub(crate) duration_ms: f64,
    /// Whether Samply exited with code 0.
    pub(crate) success: bool,
    /// Captured Samply stdout.
    pub(crate) stdout: String,
    /// Captured Samply stderr.
    pub(crate) stderr: String,
    /// Display form of the Samply command that was executed.
    pub(crate) command_line: String,
    /// Path where the profile was written.
    ///
    /// Used by tests now and will be used by Phase 4 hotspot extraction.
    #[allow(dead_code)]
    pub(crate) output_path: PathBuf,
    /// Version-specific Samply presymbolication flag selected for this run.
    pub(crate) presymbolication_flag: PresymbolicationFlag,
}

/// Check whether `samply` is available on the system PATH.
///
/// WHAT: Runs `samply --version` and checks for a successful exit.
/// WHY: Fail early with a clear message if Samply is not installed,
/// rather than failing mid-run with a confusing spawn error.
pub(crate) fn check_samply_available() -> Result<SamplyRecordCapabilities, String> {
    let version_output = Command::new("samply")
        .arg("--version")
        .output()
        .map_err(|_| {
            "Samply is not installed or not found on PATH.\n\
             Install it with: cargo install samply\n\
             See: https://github.com/nicolo-ribaudo/samply"
                .to_string()
        })?;

    if !version_output.status.success() {
        return Err(format!(
            "Samply --version returned a non-zero exit code ({}).\n\
             Samply may be installed but not functioning correctly.",
            version_output.status.code().unwrap_or(-1)
        ));
    }

    let help_output = Command::new("samply")
        .args(["record", "--help"])
        .output()
        .map_err(|e| format!("Failed to execute 'samply record --help': {}", e))?;

    if !help_output.status.success() {
        return Err(format!(
            "Samply record --help returned a non-zero exit code ({}).\n\
             Samply may be installed but the record subcommand is not usable.",
            help_output.status.code().unwrap_or(-1)
        ));
    }

    let version = String::from_utf8_lossy(&version_output.stdout)
        .trim()
        .to_string();
    let help = String::from_utf8_lossy(&help_output.stdout);
    let presymbolication_flag = PresymbolicationFlag::from_record_help(&help);

    Ok(SamplyRecordCapabilities {
        version,
        presymbolication_flag,
    })
}

/// Build the `samply record` command without executing it.
///
/// WHAT: Constructs the `Command` that would invoke Samply with all
/// required flags for this profiling run.
///
/// WHY: Separating command construction from execution lets us test
/// the command shape (flags, arguments, ordering) without requiring
/// Samply to be installed on the test machine.
pub(crate) fn build_samply_command(input: &SamplyRunInput) -> Command {
    let mut cmd = Command::new("samply");
    cmd.arg("record")
        .arg("--save-only")
        .arg("-o")
        .arg(&input.output_path);

    // Optional sampling rate.
    if let Some(rate) = input.samply_rate_hz {
        cmd.arg("--rate").arg(rate.to_string());
    }

    // Optional symbolication. The exact flag changed across Samply versions,
    // so command construction uses the capability probe from `samply record --help`.
    if input.presymbolicate
        && let Some(flag) = input.presymbolication_flag.command_flag()
    {
        cmd.arg(flag);
    }

    for symbol_dir in &input.symbol_dirs {
        cmd.arg("--symbol-dir").arg(symbol_dir);
    }

    // The `--` separator followed by the moth command and its arguments.
    cmd.arg("--")
        .arg(&input.moth_path)
        .arg(&input.command)
        .args(&input.args);

    cmd
}

/// Run Samply to record a profile for one benchmark case.
///
/// WHAT: Spawns the `samply record` command, measures wall time,
/// captures stdout/stderr, checks the exit code, and verifies the
/// output file exists.
///
/// WHY: This is the core Samply integration that the orchestrator
/// calls for each benchmark case after the observation pass.
pub(crate) fn run_samply(input: &SamplyRunInput) -> Result<ProfileProcessRun, String> {
    let start = Instant::now();
    let mut command = build_samply_command(input);
    let command_line = format_command_line(&command);

    let output = command.output().map_err(|e| {
        format!(
            "Failed to spawn Samply.\nCommand: {}\nError: {}",
            command_line, e
        )
    })?;

    let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let success = output.status.success();

    if !success {
        return Ok(ProfileProcessRun {
            duration_ms,
            success: false,
            stdout,
            stderr,
            command_line,
            output_path: input.output_path.clone(),
            presymbolication_flag: input.presymbolication_flag,
        });
    }

    // Verify Samply actually wrote the output file.
    if !input.output_path.exists() {
        return Err(format!(
            "Samply exited successfully but the profile file was not created at '{}'.\n\
             Command: {}\n\
             Samply stderr: {}",
            input.output_path.display(),
            command_line,
            stderr.trim()
        ));
    }

    // Run the first-run parser compatibility check.
    verify_profile_format(&input.output_path)?;

    Ok(ProfileProcessRun {
        duration_ms,
        success,
        stdout,
        stderr,
        command_line,
        output_path: input.output_path.clone(),
        presymbolication_flag: input.presymbolication_flag,
    })
}

fn format_command_line(command: &Command) -> String {
    std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|arg| shell_display_arg(arg.to_string_lossy().as_ref()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_display_arg(arg: &str) -> String {
    if arg.is_empty() || arg.chars().any(char::is_whitespace) {
        format!("'{}'", arg.replace('\'', "'\\''"))
    } else {
        arg.to_string()
    }
}

/// Peek the first non-whitespace byte of a gzip-compressed profile file.
///
/// WHAT: Decompresses the gzip stream and reads the first byte to
/// verify the content is JSON (starts with `{`).
///
/// WHY: Samply 0.13.1 writes gzip-compressed profiles regardless of
/// the output extension. This check catches format mismatches early,
/// before Phase 4 tries to parse the profile as JSON.
pub(crate) fn peek_profile_first_byte(path: &Path) -> Result<u8, String> {
    let file = File::open(path)
        .map_err(|e| format!("Failed to open profile file '{}': {}", path.display(), e))?;

    let reader = BufReader::new(file);
    let mut decoder = GzDecoder::new(reader);

    // Skip leading whitespace (JSON allows it before the opening brace).
    let mut buf = [0u8; 1];
    loop {
        let n = decoder.read(&mut buf).map_err(|e| {
            format!(
                "Failed to read decompressed profile from '{}': {}",
                path.display(),
                e
            )
        })?;

        if n == 0 {
            return Err(format!(
                "Profile file '{}' is empty after decompression.",
                path.display()
            ));
        }

        // Skip whitespace bytes: space, tab, newline, carriage return.
        if !buf[0].is_ascii_whitespace() {
            return Ok(buf[0]);
        }
    }
}

/// Verify that a profile file contains valid gzip-compressed JSON.
///
/// WHAT: Reads the first non-whitespace byte after gzip decompression
/// and checks that it is `{` (the start of a JSON object).
///
/// WHY: This is the Phase 3 first-run parser compatibility check.
/// If Samply ever changes its output format, this check will catch
/// it immediately with a clear error instead of a confusing Phase 4
/// parse failure.
pub(crate) fn verify_profile_format(path: &Path) -> Result<(), String> {
    let first_byte = peek_profile_first_byte(path)?;

    if first_byte == b'{' {
        Ok(())
    } else {
        Err(format!(
            "Profile file '{}' does not contain valid JSON after decompression. \
             Expected '{{' as the first non-whitespace byte, found '{}' (0x{:02x}). \
             The file may not be a valid Samply processed profile.",
            path.display(),
            first_byte as char,
            first_byte
        ))
    }
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
