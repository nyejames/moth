//! Stable visibility and namespace capture for generated materialisation.
use super::stable_types::{
    StableFunctionSignature, StableFunctionTarget, StableGenericParameter, StableReceiverKey,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::external_packages::CanonicalBindingSymbolIdentity;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::binding_environment::{
    FileVisibility, NamespaceRecord, NamespaceRecordSource, NamespaceTypeMember,
    NamespaceValueMember, SourceDeclarationTarget,
};
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::semantic_identity::OriginDeclarationId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathIdRemap};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::Arc;
#[derive(Clone, Default)]
pub(super) struct StableFileVisibility {
    pub(super) source_names: Box<[StableVisibleDeclaration]>,
    pub(super) type_alias_names: Box<[StableVisibleDeclaration]>,
    pub(super) trait_names: Box<[StableVisibleDeclaration]>,
    pub(super) external_symbols: Box<[StableExternalSymbol]>,
    pub(super) namespace_records: Box<[StableNamespaceBinding]>,
    pub(super) receiver_methods: Box<[StableReceiverMethod]>,
}

#[derive(Clone)]
pub(super) struct StableVisibleDeclaration {
    pub(super) visible_name: String,
    pub(super) local_path: PathId,
    pub(super) origin: Option<OriginDeclarationId>,
}

#[derive(Clone)]
pub(super) struct StableExternalSymbol {
    pub(super) visible_name: String,
    pub(super) identity: CanonicalBindingSymbolIdentity,
}

#[derive(Clone)]
pub(super) struct StableNamespaceBinding {
    pub(super) visible_name: String,
    pub(super) record: StableNamespaceRecord,
}

#[derive(Clone)]
pub(super) struct StableNamespaceRecord {
    pub(super) record_source: StableNamespaceRecordSource,
    pub(super) value_members: Box<[StableNamespaceValueMember]>,
    pub(super) type_members: Box<[StableNamespaceTypeMember]>,
    pub(super) child_namespaces: Box<[StableNamespaceBinding]>,
}

#[derive(Clone)]
pub(super) enum StableNamespaceRecordSource {
    SourceFile(PathId),
    ExternalPackage(String),
}

#[derive(Clone)]
pub(super) enum StableNamespaceValueMember {
    Source(StableVisibleDeclaration),
    External {
        visible_name: String,
        identity: CanonicalBindingSymbolIdentity,
    },
}

#[derive(Clone)]
pub(super) enum StableNamespaceTypeMember {
    Source(StableVisibleDeclaration),
    External {
        visible_name: String,
        identity: CanonicalBindingSymbolIdentity,
    },
}

#[derive(Clone)]
pub(super) struct StableReceiverMethod {
    pub(super) visible_name: String,
    pub(super) local_path: PathId,
    pub(super) target: StableFunctionTarget,
    pub(super) receiver: StableReceiverKey,
    pub(super) signature: StableFunctionSignature,
    pub(super) summary: Option<PublicCallSummary>,
    pub(super) generic_parameters: Box<[StableGenericParameter]>,
    pub(super) span: Option<SourceSpan>,
}
impl StableFileVisibility {
    pub(super) fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        for declaration in self
            .source_names
            .iter_mut()
            .chain(self.type_alias_names.iter_mut())
            .chain(self.trait_names.iter_mut())
        {
            declaration.remap_path_ids(remap);
        }
        for namespace in &mut self.namespace_records {
            namespace.remap_path_ids(remap);
        }
        for method in &mut self.receiver_methods {
            method.remap_path_ids(remap);
        }
    }
}

impl StableVisibleDeclaration {
    fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.local_path = remap.get(self.local_path);
    }
}

impl StableNamespaceBinding {
    fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.record.remap_path_ids(remap);
    }
}

impl StableNamespaceRecord {
    fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        if let StableNamespaceRecordSource::SourceFile(path) = &mut self.record_source {
            *path = remap.get(*path);
        }
        for member in &mut self.value_members {
            member.remap_path_ids(remap);
        }
        for member in &mut self.type_members {
            member.remap_path_ids(remap);
        }
        for namespace in &mut self.child_namespaces {
            namespace.remap_path_ids(remap);
        }
    }
}

impl StableNamespaceValueMember {
    fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        if let Self::Source(declaration) = self {
            declaration.remap_path_ids(remap);
        }
    }
}

impl StableNamespaceTypeMember {
    fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        if let Self::Source(declaration) = self {
            declaration.remap_path_ids(remap);
        }
    }
}

impl StableReceiverMethod {
    fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.local_path = remap.get(self.local_path);
        self.receiver.remap_path_ids(remap);
    }
}

// WHY: Mirrors `StableFileVisibility::collect_namespace_source_paths` below, but stays local
// because a frozen artefact must survive without its donor preparation environment.

pub(super) fn collect_namespace_source_paths(
    record: &NamespaceRecord,
    selected: &mut FxHashSet<PathId>,
) {
    for member in record.value_members.values() {
        if let NamespaceValueMember::SourceDeclaration(target) = member {
            selected.insert(target.local_path().clone());
        }
    }
    for member in record.type_members.values() {
        if let NamespaceTypeMember::SourceDeclaration(target) = member {
            selected.insert(target.local_path().clone());
        }
    }
    for child in record.child_namespaces.values() {
        collect_namespace_source_paths(child, selected);
    }
}
impl StableFileVisibility {
    pub(super) fn capture_namespace_record(
        record: &NamespaceRecord,
        external_package_registry: &ExternalPackageRegistry,
        string_table: &StringTable,
    ) -> Result<StableNamespaceRecord, CompilerError> {
        let record_source = match &record.record_source {
            NamespaceRecordSource::SourceFile(path) => {
                StableNamespaceRecordSource::SourceFile(*path)
            }
            NamespaceRecordSource::ExternalPackage(package) => {
                StableNamespaceRecordSource::ExternalPackage(
                    string_table.resolve(*package).to_owned(),
                )
            }
        };
        let mut value_members = record
            .value_members
            .iter()
            .map(|(name, member)| {
                let visible_name = string_table.resolve(*name).to_owned();
                let member = match member {
                    NamespaceValueMember::SourceDeclaration(target) => {
                        StableNamespaceValueMember::Source(StableVisibleDeclaration {
                            visible_name: visible_name.clone(),
                            local_path: *target.local_path(),
                            origin: match target {
                                SourceDeclarationTarget::Local(_) => None,
                                SourceDeclarationTarget::Imported { origin, .. } => {
                                    Some(origin.clone())
                                }
                            },
                        })
                    }
                    NamespaceValueMember::ExternalSymbol(symbol_id) => {
                        StableNamespaceValueMember::External {
                            visible_name: visible_name.clone(),
                            identity: external_package_registry
                                .canonical_symbol_identity(*symbol_id)
                                .ok_or_else(|| {
                                    CompilerError::compiler_error(
                                        "Materialisation namespace value has no canonical external identity",
                                    )
                                })?,
                        }
                    }
                };
                Ok((visible_name, member))
            })
            .collect::<Result<Vec<_>, CompilerError>>()?;
        value_members.sort_by(|left, right| left.0.cmp(&right.0));

        let mut type_members = record
            .type_members
            .iter()
            .map(|(name, member)| {
                let visible_name = string_table.resolve(*name).to_owned();
                let member = match member {
                    NamespaceTypeMember::SourceDeclaration(target) => {
                        StableNamespaceTypeMember::Source(StableVisibleDeclaration {
                            visible_name: visible_name.clone(),
                            local_path: *target.local_path(),
                            origin: match target {
                                SourceDeclarationTarget::Local(_) => None,
                                SourceDeclarationTarget::Imported { origin, .. } => {
                                    Some(origin.clone())
                                }
                            },
                        })
                    }
                    NamespaceTypeMember::ExternalSymbol(symbol_id) => {
                        StableNamespaceTypeMember::External {
                            visible_name: visible_name.clone(),
                            identity: external_package_registry
                                .canonical_symbol_identity(*symbol_id)
                                .ok_or_else(|| {
                                    CompilerError::compiler_error(
                                        "Materialisation namespace type has no canonical external identity",
                                    )
                                })?,
                        }
                    }
                };
                Ok((visible_name, member))
            })
            .collect::<Result<Vec<_>, CompilerError>>()?;
        type_members.sort_by(|left, right| left.0.cmp(&right.0));

        let mut child_namespaces = record
            .child_namespaces
            .iter()
            .map(|(name, child)| {
                Ok(StableNamespaceBinding {
                    visible_name: string_table.resolve(*name).to_owned(),
                    record: StableFileVisibility::capture_namespace_record(
                        child,
                        external_package_registry,
                        string_table,
                    )?,
                })
            })
            .collect::<Result<Vec<_>, CompilerError>>()?;
        child_namespaces.sort_by(|left, right| left.visible_name.cmp(&right.visible_name));

        Ok(StableNamespaceRecord {
            record_source,
            value_members: value_members
                .into_iter()
                .map(|(_, member)| member)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            type_members: type_members
                .into_iter()
                .map(|(_, member)| member)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            child_namespaces: child_namespaces.into_boxed_slice(),
        })
    }

    pub(super) fn materialise_namespace_record(
        record: &StableNamespaceRecord,
        external_package_registry: &ExternalPackageRegistry,
        string_table: &mut StringTable,
    ) -> Result<NamespaceRecord, CompilerError> {
        let record_source = match &record.record_source {
            StableNamespaceRecordSource::SourceFile(path) => {
                NamespaceRecordSource::SourceFile(*path)
            }
            StableNamespaceRecordSource::ExternalPackage(package) => {
                NamespaceRecordSource::ExternalPackage(string_table.intern(package))
            }
        };
        let mut materialised = NamespaceRecord::empty(record_source);
        for member in &record.value_members {
            match member {
                StableNamespaceValueMember::Source(binding) => {
                    let name = string_table.intern(&binding.visible_name);
                    let local_path = binding.local_path;
                    let target = match &binding.origin {
                        Some(origin) => SourceDeclarationTarget::Imported {
                            origin: origin.clone(),
                            local_path,
                        },
                        None => SourceDeclarationTarget::Local(local_path),
                    };
                    materialised
                        .value_members
                        .insert(name, NamespaceValueMember::SourceDeclaration(target));
                }
                StableNamespaceValueMember::External {
                    visible_name,
                    identity,
                } => {
                    let symbol_id = external_package_registry
                        .resolve_canonical_symbol(identity)
                        .ok_or_else(|| {
                            CompilerError::compiler_error(
                                "Materialisation namespace value is absent from the active registry",
                            )
                        })?;
                    materialised.value_members.insert(
                        string_table.intern(visible_name),
                        NamespaceValueMember::ExternalSymbol(symbol_id),
                    );
                }
            }
        }
        for member in &record.type_members {
            match member {
                StableNamespaceTypeMember::Source(binding) => {
                    let name = string_table.intern(&binding.visible_name);
                    let local_path = binding.local_path;
                    let target = match &binding.origin {
                        Some(origin) => SourceDeclarationTarget::Imported {
                            origin: origin.clone(),
                            local_path,
                        },
                        None => SourceDeclarationTarget::Local(local_path),
                    };
                    materialised
                        .type_members
                        .insert(name, NamespaceTypeMember::SourceDeclaration(target));
                }
                StableNamespaceTypeMember::External {
                    visible_name,
                    identity,
                } => {
                    let symbol_id = external_package_registry
                        .resolve_canonical_symbol(identity)
                        .ok_or_else(|| {
                            CompilerError::compiler_error(
                                "Materialisation namespace type is absent from the active registry",
                            )
                        })?;
                    materialised.type_members.insert(
                        string_table.intern(visible_name),
                        NamespaceTypeMember::ExternalSymbol(symbol_id),
                    );
                }
            }
        }
        for child in &record.child_namespaces {
            let name = string_table.intern(&child.visible_name);
            let child_record = Self::materialise_namespace_record(
                &child.record,
                external_package_registry,
                string_table,
            )?;
            materialised.child_namespaces.insert(name, child_record);
        }
        Ok(materialised)
    }

    pub(super) fn materialise(
        &self,
        external_package_registry: &ExternalPackageRegistry,
        path_fork: &mut crate::compiler_frontend::symbols::path_interner::PathInternerFork,
        string_table: &mut StringTable,
    ) -> Result<FileVisibility, CompilerError> {
        let mut visibility = FileVisibility::default();

        // The declaration gate is built alongside the name maps and installed once at the end,
        // so the binding loop can borrow both without splitting the visibility package.
        let mut visible_declaration_paths = FxHashSet::default();
        {
            let mut materialise_bindings =
                |bindings: &[StableVisibleDeclaration],
                 target: &mut FxHashMap<StringId, SourceDeclarationTarget>| {
                    for binding in bindings {
                        let name = string_table.intern(&binding.visible_name);
                        let local_path = binding.local_path;
                        visible_declaration_paths.insert(local_path);
                        let declaration_target = match &binding.origin {
                            Some(origin) => SourceDeclarationTarget::Imported {
                                origin: origin.clone(),
                                local_path,
                            },
                            None => SourceDeclarationTarget::Local(local_path),
                        };
                        target.insert(name, declaration_target);
                    }
                };
            materialise_bindings(&self.source_names, &mut visibility.visible_source_names);
            materialise_bindings(
                &self.type_alias_names,
                &mut visibility.visible_type_alias_names,
            );
            materialise_bindings(&self.trait_names, &mut visibility.visible_trait_names);
        }

        let error_path =
            crate::compiler_frontend::builtins::error_type::builtin_error_type_path(
                path_fork,
                string_table,
            );
        let error_name =
            string_table.intern(crate::compiler_frontend::builtins::error_type::ERROR_TYPE_NAME);
        visible_declaration_paths.insert(error_path);
        visibility.visible_declaration_paths = Arc::new(visible_declaration_paths);
        visibility
            .visible_source_names
            .insert(error_name, SourceDeclarationTarget::Local(error_path));
        for binding in &self.external_symbols {
            let symbol_id = external_package_registry
                .resolve_canonical_symbol(&binding.identity)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Materialisation external binding is absent from the active registry",
                    )
                })?;
            visibility
                .visible_external_symbols
                .insert(string_table.intern(&binding.visible_name), symbol_id);
        }
        for binding in &self.namespace_records {
            visibility.visible_namespace_records.insert(
                string_table.intern(&binding.visible_name),
                Self::materialise_namespace_record(
                    &binding.record,
                    external_package_registry,
                    string_table,
                )?,
            );
        }
        for method in &self.receiver_methods {
            let local_path = method.local_path;
            visibility
                .visible_receiver_methods
                .entry(string_table.intern(&method.visible_name))
                .or_default()
                .push(crate::compiler_frontend::headers::binding_environment::ReceiverMethodVisibility {
                    target: method.target.materialise(local_path),
                    span: method.span,
                });
        }

        Ok(visibility)
    }
    pub(super) fn materialised_selected_paths(
        &self,
    ) -> FxHashSet<PathId> {
        let mut selected = self
            .source_names
            .iter()
            .chain(self.type_alias_names.iter())
            .chain(self.trait_names.iter())
            .map(|binding| binding.local_path)
            .collect::<FxHashSet<_>>();
        selected.extend(self.receiver_methods.iter().map(|method| method.local_path));
        for namespace in &self.namespace_records {
            Self::collect_namespace_source_paths(&namespace.record, &mut selected);
        }
        selected
    }

    // WHY: Mirrors the live `collect_namespace_source_paths` above, but stays local because a
    // frozen artefact must survive without its donor preparation environment.
    fn collect_namespace_source_paths(
        record: &StableNamespaceRecord,
        selected: &mut FxHashSet<PathId>,
    ) {
        for member in &record.value_members {
            if let StableNamespaceValueMember::Source(binding) = member {
                selected.insert(binding.local_path);
            }
        }
        for member in &record.type_members {
            if let StableNamespaceTypeMember::Source(binding) = member {
                selected.insert(binding.local_path);
            }
        }
        for child in &record.child_namespaces {
            Self::collect_namespace_source_paths(&child.record, selected);
        }
    }

}
