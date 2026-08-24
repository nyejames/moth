# Audit Reports

One report per audit run: one kind, one area. Read the [audit guide](../audit-guide.md) first.

```text
AUD-0001-short-description.md
```

Findings inside use stable IDs: `AUD-0001-F01`, `AUD-0001-F02`.

Create the skeleton and add it under `Audits in progress` **before** inspecting anything. This reserves the ID and makes concurrent runs collide visibly.

Rejected, superseded and closed findings stay in their report. Only unresolved work stays in the [open-findings index](../open-audit-findings.md). Do not store raw profiles or generated inventories here - summarise, and name the command that reproduces them.

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
- [AUD-####-F##: Title](./audits/AUD-####-short-description.md#anchor)
  - `<kind>` | `<area>`
```

Link to the report. Do not copy evidence into the index.
