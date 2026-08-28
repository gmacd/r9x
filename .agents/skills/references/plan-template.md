# Design doc template

Write to `$REPO/tasks/plans/<kebab-name>.md`. Collapse or
merge sections for small plans; never silently omit **Hardware assumptions**
or **Decision records**.

```markdown
# <Title>

## Problem and constraints
What, why now, and the standing constraints that bind it (arch parity,
warning-free gates, minimal scope, Plan 9 shape).

## Prior art
What r9 already has; what Plan 9 and Linux do (with paths into
/Volumes/Code/repos); what will be composed rather than built.

## Hardware assumptions (required)
Per target: what this design assumes about the machine, which assumptions
are false where, and what happens there. Registers/constants cite document
and section.

## Design
### Data structures
The central types, who owns what state, why the special cases disappear.
### Interfaces
Public surface, its shape (file-server-shaped where possible), day-one
users.
### Init and bringup order
What depends on what; which orderings are load-bearing, stated.
### Failure policy
What panics (and why that's init-only), what returns Result, what degrades
loudly.

## Not building
What was considered and refused, so it isn't re-proposed in six months.

## Decision records
One per contested choice:
- **Decision**: what was chosen.
- **Alternatives**: what lost and why (including Phase 2 candidates).
- **Dissent**: which lens argued otherwise and the argument's strength —
  recorded, not averaged away.

## Tasks
Ordered list of tasks/ files this plan produced, with sequencing notes.
```
