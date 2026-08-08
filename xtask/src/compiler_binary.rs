//! Compiler binary builder - Builds the release and profiling Moth compiler
//!
//! This module owns building compiler binaries with timing features enabled.
//! Normal benchmark builds use the concise `timers` feature; profiling builds
//! use `detailed_timers` for verbose human-readable substage prose. It supports
//! two build profiles:
//!
//! - **Release** (`target/release/moth`): standard benchmark build.
//! - **Profiling** (`target/profiling/moth`): dedicated profiling build with
//!   full debug info and frame pointers for Samply symbolication/unwinding.
//!
//! # What this module owns
//! - Executing `cargo build --release --features timers` for normal benchmarks
//! - Executing `cargo build --profile profiling --features detailed_timers` with
//!   `RUSTFLAGS="-C force-frame-pointers=yes"`
//! - Locating built binaries in `target/release/` and `target/profiling/`
//! - Preparing symbol directories and macOS dSYM UUID diagnostics for profiling
//! - Platform-specific executable suffix handling
//!
//! # What this module does NOT own
//! - Running the built binary (see `process_runner.rs`)
//! - Benchmark orchestration or case execution (see `bench.rs`)
//! - Profile orchestration or Samply integration (see `profile/`)

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Path to the built compiler binary
///
/// A thin wrapper around `PathBuf` that identifies a successfully built
/// compiler artifact from either release or profiling profiles.
#[derive(Debug, Clone)]
pub struct CompilerBinary {
    pub path: PathBuf,
    pub symbol_dirs: Vec<PathBuf>,
    pub profiling_symbols: Option<ProfilingSymbolDiagnostics>,
}

/// Symbol preparation facts for a profiling binary.
///
/// WHAT: Captures the debug-info policy and macOS dSYM UUID health that
/// Samply symbolication depends on.
/// WHY: Raw-address profiles are not actionable. Keeping these facts beside
/// the built binary makes profile runs report whether symbol inputs were
/// actually prepared rather than assuming dSYM generation helped.
#[derive(Debug, Clone)]
pub struct ProfilingSymbolDiagnostics {
    pub debug_info_setting: &'static str,
    pub dsym_path: PathBuf,
    pub dsym_uuid_match: DsymUuidMatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DsymUuidMatch {
    #[cfg(any(target_os = "macos", test))]
    Yes,
    #[cfg(any(target_os = "macos", test))]
    No,
    Unknown,
}

impl DsymUuidMatch {
    pub fn as_str(self) -> &'static str {
        match self {
            #[cfg(any(target_os = "macos", test))]
            Self::Yes => "yes",
            #[cfg(any(target_os = "macos", test))]
            Self::No => "no",
            Self::Unknown => "unknown",
        }
    }
}

impl CompilerBinary {
    /// Return the path as a borrowed `Path`.
    pub fn as_path(&self) -> &Path {
        &self.path
    }
}

/// Build the compiler with release profile and concise timers
///
/// WHAT: builds with the `timers` feature so the benchmark subprocess emits
/// stable MOTH_BENCH timing aggregate records under MOTH_TIMERS=bench without the verbose
/// human prose and AST substage timings that detailed_timers adds.
/// WHY:  keeps benchmark output low-noise while still capturing all top-level
/// pipeline-stage metrics for attribution and regression detection.
///
/// Executes `cargo build --release --features timers`, then
/// verifies and returns the path to the built binary.
///
/// # Returns
///
/// A `CompilerBinary` pointing to the built artifact, or an error message.
pub fn build_release_compiler_with_timers(
    repository_root: &Path,
) -> Result<CompilerBinary, String> {
    let status = Command::new("cargo")
        .current_dir(repository_root)
        .args(["build", "--release", "--features", "timers"])
        .status()
        .map_err(|e| format!("Failed to execute cargo build: {}", e))?;

    if !status.success() {
        return Err("Compiler build failed".to_string());
    }

    let moth_path = repository_root.join(release_compiler_path(env::consts::EXE_SUFFIX));

    if !moth_path.exists() {
        return Err(format!(
            "moth binary not found at '{}' after build",
            moth_path.display()
        ));
    }

    Ok(CompilerBinary {
        path: moth_path,
        symbol_dirs: Vec::new(),
        profiling_symbols: None,
    })
}

/// Build the compiler with profiling profile, detailed timers, and forced frame pointers
///
/// Executes:
/// ```bash
/// RUSTFLAGS="-C force-frame-pointers=yes" cargo build --profile profiling --features detailed_timers --bin moth
/// ```
///
/// The profiling profile inherits from release but includes full debug info,
/// no stripping, thin LTO, and one codegen unit. Frame pointers are forced on
/// so Samply can unwind stacks reliably.
///
/// # Returns
///
/// A `CompilerBinary` pointing to `target/profiling/moth`, or an error message.
///
pub fn build_profiling_compiler_with_timers(
    repository_root: &Path,
) -> Result<CompilerBinary, String> {
    let status = Command::new("cargo")
        .current_dir(repository_root)
        .args([
            "build",
            "--profile",
            "profiling",
            "--features",
            "detailed_timers",
            "--bin",
            "moth",
        ])
        .env("RUSTFLAGS", "-C force-frame-pointers=yes")
        .status()
        .map_err(|e| format!("Failed to execute cargo build: {}", e))?;

    if !status.success() {
        return Err("Profiling compiler build failed".to_string());
    }

    let moth_path = repository_root.join(profiling_compiler_path(env::consts::EXE_SUFFIX));

    if !moth_path.exists() {
        return Err(format!(
            "moth binary not found at '{}' after profiling build",
            moth_path.display()
        ));
    }

    let profiling_symbols = prepare_profiling_symbol_diagnostics(&moth_path);
    let symbol_dirs = candidate_symbol_dirs_for_binary(&moth_path);

    Ok(CompilerBinary {
        symbol_dirs,
        profiling_symbols: Some(profiling_symbols),
        path: moth_path,
    })
}

/// Build the release compiler path for the current platform suffix.
///
/// WHAT: Constructs the expected binary path under `target/release/`.
/// WHY: Platform-specific executable suffixes (e.g., `.exe` on Windows)
/// must be appended correctly so `exists()` checks find the real artifact.
fn release_compiler_path(exe_suffix: &str) -> PathBuf {
    compiler_path_with_suffix("target/release/moth", exe_suffix)
}

/// Build the profiling compiler path for the current platform suffix.
///
/// WHAT: Constructs the expected binary path under `target/profiling/`.
/// WHY: The profiling build uses a different target directory than release,
/// so it needs its own path resolver.
///
pub fn profiling_compiler_path(exe_suffix: &str) -> PathBuf {
    compiler_path_with_suffix("target/profiling/moth", exe_suffix)
}

/// Resolve a base binary path with the platform executable suffix.
///
/// WHAT: Appends the platform suffix to a base binary path (e.g., `.exe`).
/// WHY: Both release and profiling paths share the same suffix logic;
/// extracting it avoids duplication.
fn compiler_path_with_suffix(base: &str, exe_suffix: &str) -> PathBuf {
    let mut moth_path = PathBuf::from(base);
    if !exe_suffix.is_empty() {
        moth_path.set_extension(exe_suffix.trim_start_matches('.'));
    }

    moth_path
}

/// Prepare symbol lookup directories for the profiling binary.
///
/// WHAT: On macOS, asks `dsymutil` to materialize a `.dSYM` bundle when the
/// tool is available, then returns deterministic symbol directories for Samply.
/// WHY: Samply can record useful stage data without symbols, but raw-address
/// function names are not actionable for optimization decisions.
fn prepare_profiling_symbol_diagnostics(moth_path: &Path) -> ProfilingSymbolDiagnostics {
    generate_macos_dsym_if_available(moth_path);

    ProfilingSymbolDiagnostics {
        debug_info_setting: "debug = true",
        dsym_path: dsym_bundle_path(moth_path),
        dsym_uuid_match: verify_macos_dsym_uuid(moth_path),
    }
}

fn candidate_symbol_dirs_for_binary(moth_path: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(parent) = moth_path.parent() {
        push_existing_dir(&mut dirs, parent.to_path_buf());
    }

    let dsym_bundle = dsym_bundle_path(moth_path);
    push_existing_dir(&mut dirs, dsym_bundle.clone());
    push_existing_dir(&mut dirs, dsym_bundle.join("Contents/Resources/DWARF"));

    dirs
}

fn push_existing_dir(dirs: &mut Vec<PathBuf>, dir: PathBuf) {
    if dir.is_dir() && !dirs.iter().any(|existing| existing == &dir) {
        dirs.push(dir);
    }
}

fn dsym_bundle_path(moth_path: &Path) -> PathBuf {
    let mut bundle_name = moth_path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| "moth".into());
    bundle_name.push(".dSYM");

    moth_path.with_file_name(bundle_name)
}

#[cfg(target_os = "macos")]
fn verify_macos_dsym_uuid(moth_path: &Path) -> DsymUuidMatch {
    let dsym_bundle = dsym_bundle_path(moth_path);
    if !dsym_bundle.exists() {
        return DsymUuidMatch::Unknown;
    }

    let binary_output = match dwarfdump_uuid(moth_path) {
        Some(output) => output,
        None => return DsymUuidMatch::Unknown,
    };
    let dsym_output = match dwarfdump_uuid(&dsym_bundle) {
        Some(output) => output,
        None => return DsymUuidMatch::Unknown,
    };

    let binary_uuids = parse_dwarfdump_uuids(&binary_output);
    let dsym_uuids = parse_dwarfdump_uuids(&dsym_output);
    if binary_uuids.is_empty() || dsym_uuids.is_empty() {
        return DsymUuidMatch::Unknown;
    }

    if binary_uuids
        .iter()
        .any(|binary_uuid| dsym_uuids.iter().any(|dsym_uuid| dsym_uuid == binary_uuid))
    {
        DsymUuidMatch::Yes
    } else {
        DsymUuidMatch::No
    }
}

#[cfg(not(target_os = "macos"))]
fn verify_macos_dsym_uuid(_moth_path: &Path) -> DsymUuidMatch {
    DsymUuidMatch::Unknown
}

#[cfg(target_os = "macos")]
fn dwarfdump_uuid(path: &Path) -> Option<String> {
    let output = Command::new("dwarfdump").arg("--uuid").arg(path).output();

    let Ok(output) = output else {
        eprintln!(
            "Warning: dwarfdump was not available; dSYM UUID match is unknown for '{}'.",
            path.display()
        );
        return None;
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!(
            "Warning: dwarfdump --uuid failed for '{}': {}",
            path.display(),
            stderr.trim()
        );
        return None;
    }

    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(any(target_os = "macos", test))]
fn parse_dwarfdump_uuids(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            match (parts.next(), parts.next()) {
                (Some("UUID:"), Some(uuid)) => Some(uuid.to_ascii_uppercase()),
                _ => None,
            }
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn generate_macos_dsym_if_available(moth_path: &Path) {
    let dsym_bundle = dsym_bundle_path(moth_path);
    let output = Command::new("dsymutil")
        .arg(moth_path)
        .arg("-o")
        .arg(&dsym_bundle)
        .output();

    let Ok(output) = output else {
        eprintln!(
            "Warning: dsymutil was not available; Samply symbolication will use binary symbols only."
        );
        return;
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!(
            "Warning: dsymutil failed while preparing '{}': {}",
            moth_path.display(),
            stderr.trim()
        );
    }
}

#[cfg(not(target_os = "macos"))]
fn generate_macos_dsym_if_available(_moth_path: &Path) {}

#[cfg(test)]
mod tests;
