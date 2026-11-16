//! Frontend character classification and Windows verbatim-path normalization for comparison and
//! rendering.

use std::path::{Path, PathBuf};

/// Strip Windows verbatim disk/UNC prefixes so canonicalised paths compare and render like
/// ordinary paths.
///
/// WHAT: converts `\\?\C:\...` to `C:\...` and `\\?\UNC\server\share\...` to
/// `\\server\share\...`, preserving every component after the prefix and root.
/// WHY: canonical Windows paths carry verbatim prefixes that break ordinary equality, prefix
/// tests and display text. This is for comparison and rendering only, not general
/// canonicalisation, and it is the identity function on non-Windows hosts.
pub(crate) fn normalize_windows_verbatim_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};

        let mut components = path.components();

        if let Some(Component::Prefix(prefix)) = components.next() {
            match prefix.kind() {
                Prefix::VerbatimDisk(disk) => {
                    // Strip \\?\C:\ → C:\, preserving `.`, `..` and other remaining components.
                    let mut new_path = PathBuf::from(format!(r"{}:\", disk as char));
                    new_path.push(components.as_path());
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
