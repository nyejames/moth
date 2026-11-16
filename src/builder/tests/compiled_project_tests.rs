//! Builder artefact registration failure tests.
//!
//! WHAT: pins the diagnostic text produced when `BuildArtifacts::add` rejects an output path.
//! WHY: these messages are the SDK's only explanation of a rejected artefact, so they must render
//!      native path values portably while keeping unrepresentable names identifiable.

use crate::builder::{BuildArtifacts, ErrorKind};
#[cfg(unix)]
use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
use std::path::Path;

#[cfg(unix)]
#[test]
fn non_utf8_artefact_paths_expose_their_bytes_with_debug_formatting() {
    let raw_path = OsString::from_vec(b"asset-\xFF.bin".to_vec());
    let path = Path::new(&raw_path);
    let error = BuildArtifacts::new()
        .add(path, Vec::new())
        .expect_err("a non-UTF-8 artefact path must be rejected");

    assert_eq!(error.kind(), ErrorKind::Build);
    assert_eq!(
        error.to_string(),
        format!("The build artefact path {path:?} is not valid UTF-8.")
    );
}

#[test]
fn rejected_artefact_paths_render_native_separators_as_portable_text() {
    let error = BuildArtifacts::new()
        .add(Path::new("nested\\..\\file.bin"), Vec::new())
        .expect_err("a parent segment must be rejected");

    assert_eq!(error.kind(), ErrorKind::Build);
    assert!(
        error
            .to_string()
            .contains("The build artefact path 'nested/../file.bin' cannot be used:"),
        "artefact path prose must use portable display text: {error}"
    );
}
