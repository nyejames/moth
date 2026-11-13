//! Classified runtime failures.
//!
//! Every failure the runtime reports is one [`RuntimeError`] variant chosen by the phase that
//! failed and by typed facts, never by matching message text. Wasmtime's own error is kept as
//! the source for display and logging, behind a standard error object so no Wasmtime type is
//! public.
//!
//! None of these is a Moth error. A declared Moth error travels through the host ABI's own
//! channel, which arrives with the adapters.

use std::error::Error;
use std::fmt;

/// The underlying failure, kept for display and never inspected for classification.
pub(super) type ErrorSource = Box<dyn Error + Send + Sync + 'static>;

/// One runtime failure, classified by the phase that produced it.
#[derive(Debug)]
#[non_exhaustive]
pub enum RuntimeError {
    // Before any instance exists.
    /// Wasmtime rejected the engine configuration on this host.
    EngineConfiguration { source: ErrorSource },

    /// The bytes are not a core Wasm module this engine accepts. This includes a valid module
    /// that needs a Wasm proposal outside the engine's profile.
    InvalidModule { source: ErrorSource },

    /// A compiled module was used with an engine that didn't compile it.
    EngineMismatch,

    /// The module's imports or exports don't match what the host provides or calls.
    Link(LinkError),

    // While guest code runs. Each of these quarantines its instance.
    /// The Wasm start function trapped, so no instance was created.
    StartTrap { source: ErrorSource },

    /// Guest code trapped during an exported call.
    GuestTrap { source: ErrorSource },

    /// A resource limit or execution deadline was exhausted. Limits checked before
    /// instantiation have no underlying Wasmtime failure.
    LimitExceeded {
        limit: LimitFault,
        source: Option<ErrorSource>,
    },

    /// Native host code failed during a guest call.
    HostFault(HostFault),

    /// A ticket, token, span or frame broke the host ABI. The instance is quarantined.
    BoundaryFault(BoundaryFault),
    /// An export argument failed a check before guest entry. The guest was not entered and
    /// the instance stays usable.
    ExportArgument(BoundaryFault),
    // After a fault.
    /// The instance faulted earlier and is quarantined. Guest code was not entered.
    InstanceFaulted,
}

/// A mismatch between a module's imports or exports and the host side.
#[derive(Debug)]
#[non_exhaustive]
pub enum LinkError {
    /// The host couldn't register an import, for example because the name is already defined.
    ImportDefinition {
        module: String,
        name: String,
        source: ErrorSource,
    },

    /// The module imports something the host doesn't provide.
    MissingImport { module: String, name: String },

    /// The host provides an import under that name with another physical Wasm type.
    ImportMismatch { source: ErrorSource },

    /// The module has no exported function with that name.
    MissingExport { name: String },

    /// The exported function has another physical Wasm signature than the call used.
    ExportSignatureMismatch { name: String, source: ErrorSource },
}

/// Which host-ABI check failed. This is not a Moth error and not a guest trap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BoundaryFault {
    /// The ticket is zero, unknown or already taken.
    UnknownTicket,
    /// The ticket is live, but the length is not the staged length.
    WrongLength,
    /// The destination range is outside the guest memory.
    DestinationOutOfBounds,
    /// A host import other than the take import was entered while a ticket was live.
    TicketLiveAtImport,
    /// The outermost guest entry returned while a ticket was live.
    TicketLiveAtReturn,
    /// The token is zero, closed, unknown or from another instance.
    TokenNotLive,
    /// The token is live, but the value has another type.
    TokenTypeMismatch,
    /// The token is already borrowed in a way this use conflicts with.
    TokenConflict,
    /// An export status or result field is not one of the canonical values.
    NoncanonicalStatus,
}

impl std::error::Error for BoundaryFault {}

impl fmt::Display for BoundaryFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnknownTicket => "the staged-text ticket is not live",
            Self::WrongLength => "the staged-text length does not match",
            Self::DestinationOutOfBounds => "the staged-text destination is outside guest memory",
            Self::TicketLiveAtImport => {
                "a host import was entered while staged text was still live"
            }
            Self::TicketLiveAtReturn => "the guest entry returned while staged text was still live",
            Self::TokenNotLive => "the resource token is not live in this instance",
            Self::TokenTypeMismatch => "the resource token has another type",
            Self::TokenConflict => "the resource token is already borrowed",
            Self::NoncanonicalStatus => "the export status or result frame is not canonical",
        })
    }
}

/// Which limit was exhausted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LimitFault {
    /// The module's initial linear memory is larger than the instance's memory ceiling.
    MemoryCeiling,
    /// The module's initial table is larger than the instance's table ceiling.
    TableCeiling,
    /// The module defines more linear memories than an instance may own.
    MemoryCount,
    /// The module defines more tables than an instance may own.
    TableCount,
    /// Guest recursion exhausted the Wasm stack.
    StackExhausted,
    /// The call's execution deadline passed before the guest returned.
    DeadlineReached,
}

/// How native host code failed during a guest call.
#[derive(Debug)]
#[non_exhaustive]
pub enum HostFault {
    /// A host import reported an operational failure.
    Failed { source: ErrorSource },

    /// A host import panicked. The panic was caught at the host-call boundary.
    Panicked { message: String },

    /// The native side broke an invariant that is neither a guest trap nor a reported host
    /// failure, such as a host import producing a result of the wrong Wasm type.
    Internal { source: ErrorSource },

    /// The embedder's context was missing or had another type, so the host function was not
    /// called.
    Context(ContextFault),
}

/// Why a host function did not receive its context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ContextFault {
    /// The holder was empty.
    Missing,
    /// The holder carried a value of another type.
    WrongType,
}

impl RuntimeError {
    pub(super) fn engine_configuration(error: wasmtime::Error) -> Self {
        Self::EngineConfiguration {
            source: error.into_boxed_dyn_error(),
        }
    }

    pub(super) fn invalid_module(error: wasmtime::Error) -> Self {
        Self::InvalidModule {
            source: error.into_boxed_dyn_error(),
        }
    }

    /// A limit rejected before guest code ran, so there is no Wasmtime failure to keep.
    pub(super) fn limit_before_entry(limit: LimitFault) -> Self {
        Self::LimitExceeded {
            limit,
            source: None,
        }
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EngineConfiguration { .. } => {
                formatter.write_str("the native engine configuration was rejected")
            }
            Self::InvalidModule { .. } => {
                formatter.write_str("the bytes are not a supported core Wasm module")
            }
            Self::EngineMismatch => {
                formatter.write_str("the module was compiled by another engine")
            }
            Self::Link(link_error) => link_error.fmt(formatter),

            Self::StartTrap { .. } => formatter.write_str("the Wasm start function trapped"),
            Self::GuestTrap { .. } => formatter.write_str("guest code trapped"),
            Self::LimitExceeded { limit, .. } => limit.fmt(formatter),
            Self::HostFault(host_fault) => host_fault.fmt(formatter),
            Self::BoundaryFault(boundary_fault) => boundary_fault.fmt(formatter),
            Self::ExportArgument(_) => {
                formatter.write_str("an export argument was rejected before the guest was entered")
            }

            Self::InstanceFaulted => {
                formatter.write_str("the instance faulted earlier and can't be called again")
            }
        }
    }
}

impl Error for RuntimeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::EngineConfiguration { source }
            | Self::InvalidModule { source }
            | Self::StartTrap { source }
            | Self::GuestTrap { source } => Some(source.as_ref()),

            Self::LimitExceeded { source, .. } => source
                .as_ref()
                .map(|source| source.as_ref() as &(dyn Error + 'static)),

            Self::Link(link_error) => link_error.source(),
            Self::HostFault(host_fault) => host_fault.source(),
            Self::ExportArgument(fault) => Some(fault),

            Self::BoundaryFault(_) | Self::EngineMismatch | Self::InstanceFaulted => None,
        }
    }
}

impl fmt::Display for LinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ImportDefinition { module, name, .. } => {
                write!(
                    formatter,
                    "the host import `{module}::{name}` couldn't be registered"
                )
            }
            Self::MissingImport { module, name } => {
                write!(
                    formatter,
                    "the module imports `{module}::{name}`, which the host doesn't provide"
                )
            }
            Self::ImportMismatch { .. } => {
                formatter.write_str("a host import has another Wasm type than the module imports")
            }
            Self::MissingExport { name } => {
                write!(formatter, "the module exports no function named `{name}`")
            }
            Self::ExportSignatureMismatch { name, .. } => {
                write!(
                    formatter,
                    "the exported function `{name}` has another Wasm signature"
                )
            }
        }
    }
}

impl Error for LinkError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ImportDefinition { source, .. }
            | Self::ImportMismatch { source }
            | Self::ExportSignatureMismatch { source, .. } => Some(source.as_ref()),

            Self::MissingImport { .. } | Self::MissingExport { .. } => None,
        }
    }
}

impl fmt::Display for LimitFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MemoryCeiling => "the module's initial memory exceeds the memory ceiling",
            Self::TableCeiling => "the module's initial table exceeds the table ceiling",
            Self::MemoryCount => "the module defines more than one linear memory",
            Self::TableCount => "the module defines more than one table",
            Self::StackExhausted => "the guest exhausted the Wasm stack",
            Self::DeadlineReached => "the guest ran past its execution deadline",
        })
    }
}

impl fmt::Display for HostFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed { .. } => formatter.write_str("a host import failed"),
            Self::Panicked { message } => write!(formatter, "a host import panicked: {message}"),
            Self::Internal { .. } => formatter.write_str("the native host side failed"),
            Self::Context(ContextFault::Missing) => {
                formatter.write_str("the host function's context is missing")
            }
            Self::Context(ContextFault::WrongType) => {
                formatter.write_str("the host function's context has another type")
            }
        }
    }
}

impl Error for HostFault {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Failed { source } | Self::Internal { source } => Some(source.as_ref()),
            Self::Panicked { .. } | Self::Context(_) => None,
        }
    }
}
