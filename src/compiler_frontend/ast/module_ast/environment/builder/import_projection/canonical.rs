//! Durable inverse canonical type interning.

use super::*;

impl<'context, 'services> AstModuleEnvironmentBuilder<'context, 'services> {
    pub(super) fn intern_imported_canonical_type(
        &mut self,
        identity: &CanonicalTypeIdentity,
    ) -> Result<TypeId, CompilerError> {
        if let Some(type_id) = self
            .type_environment
            .type_id_for_canonical_identity(identity)
        {
            return Ok(type_id);
        }

        let type_id = match identity {
            CanonicalTypeIdentity::Builtin(builtin) => match builtin {
                CanonicalBuiltinType::Bool => builtin_type_ids::BOOL,
                CanonicalBuiltinType::Int => builtin_type_ids::INT,
                CanonicalBuiltinType::Float => builtin_type_ids::FLOAT,
                CanonicalBuiltinType::Decimal => builtin_type_ids::DECIMAL,
                CanonicalBuiltinType::String => builtin_type_ids::STRING,
                CanonicalBuiltinType::Char => builtin_type_ids::CHAR,
                CanonicalBuiltinType::Range => builtin_type_ids::RANGE,
                CanonicalBuiltinType::None => builtin_type_ids::NONE,
                CanonicalBuiltinType::Error => self
                    .type_environment
                    .type_id_for_canonical_identity(identity)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Imported canonical Error type has no consumer-local builtin declaration",
                        )
                    })?,
            },
            CanonicalTypeIdentity::Option(inner) => {
                let inner_id = self.intern_imported_canonical_type(inner)?;
                self.type_environment.intern_option(inner_id)
            }
            CanonicalTypeIdentity::Collection(collection) => {
                let element_id = self.intern_imported_canonical_type(collection.element())?;
                self.type_environment
                    .intern_collection(element_id, collection.fixed_capacity())
            }
            CanonicalTypeIdentity::OrderedMap(map) => {
                let key_id = self.intern_imported_canonical_type(map.key())?;
                let value_id = self.intern_imported_canonical_type(map.value())?;
                self.type_environment.intern_map(key_id, value_id)
            }
            CanonicalTypeIdentity::FallibleCarrier(carrier) => {
                let success_id = self.intern_imported_canonical_type(carrier.success())?;
                let error_id = self.intern_imported_canonical_type(carrier.error())?;
                self.type_environment
                    .intern_fallible_carrier(success_id, error_id)
            }
            CanonicalTypeIdentity::SourceNominal(origin) => self
                .imported_type_ids_by_origin
                .get(origin)
                .copied()
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "Imported canonical source type {origin:?} has no projected consumer-local nominal declaration"
                    ))
                })?,
            CanonicalTypeIdentity::ExternalOpaque(external) => {
                let (external_type_id, _) = self
                    .context
                    .external_package_registry
                    .resolve_package_type_by_path(
                        external.package_path(),
                        external.symbol_path(),
                    )
                    .ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "Imported canonical external type {external:?} is absent from the consumer's binding package registry"
                        ))
                    })?;
                self.type_environment.intern_external(external_type_id)
            }
            CanonicalTypeIdentity::GenericInstance(instance) => {
                let base_type_id = self
                    .imported_type_ids_by_origin
                    .get(instance.base())
                    .copied()
                    .ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "Imported canonical generic instance base {:?} has no projected consumer-local nominal declaration",
                            instance.base()
                        ))
                    })?;
                let nominal_id = match self.type_environment.get(base_type_id) {
                    Some(crate::compiler_frontend::datatypes::definitions::TypeDefinition::Struct(definition)) => definition.id,
                    Some(crate::compiler_frontend::datatypes::definitions::TypeDefinition::Choice(definition)) => definition.id,
                    _ => {
                        return Err(CompilerError::compiler_error(
                            "Imported canonical generic instance base did not resolve to a nominal type",
                        ));
                    }
                };
                let mut arguments = Vec::with_capacity(instance.arguments().len());
                for argument in instance.arguments() {
                    arguments.push(self.intern_imported_canonical_type(argument)?);
                }
                self.type_environment
                    .intern_generic_instance(nominal_id, arguments.into_boxed_slice())
            }
            CanonicalTypeIdentity::GenericParameter(parameter) => self
                .imported_generic_parameter_type_ids
                .get(parameter)
                .copied()
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "Imported canonical generic parameter {parameter:?} has no projected consumer-local parameter"
                    ))
                })?,
        };

        self.type_environment
            .register_canonical_identity(identity.clone(), type_id)?;
        Ok(type_id)
    }
}
