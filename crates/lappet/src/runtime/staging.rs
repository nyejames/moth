//! Per-instance staging table for Rust-produced text.
//!
//! A ticket is a nonzero integer, unique for the life of the instance, and it encodes no host
//! address. The empty string is ticket zero and has no entry. A failed take leaves the table
//! unchanged. Limits are read from [`InstanceLimits`] on each stage, so this table doesn't keep
//! a second copy.

use super::limits::InstanceLimits;
use std::collections::HashMap;

/// Why a stage or a take was refused. The caller classifies it. This table never quarantines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StagingError {
    /// The bytes exceed the per-entry ceiling. Nothing was staged.
    EntryTooLarge,
    /// The bytes would exceed the live total. Nothing was staged.
    LiveBytesExceeded,
    /// The instance has issued every ticket it can. Nothing was staged.
    TicketsExhausted,
    /// The ticket is zero, unknown or already taken.
    UnknownTicket,
    /// The ticket is live, but the expected length is not the staged length. The entry stays.
    WrongLength,
}

/// Live staged text for one instance.
#[derive(Debug)]
pub(crate) struct StagingTable {
    /// The next ticket to issue. Starts at 1 and never goes backwards, including after a take
    /// or a clear.
    next_ticket: u64,
    entries: HashMap<u64, Vec<u8>>,
    live_bytes: usize,
}

impl StagingTable {
    pub(crate) fn new() -> Self {
        Self {
            next_ticket: 1,
            entries: HashMap::new(),
            live_bytes: 0,
        }
    }

    /// Stages `bytes` and returns its ticket.
    ///
    /// An empty slice stages nothing and returns ticket zero, which is the ABI's empty string.
    pub(crate) fn stage(
        &mut self,
        bytes: &[u8],
        limits: &InstanceLimits,
    ) -> Result<u64, StagingError> {
        if bytes.is_empty() {
            return Ok(0);
        }
        if bytes.len() > limits.max_staged_entry_bytes {
            return Err(StagingError::EntryTooLarge);
        }
        let live_bytes = self
            .live_bytes
            .checked_add(bytes.len())
            .filter(|live_bytes| *live_bytes <= limits.max_staged_live_bytes)
            .ok_or(StagingError::LiveBytesExceeded)?;
        let ticket = self.next_ticket;
        self.next_ticket = ticket
            .checked_add(1)
            .ok_or(StagingError::TicketsExhausted)?;

        self.entries.insert(ticket, bytes.to_vec());
        self.live_bytes = live_bytes;
        Ok(ticket)
    }

    /// Removes a live entry whose length is `expected_length` and returns its bytes.
    ///
    /// A miss or a length mismatch consumes nothing.
    pub(crate) fn take(
        &mut self,
        ticket: u64,
        expected_length: usize,
    ) -> Result<Vec<u8>, StagingError> {
        if ticket == 0 || !self.entries.contains_key(&ticket) {
            return Err(StagingError::UnknownTicket);
        }
        if self.entries[&ticket].len() != expected_length {
            return Err(StagingError::WrongLength);
        }

        let bytes = self
            .entries
            .remove(&ticket)
            .expect("the ticket was just found");
        self.live_bytes -= bytes.len();
        Ok(bytes)
    }

    /// A copy of a live entry, without consuming it.
    pub(crate) fn staged_bytes(&self, ticket: u64) -> Result<Vec<u8>, StagingError> {
        self.entries
            .get(&ticket)
            .cloned()
            .ok_or(StagingError::UnknownTicket)
    }

    pub(crate) fn has_live_ticket(&self) -> bool {
        !self.entries.is_empty()
    }

    /// Drops every live entry. Issued tickets stay issued.
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.live_bytes = 0;
    }
}

impl Default for StagingTable {
    fn default() -> Self {
        Self::new()
    }
}
