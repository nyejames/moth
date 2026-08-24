//! Fixture construction for the module-local folded-value store.
//!
//! WHAT: builds a [`ConstValueStore`] directly from AST [`Declaration`]s, bypassing the
//! declaration table and the real TIR template projection.
//! WHY: HIR and public-interface unit fixtures need a populated store without running AST
//! finalization. Keeping these constructors here rather than in `store.rs` holds the production
//! file to the shapes the compiler actually builds, per the style guide's rule that production
//! files must not grow test-only constructors or mutators.

use super::{
    ConstTemplateValue, ConstValueKind, ConstValueRow, ConstValueStore, ConstValueStoreError,
};
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::folded_value::{PublicConstTemplate, PublicConstTemplateKind};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;

impl ConstValueStore {
    /// Build a store for HIR and public-interface unit fixtures.
    pub(crate) fn from_test_declarations(
        declarations: Vec<Declaration>,
        type_environment: &TypeEnvironment,
    ) -> Result<Self, CompilerError> {
        let mut store = Self::default();
        for declaration in declarations {
            store.try_insert_test_declaration(declaration, type_environment)?;
        }
        Ok(store)
    }

    /// Append one declaration to a test-owned store.
    pub(crate) fn insert_test_declaration(
        &mut self,
        declaration: Declaration,
        type_environment: &TypeEnvironment,
    ) {
        self.try_insert_test_declaration(declaration, type_environment)
            .expect("test declaration must be a supported folded store value");
    }

    pub(crate) fn try_insert_test_declaration(
        &mut self,
        declaration: Declaration,
        type_environment: &TypeEnvironment,
    ) -> Result<(), CompilerError> {
        let mut template_builder =
            |_: Option<&InternedPath>,
             _: &crate::compiler_frontend::ast::templates::template::Template|
             -> Result<ConstTemplateValue, ConstValueStoreError> {
                Ok(ConstTemplateValue::Public {
                    template: PublicConstTemplate {
                        kind: PublicConstTemplateKind::Wrapper,
                        pieces: Vec::new(),
                        conditional_child_wrappers: Vec::new(),
                    },
                    kind: ConstValueKind::TemplateWrapper,
                    hir_visible: true,
                    folded: None,
                    provenance: SyntheticInterfaceProvenance::empty(),
                })
            };
        let value = self
            .insert_expression(
                &declaration.value,
                Some(&declaration.id),
                type_environment,
                &mut template_builder,
            )
            .map_err(|error| match error {
                ConstValueStoreError::Infrastructure(error) => *error,
                ConstValueStoreError::Diagnostic(diagnostic) => CompilerError::compiler_error(
                    format!("test constant store insertion received a diagnostic: {diagnostic:?}"),
                ),
            })?;
        let path = declaration.id;
        if self.values_by_path.insert(path.clone(), value).is_some() {
            return Err(CompilerError::compiler_error(
                "two finalized module constants share the defining path",
            ));
        }
        self.rows.push(ConstValueRow { path, value });
        Ok(())
    }
}
