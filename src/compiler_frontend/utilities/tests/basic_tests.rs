//! Tests for shared frontend path and text helpers.
//!
//! WHAT: protects platform-independent path rendering at the utility owner.
//! WHY: diagnostics and test assertions must not each rediscover how native paths become stable
//!      display text.

use crate::compiler_frontend::utilities::basic::portable_path_text;
use std::path::Path;

#[test]
fn portable_path_text_uses_forward_slashes() {
    let native_path = Path::new("src").join("docs").join("guide.moth");
    assert_eq!(portable_path_text(&native_path), "src/docs/guide.moth");
    assert_eq!(
        portable_path_text(Path::new(r"src\docs\guide.moth")),
        "src/docs/guide.moth"
    );
}

#[cfg(windows)]
#[test]
fn portable_path_text_strips_windows_extended_prefix() {
    assert_eq!(
        portable_path_text(Path::new(r"\\?\C:\workspace\main.moth")),
        "C:/workspace/main.moth"
    );
}
