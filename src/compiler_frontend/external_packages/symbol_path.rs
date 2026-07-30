//! Structured external symbol path within a virtual package.
//!
//! WHAT: represents a dotted external-package symbol path such as `io.input.new` as a
//! sequence of owned path components rather than a dot-joined string.
//! WHY: namespace-aware packages need stable component-level identity so the registry,
//!      import resolution, and namespace-record construction can distinguish `a.foo` from
//!      `b.foo` while sharing the same leaf name.
//!
//! The canonical representation is the component vector. Display text joins components with
//! `.` only for human-readable diagnostics; no caller should parse the joined form.
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};

use std::fmt;

/// Invalid external symbol path component.
///
/// WHAT: captures why a requested component cannot be part of a symbol path.
/// WHY: keeps validation close to the type so callers can surface structured errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidExternalSymbolComponent {
    Empty,
    ContainsPathSeparator,
    ContainsNamespaceSeparator,
}

/// Error constructing an `ExternalSymbolPath`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalSymbolPathError {
    EmptyPath,
    InvalidComponent {
        index: usize,
        component: String,
        reason: InvalidExternalSymbolComponent,
    },
}

/// Structured path to an external symbol inside one virtual package.
///
/// WHAT: a non-empty sequence of identifier-like components. A one-component path is the
/// ordinary flat package symbol; multi-component paths are nested namespace symbols.
/// WHY: the registry needs to store and look up symbols by their full path to support
/// nested namespaces without flattening them into a single string.
#[derive(Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ExternalSymbolPath {
    components: Vec<String>,
}

impl Clone for ExternalSymbolPath {
    fn clone(&self) -> Self {
        increment_frontend_counter(FrontendCounter::ExternalSymbolPathCloneCount);
        Self {
            components: self.components.clone(),
        }
    }
}

impl ExternalSymbolPath {
    /// Validates and creates a single-component path from a leaf symbol name.
    pub fn try_from_single(name: impl Into<String>) -> Result<Self, ExternalSymbolPathError> {
        Self::try_from_components(vec![name.into()])
    }

    /// Creates a single-component path from a leaf symbol name.
    ///
    /// WHAT: convenience constructor for the existing flat registration and lookup APIs.
    /// WHY: most current registrations are one-component; this keeps call sites terse.
    pub fn from_single(name: impl Into<String>) -> Self {
        Self::try_from_single(name).expect("trusted external symbol path component should be valid")
    }

    /// Creates a path from trusted component data.
    ///
    /// WHAT: convenience constructor for statically known compiler package surfaces.
    /// WHY: trusted registration tables should stay readable, while user-influenced providers
    /// must use `try_from_components` and surface validation failures as diagnostics/errors.
    pub fn from_components(components: Vec<String>) -> Self {
        Self::try_from_components(components)
            .expect("trusted external symbol path components should be valid")
    }

    /// Validates and constructs a path from arbitrary components.
    ///
    /// WHAT: safe constructor for paths built from parsed or user-influenced input.
    /// WHY: external symbol names can arrive from JS annotations or other provider output,
    /// so construction must fail gracefully rather than panic.
    pub fn try_from_components(components: Vec<String>) -> Result<Self, ExternalSymbolPathError> {
        if components.is_empty() {
            return Err(ExternalSymbolPathError::EmptyPath);
        }

        for (index, component) in components.iter().enumerate() {
            Self::validate_component(component).map_err(|reason| {
                ExternalSymbolPathError::InvalidComponent {
                    index,
                    component: component.clone(),
                    reason,
                }
            })?;
        }

        Ok(Self { components })
    }

    /// Validates a single path component.
    fn validate_component(component: &str) -> Result<(), InvalidExternalSymbolComponent> {
        if component.is_empty() {
            return Err(InvalidExternalSymbolComponent::Empty);
        }
        if component.contains('/') {
            return Err(InvalidExternalSymbolComponent::ContainsPathSeparator);
        }
        if component.contains('.') {
            return Err(InvalidExternalSymbolComponent::ContainsNamespaceSeparator);
        }
        Ok(())
    }

    /// Returns the path components.
    pub fn components(&self) -> &[String] {
        &self.components
    }

    /// Returns the leaf (last) component of the path.
    pub fn leaf(&self) -> &str {
        self.components
            .last()
            .expect("external symbol path is never empty")
    }

    /// Returns the number of components in the path.
    pub fn component_count(&self) -> usize {
        self.components.len()
    }

    /// Returns true for a one-component (flat) symbol path.
    pub fn is_single(&self) -> bool {
        self.components.len() == 1
    }

    /// Appends a component, returning a new path.
    ///
    /// WHAT: builds a child symbol path such as `parent.child` without mutating the parent.
    /// WHY: registration helpers for nested namespaces need to derive child paths cleanly.
    pub fn child(&self, component: impl Into<String>) -> Self {
        let component = component.into();
        Self::validate_component(&component)
            .expect("trusted external symbol path child component should be valid");
        let mut components = self.components.clone();
        components.push(component);
        Self { components }
    }

    /// Appends a component in place.
    ///
    /// WHAT: mutating variant of `child` for call sites that already own the path.
    pub fn push(&mut self, component: impl Into<String>) {
        let component = component.into();
        Self::validate_component(&component)
            .expect("trusted external symbol path component to push should be valid");
        self.components.push(component);
    }

    /// Human-readable dotted representation for diagnostics and logging.
    ///
    /// WHAT: joins components with `.` to produce text such as `io.input.new`.
    /// WHY: humans read dotted paths; this is a render helper, not a canonical identity.
    /// Do not parse the returned string.
    pub fn display_text(&self) -> String {
        self.components.join(".")
    }
}

impl fmt::Display for ExternalSymbolPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.display_text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_single_creates_one_component_path() {
        let path = ExternalSymbolPath::from_single("foo");
        assert_eq!(path.components(), &["foo"]);
        assert_eq!(path.leaf(), "foo");
        assert_eq!(path.component_count(), 1);
        assert!(path.is_single());
    }

    #[test]
    fn from_components_preserves_all_components() {
        let path = ExternalSymbolPath::from_components(vec![
            "io".to_owned(),
            "input".to_owned(),
            "new".to_owned(),
        ]);
        assert_eq!(path.components(), &["io", "input", "new"]);
        assert_eq!(path.leaf(), "new");
        assert_eq!(path.component_count(), 3);
        assert!(!path.is_single());
    }

    #[test]
    fn try_from_components_rejects_empty_path() {
        let result = ExternalSymbolPath::try_from_components(Vec::new());
        assert_eq!(result, Err(ExternalSymbolPathError::EmptyPath));
    }

    #[test]
    fn try_from_components_rejects_empty_component() {
        let result = ExternalSymbolPath::try_from_components(vec!["io".to_owned(), "".to_owned()]);
        assert!(
            matches!(
                result,
                Err(ExternalSymbolPathError::InvalidComponent {
                    index: 1,
                    ref component,
                    reason: InvalidExternalSymbolComponent::Empty,
                }) if component.is_empty()
            ),
            "expected empty-component error, got {result:?}"
        );
    }

    #[test]
    fn try_from_components_rejects_separators() {
        let slash = ExternalSymbolPath::try_from_components(vec!["io/input".to_owned()]);
        assert!(
            matches!(
                slash,
                Err(ExternalSymbolPathError::InvalidComponent {
                    reason: InvalidExternalSymbolComponent::ContainsPathSeparator,
                    ..
                })
            ),
            "expected path-separator error, got {slash:?}"
        );

        let dot = ExternalSymbolPath::try_from_components(vec!["io.input".to_owned()]);
        assert!(
            matches!(
                dot,
                Err(ExternalSymbolPathError::InvalidComponent {
                    reason: InvalidExternalSymbolComponent::ContainsNamespaceSeparator,
                    ..
                })
            ),
            "expected namespace-separator error, got {dot:?}"
        );
    }

    #[test]
    fn child_appends_component() {
        let parent = ExternalSymbolPath::from_single("input");
        let child = parent.child("new");
        assert_eq!(child.components(), &["input", "new"]);
        assert_eq!(parent.components(), &["input"]);
    }

    #[test]
    fn push_appends_in_place() {
        let mut path = ExternalSymbolPath::from_single("input");
        path.push("new");
        assert_eq!(path.components(), &["input", "new"]);
    }

    #[test]
    fn display_text_joins_with_dots() {
        let path = ExternalSymbolPath::from_components(vec![
            "io".to_owned(),
            "input".to_owned(),
            "new".to_owned(),
        ]);
        assert_eq!(path.display_text(), "io.input.new");
        assert_eq!(path.to_string(), "io.input.new");
    }
}
