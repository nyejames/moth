//! Operation names shared by declaration classification and execution refusals.
//!
//! WHAT: the one vocabulary that names an operation the bounded executor cannot represent.
//! WHY: declaration validation and the executor refuse the same operation at different moments. A
//!      single name keeps both refusals readable and stops the renderer falling back to "Const
//!      function this expression cannot be executed".
//! MUST NOT: decide eligibility, build diagnostics or own the reasons a refusal becomes a source
//!           error or an ineligible release candidate.

/// A typed expression form the bounded executor cannot evaluate in any context.
pub(super) const RECEIVER_METHOD_CALL: &str = "receiver method call";
pub(super) const RUNTIME_TEMPLATE: &str = "runtime template";
pub(super) const RUNTIME_TEMPLATE_SLOT: &str = "runtime template slot";

/// A returned value the executable row cannot represent.
pub(super) const CONTENT_REFERENCE: &str = "content reference";
pub(super) const ZERO_SUCCESS_RESULTS: &str = "with zero success results";
pub(super) const MULTIPLE_SUCCESS_RESULTS: &str = "with multiple success results";

/// Executed operations the interpreter reaches beyond its representable value vocabulary.
pub(super) const PROJECTED_WRITE: &str = "projected write";
pub(super) const PROJECTED_READ: &str = "projected read";
pub(super) const PLACE_PROJECTION: &str = "place projection";
pub(super) const ASSIGNMENT_TARGET: &str = "assignment target";
pub(super) const LOOP_BINDING: &str = "loop binding";
pub(super) const COLLECTION_LOOP: &str = "collection loop";
pub(super) const CONDITION: &str = "condition";
pub(super) const LOOP_CONTROL: &str = "loop control";
pub(super) const WITHOUT_FINAL_RETURN: &str = "without a final return";
pub(super) const VALUE_EXPRESSION: &str = "value expression";
pub(super) const TEMPLATE_VALUE: &str = "template value";
pub(super) const REQUIRED_CALL: &str = "required call";

/// Fallible collection operations and the handler contract they need.
pub(super) const COLLECTION_LENGTH: &str = "collection length";
pub(super) const COLLECTION_REMOVAL: &str = "collection removal";
pub(super) const COLLECTION_MUTATION: &str = "collection mutation";
pub(super) const FIXED_COLLECTION_PUSH: &str = "fixed collection push";
pub(super) const COLLECTION_BOUNDS_ERROR: &str = "collection bounds error";
pub(super) const COLLECTION_ERROR_VALUE: &str = "collection error value";
