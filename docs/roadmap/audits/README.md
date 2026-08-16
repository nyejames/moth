# Audit Reports

This directory stores durable reports from structured codebase audits. Read the [Codebase Audit Guide](../audit-guide.md), the selected [audit-kind guide](../audit-kinds/README.md), the [audit log](../audit-log.md) and [open findings](../open-audit-findings.md) before creating a report.

## Naming and identity

One report represents one audit run over one primary kind and one registered scope:

```text
AUD-0001-short-description.md
```

Findings inside that report use stable IDs:

```text
AUD-0001-F01
AUD-0001-F02
```

Create the report skeleton and add it under `Audits in progress` before inspecting implementation. This reserves the ID and exposes concurrent work.

Closed, rejected, duplicate and superseded findings remain in their reports. Only unresolved work stays in the open-findings index. Do not store raw profiles, generated inventories or large machine output here. Summarise the evidence and link or name the reproducible command.

## Report skeleton

```markdown
# AUD-####: Audit title

- State: `in progress` or `complete`
- Kind: `<kind>`
- Primary scope: `<scope-id>`
- Required context: `<scope IDs or paths>`
- Coverage: `partial` or `complete`
- Reviewed: `YYYY-MM`
- Baseline: `<known validation and performance state>`
- Revision: `<optional revision>`

## Scope, context and exclusions

## Coverage inventory

## Authorities read

## Existing findings and active plans checked

## Findings

### AUD-####-F01: Finding title

- State: `candidate`
- Kind: `<kind>`
- Scope: `<scope-id>`
- Priority: `unassigned`

#### Evidence

#### Counter-evidence checked

#### Violated contract or cost

#### Impact

#### Root owner

#### Suggested correction

#### Allowed fix scope

#### Read-only context

#### Must preserve

#### Forbidden fix forms

#### Required validation or measurement

#### Dependencies and related findings

#### Triage record

## No-finding checks

## Limitations

## Freshness update
```

A suggested correction seeds later planning. It does not approve a design, exact patch shape or broader write scope.

## Open-findings entry

Keep the live index concise and link to the report that owns the evidence:

```markdown
- [AUD-####-F##: Finding title](./audits/AUD-####-short-description.md#finding-anchor)
  - `<kind>` | `<scope-id>`
```

An audit in progress links to the report as a whole. An active-fix entry also links to the owning implementation plan, branch or pull request when one exists. Do not copy evidence or patch plans into the index.
