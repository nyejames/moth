//! Reachability-driven nominal shell and member projection.

use super::*;

fn collect_semantic_type_origins(
    semantics: &PublicDeclarationSemantics,
    origins: &mut FxHashSet<OriginTypeId>,
) {
    match semantics {
        PublicDeclarationSemantics::Function(function) => {
            for parameter in &function.parameters {
                collect_canonical_type_origins(&parameter.type_identity, origins);
                if let Some(default) = &parameter.folded_default {
                    collect_folded_type_origins(default, origins);
                }
            }
            for returned in &function.returns {
                collect_canonical_type_origins(&returned.type_identity, origins);
            }
            if let Some(error) = &function.error_return {
                collect_canonical_type_origins(error, origins);
            }
        }
        PublicDeclarationSemantics::Struct(structure) => {
            for field in &structure.fields {
                collect_canonical_type_origins(&field.type_identity, origins);
                if let Some(default) = &field.folded_default {
                    collect_folded_type_origins(default, origins);
                }
            }
            for method in &structure.receiver_methods {
                collect_receiver_method_type_origins(method, origins);
            }
        }
        PublicDeclarationSemantics::Choice(choice) => {
            for variant in &choice.variants {
                for field in &variant.payload_fields {
                    collect_canonical_type_origins(&field.type_identity, origins);
                }
            }
            for method in &choice.receiver_methods {
                collect_receiver_method_type_origins(method, origins);
            }
        }
        PublicDeclarationSemantics::TransparentAlias(alias) => {
            collect_canonical_type_origins(&alias.target_type_identity, origins);
        }
        PublicDeclarationSemantics::Constant(constant) => {
            collect_canonical_type_origins(&constant.type_identity, origins);
            collect_folded_type_origins(&constant.folded_value, origins);
        }
        PublicDeclarationSemantics::Trait(_) => {}
    }
}

fn collect_receiver_method_type_origins(
    method: &crate::compiler_frontend::public_interface::PublicReceiverMethodSemantics,
    origins: &mut FxHashSet<OriginTypeId>,
) {
    for parameter in &method.parameters {
        collect_canonical_type_origins(&parameter.type_identity, origins);
        if let Some(default) = &parameter.folded_default {
            collect_folded_type_origins(default, origins);
        }
    }
    for returned in &method.returns {
        collect_canonical_type_origins(&returned.type_identity, origins);
    }
    if let Some(error) = &method.error_return {
        collect_canonical_type_origins(error, origins);
    }
}

fn collect_canonical_type_origins(
    identity: &CanonicalTypeIdentity,
    origins: &mut FxHashSet<OriginTypeId>,
) {
    match identity {
        CanonicalTypeIdentity::Builtin(_)
        | CanonicalTypeIdentity::ModulePrivateNominal(_)
        | CanonicalTypeIdentity::ModulePrivateGenericInstance(_)
        | CanonicalTypeIdentity::ExternalOpaque(_)
        | CanonicalTypeIdentity::GenericParameter(_) => {}
        CanonicalTypeIdentity::SourceNominal(origin) => {
            origins.insert(origin.clone());
        }
        CanonicalTypeIdentity::Collection(collection) => {
            collect_canonical_type_origins(collection.element(), origins);
        }
        CanonicalTypeIdentity::OrderedMap(map) => {
            collect_canonical_type_origins(map.key(), origins);
            collect_canonical_type_origins(map.value(), origins);
        }
        CanonicalTypeIdentity::Option(inner) => {
            collect_canonical_type_origins(inner, origins);
        }
        CanonicalTypeIdentity::FallibleCarrier(carrier) => {
            collect_canonical_type_origins(carrier.success(), origins);
            collect_canonical_type_origins(carrier.error(), origins);
        }
        CanonicalTypeIdentity::GenericInstance(instance) => {
            origins.insert(instance.base().clone());
            for argument in instance.arguments() {
                collect_canonical_type_origins(argument, origins);
            }
        }
    }
}

fn collect_folded_type_origins(value: &PublicFoldedValue, origins: &mut FxHashSet<OriginTypeId>) {
    match value {
        PublicFoldedValue::Collection(values) => {
            for value in values {
                collect_folded_type_origins(value, origins);
            }
        }
        PublicFoldedValue::Record(fields) => {
            for field in fields {
                collect_folded_type_origins(&field.value, origins);
            }
        }
        PublicFoldedValue::Choice {
            type_identity,
            fields,
            ..
        } => {
            collect_canonical_type_origins(type_identity, origins);
            for field in fields {
                collect_folded_type_origins(&field.value, origins);
            }
        }
        PublicFoldedValue::Range { start, end } => {
            collect_folded_type_origins(start, origins);
            collect_folded_type_origins(end, origins);
        }
        PublicFoldedValue::OptionSome(value) => collect_folded_type_origins(value, origins),
        PublicFoldedValue::Int(_)
        | PublicFoldedValue::Float(_)
        | PublicFoldedValue::Bool(_)
        | PublicFoldedValue::Char(_)
        | PublicFoldedValue::String(_)
        | PublicFoldedValue::ConstTemplate(_)
        | PublicFoldedValue::OptionNone => {}
    }
}

impl<'context, 'services> AstModuleEnvironmentBuilder<'context, 'services> {
    pub(in crate::compiler_frontend::ast::module_ast::environment::builder) fn project_imported_nominal_declarations(
        &mut self,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerError> {
        let reachable_origins = self.reachable_imported_type_origins();
        let mut imported_nominals = self
            .binding_environment
            .imported_declarations_by_origin
            .values()
            .filter_map(|record| match &record.origin {
                OriginDeclarationId::Type(origin) if reachable_origins.contains(origin) => {
                    match &record.semantics {
                        PublicDeclarationSemantics::Struct(_)
                        | PublicDeclarationSemantics::Choice(_) => {
                            Some((origin.clone(), record.clone()))
                        }
                        _ => None,
                    }
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        imported_nominals.sort_by(|left, right| left.0.cmp(&right.0));

        for (origin, record) in &imported_nominals {
            let nominal_path = imported_nominal_path(origin, string_table);
            let generic_parameters = match &record.semantics {
                PublicDeclarationSemantics::Struct(semantics) => &semantics.generic_parameters,
                PublicDeclarationSemantics::Choice(semantics) => &semantics.generic_parameters,
                _ => {
                    return Err(CompilerError::compiler_error(
                        "Imported nominal projection received a non-nominal declaration",
                    ));
                }
            };
            let generic_parameter_list_id =
                self.register_imported_generic_parameters(generic_parameters, string_table)?;

            let type_id = match &record.semantics {
                PublicDeclarationSemantics::Struct(_) => {
                    self.type_environment
                        .register_nominal_struct(StructTypeDefinition {
                            id: NominalTypeId(0),
                            path: nominal_path,
                            fields: Box::new([]),
                            generic_parameters: generic_parameter_list_id,
                            const_record: false,
                        })
                        .1
                }
                PublicDeclarationSemantics::Choice(_) => {
                    self.type_environment
                        .register_nominal_choice(ChoiceTypeDefinition {
                            id: NominalTypeId(0),
                            path: nominal_path,
                            variants: Box::new([]),
                            generic_parameters: generic_parameter_list_id,
                        })
                        .1
                }
                _ => {
                    return Err(CompilerError::compiler_error(
                        "Imported nominal projection received a non-nominal declaration",
                    ));
                }
            };

            self.imported_type_ids_by_origin
                .insert(origin.clone(), type_id);
            self.type_environment.register_canonical_identity(
                CanonicalTypeIdentity::SourceNominal(origin.clone()),
                type_id,
            )?;
        }

        for (origin, record) in &imported_nominals {
            let type_id = self.imported_type_ids_by_origin[origin];
            match &record.semantics {
                PublicDeclarationSemantics::Struct(semantics) => {
                    self.project_imported_struct_members(type_id, semantics, string_table)?;
                }
                PublicDeclarationSemantics::Choice(semantics) => {
                    self.project_imported_choice_members(type_id, semantics, string_table)?;
                }
                _ => {
                    return Err(CompilerError::compiler_error(
                        "Imported nominal member projection received a non-nominal declaration",
                    ));
                }
            }
        }

        for (local_path, origin) in self
            .binding_environment
            .imported_declarations_by_local_path
            .clone()
        {
            let OriginDeclarationId::Type(origin_type) = &origin else {
                continue;
            };
            let Some(record) = self
                .binding_environment
                .imported_declarations_by_origin
                .get(&origin)
            else {
                continue;
            };
            let Some(type_id) = self.imported_type_ids_by_origin.get(origin_type).copied() else {
                continue;
            };
            let diagnostic_type = diagnostic_type_spelling(type_id, &self.type_environment);

            Rc::make_mut(&mut self.nominal_type_ids_by_path).insert(local_path.clone(), type_id);
            self.type_environment
                .register_nominal_path_alias(local_path.clone(), type_id)?;

            let (generic_parameters, kind) = match &record.semantics {
                PublicDeclarationSemantics::Struct(semantics) => (
                    &semantics.generic_parameters,
                    crate::compiler_frontend::headers::module_symbols::GenericDeclarationKind::Struct,
                ),
                PublicDeclarationSemantics::Choice(semantics) => (
                    &semantics.generic_parameters,
                    crate::compiler_frontend::headers::module_symbols::GenericDeclarationKind::Choice,
                ),
                _ => continue,
            };
            if !generic_parameters.is_empty() {
                let parameters = GenericParameterList {
                    parameters: generic_parameters
                        .iter()
                        .enumerate()
                        .map(|(index, parameter)| GenericParameter {
                            id: TypeParameterId(index as u32),
                            name: string_table.intern(parameter.identity.authored_name()),
                            location: Default::default(),
                            trait_bounds: Vec::new(),
                        })
                        .collect(),
                };
                let metadata =
                    crate::compiler_frontend::headers::module_symbols::GenericDeclarationMetadata {
                        kind,
                        parameters,
                        declaration_location: Default::default(),
                    };
                Rc::make_mut(&mut self.generic_declarations_by_path)
                    .insert(local_path.clone(), metadata.clone());

                let internal_path = self
                    .type_environment
                    .nominal_path(type_id)
                    .cloned()
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Imported generic nominal has no canonical local path",
                        )
                    })?;
                Rc::make_mut(&mut self.generic_declarations_by_path)
                    .insert(internal_path.clone(), metadata);

                if let PublicDeclarationSemantics::Struct(_) = &record.semantics {
                    let fields = self
                        .resolved_struct_fields_by_path
                        .get(&internal_path)
                        .cloned()
                        .ok_or_else(|| {
                            CompilerError::compiler_error(
                                "Imported generic struct has no projected field template",
                            )
                        })?;
                    Rc::make_mut(&mut self.resolved_struct_fields_by_path)
                        .insert(local_path.clone(), fields);
                }
            }
            Rc::make_mut(&mut self.declaration_table)
                .append_for_construction(Declaration {
                    id: local_path,
                    value: Expression::new(
                        ExpressionKind::NoValue,
                        Default::default(),
                        type_id,
                        diagnostic_type,
                        ValueMode::ImmutableReference,
                    ),
                })
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Imported nominal declaration path was registered more than once",
                    )
                })?;
        }

        Ok(())
    }

    /// Computes the nominal declaration closure required by directly imported declarations.
    fn reachable_imported_type_origins(&self) -> FxHashSet<OriginTypeId> {
        let mut reachable = FxHashSet::default();
        for evidence in self
            .binding_environment
            .imported_evidence_by_identity
            .values()
        {
            // Reusable evidence carries a canonical target that may be a generic nominal
            // instance even when the generated body never names that type directly. Its target
            // declaration is therefore part of the imported nominal closure before evidence is
            // projected into consumer-local TypeIds.
            collect_canonical_type_origins(
                evidence.identity.target_type_identity(),
                &mut reachable,
            );
        }
        for origin in self
            .binding_environment
            .imported_declarations_by_local_path
            .values()
        {
            if let OriginDeclarationId::Type(type_origin) = origin {
                reachable.insert(type_origin.clone());
            }
            if let Some(record) = self
                .binding_environment
                .imported_declarations_by_origin
                .get(origin)
            {
                collect_semantic_type_origins(&record.semantics, &mut reachable);
            }
        }

        let mut pending = reachable.iter().cloned().collect::<Vec<_>>();
        let mut next = 0;
        while let Some(origin) = pending.get(next).cloned() {
            next += 1;
            let Some(record) = self
                .binding_environment
                .imported_declarations_by_origin
                .get(&OriginDeclarationId::Type(origin))
            else {
                continue;
            };
            let mut discovered_origins = FxHashSet::default();
            collect_semantic_type_origins(&record.semantics, &mut discovered_origins);
            for discovered in discovered_origins {
                if reachable.insert(discovered.clone()) {
                    pending.push(discovered);
                }
            }
        }
        reachable
    }

    pub(in crate::compiler_frontend::ast::module_ast::environment::builder) fn register_imported_generic_parameters(
        &mut self,
        parameters: &[crate::compiler_frontend::public_interface::PublicGenericParameterSurface],
        string_table: &mut StringTable,
    ) -> Result<
        Option<crate::compiler_frontend::datatypes::ids::GenericParameterListId>,
        CompilerError,
    > {
        if parameters.is_empty() {
            return Ok(None);
        }

        if let Some(existing) = self
            .imported_generic_parameter_registrations
            .iter()
            .find(|registration| registration.surfaces.as_slice() == parameters)
        {
            return Ok(Some(existing.list_id));
        }

        let parsed = GenericParameterList {
            parameters: parameters
                .iter()
                .enumerate()
                .map(|(index, parameter)| GenericParameter {
                    id: TypeParameterId(index as u32),
                    name: string_table.intern(parameter.identity.authored_name()),
                    location: Default::default(),
                    trait_bounds: Vec::new(),
                })
                .collect(),
        };
        let registered = self
            .type_environment
            .register_generic_parameter_list(&parsed, &FxHashMap::default());
        for (index, parameter) in parameters.iter().enumerate() {
            let local_id = registered
                .canonical_by_local
                .get(&TypeParameterId(index as u32))
                .copied()
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Imported generic parameter registration omitted a local parameter",
                    )
                })?;
            let type_id = self
                .type_environment
                .type_id_for_generic_parameter(local_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Imported generic parameter registration omitted its canonical type",
                    )
                })?;
            self.imported_generic_parameter_type_ids
                .insert(parameter.identity.clone(), type_id);
        }

        self.imported_generic_parameter_registrations
            .push(ImportedGenericParameterRegistration {
                surfaces: parameters.to_vec(),
                list_id: registered.list_id,
                canonical_by_local: registered.canonical_by_local.clone(),
            });

        Ok(Some(registered.list_id))
    }

    fn project_imported_struct_members(
        &mut self,
        type_id: TypeId,
        semantics: &PublicStructSemantics,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerError> {
        let nominal_path = self
            .type_environment
            .struct_definition_for(type_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Imported struct shell was unavailable during member projection",
                )
            })?
            .path
            .clone();
        let mut fields = Vec::with_capacity(semantics.fields.len());
        let mut field_declarations = Vec::with_capacity(semantics.fields.len());

        for field in &semantics.fields {
            let field_type_id = self.intern_imported_canonical_type(&field.type_identity)?;
            let field_path = nominal_path.join_str(&field.name, string_table);
            fields.push(FieldDefinition {
                name: field_path.clone(),
                type_id: field_type_id,
                location: Default::default(),
            });
            let default_value = match &field.folded_default {
                Some(value) => {
                    self.project_imported_folded_value(value, field_type_id, string_table)?
                }
                None => Expression::new(
                    ExpressionKind::NoValue,
                    Default::default(),
                    field_type_id,
                    diagnostic_type_spelling(field_type_id, &self.type_environment),
                    ValueMode::ImmutableReference,
                ),
            };
            field_declarations.push(Declaration {
                id: field_path,
                value: default_value,
            });
        }

        self.type_environment
            .update_struct_fields(type_id, fields.into_boxed_slice());
        Rc::make_mut(&mut self.resolved_struct_fields_by_path)
            .insert(nominal_path.clone(), field_declarations.clone());
        if semantics.generic_parameters.is_empty() {
            self.imported_struct_definitions
                .push(AstImportedStructDefinition { nominal_path });
        }
        Ok(())
    }

    fn project_imported_choice_members(
        &mut self,
        type_id: TypeId,
        semantics: &PublicChoiceSemantics,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerError> {
        let nominal_path = self
            .type_environment
            .choice_definition_for(type_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Imported choice shell was unavailable during member projection",
                )
            })?
            .path
            .clone();
        let mut variants = Vec::with_capacity(semantics.variants.len());

        for (tag, variant) in semantics.variants.iter().enumerate() {
            let payload = if variant.payload_fields.is_empty() {
                ChoiceVariantPayloadDefinition::Unit
            } else {
                let mut fields = Vec::with_capacity(variant.payload_fields.len());
                for field in &variant.payload_fields {
                    let field_type_id =
                        self.intern_imported_canonical_type(&field.type_identity)?;
                    fields.push(FieldDefinition {
                        name: nominal_path.join_str(&field.name, string_table),
                        type_id: field_type_id,
                        location: Default::default(),
                    });
                }
                ChoiceVariantPayloadDefinition::Record {
                    fields: fields.into_boxed_slice(),
                }
            };
            variants.push(ChoiceVariantDefinition {
                name: string_table.intern(&variant.name),
                tag,
                payload,
                location: Default::default(),
            });
        }

        self.type_environment
            .update_choice_variants(type_id, variants.into_boxed_slice());
        if semantics.generic_parameters.is_empty() {
            self.imported_choice_definitions
                .push(AstChoiceDefinition { nominal_path });
        }
        Ok(())
    }
}

/// Builds the consumer-local lookup path for one stable imported nominal identity.
///
/// The path is not semantic identity. It is a collision-free local index whose components retain
/// every stable origin discriminator; `TypeEnvironment` separately owns the exact canonical map.
pub(crate) fn imported_nominal_path(
    origin: &OriginTypeId,
    string_table: &mut StringTable,
) -> InternedPath {
    // Angle brackets cannot occur in a source identifier, so this namespace is structurally
    // disjoint from every authored nominal path rather than relying on a reserved spelling.
    let mut path = InternedPath::from_single_str("<imported>", string_table);
    let package_origin = match origin.module_origin().package().origin() {
        crate::builder_surface::PackageOrigin::Core => "core",
        crate::builder_surface::PackageOrigin::Standard => "standard",
        crate::builder_surface::PackageOrigin::Builder => "builder",
        crate::builder_surface::PackageOrigin::ProjectLocal => "project",
        crate::builder_surface::PackageOrigin::Dependency => "dependency",
    };
    let root_role = match origin.module_origin().role() {
        crate::compiler_frontend::semantic_identity::ModuleRootRole::Normal => "normal",
        crate::compiler_frontend::semantic_identity::ModuleRootRole::Support => "support",
        crate::compiler_frontend::semantic_identity::ModuleRootRole::ProjectPackageFacade => {
            "facade"
        }
    };
    path.push_str(package_origin, string_table);
    path.push_str(origin.module_origin().package().name(), string_table);
    path.push_str(root_role, string_table);
    for component in origin.module_origin().logical_module_path().split('/') {
        if !component.is_empty() {
            path.push_str(component, string_table);
        }
    }
    path.push_str(origin.defining_name(), string_table);
    path
}
