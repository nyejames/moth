# Audit Reports

One report per audit run: one kind, one area. Read the [audit guide](../audit-guide.md) first.

- [open_audits](./open_audits/) contains runs in progress and completed runs with unresolved findings.
- [closed_audits](./closed_audits/) contains runs whose findings have all been resolved, rejected or superseded, plus clean runs.
- [Open findings](../open-audit-findings.md) tracks current work. The [completed audits log](../completed_audits_log.md) keeps historic resolutions. The [audit log](../audit-log.md) tracks coverage independently.

```text
AUD-0001-short-description.md
```

Findings inside use stable IDs: `AUD-0001-F01`, `AUD-0001-F02`.

Create the skeleton in `open_audits/` and add it under `Audits in progress` **before** inspecting anything. Reserve the next unused ID across both folders, so concurrent runs collide visibly.

Rejected, superseded and closed findings stay in their report. Move the report to `closed_audits/` when no finding remains open, record its closeout in the completed audits log and update incoming links. Do not store raw profiles or generated inventories here - summarise, and name the command that reproduces them.

## Skeleton

```markdown
# AUD-####: Title

- State: `in progress` | `complete`
- Kind: `<kind>`
- Area: `<area>` - one line on what it covers
- Coverage: `complete` | `partial`
- Reviewed: `YYYY-MM`
- Baseline: `<validation state, known failures, anything limiting confidence>`

## What was inspected

Every file and surface actually read, plus context read but not audited.

## Authorities read

## Existing findings and active plans checked

## Findings

### AUD-####-F01: Title

- State: `candidate`
- Kind: `<kind>`

#### Evidence

#### Counter-explanation tested

What would make this finding wrong, and why it does not.

#### Violated contract or cost

#### Root owner

#### Suggested correction

Non-authorising. Seeds later work; does not approve a design or patch shape.

#### Fix scope and preserved invariants

#### Required validation

#### Linked findings

## Checked and clean

What was inspected and found sound. This is what makes a no-findings run useful.

## Limitations

What was not covered and why.
```

## Open-findings entry

```markdown
- [AUD-####-F##: Title](./audits/open_audits/AUD-####-short-description.md#anchor)
  - `<kind>` | `<area>`
```

Link to the report. Do not copy evidence into the index.
