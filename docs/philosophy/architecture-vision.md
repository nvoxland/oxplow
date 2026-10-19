# Architecture vision

Oxplow is an understanding and steering tool for software developers.
Agents do development work; people need to understand the software,
decide what should happen, and verify what happened. The product starts
from those needs rather than the layout and assumptions of a code editor.

The intended advantage is a fresh foundation for software engineering:
descriptions, questions, evidence, and connected views are the primary
working surface. Real code remains the authority and is always available
for drill-down. Monaco and language servers serve that role; oxplow is
explicitly not an IDE.

## Collection and analysis are central

Useful understanding requires more than a transcript or a diff. Oxplow
connects development activity, code structure, changes, tests, metrics,
decisions, claims, and knowledge. Collectors bring data in; models derive
and connect it; measures, metrics, and dimensions make comparisons
possible; lenses and the agent turn it into useful explanations and views.

Built-in sources are a starting point. Every project has its own
architecture, vocabulary, external systems, and questions. Developers
and their agents should be able to add what gets collected and analyzed,
then connect it to the existing record. A deployment event linked to a
change, a test result linked to an effort, and a decision linked to the
affected code together can explain more than any one source alone.

Questions need not start with a dashboard. Ask the agent, inspect the
evidence, and refine the question. Keep a lens or workflow when it proves
useful enough to repeat.

## Personal tools, shared when useful

Agents make personal software practical. Oxplow's extension system is
intended to let developers customize their visibility and workflows to
exactly what they need, then share useful extensions with a team and,
eventually, more broadly.

The foundation should own the plumbing: identity, references, provenance,
query contracts, scheduling, cancellation, permissions, consent, retries,
state, lifecycle, and diagnostics. Extension authors should spend their
effort on their unique analysis, presentation, or workflow.

This is the design goal; the infrastructure is not equally complete for
every contribution type today. See [Extensions](../reference/extensions.md)
for the current authoring surface and restrictions.

## Safety without relying on a marketplace review process

An extension may come from a public repository, a teammate, or your own
agent. None of those origins makes its code trustworthy. Personal tools
will be too numerous and change too quickly to depend on many reviewers
examining every version.

Safety therefore belongs in the host's enforceable contracts: constrained
execution, explicit access, declared permissions approved at deploy, shared action gates,
resource limits, and inspectable behavior. Give extensions useful host
services and controlled escape paths instead of making them recreate
credentials, persistence, or authorization themselves.

The current product already has constrained scripts, declared provider
access, audited commands, and sandboxed UI components. Execution
containment and consistency across extension kinds remain work to do;
an approved permission is not a guarantee that arbitrary code is harmless.

## The platform boundary

Core owns the shared record, integrity rules, service contracts, and
navigation/interaction mechanisms. Extensions supply integrations,
analyses, interpretations, presentations, and replaceable workflow policy.
Built-in features should exercise the same public mechanisms wherever
practical.

Effort attribution and command authorization need consistent rules.
Which review signals matter, how work is grouped, which AI prompt
extracts a decision, or how a result is displayed should be replaceable.

## Current shape and future direction

Today the product has isolated worktree streams; threads with agent
sessions; turn/tool-call recording; optional effort and work-item
integration; snapshots; a semantic SQL layer; metrics and dimensions;
declarative extensions; and code drill-down. A desktop shell connects to
a per-project Rust daemon with SQLite state and an event log.

The next foundation work is to make combinations of extension types
consistent: shared run context and services, coherent activation and
resource ownership, explicit compatibility, and better authoring tools.
AI interfaces should evolve to support new functionality while keeping
provenance, usage, permissions, and cancellation under host control.

Interactive terminal and chat prompts currently come from the person.
Separate recorded AI API computations are built. Programmatic agent
sessions and additional remote-host capabilities are future work; they
should preserve the ability to use oxplow with a person driving an agent
and the platform observing it.
