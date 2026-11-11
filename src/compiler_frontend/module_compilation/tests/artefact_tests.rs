//! Compiler artefact metadata remap invariants.
//!
//! WHAT: exercises module documentation target rewriting through a non-identity path remap.
//! WHY: public metadata must carry declaration identities from the destination table, not stale
//! source-table identities.

use super::*;
use crate::compiler_frontend::symbols::path_interner::PathId;

#[test]
fn non_identity_remap_rewrites_documentation_targets() {
    let authored_target = PathId::try_from_index(2).expect("the test declaration path should fit");
    let remapped_target =
        PathId::try_from_index(7).expect("the destination declaration path should fit");
    let mut metadata = ModuleCompilerMetadata {
        entry: None,
        doc_fragments: vec![ModuleDocFragment {
            target: authored_target,
            rendered_text: "Public documentation.".to_owned(),
            span: None,
        }],
        materialisation_context: None,
    };
    let remap = PathIdRemap::from_full(vec![
        PathId::ROOT,
        PathId::try_from_index(1).expect("the intermediate path should fit"),
        remapped_target,
    ]);

    metadata.remap_path_ids(&remap);

    assert_eq!(metadata.doc_fragments.len(), 1);
    assert_eq!(metadata.doc_fragments[0].target, remapped_target);
    assert_eq!(
        metadata.doc_fragments[0].rendered_text,
        "Public documentation."
    );
}
