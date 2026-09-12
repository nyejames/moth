//! HIR-owned reactive source and template metadata.
//!
//! WHAT: carries backend-facing Reactivity V1 facts after AST has resolved source identity and
//! template subscriptions.
//! WHY: reactive declarations, parameters, and template strings keep ordinary `TypeId` identity.
//! HIR therefore preserves their runtime metadata in side tables rather than introducing wrapper
//! types or backend-specific template nodes.

use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::ids::{HirValueId, LocalId};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringIdRemap;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ReactiveSourceId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ReactiveTemplateId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum HirReactiveSourceKind {
    Declaration,
    Parameter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HirReactiveSource {
    pub(crate) id: ReactiveSourceId,
    pub(crate) local_id: LocalId,
    pub(crate) path: PathId,
    pub(crate) kind: HirReactiveSourceKind,
    pub(crate) type_id: TypeId,
    pub(crate) span: Option<SourceSpan>,
}

impl HirReactiveSource {
    pub(crate) fn remap_string_ids(&mut self, _remap: &StringIdRemap) {}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HirReactiveTemplateDependency {
    pub(crate) source: ReactiveSourceId,
    pub(crate) type_id: TypeId,
    pub(crate) span: Option<SourceSpan>,
}

impl HirReactiveTemplateDependency {
    pub(crate) fn remap_string_ids(&mut self, _remap: &StringIdRemap) {}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HirReactiveTemplateParameterDependency {
    pub(crate) parameter: LocalId,
    pub(crate) span: Option<SourceSpan>,
}

impl HirReactiveTemplateParameterDependency {
    pub(crate) fn remap_string_ids(&mut self, _remap: &StringIdRemap) {}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HirReactiveTemplate {
    pub(crate) id: ReactiveTemplateId,
    pub(crate) value_id: HirValueId,
    pub(crate) dependencies: Vec<HirReactiveTemplateDependency>,
    pub(crate) template_value_parameters: Vec<HirReactiveTemplateParameterDependency>,
    pub(crate) template_backed: bool,
    pub(crate) span: Option<SourceSpan>,
}

impl HirReactiveTemplate {
    pub(crate) fn has_runtime_reactive_dependency(&self) -> bool {
        !self.dependencies.is_empty() || !self.template_value_parameters.is_empty()
    }

    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        for dependency in &mut self.dependencies {
            dependency.remap_string_ids(remap);
        }

        for dependency in &mut self.template_value_parameters {
            dependency.remap_string_ids(remap);
        }
    }
}
