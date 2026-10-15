//! Phase-owned semantic expression storage.
//!
//! WHAT: stores expression rows and their variable-size payloads in typed buffers, addressed by
//! typed IDs.
//! WHY: fixed edges stay inline in each row, while each payload type gets one owner so freeze
//! remaps every buffer once.

use super::call_argument::{CallAccessMode, CallPassingMode};
use super::expression_rpn::{ResolvedOperation, ResolvedOperationKind};
use super::expression_types::{
    CastHandling, ConstRecordState, FallibleExpressionHandling, ResolvedCastEvidence,
};
use super::failure_facts::ExpressionFailureFacts;
use crate::compiler_frontend::arena::FrontendArenaCapacityEstimate;
use crate::compiler_frontend::ast::const_values::store::ConstValueId;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::templates::runtime_handoff::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeTemplateHandoff,
};
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::builtins::CollectionBuiltinOp;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastFallibility;
use crate::compiler_frontend::builtins::maps::MapBuiltinOp;
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType, InfrastructureErrors};
use crate::compiler_frontend::compiler_messages::{
    AstCapacityResource, CompilerDiagnostic, ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::ExternalFunctionId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::value_mode::ValueMode;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU32, Ordering};

/// Process-unique identity shared by a store and its construction boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OwnerToken(NonZeroU32);
impl OwnerToken {
    fn mint() -> Result<Self, CompilerError> {
        static NEXT_OWNER: AtomicU32 = AtomicU32::new(0);
        let previous = NEXT_OWNER
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| invariant("expression owner tokens exhausted"))?;
        let token = NonZeroU32::new(previous + 1)
            .ok_or_else(|| invariant("zero expression owner token"))?;
        Ok(Self(token))
    }
}

/// Transient construction handle whose durable ID remains four bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OwnedExpr {
    owner: OwnerToken,
    id: ExprId,
}
impl OwnedExpr {
    pub(crate) fn id(self) -> ExprId {
        self.id
    }
}

/// Borrowed durable elements attached to their issuing builder without copying a buffer.
#[derive(Debug)]
pub(crate) struct OwnedExprs<'a, T = ExprId> {
    owner: OwnerToken,
    values: &'a [T],
}
impl<T> Copy for OwnedExprs<'_, T> {}
impl<T> Clone for OwnedExprs<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}
#[derive(Debug)]
pub(crate) enum ExpressionInputs<'a, R, O> {
    Attached(OwnedExprs<'a, R>),
    Owned(Vec<O>),
}
impl<'a, R, O> From<OwnedExprs<'a, R>> for ExpressionInputs<'a, R, O> {
    fn from(values: OwnedExprs<'a, R>) -> Self {
        Self::Attached(values)
    }
}
impl<'a, R, O> From<Vec<O>> for ExpressionInputs<'a, R, O> {
    fn from(values: Vec<O>) -> Self {
        Self::Owned(values)
    }
}

/// One-based ID of an expression row in its owning `ExpressionStore`.
///
/// WHY: the non-zero index keeps `Option<ExprId>` the same size as `ExprId`. An ID is only valid
/// for the store that issued it.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ExprId(NonZeroU32);
impl ExprId {
    pub(crate) fn index(self) -> usize {
        (self.0.get() - 1) as usize
    }
    pub(super) fn at(index: usize) -> Self {
        Self(
            NonZeroU32::new(u32::try_from(index).expect("checked expression index") + 1)
                .expect("one based expression ID"),
        )
    }
}
macro_rules! side_id {
    ($($name:ident),* $(,)?) => { $(
        #[repr(transparent)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub(crate) struct $name(NonZeroU32);
        impl $name {
            pub(super) fn index(self) -> usize { (self.0.get() - 1) as usize }
        }
    )* };
}
side_id!(
    PlaceId,
    CallId,
    CollectionCallId,
    MapCallId,
    NumericOperationId,
    ComparisonOperationId,
    RangeOperationId,
    CastFactId,
    HandlingId,
    ValueBlockId,
    RuntimeTemplateId,
    RuntimeSlotApplicationId,
    TemplateFactId,
    FailureFactId,
    OccurrenceFactId
);
/// Side-table IDs that freeze rebuilds from dense side-table indices. `TemplateFactId` is
/// excluded because freeze never rebuilds it.
macro_rules! side_id_from_index {
    ($($name:ident),* $(,)?) => { $(
        impl $name {
            pub(super) fn at(index: usize) -> Self { Self(one_based_side_index(index)) }
        }
    )* };
}
side_id_from_index!(
    PlaceId,
    CallId,
    CollectionCallId,
    MapCallId,
    NumericOperationId,
    ComparisonOperationId,
    RangeOperationId,
    CastFactId,
    HandlingId,
    ValueBlockId,
    RuntimeTemplateId,
    RuntimeSlotApplicationId,
    FailureFactId,
    OccurrenceFactId
);
fn one_based_side_index(index: usize) -> NonZeroU32 {
    NonZeroU32::new(u32::try_from(index).expect("checked side index") + 1)
        .expect("one based side ID")
}
macro_rules! typed_range {
    ($($name:ident),* $(,)?) => { $(
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub(crate) struct $name { pub(super) start: u32, pub(super) len: u32 }
        impl $name {
            pub(super) fn bounds(self, size: usize) -> Result<std::ops::Range<usize>, CompilerError> {
                let end = self.start.checked_add(self.len).ok_or_else(|| invariant("expression range end overflow"))?;
                if end as usize > size { return Err(invariant("expression range outside owner")); }
                Ok(self.start as usize..end as usize)
            }
        }
    )* };
}
typed_range!(
    TypeRange,
    ExpressionRange,
    ArgumentRange,
    ConstructorFieldRange,
    MapEntryRange,
    PlaceProjectionRange
);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SuccessShape {
    Zero,
    One(TypeId),
    Multiple(TypeRange),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExpressionClassification {
    pub const_record_state: ConstRecordState,
    pub contains_regular_division: bool,
}
#[derive(Clone, Debug)]
pub(crate) struct OccurrenceFacts {
    pub provenance: SyntheticInterfaceProvenance,
    pub diagnostic_spelling: Option<StringId>,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum LogicalOperator {
    And,
    Or,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum CoercionOperation {
    ToType(TypeId),
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum ResolvedCallableTarget {
    Function(PathId),
    Host(ExternalFunctionId),
}
#[derive(Clone, Debug)]
pub(crate) struct ResolvedArgument<E = ExprId> {
    pub value: E,
    pub target_param: Option<StringId>,
    pub parameter_slot: Option<u32>,
    pub access_mode: CallAccessMode,
    pub passing_mode: CallPassingMode,
    pub span: Option<SourceSpan>,
    pub target_span: Option<SourceSpan>,
    pub marker_span: Option<SourceSpan>,
    pub following_separator_span: Option<SourceSpan>,
}
#[derive(Clone, Debug)]
pub(crate) struct ConstructorField<E = ExprId> {
    pub name: PathId,
    pub slot: u32,
    pub value: E,
}
#[derive(Clone, Debug)]
pub(crate) struct MapEntry<E = ExprId> {
    pub key: E,
    pub value: E,
}
#[derive(Clone, Debug)]
pub(crate) struct ResolvedCall {
    pub target: ResolvedCallableTarget,
    pub arguments: ArgumentRange,
    pub handling: Option<HandlingId>,
    pub requires_external_float_validation: bool,
    pub error_type: Option<TypeId>,
    pub span: Option<SourceSpan>,
}
#[derive(Clone, Debug)]
pub(crate) struct CollectionCall {
    pub operation: CollectionBuiltinOp,
    pub arguments: ArgumentRange,
    pub span: Option<SourceSpan>,
}
#[derive(Clone, Debug)]
pub(crate) struct MapCall {
    pub operation: MapBuiltinOp,
    pub receiver_requires_mutable: bool,
    pub arguments: ArgumentRange,
    pub span: Option<SourceSpan>,
}
#[derive(Clone, Debug)]
pub(crate) struct CastFacts {
    pub source_type_id: TypeId,
    pub target_type_id: TypeId,
    pub requires_optional_wrap_after_cast: bool,
    pub fallibility: BuiltinCastFallibility,
    pub evidence: ResolvedCastEvidence,
    pub handling: CastHandling,
    pub span: Option<SourceSpan>,
}
#[derive(Clone, Debug)]
pub(crate) struct HandlingRecord {
    pub handling: FallibleExpressionHandling,
    pub propagation_span: Option<SourceSpan>,
    pub error_type: Option<TypeId>,
}
#[derive(Clone, Debug)]
pub(crate) enum PlaceProjection<E = ExprId> {
    Field {
        field: StringId,
        result_type: TypeId,
    },
    Index {
        index: E,
        result_type: TypeId,
    },
}
#[derive(Clone, Debug)]
pub(crate) struct PlaceRecord {
    pub root: PathId,
    pub root_type_id: TypeId,
    pub projections: PlaceProjectionRange,
    pub type_id: TypeId,
    pub value_mode: ValueMode,
    pub span: Option<SourceSpan>,
}
/// Operation carried by one expression row.
///
/// WHAT: fixed-edge operations hold child expressions inline as `ExprId`s. Variable-size
/// operations hold ranges into typed side buffers, such as arguments, fields and map entries.
/// WHY: each row keeps a fixed size, so the row buffer never owns nested payloads.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ExpressionOp<E = ExprId> {
    Constant(ConstValueId),
    Reference(PathId),
    Copy(PlaceId),
    FieldAccess {
        base: E,
        field: StringId,
    },
    NumericUnary {
        operand: E,
        facts: NumericOperationId,
    },
    NumericBinary {
        left: E,
        right: E,
        facts: NumericOperationId,
    },
    LogicalNot {
        operand: E,
    },
    LogicalBinary {
        left: E,
        right: E,
        operation: LogicalOperator,
    },
    Compare {
        left: E,
        right: E,
        facts: ComparisonOperationId,
    },
    Range {
        start: E,
        end: E,
        facts: RangeOperationId,
    },
    Call(CallId),
    MethodCall {
        receiver: E,
        call: CallId,
    },
    CollectionBuiltin {
        receiver: E,
        call: CollectionCallId,
    },
    MapBuiltin {
        receiver: E,
        call: MapCallId,
    },
    Cast {
        source: E,
        facts: CastFactId,
    },
    HandleFallible {
        value: E,
        handling: HandlingId,
    },
    PropagateOption {
        value: E,
    },
    Coerce {
        value: E,
        operation: CoercionOperation,
    },
    Collection(ExpressionRange),
    Map(MapEntryRange),
    Struct {
        fields: ConstructorFieldRange,
    },
    Choice {
        tag: u32,
        fields: ConstructorFieldRange,
    },
    ValueBlock(ValueBlockId),
    RuntimeTemplate(RuntimeTemplateId),
    RuntimeSlotApplication(RuntimeSlotApplicationId),
    Template(TemplateFactId),
}
impl<E> ExpressionOp<E> {
    #[inline]
    pub(crate) fn map_edges<F>(self, mut map: impl FnMut(E) -> F) -> ExpressionOp<F> {
        match self {
            Self::Constant(value) => ExpressionOp::Constant(value),
            Self::Reference(path) => ExpressionOp::Reference(path),
            Self::Copy(place) => ExpressionOp::Copy(place),
            Self::FieldAccess { base, field } => ExpressionOp::FieldAccess {
                base: map(base),
                field,
            },
            Self::NumericUnary { operand, facts } => ExpressionOp::NumericUnary {
                operand: map(operand),
                facts,
            },
            Self::NumericBinary { left, right, facts } => ExpressionOp::NumericBinary {
                left: map(left),
                right: map(right),
                facts,
            },
            Self::LogicalNot { operand } => ExpressionOp::LogicalNot {
                operand: map(operand),
            },
            Self::LogicalBinary {
                left,
                right,
                operation,
            } => ExpressionOp::LogicalBinary {
                left: map(left),
                right: map(right),
                operation,
            },
            Self::Compare { left, right, facts } => ExpressionOp::Compare {
                left: map(left),
                right: map(right),
                facts,
            },
            Self::Range { start, end, facts } => ExpressionOp::Range {
                start: map(start),
                end: map(end),
                facts,
            },
            Self::Call(call) => ExpressionOp::Call(call),
            Self::MethodCall { receiver, call } => ExpressionOp::MethodCall {
                receiver: map(receiver),
                call,
            },
            Self::CollectionBuiltin { receiver, call } => ExpressionOp::CollectionBuiltin {
                receiver: map(receiver),
                call,
            },
            Self::MapBuiltin { receiver, call } => ExpressionOp::MapBuiltin {
                receiver: map(receiver),
                call,
            },
            Self::Cast { source, facts } => ExpressionOp::Cast {
                source: map(source),
                facts,
            },
            Self::HandleFallible { value, handling } => ExpressionOp::HandleFallible {
                value: map(value),
                handling,
            },
            Self::PropagateOption { value } => ExpressionOp::PropagateOption { value: map(value) },
            Self::Coerce { value, operation } => ExpressionOp::Coerce {
                value: map(value),
                operation,
            },
            Self::Collection(range) => ExpressionOp::Collection(range),
            Self::Map(range) => ExpressionOp::Map(range),
            Self::Struct { fields } => ExpressionOp::Struct { fields },
            Self::Choice { tag, fields } => ExpressionOp::Choice { tag, fields },
            Self::ValueBlock(block) => ExpressionOp::ValueBlock(block),
            Self::RuntimeTemplate(template) => ExpressionOp::RuntimeTemplate(template),
            Self::RuntimeSlotApplication(slot) => ExpressionOp::RuntimeSlotApplication(slot),
            Self::Template(template) => ExpressionOp::Template(template),
        }
    }
}
impl<E: Copy> ExpressionOp<E> {
    /// Visits inline construction edges without resolving durable side records.
    pub(crate) fn visit_edges(self, visit: &mut dyn FnMut(E)) {
        match self {
            Self::FieldAccess { base, .. } => visit(base),
            Self::NumericUnary { operand, .. } | Self::LogicalNot { operand } => visit(operand),
            Self::NumericBinary { left, right, .. }
            | Self::LogicalBinary { left, right, .. }
            | Self::Compare { left, right, .. } => {
                visit(left);
                visit(right);
            }
            Self::Range { start, end, .. } => {
                visit(start);
                visit(end);
            }
            Self::MethodCall { receiver, .. }
            | Self::CollectionBuiltin { receiver, .. }
            | Self::MapBuiltin { receiver, .. } => visit(receiver),
            Self::Cast { source, .. } => visit(source),
            Self::HandleFallible { value, .. }
            | Self::PropagateOption { value }
            | Self::Coerce { value, .. } => visit(value),
            Self::Constant(_)
            | Self::Reference(_)
            | Self::Copy(_)
            | Self::Call(_)
            | Self::Collection(_)
            | Self::Map(_)
            | Self::Struct { .. }
            | Self::Choice { .. }
            | Self::ValueBlock(_)
            | Self::RuntimeTemplate(_)
            | Self::RuntimeSlotApplication(_)
            | Self::Template(_) => {}
        }
    }
}
/// One stored expression: its operation plus the fixed metadata every expression carries.
///
/// Optional failure and occurrence facts are side-buffer IDs, so every row keeps one layout.
#[derive(Debug)]
pub(crate) struct ExpressionRow {
    pub(super) operation: ExpressionOp,
    pub(super) success: SuccessShape,
    pub(super) error_type: Option<TypeId>,
    pub(super) span: Option<SourceSpan>,
    pub(super) value_mode: ValueMode,
    pub(super) classification: ExpressionClassification,
    pub(super) failure: Option<FailureFactId>,
    pub(super) occurrence: Option<OccurrenceFactId>,
}
/// Resolved fields for one expression row, ready for `AstExpressionBuilder::append_completed`.
///
/// Optional failure and occurrence facts are moved into their side buffers and referenced by ID.
pub(crate) struct CompletedExpressionInput {
    pub operation: ExpressionOp<OwnedExpr>,
    pub success: SuccessShape,
    pub error_type: Option<TypeId>,
    pub span: Option<SourceSpan>,
    pub value_mode: ValueMode,
    pub classification: ExpressionClassification,
    pub failure: Option<ExpressionFailureFacts>,
    pub occurrence: Option<OccurrenceFacts>,
}
#[derive(Debug)]
pub(crate) enum ExpressionConstructionFailure {
    Diagnostic(CompilerDiagnostic),
    Invariant(Box<InfrastructureErrors>),
}
impl From<CompilerError> for ExpressionConstructionFailure {
    fn from(e: CompilerError) -> Self {
        Self::Invariant(e.into())
    }
}
pub(super) fn invariant(message: &str) -> CompilerError {
    CompilerError::new(message, None, ErrorType::Compiler)
}
pub(super) fn checked_extent(
    start: usize,
    len: usize,
    limit: u32,
    span: Option<SourceSpan>,
    rows: bool,
) -> Result<(u32, u32), ExpressionConstructionFailure> {
    let capacity = || {
        ExpressionConstructionFailure::Diagnostic(CompilerDiagnostic::compiler_capacity_exceeded(
            if rows {
                AstCapacityResource::ExpressionRows
            } else {
                AstCapacityResource::ExpressionSideEntries
            },
            span,
        ))
    };
    let start = u32::try_from(start).map_err(|_| capacity())?;
    let len = u32::try_from(len).map_err(|_| capacity())?;
    let end = start.checked_add(len).ok_or_else(capacity)?;
    if end > limit {
        return Err(capacity());
    }
    Ok((start, len))
}

macro_rules! storage {
    ($( $field:ident : $ty:ty ),* $(,)?) => {
        /// Typed buffers backing one phase's expressions. Each field stores a single element type.
        #[derive(Debug)]
        pub(crate) struct ExpressionStore {
            pub(super) owner: OwnerToken,
            $(pub(super) $field: Vec<$ty>,)*
        }
        impl ExpressionStore {
            pub(crate) fn new() -> Result<Self, CompilerError> {
                Ok(Self { owner: OwnerToken::mint()?, $($field: Vec::new(),)* })
            }
            pub(super) fn freeze(mut self) -> Result<Self, CompilerError> {
                self.owner = OwnerToken::mint()?;
                $(self.$field.shrink_to_fit();)*
                Ok(self)
            }
            #[cfg(test)]
            pub(crate) fn into_builder(mut self) -> Result<AstExpressionBuilder, CompilerError> {
                self.owner = OwnerToken::mint()?;
                Ok(AstExpressionBuilder {
                    storage: self,
                    candidate_serial: 0,
                    limit: u32::MAX,
                })
            }
            #[cfg(any(test, feature = "benchmark_counters"))]
            pub(crate) fn storage_statistics(&self) -> ExpressionStorageStatistics {
                let mut stats = ExpressionStorageStatistics::default();
                $(
                    stats.side_entries += self.$field.len();
                    stats.side_capacity += self.$field.capacity();
                    stats.side_bytes += self.$field.len() * std::mem::size_of::<$ty>();
                    stats.side_capacity_bytes +=
                        self.$field.capacity() * std::mem::size_of::<$ty>();
                )*
                stats.rows = self.rows.len();
                stats.row_capacity = self.rows.capacity();
                stats.row_bytes = stats.rows * std::mem::size_of::<ExpressionRow>();
                stats.row_capacity_bytes =
                    stats.row_capacity * std::mem::size_of::<ExpressionRow>();
                stats.side_entries -= stats.rows;
                stats.side_capacity -= stats.row_capacity;
                stats.side_bytes -= stats.row_bytes;
                stats.side_capacity_bytes -= stats.row_capacity_bytes;
                stats
            }
        }
    };
}
storage!(
    rows: ExpressionRow,
    children: ExprId,
    result_types: TypeId,
    arguments: ResolvedArgument,
    constructor_fields: ConstructorField,
    map_entries: MapEntry,
    places: PlaceRecord,
    projections: PlaceProjection,
    calls: ResolvedCall,
    collection_calls: CollectionCall,
    map_calls: MapCall,
    numeric: ResolvedOperation,
    comparisons: ResolvedOperation,
    ranges: ResolvedOperation,
    casts: CastFacts,
    handling: HandlingRecord,
    value_blocks: ValueBlock,
    runtime_templates: OwnedRuntimeTemplateHandoff,
    runtime_slots: OwnedRuntimeSlotApplicationHandoff,
    templates: Template,
    failures: ExpressionFailureFacts,
    occurrences: OccurrenceFacts,
);
/// Append-only builder for the expression store of one AST phase.
///
/// WHAT: owns the `ExpressionStore` while expressions are built. Each append is checked against
/// the capacity limit before it writes a row.
/// WHY: the borrow checker keeps views from outliving appends, and `freeze` consumes the builder
/// at the last-reader boundary.
///
/// Failure contract: an exhausted limit fails the append with a `Diagnostic` from
/// `CompilerDiagnostic::compiler_capacity_exceeded`. That is an ordinary construction failure.
/// No probe or fallback may turn it into a rejected candidate or a runtime classification.
/// Generated-constant materialisation and constant import report it as an internal compiler
/// error instead, because they rebuild values that already fitted once. Compound constructors
/// append side entries before their row, so a failure part way through can leave side entries
/// that no row references. The builder does not roll them back. `freeze` drops unreachable
/// entries when the store is frozen. Callers must not treat a capacity failure as recoverable by
/// appending more.
#[derive(Debug)]
pub(crate) struct AstExpressionBuilder {
    pub(super) storage: ExpressionStore,
    pub(super) candidate_serial: u64,
    pub(super) limit: u32,
}
impl AstExpressionBuilder {
    pub(crate) fn new() -> Result<Self, CompilerError> {
        Ok(Self {
            storage: ExpressionStore::new()?,
            candidate_serial: 0,
            limit: u32::MAX,
        })
    }
    /// Reserves expression rows only. Side tables grow on demand: reserving them from the estimate
    /// left most of their capacity unused on measured modules.
    pub(crate) fn with_capacity_estimate(
        estimate: FrontendArenaCapacityEstimate,
    ) -> Result<Self, CompilerError> {
        let mut builder = Self::new()?;
        builder.storage.rows.reserve(estimate.expressions);
        Ok(builder)
    }
    pub(crate) fn len(&self) -> usize {
        self.storage.rows.len()
    }
    /// Borrows the row at `id` as a view, or returns an invariant error if `id` is out of range.
    pub(crate) fn view(&self, id: ExprId) -> Result<ExpressionView<'_>, CompilerError> {
        self.storage.view(id)
    }
    /// `own` and `own_slice` attach durable IDs at the issuing builder's construction boundary.
    /// This is the only raw attachment family and cannot detect IDs re-entered through a wrong builder.
    pub(crate) fn own(&self, id: ExprId) -> OwnedExpr {
        OwnedExpr {
            owner: self.storage.owner,
            id,
        }
    }
    pub(crate) fn own_slice<'a, T>(&self, values: &'a [T]) -> OwnedExprs<'a, T> {
        OwnedExprs {
            owner: self.storage.owner,
            values,
        }
    }
    pub(crate) fn check_owned_slice<'a, T>(
        &self,
        values: OwnedExprs<'a, T>,
    ) -> Result<&'a [T], CompilerError> {
        self.storage.check_owner_token(values.owner)?;
        Ok(values.values)
    }
    pub(crate) fn check_owner(&self, expression: OwnedExpr) -> Result<(), CompilerError> {
        self.storage.check_owner(expression)
    }
    pub(crate) fn check_owned(&self, expression: OwnedExpr) -> Result<ExprId, CompilerError> {
        self.storage.check_owned(expression)
    }
    pub(crate) fn view_owned(
        &self,
        expression: OwnedExpr,
    ) -> Result<ExpressionView<'_>, CompilerError> {
        self.storage.view_owned(expression)
    }
    #[inline]
    pub(crate) fn check_operation_owned(
        &self,
        operation: ExpressionOp<OwnedExpr>,
    ) -> Result<ExpressionOp, CompilerError> {
        let operation = self.check_operation_owner(operation)?;
        let mut result = Ok(());
        operation.visit_edges(&mut |edge| {
            if result.is_ok() {
                result = self.view(edge).map(|_| ());
            }
        });
        result?;
        Ok(operation)
    }
    /// Strip transient tokens only after checking every inline owner. No row or side entry is
    /// read here: the caller's existing validation owns those checks.
    #[inline]
    fn check_operation_owner(
        &self,
        operation: ExpressionOp<OwnedExpr>,
    ) -> Result<ExpressionOp, CompilerError> {
        let mut result = Ok(());
        let operation = operation.map_edges(|edge| {
            if result.is_ok() {
                result = self.check_owner(edge);
            }
            edge.id()
        });
        result?;
        Ok(operation)
    }
    /// Borrows the growing store, so builder-time readers share the frozen store's view API.
    pub(crate) fn store(&self) -> &ExpressionStore {
        &self.storage
    }
    #[cfg(feature = "benchmark_counters")]
    pub(crate) fn storage_statistics(&self) -> ExpressionStorageStatistics {
        self.storage.storage_statistics()
    }
    /// Appends one completed row and moves its optional facts into the side buffers first.
    ///
    /// Capacity checks run before any side buffer grows, so a limit overflow is reported before
    /// a row is written.
    pub(crate) fn append_completed(
        &mut self,
        input: CompletedExpressionInput,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        // Independently supplied or mutated inputs still check every owner before accessing
        // storage. validate_operation then checks inline and side edges once, before mutation.
        let operation = self.check_operation_owner(input.operation)?;
        self.storage.validate_operation(operation, input.success)?;
        checked_extent(self.len(), 1, self.limit, input.span, true)?;
        checked_extent(
            self.storage.failures.len(),
            usize::from(input.failure.is_some()),
            self.limit,
            input.span,
            false,
        )?;
        checked_extent(
            self.storage.occurrences.len(),
            usize::from(input.occurrence.is_some()),
            self.limit,
            input.span,
            false,
        )?;
        let failure = input
            .failure
            .map(
                |fact| -> Result<FailureFactId, ExpressionConstructionFailure> {
                    let id = FailureFactId(
                        NonZeroU32::new(self.storage.failures.len() as u32 + 1).ok_or_else(
                            || {
                                ExpressionConstructionFailure::Invariant(
                                    invariant("side ID exhausted").into(),
                                )
                            },
                        )?,
                    );
                    self.storage.failures.push(fact);
                    Ok(id)
                },
            )
            .transpose()?;
        let occurrence = input
            .occurrence
            .map(
                |fact| -> Result<OccurrenceFactId, ExpressionConstructionFailure> {
                    let id = OccurrenceFactId(
                        NonZeroU32::new(self.storage.occurrences.len() as u32 + 1).ok_or_else(
                            || {
                                ExpressionConstructionFailure::Invariant(
                                    invariant("side ID exhausted").into(),
                                )
                            },
                        )?,
                    );
                    self.storage.occurrences.push(fact);
                    Ok(id)
                },
            )
            .transpose()?;
        let id = ExprId::at(self.len());
        self.storage.rows.push(ExpressionRow {
            operation,
            success: input.success,
            error_type: input.error_type,
            span: input.span,
            value_mode: input.value_mode,
            classification: input.classification,
            failure,
            occurrence,
        });
        Ok(self.own(id))
    }
}
impl ExpressionStore {
    /// Freeze's debug parity check and the freeze tests are the only readers of the row count.
    #[cfg(any(debug_assertions, test))]
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }
}
macro_rules! append_range {
    ($method:ident, $field:ident, $stored:ty, $ty:ty, $range:ident, $visit:expr, $visit_raw:expr, $raw:expr) => {
        impl AstExpressionBuilder {
            pub(crate) fn $method<'a>(
                &mut self,
                values: impl Into<ExpressionInputs<'a, $stored, $ty>>,
                span: Option<SourceSpan>,
            ) -> Result<$range, ExpressionConstructionFailure> {
                let values = values.into();
                let len = match &values {
                    ExpressionInputs::Attached(values) => {
                        self.check_owned_slice(*values)?;
                        values.values.len()
                    }
                    ExpressionInputs::Owned(values) => {
                        let mut result = Ok(());
                        for value in values {
                            ($visit)(value, &mut |edge| {
                                if result.is_ok() {
                                    result = self.check_owner(edge);
                                }
                            });
                        }
                        result?;
                        values.len()
                    }
                };
                let mut result = Ok(());
                match &values {
                    ExpressionInputs::Attached(values) => {
                        for value in values.values {
                            ($visit_raw)(value, &mut |edge| {
                                if result.is_ok() {
                                    result = self.view(edge).map(|_| ());
                                }
                            });
                        }
                    }
                    ExpressionInputs::Owned(values) => {
                        for value in values {
                            ($visit)(value, &mut |edge| {
                                if result.is_ok() {
                                    result = self.check_owned(edge).map(|_| ());
                                }
                            });
                        }
                    }
                }
                result?;
                let (start, len) =
                    checked_extent(self.storage.$field.len(), len, self.limit, span, false)?;
                match values {
                    ExpressionInputs::Attached(values) => {
                        self.storage.$field.extend(values.values.iter().cloned())
                    }
                    ExpressionInputs::Owned(values) => {
                        self.storage.$field.extend(values.into_iter().map($raw))
                    }
                }
                Ok($range { start, len })
            }
        }
    };
}
impl AstExpressionBuilder {
    pub(crate) fn append_types(
        &mut self,
        values: Vec<TypeId>,
        span: Option<SourceSpan>,
    ) -> Result<TypeRange, ExpressionConstructionFailure> {
        let (start, len) = checked_extent(
            self.storage.result_types.len(),
            values.len(),
            self.limit,
            span,
            false,
        )?;
        self.storage.result_types.extend(values);
        Ok(TypeRange { start, len })
    }
}
append_range!(
    append_children,
    children,
    ExprId,
    OwnedExpr,
    ExpressionRange,
    |value: &OwnedExpr, visit: &mut dyn FnMut(OwnedExpr)| visit(*value),
    |value: &ExprId, visit: &mut dyn FnMut(ExprId)| visit(*value),
    OwnedExpr::id
);
append_range!(
    append_arguments,
    arguments,
    ResolvedArgument,
    ResolvedArgument<OwnedExpr>,
    ArgumentRange,
    |value: &ResolvedArgument<OwnedExpr>, visit: &mut dyn FnMut(OwnedExpr)| visit(value.value),
    |value: &ResolvedArgument, visit: &mut dyn FnMut(ExprId)| visit(value.value),
    |value: ResolvedArgument<OwnedExpr>| ResolvedArgument {
        value: value.value.id(),
        target_param: value.target_param,
        parameter_slot: value.parameter_slot,
        access_mode: value.access_mode,
        passing_mode: value.passing_mode,
        span: value.span,
        target_span: value.target_span,
        marker_span: value.marker_span,
        following_separator_span: value.following_separator_span,
    }
);
append_range!(
    append_constructor_fields,
    constructor_fields,
    ConstructorField,
    ConstructorField<OwnedExpr>,
    ConstructorFieldRange,
    |value: &ConstructorField<OwnedExpr>, visit: &mut dyn FnMut(OwnedExpr)| visit(value.value),
    |value: &ConstructorField, visit: &mut dyn FnMut(ExprId)| visit(value.value),
    |value: ConstructorField<OwnedExpr>| ConstructorField {
        name: value.name,
        slot: value.slot,
        value: value.value.id(),
    }
);
append_range!(
    append_map_entries,
    map_entries,
    MapEntry,
    MapEntry<OwnedExpr>,
    MapEntryRange,
    |value: &MapEntry<OwnedExpr>, visit: &mut dyn FnMut(OwnedExpr)| {
        visit(value.key);
        visit(value.value);
    },
    |value: &MapEntry, visit: &mut dyn FnMut(ExprId)| {
        visit(value.key);
        visit(value.value);
    },
    |value: MapEntry<OwnedExpr>| MapEntry {
        key: value.key.id(),
        value: value.value.id()
    }
);
append_range!(
    append_place_projections,
    projections,
    PlaceProjection,
    PlaceProjection<OwnedExpr>,
    PlaceProjectionRange,
    |value: &PlaceProjection<OwnedExpr>, visit: &mut dyn FnMut(OwnedExpr)| {
        if let PlaceProjection::Index { index, .. } = value {
            visit(*index);
        }
    },
    |value: &PlaceProjection, visit: &mut dyn FnMut(ExprId)| {
        if let PlaceProjection::Index { index, .. } = value {
            visit(*index);
        }
    },
    |value: PlaceProjection<OwnedExpr>| match value {
        PlaceProjection::Field { field, result_type } =>
            PlaceProjection::Field { field, result_type },
        PlaceProjection::Index { index, result_type } => PlaceProjection::Index {
            index: index.id(),
            result_type
        },
    }
);
impl AstExpressionBuilder {
    /// Durable payloads re-enter through `own`, so their original owner remains a caller invariant.
    fn validate_external_edges(
        &self,
        visit: impl FnOnce(&mut dyn FnMut(ExprId)),
    ) -> Result<(), CompilerError> {
        let mut result = Ok(());
        visit(&mut |id| {
            if result.is_ok() {
                result = self.check_owned(self.own(id)).map(|_| ());
            }
        });
        result
    }
}
macro_rules! append_side {
    ($method:ident, $field:ident, $ty:ty, $id:ident, $validate:expr $(, edges $visit:expr)?) => {
        impl AstExpressionBuilder {
            pub(crate) fn $method(
                &mut self,
                value: $ty,
                span: Option<SourceSpan>,
            ) -> Result<$id, ExpressionConstructionFailure> {
                $(self.validate_external_edges(|visit| ($visit)(&value, visit))?;)?
                ($validate)(&self.storage, &value)?;
                let (start, _) =
                    checked_extent(self.storage.$field.len(), 1, self.limit, span, false)?;
                let id = $id(NonZeroU32::new(start + 1).unwrap());
                self.storage.$field.push(value);
                Ok(id)
            }
        }
    };
}
append_side!(
    append_place,
    places,
    PlaceRecord,
    PlaceId,
    |o: &ExpressionStore, v: &PlaceRecord| v.projections.bounds(o.projections.len()).map(|_| ())
);
append_side!(
    append_call,
    calls,
    ResolvedCall,
    CallId,
    |o: &ExpressionStore, v: &ResolvedCall| {
        v.arguments.bounds(o.arguments.len())?;
        if let Some(id) = v.handling {
            o.handling
                .get(id.index())
                .ok_or_else(|| invariant("handling ID outside owner"))?;
        }
        Ok::<_, CompilerError>(())
    }
);
append_side!(
    append_collection_call,
    collection_calls,
    CollectionCall,
    CollectionCallId,
    |o: &ExpressionStore, v: &CollectionCall| v.arguments.bounds(o.arguments.len()).map(|_| ())
);
append_side!(
    append_map_call,
    map_calls,
    MapCall,
    MapCallId,
    |o: &ExpressionStore, v: &MapCall| v.arguments.bounds(o.arguments.len()).map(|_| ())
);
append_side!(
    append_numeric_operation,
    numeric,
    ResolvedOperation,
    NumericOperationId,
    |_: &ExpressionStore, v: &ResolvedOperation| if matches!(
        v.kind,
        ResolvedOperationKind::Numeric(_)
    ) {
        Ok(())
    } else {
        Err(invariant("non-numeric numeric operation facts"))
    }
);
append_side!(
    append_comparison_operation,
    comparisons,
    ResolvedOperation,
    ComparisonOperationId,
    |_: &ExpressionStore, v: &ResolvedOperation| if matches!(
        v.kind,
        ResolvedOperationKind::Comparison(_) | ResolvedOperationKind::NonNumeric
    ) {
        Ok(())
    } else {
        Err(invariant("invalid comparison operation facts"))
    }
);
append_side!(
    append_range_operation,
    ranges,
    ResolvedOperation,
    RangeOperationId,
    |_: &ExpressionStore, v: &ResolvedOperation| if matches!(
        v.kind,
        ResolvedOperationKind::Range { .. }
    ) {
        Ok(())
    } else {
        Err(invariant("non-range range operation facts"))
    }
);
append_side!(
    append_cast,
    casts,
    CastFacts,
    CastFactId,
    |_: &ExpressionStore, _: &CastFacts| Ok::<_, CompilerError>(())
);
append_side!(
    append_handling,
    handling,
    HandlingRecord,
    HandlingId,
    |_: &ExpressionStore, _: &HandlingRecord| Ok::<_, CompilerError>(())
);
append_side!(
    append_template,
    templates,
    Template,
    TemplateFactId,
    |_: &ExpressionStore, _: &Template| Ok::<_, CompilerError>(())
);
append_side!(
    append_value_block,
    value_blocks,
    ValueBlock,
    ValueBlockId,
    |o: &ExpressionStore, v: &ValueBlock| o.validate_place_edges(|visit| v.visit_place_ids(visit)),
    edges |value: &ValueBlock, visit: &mut dyn FnMut(ExprId)| value.visit_expression_ids(visit)
);
append_side!(
    append_runtime_template,
    runtime_templates,
    OwnedRuntimeTemplateHandoff,
    RuntimeTemplateId,
    |_: &ExpressionStore, _: &OwnedRuntimeTemplateHandoff| Ok::<_, CompilerError>(()),
    edges |value: &OwnedRuntimeTemplateHandoff, visit: &mut dyn FnMut(ExprId)| value.visit_expression_ids(visit)
);
append_side!(
    append_runtime_slot_application,
    runtime_slots,
    OwnedRuntimeSlotApplicationHandoff,
    RuntimeSlotApplicationId,
    |_: &ExpressionStore, _: &OwnedRuntimeSlotApplicationHandoff| Ok::<_, CompilerError>(()),
    edges |value: &OwnedRuntimeSlotApplicationHandoff, visit: &mut dyn FnMut(ExprId)| value.visit_expression_ids(visit)
);

/// Read-only handle to one stored row, borrowed from the store that owns it.
#[derive(Clone, Copy)]
pub(crate) struct ExpressionView<'a> {
    pub(super) owner: &'a ExpressionStore,
    pub(super) id: ExprId,
}
impl ExpressionStore {
    pub(crate) fn owner_token(&self) -> OwnerToken {
        self.owner
    }
    /// Borrows the row at `id` as a view, or returns an invariant error if `id` is out of range.
    pub(crate) fn view(&self, id: ExprId) -> Result<ExpressionView<'_>, CompilerError> {
        self.rows
            .get(id.index())
            .ok_or_else(|| invariant("expression ID outside owner"))?;
        Ok(ExpressionView { owner: self, id })
    }
    pub(super) fn check_owner_token(&self, owner: OwnerToken) -> Result<(), CompilerError> {
        if owner != self.owner {
            return Err(invariant("expression handle from another owner"));
        }
        Ok(())
    }
    fn check_owner(&self, expression: OwnedExpr) -> Result<(), CompilerError> {
        self.check_owner_token(expression.owner)
    }
    fn check_owned(&self, expression: OwnedExpr) -> Result<ExprId, CompilerError> {
        self.check_owner(expression)?;
        self.view(expression.id)?;
        Ok(expression.id)
    }
    pub(crate) fn view_owned(
        &self,
        expression: OwnedExpr,
    ) -> Result<ExpressionView<'_>, CompilerError> {
        self.check_owner(expression)?;
        self.view(expression.id)
    }
    fn validate_place_edges(
        &self,
        visit: impl FnOnce(&mut dyn FnMut(PlaceId)),
    ) -> Result<(), CompilerError> {
        let mut result = Ok(());
        visit(&mut |id| {
            if result.is_ok() {
                result = self.visit_place_edges(id, &mut |_| ());
            }
        });
        result
    }
    fn visit_place_edges(
        &self,
        id: PlaceId,
        visit: &mut dyn FnMut(ExprId),
    ) -> Result<(), CompilerError> {
        let place = self
            .places
            .get(id.index())
            .ok_or_else(|| invariant("place ID outside owner"))?;
        for projection in &self.projections[place.projections.bounds(self.projections.len())?] {
            if let PlaceProjection::Index { index, .. } = projection {
                self.view(*index)?;
                visit(*index);
            }
        }
        Ok(())
    }
    pub(super) fn validate_operation(
        &self,
        operation: ExpressionOp,
        success: SuccessShape,
    ) -> Result<(), CompilerError> {
        if let SuccessShape::Multiple(range) = success {
            if range.len < 2 {
                return Err(invariant(
                    "Multiple success shape requires at least two slots",
                ));
            }
            range.bounds(self.result_types.len())?;
        }
        let mut error = None;
        self.visit_operation_edges(operation, &mut |id| {
            if self.view(id).is_err() {
                error = Some(invariant("expression child outside owner"));
            }
        })?;
        if let Some(error) = error {
            return Err(error);
        }
        let fact = match operation {
            ExpressionOp::NumericUnary { facts, .. }
            | ExpressionOp::NumericBinary { facts, .. } => Some(&self.numeric[facts.index()]),
            ExpressionOp::Compare { facts, .. } => Some(&self.comparisons[facts.index()]),
            ExpressionOp::Range { facts, .. } => Some(&self.ranges[facts.index()]),
            _ => None,
        };
        if let Some(fact) = fact {
            if success != SuccessShape::One(fact.result_type) {
                return Err(invariant("resolved operation result disagrees with row"));
            }
            let (left, right) = match operation {
                ExpressionOp::NumericUnary { operand, .. } => (operand, None),
                ExpressionOp::NumericBinary { left, right, .. }
                | ExpressionOp::Compare { left, right, .. } => (left, Some(right)),
                ExpressionOp::Range { start, end, .. } => (start, Some(end)),
                _ => unreachable!(),
            };
            if self.rows[left.index()].success != SuccessShape::One(fact.left_type)
                || right.map(|id| self.rows[id.index()].success)
                    != fact.right_type.map(SuccessShape::One)
            {
                return Err(invariant(
                    "resolved operation operand types disagree with edges",
                ));
            }
        }
        Ok(())
    }
    pub(super) fn visit_operation_edges(
        &self,
        operation: ExpressionOp,
        visit: &mut dyn FnMut(ExprId),
    ) -> Result<(), CompilerError> {
        macro_rules! get {
            ($field:ident,$id:expr) => {
                self.$field
                    .get($id.index())
                    .ok_or_else(|| invariant("expression side ID outside owner"))?
            };
        }
        match operation {
            ExpressionOp::Constant(_) | ExpressionOp::Reference(_) => (),
            ExpressionOp::Copy(id) => {
                let p = get!(places, id);
                for projection in &self.projections[p.projections.bounds(self.projections.len())?] {
                    if let PlaceProjection::Index { index, .. } = projection {
                        visit(*index);
                    }
                }
            }
            ExpressionOp::FieldAccess { base, .. } => visit(base),
            ExpressionOp::NumericUnary { operand, facts } => {
                get!(numeric, facts);
                visit(operand);
            }
            ExpressionOp::NumericBinary { left, right, facts } => {
                get!(numeric, facts);
                visit(left);
                visit(right);
            }
            ExpressionOp::Compare { left, right, facts } => {
                get!(comparisons, facts);
                visit(left);
                visit(right);
            }
            ExpressionOp::Range { start, end, facts } => {
                get!(ranges, facts);
                visit(start);
                visit(end);
            }
            ExpressionOp::LogicalNot { operand } => visit(operand),
            ExpressionOp::LogicalBinary { left, right, .. } => {
                visit(left);
                visit(right);
            }
            ExpressionOp::Call(id) | ExpressionOp::MethodCall { call: id, .. } => {
                if let ExpressionOp::MethodCall { receiver, .. } = operation {
                    visit(receiver);
                }
                let call = get!(calls, id);
                for arg in &self.arguments[call.arguments.bounds(self.arguments.len())?] {
                    visit(arg.value);
                }
            }
            ExpressionOp::CollectionBuiltin { receiver, call } => {
                visit(receiver);
                for arg in &self.arguments[get!(collection_calls, call)
                    .arguments
                    .bounds(self.arguments.len())?]
                {
                    visit(arg.value);
                }
            }
            ExpressionOp::MapBuiltin { receiver, call } => {
                visit(receiver);
                for arg in &self.arguments[get!(map_calls, call)
                    .arguments
                    .bounds(self.arguments.len())?]
                {
                    visit(arg.value);
                }
            }
            ExpressionOp::Cast { source, facts } => {
                get!(casts, facts);
                visit(source);
            }
            ExpressionOp::HandleFallible { value, handling } => {
                get!(handling, handling);
                visit(value);
            }
            ExpressionOp::PropagateOption { value } | ExpressionOp::Coerce { value, .. } => {
                visit(value)
            }
            ExpressionOp::Collection(range) => {
                for &id in &self.children[range.bounds(self.children.len())?] {
                    visit(id);
                }
            }
            ExpressionOp::Map(range) => {
                for entry in &self.map_entries[range.bounds(self.map_entries.len())?] {
                    visit(entry.key);
                    visit(entry.value);
                }
            }
            ExpressionOp::Struct { fields } | ExpressionOp::Choice { fields, .. } => {
                for field in
                    &self.constructor_fields[fields.bounds(self.constructor_fields.len())?]
                {
                    visit(field.value);
                }
            }
            ExpressionOp::ValueBlock(id) => {
                let value = get!(value_blocks, id);
                value.visit_expression_ids(visit);
                let mut error = None;
                value.visit_place_ids(&mut |place| {
                    if let Err(e) = self.visit_place_edges(place, visit) {
                        error = Some(e);
                    }
                });
                if let Some(error) = error {
                    return Err(error);
                }
            }
            ExpressionOp::RuntimeTemplate(id) => {
                get!(runtime_templates, id).visit_expression_ids(visit)
            }
            ExpressionOp::RuntimeSlotApplication(id) => {
                get!(runtime_slots, id).visit_expression_ids(visit)
            }
            ExpressionOp::Template(id) => {
                get!(templates, id);
            }
        }
        Ok(())
    }
}
impl<'a> ExpressionView<'a> {
    pub(crate) fn owner_token(self) -> OwnerToken {
        self.owner.owner
    }
    fn row(self) -> &'a ExpressionRow {
        &self.owner.rows[self.id.index()]
    }
    pub(crate) fn id(self) -> ExprId {
        self.id
    }
    pub(crate) fn store(self) -> &'a ExpressionStore {
        self.owner
    }
    pub(crate) fn visit_child_ids(
        self,
        visit: &mut dyn FnMut(ExprId),
    ) -> Result<(), CompilerError> {
        self.owner
            .visit_operation_edges(self.row().operation, visit)
    }
    pub(crate) fn success_shape(self) -> SuccessShape {
        self.row().success
    }
    pub(crate) fn success_types(self) -> &'a [TypeId] {
        match &self.row().success {
            SuccessShape::Zero => &[],
            SuccessShape::One(id) => std::slice::from_ref(id),
            SuccessShape::Multiple(range) => {
                &self.owner.result_types[range
                    .bounds(self.owner.result_types.len())
                    .expect("validated type range")]
            }
        }
    }
    pub(crate) fn require_single(
        self,
        receiver: ValueReceiverContext,
    ) -> Result<TypeId, CompilerDiagnostic> {
        match self.row().success {
            SuccessShape::One(id) => Ok(id),
            _ => Err(CompilerDiagnostic::invalid_value_receiver(
                receiver,
                1,
                self.success_types().len(),
                self.span(),
            )),
        }
    }
    pub(crate) fn error_type(self) -> Option<TypeId> {
        self.row().error_type
    }
    pub(crate) fn span(self) -> Option<SourceSpan> {
        self.row().span
    }
    pub(crate) fn value_mode(self) -> ValueMode {
        self.row().value_mode
    }
    pub(crate) fn classification(self) -> ExpressionClassification {
        self.row().classification
    }
    pub(crate) fn failure_facts(self) -> Option<&'a ExpressionFailureFacts> {
        self.row()
            .failure
            .map(|id| &self.owner.failures[id.index()])
    }
    pub(crate) fn occurrence_facts(self) -> Option<&'a OccurrenceFacts> {
        self.row()
            .occurrence
            .map(|id| &self.owner.occurrences[id.index()])
    }
    pub(crate) fn visit_children(self, mut visit: impl FnMut(ExpressionView<'a>)) {
        self.owner
            .visit_operation_edges(self.row().operation, &mut |id| {
                visit(self.owner.view(id).expect("validated child"))
            })
            .expect("validated operation");
    }
}
#[path = "expression_store_view.rs"]
mod views;
pub(crate) use views::*;
#[path = "expression_scratch.rs"]
mod scratch;
pub(crate) use scratch::*;
#[cfg(test)]
#[path = "tests/owner_boundary_tests.rs"]
mod owner_boundary_tests;
#[cfg(test)]
#[path = "tests/expression_store_tests.rs"]
mod tests;

/// Inline typed buffers only; nested owned strings, statements and fact vectors are separate.
#[cfg(any(test, feature = "benchmark_counters"))]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ExpressionStorageStatistics {
    pub rows: usize,
    pub row_capacity: usize,
    pub row_bytes: usize,
    pub row_capacity_bytes: usize,
    pub side_entries: usize,
    pub side_capacity: usize,
    pub side_bytes: usize,
    pub side_capacity_bytes: usize,
}

#[path = "expression_freeze.rs"]
mod freeze;
pub(crate) use freeze::ExpressionFreezeRoots;

impl From<CompilerDiagnostic> for ExpressionConstructionFailure {
    fn from(e: CompilerDiagnostic) -> Self {
        Self::Diagnostic(e)
    }
}
#[path = "expression_store_snapshot.rs"]
mod snapshot;

impl SuccessShape {
    pub(crate) fn single(self) -> Option<TypeId> {
        match self {
            Self::One(id) => Some(id),
            _ => None,
        }
    }
}
impl std::fmt::Debug for ExpressionView<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExpressionView")
            .field("id", &self.id)
            .field("operation", &self.row().operation)
            .field("success", &self.row().success)
            .finish()
    }
}
