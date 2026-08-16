//! Shared frontend utility helpers.
//!
//! These are small cross-cutting helpers that predate some newer subsystem boundaries and are
//! still reused in formatting code.

use std::path::{Path, PathBuf};

// For Windows compatibility.
pub fn normalize_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};

        let mut components = path.components();

        if let Some(Component::Prefix(prefix)) = components.next() {
            match prefix.kind() {
                Prefix::VerbatimDisk(disk) => {
                    // Strip \\?\C:\ → C:\
                    let mut new_path = PathBuf::from(format!(r"{}:\", disk as char));
                    for component in components {
                        if let Component::Normal(name) = component {
                            new_path.push(name);
                        }
                    }
                    return new_path;
                }
                Prefix::VerbatimUNC(server, share) => {
                    // Convert \\?\UNC\server\share → \\server\share
                    let mut new_path = PathBuf::from(r"\\");
                    new_path.push(server);
                    new_path.push(share);
                    new_path.push(components.as_path());
                    return new_path;
                }
                _ => {}
            }
        }
    }

    path.to_path_buf()
}

/// Render a filesystem path as deterministic, platform-independent text.
///
/// WHAT: applies the existing filesystem-path normalisation, then renders separators as `/`.
/// WHY: diagnostics, logical display text, and cross-platform assertions need stable text while
///      filesystem reads and writes must continue to use the native `Path` representation.
pub(crate) fn portable_path_text(path: impl AsRef<Path>) -> String {
    normalize_path(path.as_ref())
        .to_string_lossy()
        .replace('\\', "/")
}

/// Character classification helpers shared by tokenizer and template formatting.
///
/// WHAT: exposes source-level whitespace checks on `char`.
/// WHY: these checks are lexical formatting policy, not numeric parsing.
pub trait CharacterParsing {
    fn is_non_newline_whitespace(&self) -> bool;
}

impl CharacterParsing for char {
    fn is_non_newline_whitespace(&self) -> bool {
        self.is_whitespace() && self != &'\n' && self != &'\r'
    }
}
