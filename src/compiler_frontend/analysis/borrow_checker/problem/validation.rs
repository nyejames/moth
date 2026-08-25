//! Atomic validation for normalized BorrowProblem input.

use crate::compiler_frontend::compiler_errors::CompilerError;

use super::{BorrowProblem, CallResultProvenance, EventKind, OriginKind, RebindValue};
use super::{LoanId, PointId, ValueOriginId};
use std::collections::BTreeSet;
use std::fmt::Debug;

fn compiler_error(message: impl Into<String>) -> CompilerError {
    CompilerError::compiler_error(message)
}

fn validate_dense_ids<T>(
    rows: &[T],
    kind: &str,
    id: impl Fn(&T) -> u32,
) -> Result<(), CompilerError> {
    for (index, row) in rows.iter().enumerate() {
        let expected = u32::try_from(index)
            .map_err(|_| compiler_error(format!("{kind} table is larger than u32::MAX rows")))?;
        let actual = id(row);
        if actual != expected {
            return Err(compiler_error(format!(
                "{kind} IDs must be dense: row {index} has ID {actual}, expected {expected}"
            )));
        }
    }
    Ok(())
}

fn require_index(
    id: impl FnOnce() -> usize,
    len: usize,
    kind: &str,
    raw: u32,
) -> Result<usize, CompilerError> {
    let index = id();
    if index >= len {
        return Err(compiler_error(format!(
            "{kind} ID {raw} is outside the {len}-row problem table"
        )));
    }
    Ok(index)
}

fn validate_sorted_unique<T: Ord + Debug>(values: &[T], owner: &str) -> Result<(), CompilerError> {
    for pair in values.windows(2) {
        if pair[0] >= pair[1] {
            return Err(compiler_error(format!(
                "{owner} references must be strictly sorted and unique: {:?} then {:?}",
                pair[0], pair[1]
            )));
        }
    }
    Ok(())
}

fn validate_origin_set(
    origins: &[ValueOriginId],
    origin_count: usize,
    owner: &str,
) -> Result<(), CompilerError> {
    validate_sorted_unique(origins, owner)?;
    for origin in origins {
        require_index(
            || origin.index(),
            origin_count,
            "value-origin",
            origin.raw(),
        )?;
    }
    if origins.is_empty() {
        return Err(compiler_error(format!(
            "{owner} must name at least one value origin"
        )));
    }
    Ok(())
}

fn validate_point(point: PointId, point_count: usize, owner: &str) -> Result<(), CompilerError> {
    require_index(|| point.index(), point_count, "program-point", point.raw())
        .map(|_| ())
        .map_err(|error| compiler_error(format!("{owner}: {}", error.msg)))
}

pub(super) fn validate(problem: &BorrowProblem) -> Result<(), CompilerError> {
    let flow = problem.control_flow();
    let points = problem.points();
    let places = problem.places();
    let origins = problem.origins();
    let loans = problem.loans();
    let uses = problem.uses();
    let calls = problem.calls();
    let events = problem.events();
    let blocks = &flow.blocks;

    if blocks.is_empty() {
        return Err(compiler_error(
            "a borrow problem must contain at least one CFG block",
        ));
    }
    if points.is_empty() {
        return Err(compiler_error(
            "a borrow problem must contain at least one program point",
        ));
    }

    validate_dense_ids(blocks, "CFG block", |block| block.id.raw())?;
    validate_dense_ids(points, "program-point", |point| point.id.raw())?;
    validate_dense_ids(places, "place", |place| place.id.raw())?;
    validate_dense_ids(origins, "value-origin", |origin| origin.id.raw())?;
    validate_dense_ids(loans, "loan", |loan| loan.id.raw())?;
    validate_dense_ids(uses, "use", |use_row| use_row.id.raw())?;
    validate_dense_ids(calls, "call", |call| call.id.raw())?;
    validate_dense_ids(events, "event", |event| event.id.raw())?;

    let entry = require_index(
        || flow.entry.index(),
        blocks.len(),
        "CFG entry block",
        flow.entry.raw(),
    )?;
    if flow.exits.is_empty() {
        return Err(compiler_error(
            "a borrow problem must declare at least one CFG exit block",
        ));
    }

    let mut incoming = vec![0usize; blocks.len()];
    let mut outgoing = vec![0usize; blocks.len()];
    for edge in flow.edges.iter() {
        let from = require_index(
            || edge.from.index(),
            blocks.len(),
            "CFG source block",
            edge.from.raw(),
        )?;
        let to = require_index(
            || edge.to.index(),
            blocks.len(),
            "CFG target block",
            edge.to.raw(),
        )?;
        outgoing[from] += 1;
        incoming[to] += 1;
    }
    if incoming[entry] != 0 {
        return Err(compiler_error(
            "the CFG entry block must not have an incoming edge",
        ));
    }

    let mut exit_seen = BTreeSet::new();
    for exit in flow.exits.iter() {
        let exit_index =
            require_index(|| exit.index(), blocks.len(), "CFG exit block", exit.raw())?;
        if !exit_seen.insert(*exit) {
            return Err(compiler_error(format!(
                "CFG exit block {:?} is listed more than once",
                exit
            )));
        }
        if outgoing[exit_index] != 0 {
            return Err(compiler_error(format!(
                "CFG exit block {:?} must not have an outgoing edge",
                exit
            )));
        }
    }
    for block in blocks {
        if outgoing[block.id.index()] == 0 && !exit_seen.contains(&block.id) {
            return Err(compiler_error(format!(
                "terminal CFG block {:?} must be listed as an exit",
                block.id
            )));
        }
    }

    for point in points {
        let block_index = require_index(
            || point.block.index(),
            blocks.len(),
            "program-point block",
            point.block.raw(),
        )?;
        let block = &blocks[block_index];
        if block.id != point.block {
            return Err(compiler_error(format!(
                "program point {:?} names block {:?}, but that block row has ID {:?}",
                point.id, point.block, block.id
            )));
        }
    }

    let mut block_ranges = vec![(0_u32, 0_u32); blocks.len()];
    let mut event_seen = vec![false; events.len()];
    for block in blocks {
        let entry_point = &points[require_index(
            || block.entry.index(),
            points.len(),
            "CFG block entry point",
            block.entry.raw(),
        )?];
        let exit_point = &points[require_index(
            || block.exit.index(),
            points.len(),
            "CFG block exit point",
            block.exit.raw(),
        )?];
        if entry_point.block != block.id || exit_point.block != block.id {
            return Err(compiler_error(format!(
                "CFG block {:?} entry and exit points must belong to that block",
                block.id
            )));
        }
        if entry_point.ordinal > exit_point.ordinal {
            return Err(compiler_error(format!(
                "CFG block {:?} entry point must precede its exit point",
                block.id
            )));
        }
        block_ranges[block.id.index()] = (entry_point.ordinal, exit_point.ordinal);

        let mut previous_ordinal = None;
        for event_id in block.events.iter() {
            let event_index = require_index(
                || event_id.index(),
                events.len(),
                "block event",
                event_id.raw(),
            )?;
            if event_seen[event_index] {
                return Err(compiler_error(format!(
                    "event {:?} appears in more than one CFG block",
                    event_id
                )));
            }
            event_seen[event_index] = true;

            let event = &events[event_index];
            let point = &points[require_index(
                || event.point.index(),
                points.len(),
                "event point",
                event.point.raw(),
            )?];
            if point.block != block.id {
                return Err(compiler_error(format!(
                    "event {:?} is listed in block {:?} but belongs to block {:?}",
                    event.id, block.id, point.block
                )));
            }
            if previous_ordinal.is_some_and(|previous| point.ordinal < previous) {
                return Err(compiler_error(format!(
                    "events in CFG block {:?} are not ordered by program point",
                    block.id
                )));
            }
            previous_ordinal = Some(point.ordinal);
        }
    }
    for point in points {
        let (entry_ordinal, exit_ordinal) = block_ranges[point.block.index()];
        if point.ordinal < entry_ordinal || point.ordinal > exit_ordinal {
            return Err(compiler_error(format!(
                "program point {:?} lies outside the entry/exit range of CFG block {:?}",
                point.id, point.block
            )));
        }
    }
    if event_seen.iter().any(|seen| !seen) {
        return Err(compiler_error(
            "every normalized event must belong to exactly one CFG block",
        ));
    }

    for use_row in uses {
        validate_point(use_row.point, points.len(), "use")?;
        require_index(
            || use_row.place.index(),
            places.len(),
            "use place",
            use_row.place.raw(),
        )?;
    }

    for origin in origins {
        match &origin.kind {
            OriginKind::Fresh => {}
            OriginKind::Alias(source_origins)
            | OriginKind::ExclusiveAlias(source_origins)
            | OriginKind::Copy(source_origins)
            | OriginKind::Join(source_origins) => {
                validate_origin_set(source_origins, origins.len(), "origin derivation")?;
            }
            OriginKind::Projection { source, .. } => {
                validate_origin_set(std::slice::from_ref(source), origins.len(), "projection")?;
            }
            OriginKind::CallResult { call, provenance } => {
                require_index(|| call.index(), calls.len(), "origin call", call.raw())?;
                if let CallResultProvenance::Alias(source_origins) = provenance {
                    validate_origin_set(source_origins, origins.len(), "call-result provenance")?;
                }
            }
        }
    }

    for loan in loans {
        validate_point(loan.issued_at, points.len(), "loan issue")?;
        require_index(
            || loan.place.index(),
            places.len(),
            "loan place",
            loan.place.raw(),
        )?;
        validate_origin_set(&loan.origins, origins.len(), "loan origins")?;
        if loan.holders.is_empty() {
            return Err(compiler_error(format!(
                "loan {:?} must have at least one holder",
                loan.id
            )));
        }
        for holder in loan.holders.iter() {
            require_index(|| holder.index(), places.len(), "loan holder", holder.raw())?;
        }
        validate_sorted_unique(&loan.uses, "loan uses")?;
        for use_id in loan.uses.iter() {
            require_index(|| use_id.index(), uses.len(), "loan use", use_id.raw())?;
        }
        validate_sorted_unique(&loan.kills, "loan kills")?;
        for kill in loan.kills.iter() {
            validate_point(*kill, points.len(), "loan kill")?;
        }
    }

    let mut use_owners = vec![0usize; uses.len()];
    let mut loan_issues = vec![0usize; loans.len()];
    let mut loan_kills = BTreeSet::new();
    for event in events {
        validate_point(event.point, points.len(), "event")?;
        match &event.kind {
            EventKind::Fresh {
                destination,
                origin,
            } => {
                validate_place(*destination, places.len(), "fresh destination")?;
                validate_origin(*origin, origins.len(), "fresh origin")?;
            }
            EventKind::Alias {
                source,
                destination,
                origins: event_origins,
            }
            | EventKind::ExclusiveAlias {
                source,
                destination,
                origins: event_origins,
            } => {
                validate_place(*source, places.len(), "alias source")?;
                validate_place(*destination, places.len(), "alias destination")?;
                validate_origin_set(event_origins, origins.len(), "alias event")?;
            }
            EventKind::Copy {
                source,
                destination,
                origin,
            } => {
                validate_place(*source, places.len(), "copy source")?;
                validate_place(*destination, places.len(), "copy destination")?;
                validate_origin(*origin, origins.len(), "copy result")?;
            }
            EventKind::Rebind { destination, value } => {
                validate_place(*destination, places.len(), "rebind destination")?;
                match value {
                    RebindValue::Fresh(origin) => {
                        validate_origin(*origin, origins.len(), "fresh rebind")?;
                    }
                    RebindValue::Alias(event_origins) => {
                        validate_origin_set(event_origins, origins.len(), "alias rebind")?;
                    }
                }
            }
            EventKind::Aggregate {
                destination,
                origin,
                fields,
            } => {
                validate_place(*destination, places.len(), "aggregate destination")?;
                validate_origin(*origin, origins.len(), "aggregate origin")?;
                for field in fields.iter() {
                    validate_place(field.source, places.len(), "aggregate child")?;
                }
            }
            EventKind::CallEffect(effect) => {
                require_index(
                    || effect.call.index(),
                    calls.len(),
                    "call effect",
                    effect.call.raw(),
                )?;
                for argument in effect.arguments.iter() {
                    validate_place(argument.place, places.len(), "call argument")?;
                    let use_index = require_index(
                        || argument.use_id.index(),
                        uses.len(),
                        "call argument use",
                        argument.use_id.raw(),
                    )?;
                    let use_row = &uses[use_index];
                    if use_row.point != event.point || use_row.place != argument.place {
                        return Err(compiler_error(format!(
                            "call argument use {:?} must match its event point and place",
                            argument.use_id
                        )));
                    }
                    use_owners[use_index] += 1;
                }
                if let Some(result) = effect.result {
                    validate_place(result.place, places.len(), "call result")?;
                    validate_origin(result.origin, origins.len(), "call result origin")?;
                }
            }
            EventKind::Access { use_id } => {
                let use_index =
                    require_index(|| use_id.index(), uses.len(), "access use", use_id.raw())?;
                if uses[use_index].point != event.point {
                    return Err(compiler_error(format!(
                        "access use {:?} must match its event point",
                        use_id
                    )));
                }
                use_owners[use_index] += 1;
            }
            EventKind::LoanIssue { loan } => {
                let loan_index =
                    require_index(|| loan.index(), loans.len(), "loan issue event", loan.raw())?;
                let row = &loans[loan_index];
                if row.issued_at != event.point {
                    return Err(compiler_error(format!(
                        "loan {:?} issue event point does not match its loan row",
                        loan
                    )));
                }
                loan_issues[loan_index] += 1;
            }
            EventKind::LoanKill { loan, .. } => {
                let loan_index =
                    require_index(|| loan.index(), loans.len(), "loan kill event", loan.raw())?;
                if !loans[loan_index].kills.contains(&event.point) {
                    return Err(compiler_error(format!(
                        "loan {:?} kill event is absent from its declared kill points",
                        loan
                    )));
                }
                loan_kills.insert((*loan, event.point));
            }
        }
    }

    for (index, owners) in use_owners.iter().enumerate() {
        if *owners != 1 {
            return Err(compiler_error(format!(
                "use {:?} must be owned by exactly one normalized event, found {owners}",
                super::UseId::new(index as u32)
            )));
        }
    }
    for (index, issues) in loan_issues.iter().enumerate() {
        if *issues != 1 {
            return Err(compiler_error(format!(
                "loan {:?} must have exactly one issue event, found {issues}",
                LoanId::new(index as u32)
            )));
        }
    }
    for loan in loans {
        for kill in loan.kills.iter() {
            if !loan_kills.contains(&(loan.id, *kill)) {
                return Err(compiler_error(format!(
                    "loan {:?} declares kill point {:?} without a kill event",
                    loan.id, kill
                )));
            }
        }
    }

    Ok(())
}

fn validate_place(
    place: super::PlaceId,
    place_count: usize,
    owner: &str,
) -> Result<(), CompilerError> {
    require_index(|| place.index(), place_count, owner, place.raw()).map(|_| ())
}

fn validate_origin(
    origin: ValueOriginId,
    origin_count: usize,
    owner: &str,
) -> Result<(), CompilerError> {
    require_index(|| origin.index(), origin_count, owner, origin.raw()).map(|_| ())
}
