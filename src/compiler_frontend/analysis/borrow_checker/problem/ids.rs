//! Dense typed identities used by one normalized borrow problem.
//!
//! WHAT: keeps points, places, origins and other problem-local rows in distinct ID spaces.
//! WHY: a dense index is useful inside one function, but reusing an ID type for unrelated facts
//!      makes malformed normalized input easy to construct and hard to diagnose.

macro_rules! define_problem_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub(crate) struct $name(u32);

        impl $name {
            pub(crate) const fn new(raw: u32) -> Self {
                Self(raw)
            }

            pub(crate) const fn raw(self) -> u32 {
                self.0
            }

            pub(crate) const fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

define_problem_id!(BlockId);
define_problem_id!(PointId);
define_problem_id!(EventId);
define_problem_id!(BindingId);
define_problem_id!(PlaceId);
define_problem_id!(ValueOriginId);
define_problem_id!(LoanId);
define_problem_id!(UseId);
define_problem_id!(CallId);
