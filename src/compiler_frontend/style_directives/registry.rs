//! Merged style directive registry used by tokenizer and AST template parsing.
//!
//! Registry order is stable and intentionally used for diagnostics, so unsupported-directive
//! errors show a deterministic directive list.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::style_directives::builtins::frontend_built_in_directives;
use crate::compiler_frontend::style_directives::specs::StyleDirectiveSpec;
use crate::compiler_frontend::tokenizer::tokens::TemplateBodyMode;
use std::fmt::Write as _;
use std::sync::Arc;

/// Ordered registry used by tokenizer and AST template parsing.
///
/// The directive list is immutable once built, so it is held behind an `Arc` and cloning the
/// registry is a refcount bump. Every environment-time `ScopeContext` clones one of these, so a
/// `Vec` here charged a fresh allocation per directive per scope.
#[derive(Clone, Debug, Default)]
pub struct StyleDirectiveRegistry {
    ordered: Arc<[StyleDirectiveSpec]>,
}

impl StyleDirectiveRegistry {
    /// Frontend-owned directives that are always available.
    pub fn built_ins() -> Self {
        Self {
            ordered: frontend_built_in_directives().into(),
        }
    }

    /// Merge project-builder directives over frontend-owned directives.
    ///
    /// Rules:
    /// - Frontend-owned directive names cannot be overridden by project builders.
    /// - Project builders may only provide `StyleDirectiveKind::Handler`.
    /// - For project-owned names, later entries replace earlier entries by exact name.
    pub fn merged(builder_specs: &[StyleDirectiveSpec]) -> Result<Self, CompilerError> {
        let frontend_owned = frontend_built_in_directives();
        let mut merged = frontend_owned.clone();

        for builder_spec in builder_specs {
            if builder_spec.is_core() {
                return Err(CompilerError::compiler_error(format!(
                    "Project builder style directive '${}' cannot be registered as a core directive.",
                    builder_spec.name
                )));
            }

            if let Some(frontend_conflict) = frontend_owned
                .iter()
                .find(|spec| spec.name == builder_spec.name)
            {
                return Err(CompilerError::compiler_error(format!(
                    "Project builder style directive '${}' cannot override frontend-owned directive '${}'.",
                    builder_spec.name, frontend_conflict.name
                )));
            }

            if let Some(existing) = merged
                .iter_mut()
                .find(|spec| spec.name == builder_spec.name)
            {
                *existing = builder_spec.clone();
            } else {
                merged.push(builder_spec.clone());
            }
        }

        Ok(Self {
            ordered: merged.into(),
        })
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
