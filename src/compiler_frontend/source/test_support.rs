//! Test-only source identity and live-span setup.
//!
//! WHAT: gives focused frontend tests one small owner for an interned source path, explicit source
//! identity, string table and mutable extended-span builder.
//! WHY: tests that exercise producer-owned spans should use the same source context throughout a
//! preparation call instead of rebuilding ad hoc paths and builders at each boundary. This module
//! is compiled only for tests and does not add a production source-construction API.

use super::span::ExtendedSpanResolver;
use super::{ExtendedSpanBuilder, LocalSpan, SourceId};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::path::{Path, PathBuf};

/// One focused-test source context with an explicit identity and one live span owner.
pub(crate) struct TestSourceContext {
    source_id: SourceId,
    path: PathBuf,
    source_path: InternedPath,
    string_table: StringTable,
    span_builder: ExtendedSpanBuilder,
}

impl TestSourceContext {
    /// Create a context for the reserved compilation-root source identity.
    pub(crate) fn new(path: impl Into<PathBuf>) -> Self {
        Self::with_source_id(SourceId::COMPILATION_ROOT, path)
    }

    /// Create a context with an explicitly selected non-root or root source identity.
    pub(crate) fn with_source_id(source_id: SourceId, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut string_table = StringTable::new();
        let source_path = InternedPath::try_from_filesystem_path(&path, &mut string_table)
            .expect("test source path should be UTF-8");

        Self {
            source_id,
            path,
            source_path,
            string_table,
            span_builder: ExtendedSpanBuilder::new(),
        }
    }

    pub(crate) fn source_id(&self) -> SourceId {
        self.source_id
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn source_path(&self) -> &InternedPath {
        &self.source_path
    }

    pub(crate) fn span_builder(&self) -> &ExtendedSpanBuilder {
        &self.span_builder
    }

    pub(crate) fn span_builder_mut(&mut self) -> &mut ExtendedSpanBuilder {
        &mut self.span_builder
    }

    /// The resolver qualified for this context's source, as global spans need.
    pub(crate) fn span_resolver(&self) -> ExtendedSpanResolver<'_> {
        self.span_builder.resolver_for(self.source_id)
    }

    /// Borrow the two mutable producer-owned tables together for tokenization/preparation calls.
    pub(crate) fn preparation_parts(&mut self) -> (&mut StringTable, &mut ExtendedSpanBuilder) {
        (&mut self.string_table, &mut self.span_builder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_keeps_explicit_identity_and_resolves_long_live_span() {
        let mut context = TestSourceContext::with_source_id(
            SourceId::from_index(7),
            PathBuf::from("src/test.moth"),
        );
        let span = LocalSpan::exact(11, 1300, context.span_builder_mut())
            .expect("the test span should use the extended table");
        let range = span.resolve_with(context.span_resolver());

        assert_eq!(context.source_id(), SourceId::from_index(7));
        assert_eq!(range.start(), 11);
        assert_eq!(range.end(), 1311);
        assert_eq!(context.span_builder().len(), 1);
    }
}
