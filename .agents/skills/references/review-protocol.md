# Lens protocol — how a lens pass runs

Shared by `os-review`, `os-plan`, and `os-build`. A lens is a set of review
rules distilled from a system, a publication, or a stated principle. Lenses
are not people and findings are never attributed to people: the authority is
the source document, the system, or the rule itself.

## Running one lens

1. Read the lens file (`lenses/<lens>.md`) — sources, rules, and what the
   lens deliberately ignores.
2. Gather the material with `git`: the diff for the stated scope, plus enough
   surrounding code to judge in context rather than from the hunk alone.
3. Apply the rules. Look for the specific patterns the lens names; do not
   improvise new ones.
4. Emit findings in the output contract below.

## Code mode vs. plan mode

- **Code mode**: the scope is a commit, a range, or a working diff. Findings
  cite `file:line`.
- **Plan mode**: the scope is a design document. The rules are unchanged, but
  applied prospectively — judge the decisions the plan commits to, the
  concepts and interfaces it adds, and the assumptions it makes. Read the
  existing code the plan touches to ground the judgment. Cite plan sections
  (plus file paths where relevant), and where the plan is silent rather than
  wrong, prefer `question` severity.

## Output contract

Lens output is consumed by the skill that ran it, not by a human directly:

- One line: the overall read of the scope from this lens.
- Findings, each as:
  `SEVERITY (blocker|should-fix|nit|question) | file:line (or plan section) | what is wrong and why it matters here | principle (lens, source)`
- If nothing survives scrutiny: `No findings from this lens.`

Only report findings defensible under challenge. Never pad. A clean diff gets
"no findings", and that answer is respected.

## Verification — what keeps the panel honest

The panel over-generates by design. Before any finding reaches a report:

- **Read the cited lines.** Drop findings that misread the code, cite lines
  that do not exist, or describe the pre-diff state.
- **Pastiche check.** Would this lens actually spend review capital here, or
  is it a generic nitpick wearing a famous source? A rule cited without a
  concrete cost to a reader, a maintainer, or the CPU is not a finding.
- **Dedup.** Same `file:line` and same underlying issue from several lenses →
  one finding with combined attribution. Convergence across lenses raises
  confidence; say so.
- **Evidence for the severe ones.** `blocker` and `should-fix` findings must
  be checked against evidence — the code, the witness trees at
  `/Volumes/Code/repos/{linux,plan9}`, or the cited specification. Downgrade
  to `question` anything that cannot be verified.

## Disagreement between lenses

Lenses genuinely pull in opposite directions — late binding versus
no-premature-abstraction is the standing example, and the tension is real
rather than a defect. Never average it away: state both positions, decide
which fits this repo here, and record the decision with its dissent.
