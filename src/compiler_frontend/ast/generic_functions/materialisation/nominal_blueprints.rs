//! Stable nominal blueprints and generated-local nominal reconstruction.

use super::preparation_freeze::{ModuleMaterialisationCapture, ModuleMaterialisationPreparation};
use super::semantic_closure::collect_reachable_nominal_identities;
use super::stable_types::{
    GeneratedFoldedValueMaterialiser, GeneratedValueMaterialisationServices,
    MaterialisationNominalOriginResolver, MaterialisationNominalSource,
    const_store_error_into_compiler_error, import_canonical_folded_value,
};
use crate::compiler_frontend::ast::ast_nodes::{Declaration, DeclarationShell, DeclarationValue};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalTraitIdentity, CanonicalTypeIdentity, CanonicalTypeProjectionContext,
    ExportedGenericParameterIdentity, ExternalOpaqueTypeIdentity, GenericDeclarationOrigin,
    canonical_builtin_for_key, intern_canonical_builtin, project_type_id_to_canonical_identity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceTypeDefinition, ChoiceVariantDefinition, ChoiceVariantPayloadDefinition, FieldDefinition,
    StructTypeDefinition, TypeDefinition,
};
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_parameters::TypeParameterId;
use crate::compiler_frontend::datatypes::ids::{
    BuiltinTypeConstructor, GenericParameterId, NominalTypeId, TypeConstructor, TypeId,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::{
    FoldedValueGenericParameterResolver, PublicFoldedValue,
};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::public_interface::PublicDeclarationSemantics;
use crate::compiler_frontend::semantic_identity::OriginTypeCategory;
use crate::compiler_frontend::semantic_identity::{ModuleRootRole, OriginDeclarationId};
#[cfg(test)]
use crate::compiler_frontend::semantic_identity::{OriginTypeId, StableModuleOriginIdentity};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::{FxHashMap, FxHashSet};
use std::rc::Rc;
/// Immutable requester-owned definition used to project one nominal into a generated-local
/// type environment.
///
/// The blueprint carries owned names, stable type identities and declaration-local generic
/// parameter slots, plus stable source locations used only for diagnostic and resource-origin
/// provenance. It contains no requester `TypeId`, `NominalTypeId`, `GenericParameterId`,
/// `PathId` or `StringId`. Registering every shell before populating members makes mutually
/// referential definitions safe without reopening the requester environment during materialisation.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct NominalMaterialisationBlueprint {
    pub(super) generic_parameters: Box<[NominalGenericParameterBlueprint]>,
    pub(super) definition: NominalMaterialisationDefinition,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct NominalGenericParameterBlueprint {
    pub(super) name: String,
    pub(super) exported_identity: Option<ExportedGenericParameterIdentity>,
    pub(super) bounds: Box<[CanonicalTraitIdentity]>,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum NominalMaterialisationDefinition {
    Struct {
        fields: Box<[NominalFieldBlueprint]>,
        const_record: bool,
    },
    Choice {
        variants: Box<[NominalChoiceVariantBlueprint]>,
    },
}

#[derive(Clone)]
pub(super) struct NominalFieldBlueprint {
    pub(super) name: String,
    pub(super) field_type: MaterialisationTypeBlueprint,
    pub(super) folded_default: Option<PublicFoldedValue>,
    /// Exact authored range used when materialising diagnostics and resource origins.
    pub(super) span: Option<crate::compiler_frontend::source::SourceSpan>,
}

impl PartialEq for NominalFieldBlueprint {
    fn eq(&self, other: &Self) -> bool {
        // WHY: `location` is diagnostic provenance only; it never decides semantic agreement,
        // because imported projections legitimately carry the default range.
        self.name == other.name
            && self.field_type == other.field_type
            && self.folded_default == other.folded_default
    }
}

impl Eq for NominalFieldBlueprint {}

impl NominalFieldBlueprint {
    fn merge_provenance_from(&mut self, other: &Self) {
        if self.span.is_none() {
            self.span = other.span;
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct NominalChoiceVariantBlueprint {
    pub(super) name: String,
    pub(super) tag: usize,
    pub(super) payload_fields: Box<[NominalFieldBlueprint]>,
}

/// Closed type shape used inside a nominal blueprint.
///
/// Canonical identities cover stable closed leaves. Declaration-local parameters and shapes that
/// contain them remain explicit so private generic nominals never acquire exported parameter
/// identities merely to support generated-local layout.
#[derive(Clone, PartialEq, Eq)]
pub(super) enum MaterialisationTypeBlueprint {
    Canonical(CanonicalTypeIdentity),
    GenericParameter(usize),
    Collection {
        element: Box<MaterialisationTypeBlueprint>,
        fixed_capacity: Option<usize>,
    },
    OrderedMap {
        key: Box<MaterialisationTypeBlueprint>,
        value: Box<MaterialisationTypeBlueprint>,
    },
    Option(Box<MaterialisationTypeBlueprint>),
    GenericInstance {
        base: CanonicalTypeIdentity,
        arguments: Box<[MaterialisationTypeBlueprint]>,
    },
}

impl NominalMaterialisationBlueprint {
    pub(super) fn merge_provenance_from(&mut self, other: &Self) {
        match (&mut self.definition, &other.definition) {
            (
                NominalMaterialisationDefinition::Struct { fields, .. },
                NominalMaterialisationDefinition::Struct {
                    fields: other_fields,
                    ..
                },
            ) => {
                for (field, other_field) in fields.iter_mut().zip(other_fields) {
                    field.merge_provenance_from(other_field);
                }
            }
            (
                NominalMaterialisationDefinition::Choice { variants },
                NominalMaterialisationDefinition::Choice {
                    variants: other_variants,
                },
            ) => {
                for (variant, other_variant) in variants.iter_mut().zip(other_variants) {
                    for (field, other_field) in variant
                        .payload_fields
                        .iter_mut()
                        .zip(&other_variant.payload_fields)
                    {
                        field.merge_provenance_from(other_field);
                    }
                }
            }
            _ => unreachable!("semantic equality guarantees matching nominal blueprint kinds"),
        }
    }
}
impl MaterialisationTypeBlueprint {
    pub(super) fn collect_nominal_identities(
        &self,
        identities: &mut FxHashSet<CanonicalTypeIdentity>,
    ) {
        match self {
            Self::Canonical(identity) => {
                collect_reachable_nominal_identities(identity, identities);
            }
            Self::GenericParameter(_) => {}
            Self::Collection { element, .. } | Self::Option(element) => {
                element.collect_nominal_identities(identities);
            }
            Self::OrderedMap { key, value } => {
                key.collect_nominal_identities(identities);
                value.collect_nominal_identities(identities);
            }
            Self::GenericInstance { base, arguments } => {
                identities.insert(base.clone());
                for argument in arguments {
                    argument.collect_nominal_identities(identities);
                }
            }
        }
    }
}
pub(super) fn materialised_nominal_declaration(
    local_path: PathId,
    type_id: TypeId,
    type_environment: &TypeEnvironment,
) -> Result<Declaration, CompilerError> {
    let diagnostic_type = match type_environment.get(type_id) {
        Some(TypeDefinition::Struct(definition)) => DataType::Struct {
            nominal_path: local_path,
            type_id,
            const_record: definition.const_record,
            generic_instance_key: None,
        },
        Some(TypeDefinition::Choice(_)) => DataType::Choices {
            nominal_path: local_path,
            type_id,
            generic_instance_key: None,
        },
        _ => {
            return Err(CompilerError::compiler_error(
                "Materialised nominal binding has no struct or choice definition",
            ));
        }
    };
    Ok(Declaration {
        id: local_path,
        type_id,
        value: DeclarationValue::Shell(match &diagnostic_type {
            DataType::Struct { .. } => DeclarationShell::Struct,
            _ => DeclarationShell::Choice,
        }),
        diagnostic_type,
        value_mode: ValueMode::ImmutableReference,
        binding_span: None,
        config_qualifier: None,
    })
}

pub(super) fn materialised_struct_fields(
    type_id: TypeId,
    type_environment: &mut TypeEnvironment,
    blueprint: &NominalMaterialisationBlueprint,
    nominal_source: &impl MaterialisationNominalSource,
    services: &mut GeneratedValueMaterialisationServices<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Option<Vec<Declaration>>, CompilerError> {
    if !matches!(
        type_environment.get(type_id),
        Some(TypeDefinition::Struct(_))
    ) {
        return Ok(None);
    }
    let Some(fields) = type_environment.fields_for(type_id).map(<[_]>::to_vec) else {
        return Ok(None);
    };
    let NominalMaterialisationDefinition::Struct {
        fields: blueprint_fields,
        ..
    } = &blueprint.definition
    else {
        return Err(CompilerError::compiler_error(
            "Materialised struct has a non-struct nominal blueprint",
        ));
    };
    if fields.len() != blueprint_fields.len() {
        return Err(CompilerError::compiler_error(
            "Materialised struct field count disagrees with its stable blueprint",
        ));
    }
    let mut declarations = Vec::with_capacity(fields.len());
    for (field, blueprint_field) in fields.iter().zip(blueprint_fields) {
        let field_name_matches = path_fork
            .component(field.name)
            .is_some_and(|name| string_table.resolve(name) == blueprint_field.name);
        if !field_name_matches {
            return Err(CompilerError::compiler_error(
                "Materialised struct field name disagrees with its stable blueprint",
            ));
        }
        let field_span = blueprint_field.span;
        let value = if let Some(default) = blueprint_field.folded_default.as_ref() {
            let mut materialiser = GeneratedFoldedValueMaterialiser {
                type_environment,
                external_registry: services.external_registry,
                nominal_source,
                module_resources: Rc::clone(&services.module_resources),
                path_fork,
            };
            let value_id = import_canonical_folded_value(
                &mut materialiser,
                default,
                field.type_id,
                services.const_values,
                string_table,
                field_span,
            )
            .map_err(const_store_error_into_compiler_error)?;
            DeclarationValue::Default(value_id)
        } else {
            DeclarationValue::Absent
        };
        declarations.push(Declaration {
            id: field.name,
            type_id: field.type_id,
            diagnostic_type: diagnostic_type_spelling(field.type_id, type_environment),
            value_mode: ValueMode::ImmutableReference,
            value,
            binding_span: field_span,
            config_qualifier: None,
        });
    }
    Ok(Some(declarations))
}
impl ModuleMaterialisationCapture {
    /// Freeze every requester-visible nominal definition into owned, stable semantic data.
    ///
    /// Imported and local aliases can add several lookup paths for the same nominal. The
    /// canonical identity is the sole blueprint key, so each definition is captured once in
    /// deterministic identity order.
    pub(super) fn install_nominal_blueprints(
        &mut self,
        const_values: &ConstValueStore,
        resources: &ModuleResourceTable,
        path_fork: &crate::compiler_frontend::symbols::path_interner::PathInternerFork,
    ) -> Result<(), CompilerError> {
        let mut nominal_type_ids = FxHashMap::default();
        for (identity, type_id) in self.type_environment.canonical_type_identities() {
            if matches!(
                identity,
                CanonicalTypeIdentity::SourceNominal(_)
                    | CanonicalTypeIdentity::ModulePrivateNominal(_)
            ) {
                nominal_type_ids.entry(identity.clone()).or_insert(type_id);
            }
        }

        let mut nominal_type_ids = nominal_type_ids.into_iter().collect::<Vec<_>>();
        nominal_type_ids.sort_by(|left, right| left.0.cmp(&right.0));

        let mut blueprints = FxHashMap::default();
        for (identity, type_id) in nominal_type_ids {
            let blueprint =
                self.nominal_blueprint(const_values, &identity, type_id, resources, path_fork)?;
            blueprints.insert(identity, blueprint);
        }
        self.nominal_blueprints = blueprints;
        Ok(())
    }

    fn nominal_blueprint(
        &self,
        const_values: &ConstValueStore,
        identity: &CanonicalTypeIdentity,
        type_id: TypeId,
        resources: &ModuleResourceTable,
        path_fork: &crate::compiler_frontend::symbols::path_interner::PathInternerFork,
    ) -> Result<NominalMaterialisationBlueprint, CompilerError> {
        let generic_parameter_list_id = match self.type_environment.get(type_id) {
            Some(TypeDefinition::Struct(definition)) => definition.generic_parameters,
            Some(TypeDefinition::Choice(definition)) => definition.generic_parameters,
            _ => {
                return Err(CompilerError::compiler_error(
                    "Materialisation nominal blueprint target is not a struct or choice",
                ));
            }
        };

        let (generic_parameters, parameter_slots) = if let Some(generic_parameter_list_id) =
            generic_parameter_list_id
        {
            let generic_parameter_list = self
                .type_environment
                .generic_parameters(generic_parameter_list_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Materialisation nominal references a missing generic parameter list",
                    )
                })?;
            let parameter_slots = generic_parameter_list
                .parameters
                .iter()
                .enumerate()
                .map(|(index, parameter)| (parameter.id, index))
                .collect::<FxHashMap<_, _>>();
            let public_parameters = self.public_nominal_generic_parameters(identity)?;
            if let Some(public_parameters) = public_parameters
                && public_parameters.len() != generic_parameter_list.parameters.len()
            {
                return Err(CompilerError::compiler_error(
                    "Materialisation nominal public generic surface has inconsistent arity",
                ));
            }

            let generic_parameters = generic_parameter_list
                    .parameters
                    .iter()
                    .enumerate()
                    .map(|(index, parameter)| {
                let name = self.string_table.resolve(parameter.name).to_owned();
                if let Some(public_parameters) = public_parameters {
                    let public_parameter = &public_parameters[index];
                    if public_parameter.identity.authored_name() != name {
                        return Err(CompilerError::compiler_error(
                            "Materialisation nominal public generic parameter name disagrees with its local definition",
                        ));
                    }
                    return Ok(NominalGenericParameterBlueprint {
                        name,
                        exported_identity: Some(public_parameter.identity.clone()),
                        bounds: public_parameter.bounds.clone().into_boxed_slice(),
                    });
                }

                let exported_identity = match identity {
                    CanonicalTypeIdentity::SourceNominal(origin) => Some(
                        ExportedGenericParameterIdentity::new(
                            GenericDeclarationOrigin::nominal_type(origin.clone())?,
                            index as u32,
                            name.clone(),
                        ),
                    ),
                    CanonicalTypeIdentity::ModulePrivateNominal(_) => None,
                    _ => {
                        return Err(CompilerError::compiler_error(
                            "Materialisation generic parameter owner is not nominal",
                        ));
                    }
                };
                let bounds = parameter
                    .trait_bounds
                    .iter()
                    .map(|trait_id| {
                        self.trait_environment
                            .canonical_identity_for_id(*trait_id)
                            .cloned()
                            .ok_or_else(|| {
                                CompilerError::compiler_error(
                                    "Materialisation nominal generic bound has no canonical trait identity",
                                )
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .into_boxed_slice();
                Ok(NominalGenericParameterBlueprint {
                    name,
                    exported_identity,
                    bounds,
                })
                    })
                    .collect::<Result<Box<[_]>, CompilerError>>()?;
            (generic_parameters, parameter_slots)
        } else {
            (
                Vec::<NominalGenericParameterBlueprint>::new().into_boxed_slice(),
                FxHashMap::default(),
            )
        };

        let definition = match self.type_environment.get(type_id) {
            Some(TypeDefinition::Struct(definition)) => NominalMaterialisationDefinition::Struct {
                fields: self.nominal_field_blueprints(
                    const_values,
                    &definition.fields,
                    &parameter_slots,
                    self.type_environment.nominal_path(type_id),
                    resources,
                    path_fork,
                )?,
                const_record: definition.const_record,
            },
            Some(TypeDefinition::Choice(definition)) => NominalMaterialisationDefinition::Choice {
                variants: self.nominal_choice_blueprints(
                    const_values,
                    &definition.variants,
                    &parameter_slots,
                    self.type_environment.nominal_path(type_id),
                    resources,
                    path_fork,
                )?,
            },
            None => {
                return Err(CompilerError::compiler_error(
                    "Materialisation nominal blueprint target is not a registered type",
                ));
            }
            Some(_) => {
                return Err(CompilerError::compiler_error(
                    "Materialisation nominal blueprint target is not a struct or choice",
                ));
            }
        };

        Ok(NominalMaterialisationBlueprint {
            generic_parameters,
            definition,
        })
    }

    fn public_nominal_generic_parameters(
        &self,
        identity: &CanonicalTypeIdentity,
    ) -> Result<
        Option<&[crate::compiler_frontend::public_interface::PublicGenericParameterSurface]>,
        CompilerError,
    > {
        let CanonicalTypeIdentity::SourceNominal(origin) = identity else {
            return Ok(None);
        };
        let Some(record) = self
            .binding_environment
            .imported_declarations_by_origin
            .get(&OriginDeclarationId::Type(origin.clone()))
        else {
            return Ok(None);
        };
        match &record.semantics {
            PublicDeclarationSemantics::Struct(semantics) => {
                Ok(Some(&semantics.generic_parameters))
            }
            PublicDeclarationSemantics::Choice(semantics) => {
                Ok(Some(&semantics.generic_parameters))
            }
            _ => Err(CompilerError::compiler_error(
                "Materialisation nominal origin resolved to a non-nominal public declaration",
            )),
        }
    }

    fn field_declaration(
        &self,
        nominal_path: &PathId,
        field_name: StringId,
        path_fork: &crate::compiler_frontend::symbols::path_interner::PathInternerFork,
    ) -> Option<&Declaration> {
        self.resolved_struct_fields_by_path
            .get(nominal_path)?
            .iter()
            .find(|declaration| path_fork.component(declaration.id) == Some(field_name))
    }

    fn nominal_field_blueprints(
        &self,
        const_values: &ConstValueStore,
        fields: &[FieldDefinition],
        parameter_slots: &FxHashMap<GenericParameterId, usize>,
        nominal_path: Option<&PathId>,
        resources: &ModuleResourceTable,
        path_fork: &crate::compiler_frontend::symbols::path_interner::PathInternerFork,
    ) -> Result<Box<[NominalFieldBlueprint]>, CompilerError> {
        let paths = path_fork;
        fields
            .iter()
            .map(|field| {
                let name = paths.component(field.name).ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Materialisation nominal field path has no defining name",
                    )
                })?;
                let field_declaration =
                    nominal_path.and_then(|path| self.field_declaration(path, name, path_fork));
                let folded_default = field_declaration
                    .and_then(|declaration| {
                        declaration
                            .value
                            .default()
                            .map(|value_id| (declaration.id, value_id))
                    })
                    .map(|(path, value_id)| {
                        self.stable_folded_value_at_default_path(
                            const_values,
                            &path,
                            value_id,
                            resources,
                            path_fork,
                        )
                    })
                    .transpose()?;
                let authored_span = field_declaration
                    .map(|declaration| declaration.binding_span)
                    .unwrap_or(field.span);
                Ok(NominalFieldBlueprint {
                    name: self.string_table.resolve(name).to_owned(),
                    field_type: self
                        .materialisation_type_blueprint(field.type_id, parameter_slots)?,
                    folded_default,
                    span: authored_span,
                })
            })
            .collect::<Result<Box<[_]>, CompilerError>>()
    }

    fn nominal_choice_blueprints(
        &self,
        const_values: &ConstValueStore,
        variants: &[ChoiceVariantDefinition],
        parameter_slots: &FxHashMap<GenericParameterId, usize>,
        nominal_path: Option<&PathId>,
        resources: &ModuleResourceTable,
        path_fork: &crate::compiler_frontend::symbols::path_interner::PathInternerFork,
    ) -> Result<Box<[NominalChoiceVariantBlueprint]>, CompilerError> {
        variants
            .iter()
            .map(|variant| {
                let payload_fields = match &variant.payload {
                    ChoiceVariantPayloadDefinition::Unit => Box::new([]),
                    ChoiceVariantPayloadDefinition::Record { fields } => self
                        .nominal_field_blueprints(
                            const_values,
                            fields,
                            parameter_slots,
                            nominal_path,
                            resources,
                            path_fork,
                        )?,
                };
                Ok(NominalChoiceVariantBlueprint {
                    name: self.string_table.resolve(variant.name).to_owned(),
                    tag: variant.tag,
                    payload_fields,
                })
            })
            .collect::<Result<Box<[_]>, CompilerError>>()
    }

    pub(super) fn materialisation_type_blueprint(
        &self,
        type_id: TypeId,
        parameter_slots: &FxHashMap<GenericParameterId, usize>,
    ) -> Result<MaterialisationTypeBlueprint, CompilerError> {
        let definition = self.type_environment.get(type_id).ok_or_else(|| {
            CompilerError::compiler_error(
                "Materialisation nominal member references an unknown local type",
            )
        })?;
        match definition {
            TypeDefinition::GenericParameter(parameter) => parameter_slots
                .get(&parameter.id)
                .copied()
                .map(MaterialisationTypeBlueprint::GenericParameter)
                .or_else(|| {
                    self.type_environment
                        .canonical_identity_for_type_id(type_id)
                        .cloned()
                        .map(MaterialisationTypeBlueprint::Canonical)
                })
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Materialisation nominal member references a foreign unresolved generic parameter",
                    )
                }),
            TypeDefinition::Builtin(builtin) => Ok(MaterialisationTypeBlueprint::Canonical(
                CanonicalTypeIdentity::Builtin(canonical_builtin_for_key(builtin.key)),
            )),
            TypeDefinition::Struct(_) | TypeDefinition::Choice(_) => {
                self.type_environment
                    .canonical_identity_for_type_id(type_id)
                    .cloned()
                    .map(MaterialisationTypeBlueprint::Canonical)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "Materialisation nominal member TypeId({}) has no canonical identity: {definition:?}",
                            type_id.0,
                        ))
                    })
            }
            TypeDefinition::External(external) => {
                let (package, symbol_path) = self
                    .external_package_registry
                    .resolve_type_package_and_symbol_path(external.type_id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Materialisation nominal external member type is absent from the binding registry",
                        )
                    })?;
                Ok(MaterialisationTypeBlueprint::Canonical(
                    CanonicalTypeIdentity::ExternalOpaque(ExternalOpaqueTypeIdentity::new(
                        package,
                        symbol_path.clone(),
                    )),
                ))
            }
            TypeDefinition::Constructed(constructed) => {
                self.constructed_materialisation_type_blueprint(constructed, parameter_slots)
            }
            TypeDefinition::GenericInstance(instance) => {
                let base_type_id = self
                    .type_environment
                    .type_id_for_nominal_id(instance.base)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Materialisation generic instance has no nominal base type",
                        )
                    })?;
                let base = self
                    .type_environment
                    .canonical_identity_for_type_id(base_type_id)
                    .cloned()
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Materialisation generic instance base has no canonical identity",
                        )
                    })?;
                let arguments = instance
                    .arguments
                    .iter()
                    .map(|argument| {
                        self.materialisation_type_blueprint(*argument, parameter_slots)
                    })
                    .collect::<Result<Box<[_]>, _>>()?;
                Ok(MaterialisationTypeBlueprint::GenericInstance { base, arguments })
            }
            TypeDefinition::Function(_) => Err(CompilerError::compiler_error(
                "Materialisation nominal member cannot contain a function type",
            )),
            TypeDefinition::AnonymousConstRecordMarker => Ok(
                MaterialisationTypeBlueprint::Canonical(CanonicalTypeIdentity::AnonymousConstRecord),
            ),
        }
    }

    fn constructed_materialisation_type_blueprint(
        &self,
        constructed: &crate::compiler_frontend::datatypes::definitions::ConstructedTypeDefinition,
        parameter_slots: &FxHashMap<GenericParameterId, usize>,
    ) -> Result<MaterialisationTypeBlueprint, CompilerError> {
        let arguments = constructed.arguments.as_ref();
        let project = |type_id| self.materialisation_type_blueprint(type_id, parameter_slots);
        match constructed.constructor {
            TypeConstructor::Builtin(BuiltinTypeConstructor::Collection { fixed_capacity }) => {
                let [element] = arguments else {
                    return Err(materialisation_type_arity_error(
                        "collection",
                        1,
                        arguments.len(),
                    ));
                };
                Ok(MaterialisationTypeBlueprint::Collection {
                    element: Box::new(project(*element)?),
                    fixed_capacity,
                })
            }
            TypeConstructor::Builtin(BuiltinTypeConstructor::OrderedMap) => {
                let [key, value] = arguments else {
                    return Err(materialisation_type_arity_error(
                        "ordered map",
                        2,
                        arguments.len(),
                    ));
                };
                Ok(MaterialisationTypeBlueprint::OrderedMap {
                    key: Box::new(project(*key)?),
                    value: Box::new(project(*value)?),
                })
            }
            TypeConstructor::Builtin(BuiltinTypeConstructor::Option) => {
                let [inner] = arguments else {
                    return Err(materialisation_type_arity_error(
                        "option",
                        1,
                        arguments.len(),
                    ));
                };
                Ok(MaterialisationTypeBlueprint::Option(Box::new(project(
                    *inner,
                )?)))
            }
        }
    }
}

fn materialisation_type_arity_error(shape: &str, expected: usize, actual: usize) -> CompilerError {
    CompilerError::compiler_error(format!(
        "Materialisation {shape} type has malformed arity: expected {expected}, found {actual}",
    ))
}

pub(super) fn intern_generated_canonical_type(
    identity: &CanonicalTypeIdentity,
    type_environment: &mut TypeEnvironment,
    external_registry: &ExternalPackageRegistry,
    nominal_source: &impl MaterialisationNominalSource,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<TypeId, CompilerError> {
    if let Some(type_id) = type_environment.type_id_for_canonical_identity(identity) {
        return Ok(type_id);
    }

    let type_id = match identity {
        CanonicalTypeIdentity::Builtin(builtin) => {
            match intern_canonical_builtin(*builtin, type_environment) {
                Some(type_id) => type_id,
                // `Error` is a source-declared nominal, so its `TypeId` must come from the
                // declaring environment rather than the seeded builtin layout.
                None => type_environment
                    .type_id_for_canonical_identity(identity)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Generated canonical Error type has no declaring-context builtin declaration",
                        )
                    })?,
            }
        }
        CanonicalTypeIdentity::Option(inner) => {
            let inner = intern_generated_canonical_type(
                inner,
                type_environment,
                external_registry,
                nominal_source,
                string_table,
                path_fork,
            )?;
            type_environment.intern_option(inner)
        }
        CanonicalTypeIdentity::Collection {
            element,
            fixed_capacity,
        } => {
            let element = intern_generated_canonical_type(
                element,
                type_environment,
                external_registry,
                nominal_source,
                string_table,
                path_fork,
            )?;
            type_environment.intern_collection(element, *fixed_capacity)
        }
        CanonicalTypeIdentity::OrderedMap { key, value } => {
            let key = intern_generated_canonical_type(
                key,
                type_environment,
                external_registry,
                nominal_source,
                string_table,
                path_fork,
            )?;
            let value = intern_generated_canonical_type(
                value,
                type_environment,
                external_registry,
                nominal_source,
                string_table,
                path_fork,
            )?;
            type_environment.intern_map(key, value)
        }
        CanonicalTypeIdentity::SourceNominal(_)
        | CanonicalTypeIdentity::ModulePrivateNominal(_) => intern_materialisation_nominal(
            identity,
            nominal_source,
            type_environment,
            external_registry,
            string_table,
            path_fork,
        )?,
        CanonicalTypeIdentity::ExternalOpaque(external) => {
            let (external_type_id, _) = external_registry
                .resolve_canonical_package_type_by_path(external.package(), external.symbol_path())
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Generated canonical external type is absent from the binding registry",
                    )
                })?;
            type_environment.intern_external(external_type_id)
        }
        CanonicalTypeIdentity::GenericInstance { base, arguments } => {
            let base_identity = CanonicalTypeIdentity::SourceNominal(base.clone());
            let base_type_id = intern_generated_canonical_type(
                &base_identity,
                type_environment,
                external_registry,
                nominal_source,
                string_table,
                path_fork,
            )?;
            let nominal_id = match type_environment.get(base_type_id) {
                Some(TypeDefinition::Struct(definition)) => definition.id,
                Some(TypeDefinition::Choice(definition)) => definition.id,
                _ => {
                    return Err(CompilerError::compiler_error(
                        "Generated canonical generic base is not nominal",
                    ));
                }
            };
            let mut interned_arguments = Vec::with_capacity(arguments.len());
            for argument in arguments.iter() {
                interned_arguments.push(intern_generated_canonical_type(
                    argument,
                    type_environment,
                    external_registry,
                    nominal_source,
                    string_table,
                    path_fork,
                )?);
            }
            type_environment
                .intern_generic_instance(nominal_id, interned_arguments.into_boxed_slice())
        }
        CanonicalTypeIdentity::ModulePrivateGenericInstance { base, arguments } => {
            let base_identity = CanonicalTypeIdentity::ModulePrivateNominal(base.clone());
            let base_type_id = intern_generated_canonical_type(
                &base_identity,
                type_environment,
                external_registry,
                nominal_source,
                string_table,
                path_fork,
            )?;
            let nominal_id = match type_environment.get(base_type_id) {
                Some(TypeDefinition::Struct(definition)) => definition.id,
                Some(TypeDefinition::Choice(definition)) => definition.id,
                _ => {
                    return Err(CompilerError::compiler_error(
                        "Generated private generic base is not nominal",
                    ));
                }
            };
            let mut interned_arguments = Vec::with_capacity(arguments.len());
            for argument in arguments.iter() {
                interned_arguments.push(intern_generated_canonical_type(
                    argument,
                    type_environment,
                    external_registry,
                    nominal_source,
                    string_table,
                    path_fork,
                )?);
            }
            type_environment
                .intern_generic_instance(nominal_id, interned_arguments.into_boxed_slice())
        }
        CanonicalTypeIdentity::GenericParameter(_) => {
            return Err(CompilerError::compiler_error(
                "Generated request retained an unresolved generic parameter",
            ));
        }
        CanonicalTypeIdentity::AnonymousConstRecord => {
            type_environment.anonymous_const_record_type()
        }
    };
    type_environment.register_canonical_identity(identity.clone(), type_id)?;
    Ok(type_id)
}
pub(super) fn intern_materialisation_type_blueprint(
    blueprint: &MaterialisationTypeBlueprint,
    generic_parameter_type_ids: &[TypeId],
    nominal_source: &impl MaterialisationNominalSource,
    type_environment: &mut TypeEnvironment,
    external_registry: &ExternalPackageRegistry,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<TypeId, CompilerError> {
    match blueprint {
        MaterialisationTypeBlueprint::Canonical(identity) => intern_generated_canonical_type(
            identity,
            type_environment,
            external_registry,
            nominal_source,
            string_table,
            path_fork,
        ),
        MaterialisationTypeBlueprint::GenericParameter(slot) => generic_parameter_type_ids
            .get(*slot)
            .copied()
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Materialisation nominal member references an invalid generic parameter slot",
                )
            }),
        MaterialisationTypeBlueprint::Collection {
            element,
            fixed_capacity,
        } => {
            let element = intern_materialisation_type_blueprint(
                element,
                generic_parameter_type_ids,
                nominal_source,
                type_environment,
                external_registry,
                string_table,
                path_fork,
            )?;
            Ok(type_environment.intern_collection(element, *fixed_capacity))
        }
        MaterialisationTypeBlueprint::OrderedMap { key, value } => {
            let key = intern_materialisation_type_blueprint(
                key,
                generic_parameter_type_ids,
                nominal_source,
                type_environment,
                external_registry,
                string_table,
                path_fork,
            )?;
            let value = intern_materialisation_type_blueprint(
                value,
                generic_parameter_type_ids,
                nominal_source,
                type_environment,
                external_registry,
                string_table,
                path_fork,
            )?;
            Ok(type_environment.intern_map(key, value))
        }
        MaterialisationTypeBlueprint::Option(inner) => {
            let inner = intern_materialisation_type_blueprint(
                inner,
                generic_parameter_type_ids,
                nominal_source,
                type_environment,
                external_registry,
                string_table,
                path_fork,
            )?;
            Ok(type_environment.intern_option(inner))
        }
        MaterialisationTypeBlueprint::GenericInstance { base, arguments } => {
            let base_type_id = intern_generated_canonical_type(
                base,
                type_environment,
                external_registry,
                nominal_source,
                string_table,
                path_fork,
            )?;
            let nominal_id = match type_environment.get(base_type_id) {
                Some(TypeDefinition::Struct(definition)) => definition.id,
                Some(TypeDefinition::Choice(definition)) => definition.id,
                _ => {
                    return Err(CompilerError::compiler_error(
                        "Materialisation generic instance base is not nominal",
                    ));
                }
            };
            let expected_arity = type_environment
                .generic_parameter_list_id_for_type(base_type_id)
                .and_then(|list_id| type_environment.generic_parameters(list_id))
                .map(|parameters| parameters.parameters.len())
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Materialisation generic instance base has no generic parameter list",
                    )
                })?;
            if arguments.len() != expected_arity {
                return Err(materialisation_type_arity_error(
                    "generic instance",
                    expected_arity,
                    arguments.len(),
                ));
            }
            let arguments = arguments
                .iter()
                .map(|argument| {
                    intern_materialisation_type_blueprint(
                        argument,
                        generic_parameter_type_ids,
                        nominal_source,
                        type_environment,
                        external_registry,
                        string_table,
                        path_fork,
                    )
                })
                .collect::<Result<Box<[_]>, _>>()?;
            Ok(type_environment.intern_generic_instance(nominal_id, arguments))
        }
    }
}

fn materialisation_nominal_path(
    identity: &CanonicalTypeIdentity,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<PathId, CompilerError> {
    let mut path = PathId::ROOT;
    let mut push_component = |component: &str| -> Result<(), CompilerError> {
        path = path_fork
            .try_intern_child(path, string_table.intern(component))
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "path table exhausted while interning materialised nominal path",
                )
            })?;
        Ok(())
    };
    push_component("<materialised>")?;
    // Type display and generic base names read the definition path's leaf. Keep the stable
    // defining spelling last, with identity/category qualifiers confined to its parent path.
    match identity {
        CanonicalTypeIdentity::SourceNominal(origin) => {
            push_component("source")?;
            append_materialisation_module_origin(origin.module_origin(), &mut push_component)?;
            push_component(origin_type_category_name(origin.category()))?;
            push_component(origin.defining_name())?;
        }
        CanonicalTypeIdentity::ModulePrivateNominal(identity) => {
            push_component("private")?;
            append_materialisation_module_origin(identity.module_origin(), &mut push_component)?;
            push_component(origin_type_category_name(identity.category()))?;
            push_component(identity.defining_path())?;
        }
        _ => {
            return Err(CompilerError::compiler_error(
                "Materialisation nominal path requested for a non-nominal identity",
            ));
        }
    }
    Ok(path)
}

fn append_materialisation_module_origin(
    origin: &crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity,
    push_component: &mut impl FnMut(&str) -> Result<(), CompilerError>,
) -> Result<(), CompilerError> {
    let package_origin = match origin.package().origin() {
        crate::builder_surface::PackageOrigin::Core => "core",
        crate::builder_surface::PackageOrigin::Builder => "builder",
        crate::builder_surface::PackageOrigin::ProjectLocal => "project",
        crate::builder_surface::PackageOrigin::Dependency => "dependency",
    };
    let root_role = match origin.role() {
        ModuleRootRole::Normal => "normal",
        ModuleRootRole::Support => "support",
        ModuleRootRole::ProjectPackageFacade => "facade",
    };
    push_component(package_origin)?;
    push_component(origin.package().name())?;
    push_component(root_role)?;
    for component in origin.logical_module_path().split('/') {
        if !component.is_empty() {
            push_component(component)?;
        }
    }
    Ok(())
}

fn origin_type_category_name(category: OriginTypeCategory) -> &'static str {
    match category {
        OriginTypeCategory::Struct => "struct",
        OriginTypeCategory::Choice => "choice",
        OriginTypeCategory::TransparentAlias => "alias",
    }
}

fn intern_materialisation_nominal<N: MaterialisationNominalSource>(
    identity: &CanonicalTypeIdentity,
    nominal_source: &N,
    type_environment: &mut TypeEnvironment,
    external_registry: &ExternalPackageRegistry,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<TypeId, CompilerError> {
    if let Some(type_id) = type_environment.type_id_for_canonical_identity(identity) {
        return Ok(type_id);
    }
    let blueprint = nominal_source
        .nominal_blueprint(identity)
        .cloned()
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Generated request nominal {identity:?} is absent from {}",
                N::SOURCE_LABEL
            ))
        })?;

    // Bounds remain exact stable facts on the immutable blueprint. Reconstructing a concrete
    // nominal for field/variant substitution does not re-run declaration-site evidence solving.
    let generic_parameter_registration = (!blueprint.generic_parameters.is_empty()).then(|| {
        type_environment.register_generic_parameter_list(
            blueprint
                .generic_parameters
                .iter()
                .enumerate()
                .map(|(index, parameter)| {
                    (
                        TypeParameterId(index as u32),
                        string_table.intern(&parameter.name),
                    )
                }),
            &FxHashMap::default(),
        )
    });
    let generic_parameter_list_id = generic_parameter_registration
        .as_ref()
        .map(|registration| registration.list_id);
    let generated_path = materialisation_nominal_path(identity, string_table, path_fork)?;
    let type_id = match &blueprint.definition {
        NominalMaterialisationDefinition::Struct { const_record, .. } => {
            type_environment
                .register_nominal_struct(StructTypeDefinition {
                    id: NominalTypeId(0),
                    path: generated_path,
                    fields: Box::new([]),
                    generic_parameters: generic_parameter_list_id,
                    const_record: *const_record,
                })
                .1
        }
        NominalMaterialisationDefinition::Choice { .. } => {
            type_environment
                .register_nominal_choice(ChoiceTypeDefinition {
                    id: NominalTypeId(0),
                    path: generated_path,
                    variants: Box::new([]),
                    generic_parameters: generic_parameter_list_id,
                })
                .1
        }
    };
    type_environment.register_canonical_identity(identity.clone(), type_id)?;

    let parameter_type_ids = if let Some(registration) = generic_parameter_registration {
        blueprint
            .generic_parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| {
                let parameter_id = registration
                    .canonical_by_local
                    .get(&TypeParameterId(index as u32))
                    .copied()
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Generated nominal parameter registration omitted a parameter slot",
                        )
                    })?;
                let parameter_type_id = type_environment
                    .type_id_for_generic_parameter(parameter_id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Generated nominal parameter registration omitted its type handle",
                        )
                    })?;
                if let Some(exported_identity) = &parameter.exported_identity {
                    type_environment.register_canonical_identity(
                        CanonicalTypeIdentity::GenericParameter(exported_identity.clone()),
                        parameter_type_id,
                    )?;
                }
                Ok(parameter_type_id)
            })
            .collect::<Result<Vec<_>, CompilerError>>()?
    } else {
        Vec::new()
    };

    match &blueprint.definition {
        NominalMaterialisationDefinition::Struct { fields, .. } => {
            let nominal_path =
                type_environment
                    .nominal_path(type_id)
                    .copied()
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Generated struct nominal shell has no local path",
                        )
                    })?;
            let fields = fields
                .iter()
                .map(|field| {
                    let name = path_fork
                        .try_intern_child(nominal_path, string_table.intern(&field.name))
                        .ok_or_else(|| {
                            CompilerError::compiler_error(
                                "path table exhausted while interning materialised field",
                            )
                        })?;
                    Ok(FieldDefinition {
                        name,
                        type_id: intern_materialisation_type_blueprint(
                            &field.field_type,
                            &parameter_type_ids,
                            nominal_source,
                            type_environment,
                            external_registry,
                            string_table,
                            path_fork,
                        )?,
                        span: None,
                    })
                })
                .collect::<Result<Box<[_]>, CompilerError>>()?;
            type_environment.update_struct_fields(type_id, fields);
        }
        NominalMaterialisationDefinition::Choice { variants } => {
            let nominal_path =
                type_environment
                    .nominal_path(type_id)
                    .copied()
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Generated choice nominal shell has no local path",
                        )
                    })?;
            let variants = variants
                .iter()
                .map(|variant| {
                    let payload = if variant.payload_fields.is_empty() {
                        ChoiceVariantPayloadDefinition::Unit
                    } else {
                        let fields = variant
                            .payload_fields
                            .iter()
                            .map(|field| {
                                let name = path_fork
                                    .try_intern_child(
                                        nominal_path,
                                        string_table.intern(&field.name),
                                    )
                                    .ok_or_else(|| {
                                        CompilerError::compiler_error(
                                            "path table exhausted while interning materialised field",
                                        )
                                    })?;
                                Ok(FieldDefinition {
                                    name,
                                    type_id: intern_materialisation_type_blueprint(
                                        &field.field_type,
                                        &parameter_type_ids,
                                        nominal_source,
                                        type_environment,
                                        external_registry,
                                        string_table,
                                        path_fork,
                                    )?,
                                    span: None,
                                })
                            })
                            .collect::<Result<Box<[_]>, CompilerError>>()?;
                        ChoiceVariantPayloadDefinition::Record { fields }
                    };
                    Ok(ChoiceVariantDefinition {
                        name: string_table.intern(&variant.name),
                        tag: variant.tag,
                        payload,
                        span: None,
                    })
                })
                .collect::<Result<Box<[_]>, CompilerError>>()?;
            type_environment.update_choice_variants(type_id, variants);
        }
    }

    Ok(type_id)
}

impl ModuleMaterialisationCapture {
    pub(super) fn stable_type_identity(
        &self,
        type_id: TypeId,
    ) -> Result<CanonicalTypeIdentity, CompilerError> {
        stable_type_identity(
            type_id,
            &self.type_environment,
            &self.external_package_registry,
        )
    }
}

impl ModuleMaterialisationPreparation {
    pub(super) fn stable_type_identity(
        &self,
        type_id: TypeId,
    ) -> Result<CanonicalTypeIdentity, CompilerError> {
        stable_type_identity(
            type_id,
            &self.type_environment,
            &self.external_package_registry,
        )
    }
}

fn stable_type_identity(
    type_id: TypeId,
    type_environment: &TypeEnvironment,
    external_package_registry: &ExternalPackageRegistry,
) -> Result<CanonicalTypeIdentity, CompilerError> {
    let nominal_origins = MaterialisationNominalOriginResolver { type_environment };
    let generic_parameter_origins = FoldedValueGenericParameterResolver;
    let projection_context = CanonicalTypeProjectionContext::new(
        &nominal_origins,
        &generic_parameter_origins,
        external_package_registry,
    );
    project_type_id_to_canonical_identity(type_id, type_environment, &projection_context)
}
#[cfg(test)]
#[path = "../tests/blueprint_tests.rs"]
mod blueprint_tests;
