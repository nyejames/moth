//! Public module-root export surface validation.
//!
//! WHAT: rejects explicit public exports whose authored type surfaces require a type name that is
//! not part of the same module-root public export surface, and exported trait metadata relations
//! that expose private trait names.
//! WHY: consumers can only name declarations exposed by the module-root public export surface.
//! AST environment owns this check because it has canonical `TypeId`s, resolved trait identities,
//! and the header-built public export maps.

use super::builder::AstModuleEnvironmentBuilder;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;

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
                    for public_root_file in self.public_export_roots_for_header(header) {
                        self.validate_public_trait_incompatibility_surface(
                            incompatibility,
                            &public_root_file,
                            trait_environment,
                            string_table,
                        )?;
                    }
                }
                continue;
            }

            if !header.kind.is_authored_public_export_declaration() {
                continue;
            }
            let public_roots = self.public_export_roots_for_header(header);
            if public_roots.is_empty() {
                continue;
            }

            let exported_name = self
                .path_fork
                .component(header.declaration_path)
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        "Public export header had no source-path name.",
                    )
                })?;
            for public_root_file in public_roots {
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
                            &public_root_file,
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
                                &public_root_file,
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
                                    &public_root_file,
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
                            &public_root_file,
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
                            &public_root_file,
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

        if let Some(returns) = signature.return_contract.ordinary() {
            for type_id in returns.success() {
                self.validate_public_type_id(
                    exported_name,
                    *type_id,
                    public_root_file,
                    return_span,
                    trait_environment,
                )?;
            }
            if let Some(type_id) = returns.error() {
                self.validate_public_type_id(
                    exported_name,
                    type_id,
                    public_root_file,
                    return_span,
                    trait_environment,
                )?;
            }
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
                }) && definition.return_contract.ordinary().is_none_or(|returns| {
                    returns.success().iter().all(|return_type| {
                        self.public_type_id_is_nameable(
                            *return_type,
                            public_root_file,
                            _trait_environment,
                            visited_types,
                        )
                    }) && returns.error().is_none_or(|error_type| {
                        self.public_type_id_is_nameable(
                            error_type,
                            public_root_file,
                            _trait_environment,
                            visited_types,
                        )
                    })
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

        let export_root_file = if self
            .module_symbols
            .module_root_public_exports
            .contains_key(public_root_file)
        {
            Some(*public_root_file)
        } else {
            self.module_symbols
                .file_module_membership
                .get(public_root_file)
                .and_then(|module_root| {
                    self.module_symbols
                        .module_root_boundaries
                        .iter()
                        .find(|boundary| &boundary.module_root == module_root)
                        .map(|boundary| boundary.root_file)
                })
        };
        if let Some(export_root_file) = export_root_file
            && let Some(entries) = self
                .module_symbols
                .module_root_public_exports
                .get(&export_root_file)
            && entries
                .iter()
                .any(|entry| entry.target.is_source_path(path))
        {
            return true;
        }

        false
    }

    /// Every module-root or package-root file that exports this header's declaration.
    ///
    /// WHAT: a root-file declaration has one exporting root. A shared helper can be absent from
    ///       one paired surface and re-exported by the other, or by both; each exporting root owns
    ///       a separate public nameability scope.
    /// WHY: validating a helper once against the first boundary of its directory lets a support-only
    ///      export skip public-surface validation, and lets a declaration exported by both surfaces
    ///      be checked against only one surface's private names.
    pub(in crate::compiler_frontend::ast) fn public_export_roots_for_header(
        &self,
        header: &Header,
    ) -> Vec<PathId> {
        self.public_export_roots_for_path(&header.declaration_path, self.header_source_path(header))
    }

    /// Every root that exports one declaration from the given source file.
    pub(in crate::compiler_frontend::ast) fn public_export_roots_for_path(
        &self,
        declaration_path: &PathId,
        source_path: PathId,
    ) -> Vec<PathId> {
        let mut roots = Vec::new();

        if self
            .module_symbols
            .module_root_public_exports
            .get(&source_path)
            .is_some_and(|entries| {
                entries
                    .iter()
                    .any(|entry| entry.target.is_source_path(declaration_path))
            })
        {
            roots.push(source_path);
        }

        if let Some(module_root) = self.module_symbols.file_module_membership.get(&source_path) {
            for boundary in &self.module_symbols.module_root_boundaries {
                if &boundary.module_root != module_root
                    || self
                        .module_symbols
                        .module_root_public_exports
                        .get(&boundary.root_file)
                        .is_none_or(|entries| {
                            !entries
                                .iter()
                                .any(|entry| entry.target.is_source_path(declaration_path))
                        })
                {
                    continue;
                }
                roots.push(boundary.root_file);
            }
        }

        if let Some(prefix) = self
            .module_symbols
            .file_package_membership
            .get(&source_path)
            && let Some(entries) = self
                .module_symbols
                .source_package_public_exports
                .get(prefix)
            && entries
                .iter()
                .any(|entry| entry.target.is_source_path(declaration_path))
            && let Some(root_file) = self.module_symbols.source_package_root_files.get(prefix)
        {
            roots.push(*root_file);
        }

        roots.sort_unstable();
        roots.dedup();
        roots
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
