//! Frontend style directive registry used by tokenizer and AST template parsing.
//!
//! Registry order is stable and intentionally used for diagnostics, so unsupported-directive
//! errors show a deterministic directive list.

use crate::compiler_frontend::style_directives::builtins::frontend_built_in_directives;
use crate::compiler_frontend::style_directives::specs::{DirectiveOccurrence, StyleDirectiveSpec};
use crate::compiler_frontend::tokenizer::tokens::TemplateBodyMode;
use std::fmt::Write as _;
use std::sync::{Arc, LazyLock};

/// Immutable frontend directive set shared by every registry clone.
///
/// WHAT: the ordered specs behind `StyleDirectiveRegistry::built_ins`.
/// WHY: header parsing, tokenization and template parsing all need the same directive facts; one
///      cached owner keeps every registry instance on the same `Arc` instead of rebuilding the
///      spec vector per call or per scope.
static BUILT_IN_SPECS: LazyLock<Arc<[StyleDirectiveSpec]>> =
    LazyLock::new(|| frontend_built_in_directives().into());

/// Ordered registry used by tokenizer and AST template parsing.
///
/// The directive list is immutable once built, so it is held behind an `Arc` and cloning the
/// registry is a refcount bump. Every environment-time `ScopeContext` clones one into its
/// `ScopeShared`, so a `Vec` here deep-cloned every directive spec per scope.
#[derive(Clone, Debug)]
pub struct StyleDirectiveRegistry {
    ordered: Arc<[StyleDirectiveSpec]>,
}

impl StyleDirectiveRegistry {
    /// Frontend-owned directives available under every builder.
    pub fn built_ins() -> Self {
        Self {
            ordered: Arc::clone(&BUILT_IN_SPECS),
        }
    }

    pub fn find(&self, name: &str) -> Option<&StyleDirectiveSpec> {
        self.ordered.iter().find(|spec| spec.name == name)
    }

    /// Resolve only the template-body tokenization mode for a directive.
    pub fn body_mode_for(&self, name: &str) -> Option<TemplateBodyMode> {
        self.find(name).map(|spec| spec.body_mode)
    }

    /// Render the deterministic directive list that may appear at `occurrence`.
    ///
    /// WHAT: filters by the spec's declared attachment contexts, so a source-position diagnostic
    ///       never suggests a name that only a template head accepts and the reverse.
    /// WHY: the full registry is one ordered list, but a suggestion is only useful when the named
    ///       directive could actually be written at the failing position.
    pub fn supported_directives_for_diagnostic(&self, occurrence: DirectiveOccurrence) -> String {
        let mut rendered = String::new();
        let allowed = self.ordered.iter().filter(|spec| match occurrence {
            DirectiveOccurrence::TemplateHead => spec.contexts.allows_template_head(),
            DirectiveOccurrence::Source => spec.contexts.allows_source_modifier(),
        });
        for (index, spec) in allowed.enumerate() {
            if index > 0 {
                rendered.push_str(", ");
            }
            let _ = write!(rendered, "'${}'", spec.name);
        }
        rendered
    }
}
