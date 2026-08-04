//! Local system identity for benchmark tracking
//!
//! This module provides privacy-safe system detection and persistence.
//! It creates and reads `benchmarks/local-data/system.toml` to give each
//! clone a stable identity without using machine-derived identifiers
//! (MAC address, hostname, username, disk serial, etc.).
//!
//! WHAT: Detects coarse OS/architecture, generates random local IDs, and
//!       persists them in a tiny TOML-like file.
//! WHY:  Enables per-system benchmark comparison and monthly summaries
//!       without exposing private machine information in tracked files.

use crate::bench_time::BenchmarkTimestamp;
use crate::bench_types::BenchmarkSystem;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::SystemTime;
use std::{env, fs, process};

/// Controls whether a missing local benchmark identity may be created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemIdentityMode {
    /// Read an existing identity only.
    ReadOnly,
    /// Create and persist an identity when none exists yet.
    CreateIfMissing,
}

/// Load or create the local system identity at one repository-anchored path.
///
/// - `CreateIfMissing`: creates the local-data directory and `system.toml`
///   when the file is missing.
/// - `ReadOnly`: returns `Ok(None)` when the file is missing.
///
/// Returns `Some(BenchmarkSystem)` when identity is available, `None` when
/// read-only mode is used and the file is missing.
pub(crate) fn load_or_create_system_at(
    path: &Path,
    mode: SystemIdentityMode,
) -> Result<Option<BenchmarkSystem>, String> {
    if path.exists() {
        let contents =
            fs::read_to_string(path).map_err(|e| format!("Failed to read system.toml: {}", e))?;
        return parse_system_toml(&contents).map(Some);
    }

    if mode == SystemIdentityMode::ReadOnly {
        return Ok(None);
    }
    let parent = path
        .parent()
        .ok_or_else(|| "system.toml path has no parent directory".to_string())?;
    fs::create_dir_all(parent).map_err(|e| {
        format!(
            "Failed to create local-data directory '{}': {}",
            parent.display(),
            e
        )
    })?;

    let (system_uuid, public_system_id) = generate_ids();
    let display_name = detect_display_name();

    let system = BenchmarkSystem {
        system_uuid,
        public_system_id,
        display_name,
    };

    write_system_toml(path, &system)?;
    Ok(Some(system))
}

/// Auto-detect a human-readable display name from OS and architecture.
///
/// Mappings are coarse by design; users may edit `system.toml` to change
/// "macOS Apple Silicon" to "macOS M1", for example.
fn detect_display_name() -> String {
    let os = env::consts::OS;
    let arch = env::consts::ARCH;

    match (os, arch) {
        ("macos", "aarch64") => "macOS Apple Silicon".to_string(),
        ("macos", "x86_64") => "macOS x64".to_string(),
        ("linux", "x86_64") => "Linux x64".to_string(),
        ("windows", "x86_64") => "Windows x64".to_string(),
        _ => format!("{} {}", os, arch),
    }
}

/// Generate a stable UUID and public ID from a one-time entropy mix.
///
/// WHAT: Combines UNIX timestamp nanos, process ID, and a stack-address salt
///       to produce random-looking but locally-stable identifiers.
/// WHY:  Non-security uniqueness is sufficient; avoids adding dependencies.
fn generate_ids() -> (String, String) {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("System time before UNIX epoch")
        .as_nanos();
    let pid = process::id() as u128;
    let salt: usize = &nanos as *const _ as usize;

    // Vary the byte ordering so uuid and public_id are derived differently
    let mix_uuid_1 = format!("uuid-{}-{}-{}", nanos, pid, salt);
    let mix_uuid_2 = format!("uuid-{}-{}-{}", salt, nanos, pid);
    let mix_public = format!("pub-{}-{}-{}", pid, salt, nanos);

    let h1 = hash_bytes(mix_uuid_1.as_bytes());
    let h2 = hash_bytes(mix_uuid_2.as_bytes());
    let h3 = hash_bytes(mix_public.as_bytes());

    let uuid = format!("{:016X}{:016X}", h1, h2);
    let public_id = format!("{:06X}", h3 & 0xFFFFFF);

    (uuid, public_id)
}

/// Hash a byte slice into a u64 using the standard library hasher.
fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for b in bytes {
        b.hash(&mut hasher);
    }
    hasher.finish()
}

/// Parse the tiny system.toml schema into a `BenchmarkSystem`.
///
/// Only `system_uuid`, `public_system_id`, and `display_name` are read.
/// `created_at` is ignored.
fn parse_system_toml(contents: &str) -> Result<BenchmarkSystem, String> {
    let mut system_uuid = None;
    let mut public_system_id = None;
    let mut display_name = None;

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let Some((key, raw_value)) = line.split_once('=') else {
            continue;
        };

        let key = key.trim();
        let value = raw_value.trim().trim_matches('"').to_string();

        match key {
            "system_uuid" => system_uuid = Some(value),
            "public_system_id" => public_system_id = Some(value),
            "display_name" => display_name = Some(value),
            _ => {}
        }
    }

    let system_uuid = system_uuid.ok_or("system.toml missing system_uuid")?;
    let public_system_id = public_system_id.ok_or("system.toml missing public_system_id")?;

    // If display_name is missing from the file, fall back to auto-detection.
    // This supports manual edits where a user might delete the line accidentally.
    let display_name = display_name.unwrap_or_else(detect_display_name);

    Ok(BenchmarkSystem {
        system_uuid,
        public_system_id,
        display_name,
    })
}

/// Write system.toml in the tiny manual TOML schema.
///
/// The file is human-editable; `display_name` may be changed by the user.
fn write_system_toml(path: &Path, system: &BenchmarkSystem) -> Result<(), String> {
    let ts = BenchmarkTimestamp::now();

    let contents = format!(
        "system_uuid = \"{}\"\n\
         public_system_id = \"{}\"\n\
         display_name = \"{}\"\n\
         created_at = \"{:04}-{:02}-{:02}T{:02}:{:02}\"\n",
        system.system_uuid,
        system.public_system_id,
        escape_toml_string(&system.display_name),
        ts.year,
        ts.month,
        ts.day,
        ts.hour,
        ts.minute,
    );

    fs::write(path, contents).map_err(|e| format!("Failed to write system.toml: {}", e))
}

/// Minimal TOML string escaping for the tiny system.toml schema.
///
/// WHAT: Escapes backslash and double-quote so manually-edited display names
///       do not corrupt the file.
fn escape_toml_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests;
