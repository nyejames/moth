//! Public module-root export surface validation.
//!
//! WHAT: rejects explicit public exports whose authored type surfaces require a type name that is
//! not part of the same module-root public export surface, and exported trait metadata relations
//! that expose private trait names.
//! WHY: consumers can only name declarations exposed by the module-root public export surface.
//! AST environment owns this check because it has canonical `TypeId`s, resolved trait identities,
//! and the header-built public export maps.

use super::builder::AstModuleEnvironmentBuilder;
use crate::compiler_frontend::compiler_errors::CompilerError;

use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::compiler_messages::InvalidTraitIncompatibilityReason;
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, TypeConstructor, TypeId};
use crate::compiler_frontend::headers::parse_file_headers::{Header, HeaderKind};
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::traits::definitions::TraitVisibility;
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::syntax::TraitIncompatibilitySyntax;

use rustc_hash::FxHashSet;
use std::sync::Arc;

impl<'context, 'services> AstModuleEnvironmentBuilder<'context, 'services> {
    /// Validate all explicit public authored declarations and trait metadata in a module root.
    ///
    /// WHAT: walks the resolved type IDs for signatures, fields, payloads, aliases, and explicit
    /// constant annotations, then validates exported trait incompatibility relations. Type walks
    /// recurse through option/collection/function/generic shapes.
    /// WHY: exported declarations and exported trait metadata are consumed from the public export
    /// surface alone, so every named type or trait they expose must also be public there.
    pub(in crate::compiler_frontend::ast) fn validate_public_export_surfaces(
        &mut self,
        sorted_headers: &[Header],
        trait_environment: &TraitEnvironment,
        string_table: &mut StringTable,
    ) -> Result<(), ExpressionParseError> {
        for header in sorted_headers {
            if is_public_export_trait_incompatibility_header(header) {
                if let HeaderKind::TraitIncompatibility { incompatibility } = &header.kind {
                    self.validate_public_trait_incompatibility_surface(
                        incompatibility,
                        &self.header_source_path(header),
                        trait_environment,
                        string_table,
                    )?;
                }
                continue;
            }

            if !self.header_is_public_export_declaration(header) {
                continue;
            }

            let exported_name = self
                .path_fork
                .component(header.declaration_path)
                .ok_or_else(|| {
                    CompilerError::compiler_error("Public export header had no source-path name.")
                })?;

            match &header.kind {
                HeaderKind::Function { .. } => {
                    let Some(resolved_signature) = self
                        .resolved_function_signatures_by_path
                        .get(&header.declaration_path)
                    else {
                        continue;
                    };
                    self.validate_public_function_surface(
                        exported_name,
                        &resolved_signature.signature,
                        &self.header_source_path(header),
                        header_name_span(header),
                        trait_environment,
                    )?;
                }

                HeaderKind::Struct { .. } => {
                    let Some(fields) = self
                        .resolved_struct_fields_by_path
                        .get(&header.declaration_path)
                    else {
                        continue;
                    };

                    for field in fields {
                        self.validate_public_type_id(
                            exported_name,
                            field.type_id,
                            &self.header_source_path(header),
                            field.binding_span,
                            trait_environment,
                        )?;
                    }
                }

                HeaderKind::Choice { .. } => {
                    let Some(variants) = self
                        .choice_variant_shells_by_path
                        .get(&header.declaration_path)
                    else {
                        continue;
                    };

                    for variant in variants {
                        let crate::compiler_frontend::declaration_syntax::choice::ChoiceVariantPayload::Record {
                            fields,
                        } = &variant.payload
                        else {
                            continue;
                        };

                        for field in fields {
                            self.validate_public_type_id(
                                exported_name,
                                field.type_id,
                                &self.header_source_path(header),
                                field.binding_span,
                                trait_environment,
                            )?;
                        }
                    }
                }

                HeaderKind::TypeAlias { .. } => {
                    let Some(alias) = self
                        .resolved_type_aliases_by_path
                        .get(&header.declaration_path)
                    else {
                        continue;
                    };

                    self.validate_public_type_id(
                        exported_name,
                        alias.target_type_id,
                        &self.header_source_path(header),
                        header_name_span(header),
                        trait_environment,
                    )?;
                }

                HeaderKind::Constant { declaration, .. } => {
                    let Some(resolved_declaration) =
                        self.declaration_table.get_by_path(&header.declaration_path)
                    else {
                        continue;
                    };

                    self.validate_public_type_id(
                        exported_name,
                        resolved_declaration.type_id,
                        &self.header_source_path(header),
                        declaration.span,
                        trait_environment,
                    )?;
                }

                HeaderKind::Trait { .. } => {}

                HeaderKind::ConstTemplate { .. }
                | HeaderKind::StartFunction
                | HeaderKind::TraitConformance { .. }
                | HeaderKind::TraitIncompatibility { .. } => {}
            }
        }

        Ok(())
    }

    fn validate_public_function_surface(
        &self,
        exported_name: StringId,
        signature: &FunctionSignature,
        public_root_file: &PathId,
        return_span: Option<SourceSpan>,
        trait_environment: &TraitEnvironment,
    ) -> Result<(), ExpressionParseError> {
        for parameter in &signature.parameters {
            for type_id in parameter.semantic_type_ids() {
                self.validate_public_type_id(
                    exported_name,
                    type_id,
                    public_root_file,
                    parameter.binding_span,
                    trait_environment,
                )?;
            }
        }

        for return_slot in &signature.returns {
            let Some(type_id) = return_slot.type_id else {
                continue;
            };

            self.validate_public_type_id(
                exported_name,
                type_id,
                public_root_file,
                return_span,
                trait_environment,
            )?;
        }

        Ok(())
    }

    fn validate_public_type_id(
        &self,
        exported_name: StringId,
        type_id: TypeId,
        public_root_file: &PathId,
        span: Option<SourceSpan>,
        trait_environment: &TraitEnvironment,
    ) -> Result<(), ExpressionParseError> {
        increment_ast_counter(AstCounter::PublicSurfaceValidationChecks);

        let mut visited_types = FxHashSet::default();
        if self.public_type_id_is_nameable(
            type_id,
            public_root_file,
            trait_environment,
            &mut visited_types,
        ) {
            return Ok(());
        }

        Err(generics_traits_api::private_type_in_exported_api(exported_name, type_id, span).into())
    }

    pub(in crate::compiler_frontend::ast) fn public_type_id_is_nameable(
        &self,
        type_id: TypeId,
        public_root_file: &PathId,
        _trait_environment: &TraitEnvironment,
        visited_types: &mut FxHashSet<TypeId>,
    ) -> bool {
        if !visited_types.insert(type_id) {
            return true;
        }

        match self.type_environment.get(type_id) {
            Some(TypeDefinition::Builtin(..))
            | Some(TypeDefinition::External(..))
            | Some(TypeDefinition::GenericParameter(..))
            // The compile-time-only anonymous const-record marker is public folded-value
            // vocabulary, not a source-private nominal.
            | Some(TypeDefinition::AnonymousConstRecordMarker) => true,

            Some(TypeDefinition::Struct(definition)) => {
                self.source_path_is_public_from_root_file(&definition.path, public_root_file)
            }

            Some(TypeDefinition::Choice(definition)) => {
                self.source_path_is_public_from_root_file(&definition.path, public_root_file)
            }

            Some(TypeDefinition::Constructed(definition)) => {
                self.type_constructor_is_public(&definition.constructor, public_root_file)
                    && definition.arguments.iter().all(|argument| {
                        self.public_type_id_is_nameable(
                            *argument,
                            public_root_file,
                            _trait_environment,
                            visited_types,
                        )
                    })
            }

            Some(TypeDefinition::Function(definition)) => {
                definition.parameters.iter().all(|parameter| {
                    self.public_type_id_is_nameable(
                        parameter.type_id,
                        public_root_file,
                        _trait_environment,
                        visited_types,
                    )
                }) && definition.returns.iter().all(|return_type| {
                    self.public_type_id_is_nameable(
                        *return_type,
                        public_root_file,
                        _trait_environment,
                        visited_types,
                    )
                }) && definition.error_return.is_none_or(|error_type| {
                    self.public_type_id_is_nameable(
                        error_type,
                        public_root_file,
                        _trait_environment,
                        visited_types,
                    )
                })
            }

            Some(TypeDefinition::GenericInstance(definition)) => {
                self.nominal_id_is_public(definition.base, public_root_file)
                    && definition.arguments.iter().all(|argument| {
                        self.public_type_id_is_nameable(
                            *argument,
                            public_root_file,
                            _trait_environment,
                            visited_types,
                        )
                    })
            }

            None => false,
        }
    }

    fn type_constructor_is_public(
        &self,
        constructor: &TypeConstructor,
        _public_root_file: &PathId,
    ) -> bool {
        match constructor {
            TypeConstructor::Builtin(_) => true,
        }
    }

    fn nominal_id_is_public(&self, nominal_id: NominalTypeId, public_root_file: &PathId) -> bool {
        self.type_environment
            .nominal_path_by_id(nominal_id)
            .is_some_and(|path| self.source_path_is_public_from_root_file(path, public_root_file))
    }

    /// Whether a trait may appear in a public signature or relation, judged by its owner.
    ///
    /// WHAT: core traits and source traits marked `exported` are public at their owner. An
    ///      unexported source trait counts only when `public_root_file` re-exports it. Callers
    ///      pass the root of the module that owns the declaration naming the trait.
    /// WHY: an unexported trait belongs to the module that names it, so that module's root is its
    ///      owner. Traits imported from a provider's public interface are marked `exported`, so a
    ///      public generic may be bounded by them without the consumer re-exporting them. A trait
    ///      private at its owner is still rejected.
    pub(in crate::compiler_frontend::ast) fn public_trait_definition_is_nameable(
        &self,
        trait_definition: &crate::compiler_frontend::traits::definitions::ResolvedTraitDefinition,
        public_root_file: &PathId,
        trait_environment: &TraitEnvironment,
    ) -> bool {
        match trait_definition.visibility {
            TraitVisibility::Core | TraitVisibility::Source { exported: true } => true,
            TraitVisibility::Source { exported: false } => trait_environment
                .paths_for(trait_definition.id)
                .iter()
                .any(|path| self.source_path_is_public_from_root_file(path, public_root_file)),
        }
    }

    pub(in crate::compiler_frontend::ast) fn source_path_is_public_from_root_file(
        &self,
        path: &PathId,
        public_root_file: &PathId,
    ) -> bool {
        if self
            .module_symbols
            .builtin_visible_symbol_paths
            .contains(path)
        {
            return true;
        }

        if let Some(package_prefix) = self
            .module_symbols
            .file_package_membership
            .get(public_root_file)
            && let Some(entries) = self
                .module_symbols
                .source_package_public_exports
                .get(package_prefix)
            && entries
                .iter()
                .any(|entry| entry.target.is_source_path(path))
        {
            return true;
        }

        if let Some(module_root) = self
            .module_symbols
            .file_module_membership
            .get(public_root_file)
            && let Some(entries) = self
                .module_symbols
                .module_root_public_exports
                .get(module_root)
            && entries
                .iter()
                .any(|entry| entry.target.is_source_path(path))
        {
            return true;
        }

        false
    }

    /// Whether a header is a public authored declaration reachable from its module root.
    ///
    /// WHAT: the declaration kind must pass the shared `HeaderKind` gate, and the root that owns
    ///       the declaring file must report the declaration as public, either directly or through
    ///       a re-export.
    /// WHY: a re-exported declaration is consumed through the root's public surface exactly like a
    ///      direct export, so its private field, parameter, and requirement types must be checked
    ///      too. Gating on the header's own file role and export mode skipped re-exports, which
    ///      then failed later as an internal compiler error.
    pub(in crate::compiler_frontend::ast) fn header_is_public_export_declaration(
        &self,
        header: &Header,
    ) -> bool {
        header.kind.is_authored_public_export_declaration()
            && self.source_path_is_public_from_root_file(
                &header.declaration_path,
                &self.header_source_path(header),
            )
    }

    /// Validate exported `TRAIT must not TRAIT` relations in a module-root public export.
    ///
    /// WHAT: an exported incompatibility relation is part of the module root's public trait metadata,
    ///      so consumers must be able to name both sides from that public export. The check rejects a
    ///      relation when exactly one side is public/nameable from that export and the other is not.
    ///      Core traits are always public/nameable; private-private relations remain valid.
    /// WHY: without this check, a public trait could reference a private trait name in exported
    ///      metadata, forcing consumers to know a name they cannot legally spell.
    fn validate_public_trait_incompatibility_surface(
        &self,
        incompatibility: &TraitIncompatibilitySyntax,
        public_root_file: &PathId,
        trait_environment: &TraitEnvironment,
        string_table: &mut StringTable,
    ) -> Result<(), ExpressionParseError> {
        let visibility = Arc::clone(self.binding_environment.visibility_for(public_root_file)?);

        let subject_id = self.resolve_visible_trait_name(
            incompatibility.subject.name,
            Some(incompatibility.subject.span),
            &visibility,
            trait_environment,
            string_table,
        )?;

        let subject_is_nameable = trait_environment.get(subject_id).is_some_and(|definition| {
            self.public_trait_definition_is_nameable(
                definition,
                public_root_file,
                trait_environment,
            )
        });

        for incompatible_trait in &incompatibility.incompatible_traits {
            let incompatible_id = self.resolve_visible_trait_name(
                incompatible_trait.name,
                Some(incompatible_trait.span),
                &visibility,
                trait_environment,
                string_table,
            )?;

            let incompatible_is_nameable =
                trait_environment
                    .get(incompatible_id)
                    .is_some_and(|definition| {
                        self.public_trait_definition_is_nameable(
                            definition,
                            public_root_file,
                            trait_environment,
                        )
                    });

            if subject_is_nameable != incompatible_is_nameable {
                let diagnostic = generics_traits_api::invalid_trait_incompatibility(
                    incompatibility.subject.name,
                    Some(incompatible_trait.name),
                    InvalidTraitIncompatibilityReason::PrivateTraitSurfaceLeak,
                    Some(incompatible_trait.span),
                );
                return Err(diagnostic.into());
            }
        }

        Ok(())
    }
}

fn is_public_export_trait_incompatibility_header(header: &Header) -> bool {
    header.file_role.is_export_capable()
        && header.export_mode.is_public()
        && matches!(header.kind, HeaderKind::TraitIncompatibility { .. })
}

fn header_name_span(header: &Header) -> Option<SourceSpan> {
    header.name_span
}
