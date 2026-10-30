//! Source-backed package root-file identity and dependency prefix matching.
//!
//! WHAT: exposes the shared root/config filename owner, whole-component prefix matching and their
//! focused tests.
//! WHY: Stage 0, path resolution and header dependency validation classify root files and package
//! prefixes through one shared policy.

use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

pub(crate) mod root_file;

/// Return the registered package prefix spelled by `components`' leading whole components, with
/// its component count.
///
/// WHY: builder prefixes may span several components, such as `web/site`, so lookups keyed by a
/// complete prefix can't compare only the first dependency component. Registration rejects
/// overlapping prefixes, so at most one prefix matches.
pub(crate) fn match_package_prefix<'a>(
    prefixes: impl IntoIterator<Item = &'a String>,
    components: &[StringId],
    string_table: &StringTable,
) -> Option<(&'a String, usize)> {
    prefixes.into_iter().find_map(|prefix| {
        let mut matched = 0;
        for expected in prefix.split('/') {
            let actual = components.get(matched)?;
            if string_table.resolve(*actual) != expected {
                return None;
            }
            matched += 1;
        }
        Some((prefix, matched))
    })
}

#[cfg(test)]
#[path = "tests/root_file_tests.rs"]
mod tests;
