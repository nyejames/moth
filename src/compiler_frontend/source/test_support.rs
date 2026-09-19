//! Test-only source identity and live-span setup.
//!
//! WHAT: gives focused frontend tests one small owner for an interned source path, explicit source
//! identity, string table and mutable extended-span builder.
//! WHY: tests that exercise producer-owned spans should use the same source context throughout a
//! preparation call instead of rebuilding ad hoc paths and builders at each boundary. This module
//! is compiled only for tests and does not add a production source-construction API.

use super::span::ExtendedSpanResolver;
use super::{ExtendedSpanBuilder, LocalSpan, SourceDatabase, SourceId};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::{LexedSource, TokenizeResult, tokenize};
use crate::compiler_frontend::tokenizer::tokens::{TokenIndex, TokenTag, TokenizerEntryMode};
use std::path::{Path, PathBuf};

/// One focused-test source context with an explicit identity and one live span owner.
pub(crate) struct TestSourceContext {
    source_id: SourceId,
    path: PathBuf,
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
        let string_table = StringTable::new();

        Self {
            source_id,
            path,
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

    /// Tokenize `source` through the real lexer with this context's identity, path, string
    /// table and live span builder.
    ///
    /// The returned `LexedSource` owns the canonical token arrays and the preparing path table;
    /// focused tests should pass those allocations to the same source-owner boundary as
    /// production rather than recreating a compatibility token stream.
    pub(crate) fn tokenize(
        &mut self,
        source: &str,
        path_fork: &mut PathInternerFork,
        style_directives: &StyleDirectiveRegistry,
        entry_mode: TokenizerEntryMode,
    ) -> TokenizeResult<LexedSource> {
        let source_id = self.source_id;
        let Self {
            path,
            string_table,
            span_builder,
            ..
        } = &mut *self;
        let interned_path = path_fork
            .try_intern_filesystem_path(path, string_table)
            .expect("test path should be UTF-8");
        tokenize(
            source,
            interned_path,
            entry_mode,
            style_directives,
            string_table,
            path_fork,
            source_id,
            span_builder,
        )
    }

    /// Borrow the two mutable producer-owned tables together for tokenization/preparation calls.
    pub(crate) fn preparation_parts(&mut self) -> (&mut StringTable, &mut ExtendedSpanBuilder) {
        (&mut self.string_table, &mut self.span_builder)
    }
}

/// Build a one-source database with `text` retained, for tests that resolve against a snapshot.
///
/// WHY: line ranges, line text, span resolution and render bridges all need the same loaded
/// record shape; sharing one constructor keeps those suites asserting behavior instead of
/// rebuilding registration scaffolding.
pub(crate) fn database_with_retained_text(text: &str) -> (SourceDatabase, SourceId) {
    let source_path = PathBuf::from("/project/main.moth");
    let mut string_table = StringTable::new();
    let mut database = SourceDatabase::build(
        std::iter::once(&source_path),
        &source_path,
        None,
        &mut string_table,
    )
    .expect("source identity should build");
    let source_id = database
        .get_by_canonical_path(&source_path)
        .expect("source should be registered")
        .id;
    database
        .retain_text(source_id, text.to_owned())
        .expect("source text should be retained");
    (database, source_id)
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

    #[test]
    fn tokenize_keeps_source_identity_and_live_span_ownership() {
        let source = "value = 1\n";
        let source_id = SourceId::from_index(3);
        let mut context =
            TestSourceContext::with_source_id(source_id, PathBuf::from("src/token.moth"));
        let mut path_fork = PathInternerFork::empty();
        let style_directives = StyleDirectiveRegistry::built_ins();

        let lexed = context
            .tokenize(
                source,
                &mut path_fork,
                &style_directives,
                TokenizerEntryMode::SourceFile,
            )
            .expect("source should tokenize");

        assert_eq!(lexed.file_id, source_id);
        assert_eq!(context.source_id(), source_id);
        assert_eq!(lexed.tokens.source(), source_id);
        assert!(
            path_fork.try_depth(lexed.logical_path).is_some(),
            "the interned path must come from the caller-owned fork"
        );
        let first_span = (0..lexed.tokens.len())
            .find_map(|index| {
                let index = TokenIndex::try_from_index(index)?;
                let token = lexed.tokens.token(index).ok()?;
                (token.tag() == TokenTag::SYMBOL).then_some(token.span())
            })
            .expect("source should tokenize a symbol");
        let range = first_span.resolve_with(context.span_resolver());
        assert_eq!(
            source.get(range.start() as usize..range.end() as usize),
            Some("value"),
            "token spans must resolve through the context's live builder"
        );
    }
}
