//! Stable file-reference rows retained by generic-function materialisation.
//!
//! The capture representation owns only compact path handles, prepared reference classes and
//! portable outcomes. Donor-local paths and string identifiers are resolved before a row crosses
//! the materialisation boundary.

use crate::compiler_frontend::ast::module_ast::scope_context::{
    FrozenResolvedFileReference, FrozenResolvedFileReferenceOutcome,
    Stage0ResolvedFileReferenceOutcome, Stage0ResolvedFileReferenceView,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::folded_value::{OwnedFoldedString, PublicFoldedValue};
use crate::compiler_frontend::paths::file_references::PreparedFileReferenceClass;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::paths::resource_identity::PortableResourcePath;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

#[derive(Clone)]
pub(super) struct StableResolvedFileReference {
    pub(super) path_syntax: PathSyntaxId,
    pub(super) class: PreparedFileReferenceClass,
    pub(super) outcome: StableResolvedFileReferenceOutcome,
}

#[derive(Clone)]
pub(super) enum StableResolvedFileReferenceOutcome {
    NoPhysicalTarget,
    Content { value: OwnedFoldedString },
    Resource { owner_relative_path: StringId },
    IdentifiedSourceKind,
}

impl StableResolvedFileReference {
    pub(super) fn capture(
        path_syntax: PathSyntaxId,
        resolved: Stage0ResolvedFileReferenceView<'_>,
        intern_resource_path: &mut impl FnMut(&str) -> StringId,
        content_value_at_path: &impl Fn(&InternedPath) -> Result<PublicFoldedValue, CompilerError>,
    ) -> Result<Self, CompilerError> {
        let outcome = match resolved.outcome {
            Stage0ResolvedFileReferenceOutcome::NoPhysicalTarget => {
                StableResolvedFileReferenceOutcome::NoPhysicalTarget
            }
            Stage0ResolvedFileReferenceOutcome::Content {
                logical_path,
                value,
            } => {
                let value = match value {
                    Some(value) => value.clone(),
                    None => {
                        let logical_path = logical_path.ok_or_else(|| {
                            CompilerError::compiler_error(
                                "ordinary content reference had no logical source path before capture",
                            )
                        })?;
                        capture_public_content_value(content_value_at_path(logical_path)?)?
                    }
                };
                StableResolvedFileReferenceOutcome::Content { value }
            }
            Stage0ResolvedFileReferenceOutcome::Resource {
                owner_relative_path,
                ..
            } => StableResolvedFileReferenceOutcome::Resource {
                owner_relative_path: intern_resource_path(owner_relative_path.as_str()),
            },
            Stage0ResolvedFileReferenceOutcome::IdentifiedSourceKind => {
                StableResolvedFileReferenceOutcome::IdentifiedSourceKind
            }
            Stage0ResolvedFileReferenceOutcome::Diagnostic(_) => {
                return Err(CompilerError::compiler_error(
                    "persistent generic body retained a diagnosed file reference; generic body validation must report it before capture",
                ));
            }
        };

        Ok(Self {
            path_syntax,
            class: resolved.class,
            outcome,
        })
    }

    pub(super) fn materialise(
        &self,
        remap: &[StringId],
        string_table: &StringTable,
    ) -> Result<FrozenResolvedFileReference, CompilerError> {
        let outcome = match &self.outcome {
            StableResolvedFileReferenceOutcome::NoPhysicalTarget => {
                FrozenResolvedFileReferenceOutcome::NoPhysicalTarget
            }
            StableResolvedFileReferenceOutcome::Content { value } => {
                FrozenResolvedFileReferenceOutcome::Content {
                    value: value.clone(),
                }
            }
            StableResolvedFileReferenceOutcome::Resource {
                owner_relative_path,
            } => {
                let owner_relative_path = pool_remap(*owner_relative_path, remap)?;
                FrozenResolvedFileReferenceOutcome::Resource {
                    owner_relative_path: PortableResourcePath::from_portable_spelling(
                        string_table.resolve(owner_relative_path).to_owned(),
                    )?,
                }
            }
            StableResolvedFileReferenceOutcome::IdentifiedSourceKind => {
                FrozenResolvedFileReferenceOutcome::IdentifiedSourceKind
            }
        };

        Ok(FrozenResolvedFileReference {
            path_syntax: self.path_syntax,
            class: self.class,
            outcome,
        })
    }
}

fn capture_public_content_value(
    value: PublicFoldedValue,
) -> Result<OwnedFoldedString, CompilerError> {
    let PublicFoldedValue::String(value) = value else {
        return Err(CompilerError::compiler_error(
            "synthetic content constant did not fold to a String value",
        ));
    };
    Ok(value)
}

fn pool_remap(id: StringId, remap: &[StringId]) -> Result<StringId, CompilerError> {
    let index = id.index() as usize;
    remap.get(index).copied().ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "frozen generic payload references out-of-range pool entry {index}"
        ))
    })
}
