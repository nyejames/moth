//! Dense identities for build-local logical paths.
//!
//! WHAT: owns the mutable parent-linked trie, its module-local forks and its lookup-only table
//!       for complete-path operations. `SourceDatabase` embeds the mutable builder so source
//!       slots and path nodes share one build-lifetime identity base.
//! WHY:  logical compiler identity needs compact shared prefixes, while filesystem `PathBuf` and
//!       source snapshot identity remain separate owners. Source consumers resolve logical paths
//!       through the source database.
//!
//! Path domains:
//!
//! - Compiler logical and semantic component paths share this table as `PathId`. Their component
//!   semantics are compatible, so one dense trie serves both without a second interner.
//! - Filesystem paths remain cold `Path` and `PathBuf` values owned by source records and IO.
//!   Interning a filesystem path converts its components into logical IDs; the table never stores
//!   a `PathBuf` identity.
//! - Rendered free text is never interned as a path. Rendering resolves components through a
//!   string table into portable or native spellings without creating new identities.
//!
//! The implementation is split by data lifetime:
//!
//! - [`id`] defines the four-byte complete-path handle.
//! - [`builder`] interns one component at a time, merges worker deltas and freezes the dense
//!   node arrays.
//! - [`fork`] snapshots an immutable base for module-local deltas that merge deterministically.
//! - [`remap`] rewrites worker-local identities after a merge.
//! - [`frozen`] resolves and renders paths without mutable interning.

use crate::compiler_frontend::symbols::string_interning::{StringId, StringTableResolver};
use crate::compiler_frontend::utilities::portable_path_display::portable_path_text;
use std::ffi::OsString;
use std::path::{Component, MAIN_SEPARATOR_STR, Path, PathBuf};

/// A filesystem path containing a component that cannot be represented as UTF-8.
///
/// Filesystem identity is exact or rejected; lossy conversion could collapse distinct names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonUtf8PathComponent {
    pub(crate) path: PathBuf,
}

mod builder;
mod fork;
mod frozen;
mod id;
mod remap;

#[cfg(test)]
pub(crate) mod test_exhaustion;

#[cfg(test)]
pub(crate) use test_exhaustion::ForcedExhaustionGuard;

#[cfg(test)]
mod tests;

pub(crate) use builder::{PathInternError, PathInternerBuilder};
pub(crate) use fork::{PathInternerFork, PathInternerForkSource};
pub(crate) use frozen::PathTable;
pub(crate) use id::PathId;
pub(crate) use remap::PathIdRemap;

/// Render one resolved component sequence as portable text with `/` separators.
///
/// WHAT: the single portable renderer for forks, frozen tables and the diagnostic render context.
/// WHY: filesystem interning records the platform root (and any drive or share prefix) as an
/// ordinary first component. Joining every component with `/` would therefore insert a separator
/// after the root (`//project/src/page.mtf`) or mix separators (`C:/\project`). Rooted identities
/// are reassembled as native paths and displayed through the one host-path formatter, which owns
/// prefix and root spelling. Relative logical paths keep the exact component join because their
/// components may contain literal backslashes that are name text; the display formatter would
/// rewrite those.
pub(crate) fn render_portable_components<T: StringTableResolver + ?Sized>(
    components: &[StringId],
    string_table: &T,
) -> String {
    if let Some(native) = rooted_native_path(components, string_table) {
        return portable_path_text(native);
    }

    let Some((first, rest)) = components.split_first() else {
        return String::new();
    };

    let mut rendered = String::new();
    rendered.push_str(string_table.resolve(*first));

    for component in rest {
        rendered.push('/');
        rendered.push_str(string_table.resolve(*component));
    }

    rendered
}

/// Reassemble a resolved component sequence as a native path by pushing each component.
///
/// Native rendering keeps `PathBuf::push` semantics so relative and drive-relative spellings are
/// reconstructed the same way this renderer has always done. Rooted portable display must not use
/// this path: pushing `.`/`..` into a verbatim Windows path silently normalises it away.
fn render_native_components<T: StringTableResolver + ?Sized>(
    components: &[StringId],
    string_table: &T,
) -> PathBuf {
    let mut native = PathBuf::new();

    for component in components {
        native.push(string_table.resolve(*component));
    }

    native
}

/// Reassemble a rooted component sequence as a native path without reparsing any component.
///
/// WHAT: concatenates the interned prefix, root and name spellings with the native separator.
/// WHY: the component sequence already carries the parsed prefix and root, and `PathBuf::push`
/// reinterprets later components as paths. Pushing `..` into an accumulated verbatim Windows path
/// pops the preceding name, so display text would not match `portable_path_text` on the original
/// host path. A rooted identity is only returned when the concatenation still has a host root.
fn rooted_native_path<T: StringTableResolver + ?Sized>(
    components: &[StringId],
    string_table: &T,
) -> Option<PathBuf> {
    let first = string_table.resolve(*components.first()?);
    if !component_may_encode_host_root(first) {
        return None;
    }

    let mut native = OsString::new();
    let mut needs_separator = false;

    for component in components {
        let text = string_table.resolve(*component);
        let is_root = component_is_native_root(text);
        if needs_separator && !is_root {
            native.push(MAIN_SEPARATOR_STR);
        }
        native.push(text);
        needs_separator = !is_root;
    }

    let native = PathBuf::from(native);
    native.has_root().then_some(native)
}

/// Whether a component is the host root directory rather than a name.
///
/// Filesystem interning stores the platform root as an ordinary component: `/` on Unix and `\`
/// on Windows. `Component::as_os_str` always spells the root with the platform separator, even
/// when the authored Windows path used `/`.
fn component_is_native_root(component: &str) -> bool {
    component == MAIN_SEPARATOR_STR
}

/// Whether a first component is a host prefix or root component rather than a name.
///
/// The component spelling is parsed exactly as filesystem interning parsed it, so drive prefixes,
/// verbatim prefixes and UNC prefixes in either separator spelling are recognised without
/// hard-coding their text.
fn component_may_encode_host_root(first: &str) -> bool {
    matches!(
        Path::new(first).components().next(),
        Some(Component::Prefix(_) | Component::RootDir)
    )
}
