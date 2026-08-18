use super::*;

/// Assert that no filesystem node exists at `path`.
///
/// Only `NotFound` is accepted as missing — permission errors and dangling
/// symlinks are not absence.
#[track_caller]
fn assert_path_missing(path: &std::path::Path) {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(metadata) => panic!("expected no filesystem node at {path:?}, found {metadata:?}"),
        Err(error) => panic!("failed to inspect {path:?} for absence assertion: {error}"),
    }
}

/// Assert that `path` is a regular file.
#[track_caller]
fn assert_regular_file(path: &std::path::Path) {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            assert!(
                metadata.is_file(),
                "expected a regular file at {path:?}, found {metadata:?}"
            );
        }
        Err(error) => panic!("failed to inspect {path:?} for regular-file assertion: {error}"),
    }
}

#[test]
fn test_detect_display_name_mappings() {
    // We cannot change the actual OS/ARCH at runtime, but we can verify
    // the function returns a non-empty string and that known combos would
    // map correctly by testing the match logic indirectly.
    let name = detect_display_name();
    assert!(!name.is_empty());

    // Verify that the current platform mapping at least contains the OS name.
    let os = env::consts::OS;
    assert!(
        name.to_lowercase().contains(&os.to_lowercase()),
        "display_name '{}' should contain OS '{}'",
        name,
        os
    );
}

#[test]
fn test_generate_ids_format() {
    let (uuid, public_id) = generate_ids();

    // UUID: 32 uppercase hex characters
    assert_eq!(uuid.len(), 32);
    assert!(uuid.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(
        uuid.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_numeric())
    );

    // Public ID: 6 uppercase hex characters
    assert_eq!(public_id.len(), 6);
    assert!(public_id.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(
        public_id
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_numeric())
    );
}

#[test]
fn test_parse_system_toml_valid() {
    let toml = r#"system_uuid = "9F0E0DAB7C7A4D07A1D7E6C50F9C7A01"
public_system_id = "B7F2A9"
display_name = "macOS M1"
created_at = "2026-05-10T15:21"
"#;

    let system = parse_system_toml(toml).unwrap();
    assert_eq!(system.system_uuid, "9F0E0DAB7C7A4D07A1D7E6C50F9C7A01");
    assert_eq!(system.public_system_id, "B7F2A9");
    assert_eq!(system.display_name, "macOS M1");
}

#[test]
fn test_parse_system_toml_missing_display_name_falls_back() {
    let toml = r#"system_uuid = "9F0E0DAB7C7A4D07A1D7E6C50F9C7A01"
public_system_id = "B7F2A9"
created_at = "2026-05-10T15:21"
"#;

    let system = parse_system_toml(toml).unwrap();
    assert_eq!(system.system_uuid, "9F0E0DAB7C7A4D07A1D7E6C50F9C7A01");
    assert_eq!(system.public_system_id, "B7F2A9");
    // Should fall back to auto-detected display name
    assert!(!system.display_name.is_empty());
}

#[test]
fn test_parse_system_toml_missing_uuid_errors() {
    let toml = r#"public_system_id = "B7F2A9"
display_name = "macOS M1"
"#;

    let result = parse_system_toml(toml);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("system_uuid"));
}

#[test]
fn test_parse_system_toml_missing_public_id_errors() {
    let toml = r#"system_uuid = "9F0E0DAB7C7A4D07A1D7E6C50F9C7A01"
display_name = "macOS M1"
"#;

    let result = parse_system_toml(toml);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("public_system_id"));
}

#[test]
fn test_escape_toml_string() {
    assert_eq!(escape_toml_string("macOS M1"), "macOS M1");
    assert_eq!(escape_toml_string("with \"quotes\""), "with \\\"quotes\\\"");
    assert_eq!(escape_toml_string("a\\b"), "a\\\\b");
}

#[test]
fn test_write_and_read_system_toml_roundtrip() {
    let temp = tempfile::tempdir().expect("should create temp dir");
    let path = temp.path().join("bench_system_test_roundtrip.toml");

    let system = BenchmarkSystem {
        system_uuid: "AABBCCDD11223344556677889900AABB".to_string(),
        public_system_id: "DEADBE".to_string(),
        display_name: "Test System".to_string(),
    };

    write_system_toml(&path, &system).unwrap();
    let contents = std::fs::read_to_string(&path).unwrap();
    let parsed = parse_system_toml(&contents).unwrap();

    assert_eq!(parsed.system_uuid, system.system_uuid);
    assert_eq!(parsed.public_system_id, system.public_system_id);
    assert_eq!(parsed.display_name, system.display_name);
}

#[test]
fn test_load_or_create_system_check_mode_missing_file() {
    // Use a nested path under a fresh temp dir — non-existence is the contract.
    let temp = tempfile::tempdir().expect("should create temp dir");
    let toml_path = temp.path().join("nested").join("system.toml");

    // Check mode with missing file should return Ok(None) and create nothing
    let result = load_or_create_system_at(&toml_path, SystemIdentityMode::ReadOnly);
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
    assert_path_missing(&toml_path);
    // The parent directory should not have been created by check mode.
    assert_path_missing(&temp.path().join("nested"));
}

#[test]
fn test_load_or_create_system_record_mode_creates_file() {
    let temp = tempfile::tempdir().expect("should create temp dir");
    let toml_path = temp.path().join("system.toml");

    // Record mode with missing file should create it
    let result = load_or_create_system_at(&toml_path, SystemIdentityMode::CreateIfMissing);
    assert!(result.is_ok());
    let system1 = result.unwrap().expect("Record mode should return Some");
    assert_regular_file(&toml_path);

    // Record mode again should reuse the existing file
    let result2 = load_or_create_system_at(&toml_path, SystemIdentityMode::CreateIfMissing);
    assert!(result2.is_ok());
    let system2 = result2.unwrap().expect("Record mode should return Some");
    assert_eq!(system1.system_uuid, system2.system_uuid);
    assert_eq!(system1.public_system_id, system2.public_system_id);
}

#[test]
fn test_load_or_create_system_record_then_check_reuses() {
    let temp = tempfile::tempdir().expect("should create temp dir");
    let toml_path = temp.path().join("system.toml");

    // Record mode creates the file
    let record_result = load_or_create_system_at(&toml_path, SystemIdentityMode::CreateIfMissing);
    let system_from_record = record_result.unwrap().unwrap();

    // Check mode should read the existing file
    let check_result = load_or_create_system_at(&toml_path, SystemIdentityMode::ReadOnly);
    let system_from_check = check_result.unwrap().unwrap();

    assert_eq!(
        system_from_record.system_uuid,
        system_from_check.system_uuid
    );
    assert_eq!(
        system_from_record.public_system_id,
        system_from_check.public_system_id
    );
}
