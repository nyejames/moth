//! Smoke tests for the feature-gated boracle seam.

use super::super::problem::{
    BlockId, BorrowProblem, BorrowProblemParts, CfgBlock, PointId, ProgramPoint,
};

#[test]
fn boracle_feature_marker_is_present() {
    assert_eq!(super::BORACLE_FEATURE_MARKER, "boracle");
}

#[test]
fn boracle_dump_accepts_and_formats_a_validated_problem() {
    let problem = BorrowProblem::new(BorrowProblemParts {
        points: vec![
            ProgramPoint::new(PointId::new(0), BlockId::new(0), 0),
            ProgramPoint::new(PointId::new(1), BlockId::new(0), 1),
        ],
        blocks: vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(1),
            Vec::new(),
        )],
        entry: BlockId::new(0),
        exits: vec![BlockId::new(0)],
        ..BorrowProblemParts::default()
    })
    .expect("minimal problem should validate");

    let dump = super::dump_validated_problem(&problem).expect("dump should validate again");

    assert!(dump.contains("BorrowProblem"));
}
