# Pair programming

A coding agent can handle implementation while the developer holds the
larger picture: what the software should do, where a change fits, and
whether the result is sound. Oxplow is built for that developer's need
to understand and steer the work.

## What the agent is good at

Agents search, write code, run tests, and try alternatives. They can
also build the custom collectors, analyses, and views that make a
particular project understandable. That personal development should
be practical without rebuilding the host's infrastructure.

## What the human brings

The developer supplies direction, domain knowledge, taste, and
judgment. A useful explanation connects implementation activity to
those concerns, then provides evidence when a closer look is needed.
A diff alone rarely supplies all that context.

## How oxplow supports the navigator

| Need | Oxplow surface |
|---|---|
| Understand the software | Descriptions, connected references, knowledge, and custom lenses |
| See what changed | Observed activity, snapshots, analysis, and diffs |
| Preserve scope and rationale | Optional work-list and knowledge providers |
| Develop in parallel | Separate stream worktrees and a writer thread per stream |
| Ask and investigate | Agent sessions in research and review threads |
| Compare evidence | Collected facts, metrics, and dimensions |
| Tailor the workflow | Extensions that connect data, presentation, and actions |

The developer chooses the questions and views. Code remains available
for inspection, but it does not dictate the starting point.

## Measuring the work as well as the code

Commits do not explain every attempt, test result, or intervention that
led to them. Collected activity and test evidence make more of that
process available for analysis. Developers can add their own collectors
and metrics to ask whether a workflow is improving.

A metric such as time to a passing test or effort spent on rework needs
an explicit definition and suitable collected evidence. These are
questions the extensible data model can support, rather than a promise
that every such metric ships configured. See [Metrics](../guide/metrics.md).

## Staying present during development

Oxplow aims to make work understandable while it is happening. An agent
can explain a decision, a lens can surface a concerning trend, and a
reference can lead from that description to the relevant change.
Work items and knowledge help when enabled, without becoming mandatory
bookkeeping before an edit or the end of a turn.

This approach gives developers room to invent their own ways of
supervising software development as agents' capabilities change.
