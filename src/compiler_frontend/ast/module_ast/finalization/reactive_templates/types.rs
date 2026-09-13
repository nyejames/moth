//! Shared types for the reactive template metadata propagation pass.
//!
//! WHAT: holds the per-function flow record and the value environment that tracks
//! reactive template metadata for declarations and assignments.
//! WHY: these structures are used by the flow analysis, metadata collection, and
//! annotation phases, so they live in one small submodule to avoid cross-file
//! duplication.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ReactiveTemplateMetadata,
};

use crate::compiler_frontend::symbols::path_interner::PathId;
use rustc_hash::FxHashMap;

#[derive(Clone, Debug)]
pub(super) struct FunctionTemplateFlow {
    pub(super) parameters: Vec<Declaration>,
    pub(super) success_returns: Vec<Option<ReactiveTemplateMetadata>>,
}

impl PartialEq for FunctionTemplateFlow {
    fn eq(&self, other: &Self) -> bool {
        self.success_returns == other.success_returns
    }
}

impl Eq for FunctionTemplateFlow {}

#[derive(Clone, Debug, Default)]
pub(super) struct ReactiveTemplateValueEnvironment {
    values: FxHashMap<PathId, Option<ReactiveTemplateMetadata>>,
}

impl ReactiveTemplateValueEnvironment {
    pub(super) fn for_parameters(parameters: &[Declaration]) -> Self {
        let mut environment = Self::default();

        for parameter in parameters {
            environment.record_declaration(parameter);
        }

        environment
    }

    pub(super) fn record_declaration(&mut self, declaration: &Declaration) {
        self.values.insert(
            declaration.id.clone(),
            declaration.value.reactive_template.clone(),
        );
    }

    pub(super) fn record_assignment(&mut self, path: &PathId, value: &Expression) {
        self.values
            .insert(path.clone(), value.reactive_template.clone());
    }

    pub(super) fn record_binding_metadata(
        &mut self,
        path: &PathId,
        metadata: Option<ReactiveTemplateMetadata>,
    ) {
        self.values.insert(path.clone(), metadata);
    }

    pub(super) fn metadata_for_path(
        &self,
        path: &PathId,
    ) -> Option<ReactiveTemplateMetadata> {
        self.values.get(path).cloned().flatten()
    }
}

pub(super) fn merge_optional_metadata(
    target: &mut Option<ReactiveTemplateMetadata>,
    source: Option<ReactiveTemplateMetadata>,
) {
    let Some(source) = source else {
        return;
    };

    match target {
        Some(existing) => existing.merge_from(&source),
        None => *target = Some(source),
    }
}

pub(super) fn reference_path_for_place_expression(
    place: &crate::compiler_frontend::ast::expressions::expression_rpn::PlaceExpression,
) -> Option<&PathId> {
    use crate::compiler_frontend::ast::expressions::expression_rpn::PlaceExpressionKind;

    match &place.kind {
        PlaceExpressionKind::Local(path) => Some(path),
        PlaceExpressionKind::Field { base, .. } => reference_path_for_place_expression(base),
    }
}
