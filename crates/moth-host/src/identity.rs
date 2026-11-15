//! The canonical identity a native runtime matches one host binding by.

use std::fmt;

/// The canonical identity of one host binding.
///
/// WHAT: the canonical package path plus the structured package-local symbol path, such as
/// `@platform` and `["has_prefix"]`.
/// WHY: a runtime matches declared implementations against requirements by this value, never by
/// parsing the item string, an import index or an order of declaration. Two identities are equal
/// only when both the package and every symbol component are equal. The physical import module
/// and item spellings are owned by the native host ABI and arrive with the compiler's host
/// manifest.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostBindingIdentity {
    package_path: String,
    symbol_path: Vec<String>,
}

impl HostBindingIdentity {
    /// Creates the identity for one binding.
    ///
    /// `package_path` is the canonical package path, such as `@platform`, and `symbol_path`
    /// holds the package-local symbol components, such as `input` and `new`. The compiler has
    /// already validated both when it produces a requirement.
    pub fn new(
        package_path: impl Into<String>,
        symbol_path: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            package_path: package_path.into(),
            symbol_path: symbol_path.into_iter().map(Into::into).collect(),
        }
    }

    /// The canonical package path.
    pub fn package_path(&self) -> &str {
        &self.package_path
    }

    /// The package-local symbol components in order.
    pub fn symbol_path(&self) -> &[String] {
        &self.symbol_path
    }
}

impl fmt::Display for HostBindingIdentity {
    /// Writes the canonical `@package#symbol.path` spelling used to name the identity in errors.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}#", self.package_path)?;
        let mut components = self.symbol_path.iter();
        if let Some(first) = components.next() {
            formatter.write_str(first)?;
        }
        for component in components {
            write!(formatter, ".{component}")?;
        }
        Ok(())
    }
}
