use super::*;
use std::path::{Path, PathBuf};

// ----------------------------
//  Release compiler path
// ----------------------------

#[test]
fn release_compiler_path_without_suffix_uses_unix_name() {
    assert_eq!(release_compiler_path(""), Path::new("target/release/moth"));
}

#[test]
fn release_compiler_path_with_suffix_uses_platform_extension() {
    assert_eq!(
        release_compiler_path(".exe"),
        Path::new("target/release/moth.exe")
    );
}

// ----------------------------
//  Profiling compiler path
// ----------------------------

#[test]
fn profiling_compiler_path_without_suffix_uses_unix_name() {
    assert_eq!(
        profiling_compiler_path(""),
        Path::new("target/profiling/moth")
    );
}

#[test]
fn profiling_compiler_path_with_suffix_uses_platform_extension() {
    assert_eq!(
        profiling_compiler_path(".exe"),
        Path::new("target/profiling/moth.exe")
    );
}

// ----------------------------
//  CompilerBinary wrapper
// ----------------------------

#[test]
fn compiler_binary_exposes_borrowed_path() {
    let binary = CompilerBinary {
        path: PathBuf::from("target/release/moth.exe"),
        symbol_dirs: Vec::new(),
        profiling_symbols: None,
    };

    assert_eq!(binary.as_path(), Path::new("target/release/moth.exe"));
}

#[test]
fn compiler_binary_clones_release_path() {
    let binary = CompilerBinary {
        path: PathBuf::from("target/release/moth"),
        symbol_dirs: Vec::new(),
        profiling_symbols: None,
    };
    let cloned = binary.clone();

    assert_eq!(binary.as_path(), cloned.as_path());
}

#[test]
fn compiler_binary_clones_profiling_path() {
    let binary = CompilerBinary {
        path: PathBuf::from("target/profiling/moth"),
        symbol_dirs: vec![PathBuf::from("target/profiling")],
        profiling_symbols: Some(ProfilingSymbolDiagnostics {
            debug_info_setting: "debug = true",
            dsym_path: PathBuf::from("target/profiling/moth.dSYM"),
            dsym_uuid_match: DsymUuidMatch::Unknown,
        }),
    };
    let cloned = binary.clone();

    assert_eq!(binary.as_path(), cloned.as_path());
    assert_eq!(binary.symbol_dirs, cloned.symbol_dirs);
    assert_eq!(
        cloned.profiling_symbols.unwrap().debug_info_setting,
        "debug = true"
    );
}

#[test]
fn dsym_bundle_path_uses_binary_file_name() {
    assert_eq!(
        dsym_bundle_path(Path::new("target/profiling/moth")),
        Path::new("target/profiling/moth.dSYM")
    );
}

#[test]
fn parse_dwarfdump_uuids_extracts_uppercase_uuid_values() {
    let output = "\
UUID: abcdefab-1234-5678-90ab-abcdefabcdef (arm64) target/profiling/moth
UUID: 11111111-2222-3333-4444-555555555555 (x86_64) target/profiling/moth
";

    assert_eq!(
        parse_dwarfdump_uuids(output),
        vec![
            "ABCDEFAB-1234-5678-90AB-ABCDEFABCDEF".to_string(),
            "11111111-2222-3333-4444-555555555555".to_string()
        ]
    );
}

#[test]
fn dsym_uuid_match_display_labels_are_stable() {
    assert_eq!(DsymUuidMatch::Yes.as_str(), "yes");
    assert_eq!(DsymUuidMatch::No.as_str(), "no");
    assert_eq!(DsymUuidMatch::Unknown.as_str(), "unknown");
}
