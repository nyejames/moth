//! Direct Moth template compilation API for the HTML project builder.
//!
//! WHAT: exposes a bounded API that turns `.mtf` files or in-memory Moth template sources into folded
//! content strings.
//! WHY: tooling can compile Moth template content without invoking HTML artifact planning, HIR
//! generation, borrow validation, output folders, or CLI/project-type behavior.
//!
//! The implementation deliberately routes through the existing tokenizer, synthetic Moth template
//! header preparation, dependency sorting, and AST folding pipeline. This module must not grow a
//! parallel Markdown/template renderer.

// The direct API is intentionally crate-local today. Keep the allowance at this module boundary so
// tooling can adopt the stable surface without forcing artificial in-tree callers.
#![allow(dead_code)]

mod bundle;
mod compile;
mod input;
mod output;
mod render;
mod scope;

// This is the crate-facing API surface for future HTML tooling and command wrappers.
#[allow(unused_imports)]
pub(crate) use compile::compile_moth_template;
#[allow(unused_imports)]
pub(crate) use input::{MothTemplateCompileRequest, MothTemplateInput, MothTemplateSource};
#[allow(unused_imports)]
pub(crate) use output::{CompiledMothTemplateDocument, MothTemplateCompileOutput};
#[allow(unused_imports)]
pub(crate) use scope::{MothTemplatePathScope, MothTemplateScopeConstant};

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;
