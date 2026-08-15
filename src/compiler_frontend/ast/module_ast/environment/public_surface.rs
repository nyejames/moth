//! Public module-root export surface validation.
//!
//! WHAT: rejects explicit public exports whose authored type surfaces require a type name that is
//! not part of the same module-root public export surface, and exported trait metadata relations
//! that expose private trait names.
//! WHY: consumers can only name declarations exposed by the module-root public export surface.
//! AST environment owns this check because it has canonical `TypeId`s, resolved trait identities,
//! and the header-built public export maps.

use super::builder::AstModuleEnvironmentBuilder;

use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::type_resolution::resolve_diagnostic_type_to_type_id_checked;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTraitIncompatibilityReason,
};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, TypeConstructor, TypeId};
use crate::compiler_frontend::headers::parse_file_headers::{Header, HeaderKind};
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::traits::definitions::TraitVisibility;
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::syntax::TraitIncompatibilitySyntax;

use rustc_hash::FxHashSet;

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
    ) -> Result<(), CompilerMessages> {
        for header in sorted_headers {
            if is_public_export_trait_incompatibility_header(header) {
                if let HeaderKind::TraitIncompatibility { incompatibility } = &header.kind {
                    self.validate_public_trait_incompatibility_surface(
                        incompatibility,
                        &header.source_file,
                        trait_environment,
                        string_table,
                    )?;
                }
                continue;
            }

            if !header_is_public_export_declaration(header) {
                continue;
            }

            let exported_name = header.tokens.src_path.name().ok_or_else(|| {
                self.error_messages(
                    CompilerError::compiler_error("Public export header had no source-path name."),
                    string_table,
                )
            })?;

            match &header.kind {
                HeaderKind::Function { .. } => {
                    let Some(resolved_signature) = self
                        .resolved_function_signatures_by_path
                        .get(&header.tokens.src_path)
                    else {
                        continue;
                    };
                    self.validate_public_function_surface(
                        exported_name,
                        &resolved_signature.signature,
                        &header.source_file,
                        header.name_location.clone(),
                        trait_environment,
                        string_table,
                    )?;
                }

                HeaderKind::Struct { .. } => {
                    let Some(fields) = self
                        .resolved_struct_fields_by_path
                        .get(&header.tokens.src_path)
                    else {
                        continue;
                    };

                    for field in fields {
                        self.validate_public_type_id(
                            exported_name,
                            field.value.type_id,
                            &header.source_file,
                            field.value.location.clone(),
                            trait_environment,
                            string_table,
                        )?;
                    }
                }

                HeaderKind::Choice { .. } => {
                    let Some(variants) = self
                        .choice_variant_shells_by_path
                        .get(&header.tokens.src_path)
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
                                field.value.type_id,
                                &header.source_file,
                                field.value.location.clone(),
                                trait_environment,
                                string_table,
                            )?;
                        }
                    }
                }

                HeaderKind::TypeAlias { .. } => {
                    let alias_path = header.tokens.src_path.clone();
                    let Some(annotation) = self.resolved_type_aliases_by_path.get(&alias_path)
                    else {
                        continue;
                    };

                    let type_id = match annotation.type_id {
                        Some(type_id) => type_id,
                        None => {
                            let resolved_type_id = resolve_diagnostic_type_to_type_id_checked(
                                &annotation.diagnostic_type,
                                &mut self.type_environment,
                                &header.name_location,
                            )
                            .map_err(|diagnostic| {
                                self.diagnostic_messages(*diagnostic, string_table)
                            })?;

                            // Retain the materialized target TypeId so the resolved public
                            // type-root table consumes the retained fact instead of resolving
                            // the alias target a second time. The entry was proven present
                            // above; an absent entry here is an internal invariant failure.
                            let Some(alias_annotation) =
                                self.resolved_type_aliases_by_path.get_mut(&alias_path)
                            else {
                                return Err(self.error_messages(
                                    CompilerError::compiler_error(
                                        "Public transparent alias annotation was absent during target TypeId writeback.",
                                    ),
                                    string_table,
                                ));
                            };
                            alias_annotation.type_id = Some(resolved_type_id);
                            resolved_type_id
                        }
                    };

                    self.validate_public_type_id(
                        exported_name,
                        type_id,
                        &header.source_file,
                        header.name_location.clone(),
                        trait_environment,
                        string_table,
                    )?;
                }

                HeaderKind::Constant { declaration, .. } => {
                    let Some(resolved_declaration) =
                        self.declaration_table.get_by_path(&header.tokens.src_path)
                    else {
                        continue;
                    };

                    self.validate_public_type_id(
                        exported_name,
                        resolved_declaration.value.type_id,
                        &header.source_file,
                        declaration.location.clone(),
                        trait_environment,
                        string_table,
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
        public_root_file: &InternedPath,
        return_location: SourceLocation,
        trait_environment: &TraitEnvironment,
        string_table: &StringTable,
    ) -> Result<(), CompilerMessages> {
        for parameter in &signature.parameters {
            self.validate_public_type_id(
                exported_name,
                parameter.value.type_id,
                public_root_file,
                parameter.value.location.clone(),
                trait_environment,
                string_table,
            )?;
        }

        for return_slot in &signature.returns {
            let Some(type_id) = return_slot.type_id else {
                continue;
            };

            self.validate_public_type_id(
                exported_name,
                type_id,
                public_root_file,
                return_location.clone(),
                trait_environment,
                string_table,
            )?;
        }

        Ok(())
    }

    fn validate_public_type_id(
        &self,
        exported_name: StringId,
        type_id: TypeId,
        public_root_file: &InternedPath,
        location: SourceLocation,
        trait_environment: &TraitEnvironment,
        string_table: &StringTable,
    ) -> Result<(), CompilerMessages> {
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

        Err(self.diagnostic_messages(
            CompilerDiagnostic::private_type_in_exported_api(exported_name, type_id, location),
            string_table,
        ))
    }

    pub(in crate::compiler_frontend::ast) fn public_type_id_is_nameable(
        &self,
        type_id: TypeId,
        public_root_file: &InternedPath,
        _trait_environment: &TraitEnvironment,
        visited_types: &mut FxHashSet<TypeId>,
    ) -> bool {
        if !visited_types.insert(type_id) {
            return true;
        }

        match self.type_environment.get(type_id) {
            Some(TypeDefinition::Builtin(..))
            | Some(TypeDefinition::External(..))
            | Some(TypeDefinition::GenericParameter(..)) => true,

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
        _public_root_file: &InternedPath,
    ) -> bool {
        match constructor {
            TypeConstructor::Builtin(_) => true,
        }
    }

    fn nominal_id_is_public(
        &self,
        nominal_id: NominalTypeId,
        public_root_file: &InternedPath,
    ) -> bool {
        self.type_environment
            .nominal_path_by_id(nominal_id)
            .is_some_and(|path| self.source_path_is_public_from_root_file(path, public_root_file))
    }

    pub(in crate::compiler_frontend::ast) fn public_trait_definition_is_nameable(
        &self,
        trait_definition: &crate::compiler_frontend::traits::definitions::ResolvedTraitDefinition,
        public_root_file: &InternedPath,
        trait_environment: &TraitEnvironment,
    ) -> bool {
        match trait_definition.visibility {
            TraitVisibility::Core => true,
            TraitVisibility::Source { .. } => trait_environment
                .paths_for(trait_definition.id)
                .iter()
                .any(|path| self.source_path_is_public_from_root_file(path, public_root_file)),
        }
    }

    pub(in crate::compiler_frontend::ast) fn source_path_is_public_from_root_file(
        &self,
        path: &InternedPath,
        public_root_file: &InternedPath,
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
        public_root_file: &InternedPath,
        trait_environment: &TraitEnvironment,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        let visibility = self
            .binding_environment
            .visibility_for(public_root_file)
            .map_err(|error| self.error_messages(error, string_table))?
            .clone();

        let subject_id = self.resolve_visible_trait_reference(
            &incompatibility.subject,
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
            let incompatible_id = self.resolve_visible_trait_reference(
                incompatible_trait,
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
                return Err(self.diagnostic_messages(
                    CompilerDiagnostic::invalid_trait_incompatibility(
                        incompatibility.subject.name,
                        Some(incompatible_trait.name),
                        InvalidTraitIncompatibilityReason::PrivateTraitSurfaceLeak,
                        incompatible_trait.location.clone(),
                    ),
                    string_table,
                ));
            }
        }

        Ok(())
    }
}

/// Whether a header is a public authored module-root export declaration.
///
/// WHAT: only declarations marked public by a strict module-root file `export:` block and
///       admitted by the shared declaration-kind gate become public export entries. The
///       declaration-kind gate is the shared [`HeaderKind::is_authored_public_export_declaration`]
///       owner so the header, AST and semantic-origin public-export predicates cannot drift;
///       this predicate keeps the stage-local file-role and export-mode policy (export-capable
///       roots plus explicit `Public` mode).
fn header_is_public_export_declaration(header: &Header) -> bool {
    header.file_role.is_export_capable()
        && header.export_mode.is_public()
        && header.kind.is_authored_public_export_declaration()
}

fn is_public_export_trait_incompatibility_header(header: &Header) -> bool {
    header.file_role.is_export_capable()
        && header.export_mode.is_public()
        && matches!(header.kind, HeaderKind::TraitIncompatibility { .. })
}
