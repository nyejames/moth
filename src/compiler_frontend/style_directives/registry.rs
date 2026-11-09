//! Frontend style directive registry used by tokenizer and AST template parsing.
//!
//! Registry order is stable and intentionally used for diagnostics, so unsupported-directive
//! errors show a deterministic directive list.

use crate::compiler_frontend::style_directives::builtins::frontend_built_in_directives;
use crate::compiler_frontend::style_directives::specs::StyleDirectiveSpec;
use crate::compiler_frontend::tokenizer::tokens::TemplateBodyMode;
use std::fmt::Write as _;
use std::sync::Arc;

/// Ordered registry used by tokenizer and AST template parsing.
///
/// The directive list is immutable once built, so it is held behind an `Arc` and cloning the
/// registry is a refcount bump. Every environment-time `ScopeContext` clones one into its
/// `ScopeShared`, so a `Vec` here deep-cloned every directive spec per scope.
#[derive(Clone, Debug, Default)]
pub struct StyleDirectiveRegistry {
    ordered: Arc<[StyleDirectiveSpec]>,
}

impl StyleDirectiveRegistry {
    /// Frontend-owned directives available under every builder.
    pub fn built_ins() -> Self {
        Self {
            ordered: frontend_built_in_directives().into(),
        }
    }

    pub fn find(&self, name: &str) -> Option<&StyleDirectiveSpec> {
        self.ordered.iter().find(|spec| spec.name == name)
    }

    /// Resolve only the template-body tokenization mode for a directive.
    pub fn body_mode_for(&self, name: &str) -> Option<TemplateBodyMode> {
        self.find(name).map(|spec| spec.body_mode)
    }

    /// Render a deterministic directive list for user-facing diagnostics.
    pub fn supported_directives_for_diagnostic(&self) -> String {
        let mut rendered = String::new();
        for (index, spec) in self.ordered.iter().enumerate() {
            if index > 0 {
                rendered.push_str(", ");
            }
            let _ = write!(rendered, "'${}'", spec.name);
        }
        rendered
    }
}
