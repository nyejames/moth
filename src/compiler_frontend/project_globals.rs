//! Immutable compiler-side project-global synthetic interface.
//!
//! WHAT: projects the build-system-owned folded `project` fields into the ordinary
//! [`PublicSemanticInterface`] declaration vocabulary and retains one deterministic metadata row
//! per field. The metadata keeps diagnostic location, build fingerprint and synthetic-interface
//! provenance beside the stable member identity.
//! WHY: `@project` is a synthetic compile-time provider, not an ordinary source module. Keeping its
//! constants on the existing public-interface and folded-value vocabularies lets provider binding
//! consume one borrowed interface without introducing AST, HIR, runtime or recursive project-value
//! representations.
//!
//! This module owns only the compiler-side value contract. Stage 0 owns visibility and provider
//! registration; callers must explicitly bind the returned interface through the ordinary provider
//! dependency boundary.

use crate::compiler_frontend::build_config::BuildConfigFingerprint;
use crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::folded_value::PublicFoldedValue;
use crate::compiler_frontend::public_interface::{
    PublicConstantSemantics, PublicDeclarationRecord, PublicDeclarationSemantics,
    PublicDiagnosticLocation, PublicSemanticInterface,
};
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, OriginConstantId, OriginDeclarationId, StableModuleOriginIdentity,
    StablePackageIdentity,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};

/// The reserved dependency-root spelling for the synthetic project-global interface.
///
/// Dependency paths store the provider prefix without the leading `@` path introducer, so the
/// canonical one-component path is `project` while source authors write `@project`.
pub(crate) const PROJECT_GLOBALS_DEPENDENCY_NAME: &str = "project";

/// One owned project-global field entering [`ProjectGlobalsInterface`].
///
/// The field name is used as both the public binding name and the defining name in its stable
/// [`OriginConstantId`]. Values and type identities are already in the compiler-owned portable
/// vocabularies; no donor AST, HIR or runtime value is retained here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectGlobalsFieldInput {
    pub(crate) name: String,
    pub(crate) type_identity: CanonicalTypeIdentity,
    pub(crate) value: PublicFoldedValue,
    pub(crate) location: PublicDiagnosticLocation,
    pub(crate) fingerprint: BuildConfigFingerprint,
    pub(crate) provenance: SyntheticInterfaceProvenance,
}

impl ProjectGlobalsFieldInput {
    /// Construct one owned project-global field input.
    pub(crate) fn new(
        name: impl Into<String>,
        type_identity: CanonicalTypeIdentity,
        value: PublicFoldedValue,
        location: PublicDiagnosticLocation,
        fingerprint: BuildConfigFingerprint,
        provenance: SyntheticInterfaceProvenance,
    ) -> Self {
        Self {
            name: name.into(),
            type_identity,
            value,
            location,
            fingerprint,
            provenance,
        }
    }
}

/// Member-granular metadata retained beside one project-global field.
///
/// Metadata is ordered by [`SyntheticInterfaceMemberIdentity::member`] in the owning interface.
/// The identity, diagnostic location, fingerprint and provenance remain independent facts: source
/// coordinates do not affect stable declaration identity, and provenance does not alter folded
/// value semantics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectGlobalsMemberMetadata {
    pub(crate) identity: SyntheticInterfaceMemberIdentity,
    pub(crate) location: PublicDiagnosticLocation,
    pub(crate) fingerprint: BuildConfigFingerprint,
    pub(crate) provenance: SyntheticInterfaceProvenance,
}

impl ProjectGlobalsMemberMetadata {
    /// The stable synthetic-interface member identity.
    #[cfg(test)]
    pub(crate) fn identity(&self) -> &SyntheticInterfaceMemberIdentity {
        &self.identity
    }

    /// The portable source location retained for diagnostics.
    #[cfg(test)]
    pub(crate) fn location(&self) -> &PublicDiagnosticLocation {
        &self.location
    }

    /// The semantic build-configuration fingerprint for this field.
    #[cfg(test)]
    pub(crate) fn fingerprint(&self) -> BuildConfigFingerprint {
        self.fingerprint
    }

    /// The member-granular synthetic-interface provenance for this field.
    #[cfg(test)]
    pub(crate) fn provenance(&self) -> &SyntheticInterfaceProvenance {
        &self.provenance
    }
}

/// Immutable synthetic provider for direct project fields under `@project`.
///
/// The ordinary public interface carries one constant declaration and export binding for every
/// field. The parallel metadata vector is sorted by the same public field name, making both
/// provider binding and metadata lookup deterministic regardless of input order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectGlobalsInterface {
    interface: PublicSemanticInterface,
    members: Vec<ProjectGlobalsMemberMetadata>,
}
impl ProjectGlobalsInterface {
    /// Build the immutable project-global interface for one project package boundary.
    ///
    /// Every field receives a project-context synthetic member identity and a stable constant
    /// origin. Inputs are sorted by authored field name before any identities are created. A second
    /// field with the same name is rejected rather than overwriting the first field.
    pub(crate) fn new(
        project_package: StablePackageIdentity,
        mut fields: Vec<ProjectGlobalsFieldInput>,
    ) -> Result<Self, CompilerError> {
        fields.sort_by(|left, right| left.name.cmp(&right.name));

        for pair in fields.windows(2) {
            if pair[0].name == pair[1].name {
                return Err(CompilerError::compiler_error(format!(
                    "duplicate project-global field name '{}'",
                    pair[0].name
                )));
            }
        }

        let module_origin = StableModuleOriginIdentity::project_context(project_package);
        let mut export_bindings = Vec::with_capacity(fields.len());
        let mut declarations = Vec::with_capacity(fields.len());
        let mut members = Vec::with_capacity(fields.len());

        for field in fields {
            let ProjectGlobalsFieldInput {
                name,
                type_identity,
                value,
                location,
                fingerprint,
                provenance,
            } = field;

            let constant_origin = OriginConstantId::new(module_origin.clone(), name.clone());
            let declaration_origin = OriginDeclarationId::Constant(constant_origin);

            export_bindings.push(ExportBinding::new(
                module_origin.clone(),
                name.clone(),
                declaration_origin.clone(),
            ));
            declarations.push(PublicDeclarationRecord {
                origin: declaration_origin,
                synthetic_interface_provenance: provenance.clone(),
                semantics: PublicDeclarationSemantics::Constant(PublicConstantSemantics {
                    type_identity,
                    folded_value: value,
                }),
            });
            members.push(ProjectGlobalsMemberMetadata {
                identity: SyntheticInterfaceMemberIdentity::new(
                    SyntheticInterfaceClass::ProjectContext,
                    PROJECT_GLOBALS_DEPENDENCY_NAME,
                    name,
                ),
                location,
                fingerprint,
                provenance,
            });
        }

        let interface = PublicSemanticInterface {
            module_origin,
            export_bindings,
            export_diagnostic_provenance: Vec::new(),
            binding_exports: Vec::new(),
            declarations,
            reusable_evidence: Vec::new(),
            concrete_call_summaries: Vec::new(),
        };
        interface.validate_for_publication()?;

        Ok(Self { interface, members })
    }

    /// Borrow the ordinary public semantic interface used by provider binding.
    pub(crate) fn interface(&self) -> &PublicSemanticInterface {
        &self.interface
    }

    /// Borrow member metadata in deterministic public-field-name order.
    #[cfg(test)]
    pub(crate) fn members(&self) -> &[ProjectGlobalsMemberMetadata] {
        &self.members
    }

    /// Find one field's metadata by its exact public name.
    #[cfg(test)]
    pub(crate) fn member(&self, name: &str) -> Option<&ProjectGlobalsMemberMetadata> {
        self.members
            .binary_search_by(|metadata| metadata.identity.member().cmp(name))
            .ok()
            .map(|index| &self.members[index])
    }
}

/// Return whether a retained dependency path names exactly the reserved `@project` root.
///
/// The path parser consumes the leading `@` introducer, so this compares one interned component
/// against `project`. Component count is checked explicitly: nested paths such as
/// `@project/details` and coincident suffixes never claim the synthetic root.
pub(crate) fn is_project_globals_dependency(
    dependency_path: &InternedPath,
    string_table: &StringTable,
) -> bool {
    dependency_path.len() == 1
        && dependency_path.name_str(string_table) == Some(PROJECT_GLOBALS_DEPENDENCY_NAME)
}

/// Return whether a dependency path enters the permanently reserved `@project` namespace.
///
/// The exact root is handled as the synthetic provider. Descendants are never valid filesystem,
/// source-package or binding-provider paths, so Stage 0 can reject them before ordinary discovery.
pub(crate) fn is_project_globals_namespace(
    dependency_path: &InternedPath,
    string_table: &StringTable,
) -> bool {
    dependency_path
        .as_components()
        .first()
        .is_some_and(|component| {
            string_table.resolve(*component) == PROJECT_GLOBALS_DEPENDENCY_NAME
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler_frontend::canonical_type_identity::{
        CanonicalBuiltinType, CanonicalTypeIdentity,
    };
    use crate::compiler_frontend::semantic_identity::ModuleRootRole;

    fn location(line: i32) -> PublicDiagnosticLocation {
        PublicDiagnosticLocation {
            scope_components: vec!["config.moth".to_owned()],
            start_line: line,
            start_column: 2,
            end_line: line,
            end_column: 9,
        }
    }

    fn field(name: &str, line: i32, fingerprint: u64) -> ProjectGlobalsFieldInput {
        let member = SyntheticInterfaceMemberIdentity::new(
            SyntheticInterfaceClass::ProjectContext,
            PROJECT_GLOBALS_DEPENDENCY_NAME,
            name,
        );
        ProjectGlobalsFieldInput::new(
            name,
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int),
            PublicFoldedValue::Int(line),
            location(line),
            BuildConfigFingerprint(fingerprint),
            SyntheticInterfaceProvenance::single(member),
        )
    }

    #[test]
    fn fields_are_sorted_deterministically_and_projected_as_constants() {
        let interface = ProjectGlobalsInterface::new(
            StablePackageIdentity::project_local("ordering"),
            vec![field("zeta", 2, 20), field("alpha", 1, 10)],
        )
        .expect("unique project fields should construct");

        let names: Vec<_> = interface
            .members()
            .iter()
            .map(|metadata| metadata.identity().member())
            .collect();
        assert_eq!(names, ["alpha", "zeta"]);

        let binding_names: Vec<_> = interface
            .interface()
            .export_bindings
            .iter()
            .map(|binding| binding.public_name())
            .collect();
        assert_eq!(binding_names, ["alpha", "zeta"]);
        assert!(matches!(
            interface.interface().declarations[0].semantics,
            PublicDeclarationSemantics::Constant(_)
        ));
        assert_eq!(
            interface.interface().declarations[0].synthetic_interface_provenance,
            SyntheticInterfaceProvenance::single(SyntheticInterfaceMemberIdentity::new(
                SyntheticInterfaceClass::ProjectContext,
                PROJECT_GLOBALS_DEPENDENCY_NAME,
                "alpha",
            ))
        );
    }

    #[test]
    fn duplicate_field_names_are_rejected() {
        let error = ProjectGlobalsInterface::new(
            StablePackageIdentity::project_local("duplicates"),
            vec![field("same", 1, 1), field("same", 2, 2)],
        )
        .expect_err("duplicate project fields must fail");

        assert!(
            error
                .msg
                .contains("duplicate project-global field name 'same'")
        );
    }

    #[test]
    fn metadata_retains_location_fingerprint_and_provenance() {
        let interface = ProjectGlobalsInterface::new(
            StablePackageIdentity::project_local("metadata"),
            vec![field("version", 17, 0xfeed)],
        )
        .expect("project field should construct");
        let metadata = interface
            .member("version")
            .expect("metadata should be indexed by field name");

        assert_eq!(metadata.location().start_line, 17);
        assert_eq!(metadata.fingerprint(), BuildConfigFingerprint(0xfeed));
        assert_eq!(
            metadata.identity(),
            &SyntheticInterfaceMemberIdentity::new(
                SyntheticInterfaceClass::ProjectContext,
                PROJECT_GLOBALS_DEPENDENCY_NAME,
                "version",
            )
        );
        assert_eq!(
            metadata.provenance(),
            &SyntheticInterfaceProvenance::single(metadata.identity().clone())
        );
    }

    #[test]
    fn project_context_provenance_uses_synthetic_origin_not_real_facade() {
        let package = StablePackageIdentity::project_local("context");
        let interface = ProjectGlobalsInterface::new(package.clone(), vec![field("name", 1, 1)])
            .expect("project field should construct");
        let synthetic_origin = StableModuleOriginIdentity::project_context(package.clone());
        let facade_origin = StableModuleOriginIdentity::from_portable_path(
            package,
            String::new(),
            ModuleRootRole::ProjectPackageFacade,
        );

        assert_eq!(interface.interface().module_origin, synthetic_origin);
        assert_ne!(synthetic_origin, facade_origin);
        assert_eq!(
            interface.members()[0].identity().class(),
            SyntheticInterfaceClass::ProjectContext
        );
        assert_eq!(interface.members()[0].identity().interface(), "project");
        assert!(!interface.members()[0].provenance().is_empty());
    }

    #[test]
    fn project_dependency_helper_matches_only_exact_one_component_root() {
        let mut strings = StringTable::new();
        let project = InternedPath::from_single_str("project", &mut strings);
        let nested = InternedPath::from_components(vec![
            strings.intern("project"),
            strings.intern("details"),
        ]);
        let other = InternedPath::from_single_str("projects", &mut strings);

        assert!(is_project_globals_dependency(&project, &strings));
        assert!(!is_project_globals_dependency(&nested, &strings));
        assert!(!is_project_globals_dependency(&other, &strings));
        assert!(is_project_globals_namespace(&project, &strings));
        assert!(is_project_globals_namespace(&nested, &strings));
        assert!(!is_project_globals_namespace(&other, &strings));
    }
}
