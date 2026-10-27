use super::{
    ClassifiedSource, SourceClassification, SourceFileKind, SourceLogicalIdentity, SourceOwnership,
    UnrootedSourceLogicalPath, validate_unique_source_logical_identities,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailureKind;
use std::path::PathBuf;

#[test]
fn duplicate_logical_identity_is_rejected_with_identity_and_physical_paths() {
    let classified_source = |canonical_path: &str| ClassifiedSource {
        canonical_path: PathBuf::from(canonical_path),
        classification: SourceClassification::CompilerSemantic(SourceFileKind::Moth),
        supported: true,
        logical_identity: SourceLogicalIdentity::Unrooted(
            UnrootedSourceLogicalPath::from_portable("duplicate.moth".to_owned()),
        ),
        ownership: SourceOwnership::Unrooted,
    };
    let classified = vec![
        classified_source("/first/duplicate.moth"),
        classified_source("/second/duplicate.moth"),
    ];

    let error = validate_unique_source_logical_identities(&classified)
        .expect_err("duplicate logical identities must be rejected");
    assert_eq!(error.kind, InfrastructureFailureKind::Internal);
    assert!(error.message.contains("/first/duplicate.moth"));
    assert!(error.message.contains("/second/duplicate.moth"));
}
