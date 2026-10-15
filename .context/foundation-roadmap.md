# Foundation roadmap: a customizable understanding engine

Planning baseline: 2026-10-09. The nine foundation review findings are
accepted. Documentation alignment is completed in this change; the
runtime work below is tracked by each item's status. Implementation has
started with refusal of silently truncated `sql.read` results.

## Purpose and architectural direction

Oxplow helps software developers understand their software and the work
agents are doing on it. Descriptions, questions, connected evidence and
custom views are the starting point. Code and diffs are available for
drill-down. It is explicitly not trying to become an IDE: an editor,
terminal, LSP integration or isolated worktree is supporting infrastructure,
not the organizing model for the product.

Collection, analysis, metrics and dimensions are central. Built-in data
provides a useful starting point; developers and their agents add what
matters to their own project, connect it to existing evidence, and decide
how to present it. Extensions should connect data and functionality to
create understanding, rather than just add isolated panels.

The current code supports this direction: the shared semantic layer,
collector entities and metric facts, dimensions, lenses, references and
page graph already support views over development evidence. Code
inspection and sessions sit within that graph. The review gaps concern
composability, lifecycle, policy and safety; they do not require turning
the product into an IDE or replacing its data foundation.

The customization bet is that an agent makes a personal tool practical
to build, then the developer can share it with a team and eventually
publicly. Agent output and shared extensions must both be treated as
untrusted. Store review and reputation cannot be the execution boundary.
The host must provide the common infrastructure and enforce authority.

## Design rules for the work

1. **Unify infrastructure, preserve domain contracts.** Use shared
   identity, descriptors, context, services, event envelopes, grants,
   lifetimes, diagnostics and authoring tools. A collector and an AI
   provider can retain different protocols; their authority and execution
   guarantees should not be reinvented independently.
2. **Require services; grant authority separately.** Declaring network,
   file, credential, model or command access is a request, not permission.
   Bind authority to the approved version and enforce it at the host or
   OS boundary. Record safe usage evidence for inspection and custom
   policy. A restricted external process needs real confinement; a
   broader opt-in tier must say exactly what authority it has.
3. **Use the same public contracts for bundled and personal extensions.**
   Extraction should improve the public API, not create private paths.
   Authors should reuse collection, analysis, references, jobs, state,
   configuration, permission checks, navigation and diagnostics.
4. **Keep external work out of database transactions.** Deterministic
   commands/effects publish intent; durable jobs perform external work
   and publish recorded results. Cancellation stops execution, not only
   the caller's wait. Restart and duplicate delivery have explicit rules.
5. **Make data trustworthy to interpret.** Expose truncation, collection
   freshness, provenance and coverage. A query finishing successfully
   does not prove its input collection is complete or current.
6. **Keep the understanding model open.** Preserve entities, measures,
   dimensions and reference links as reusable data. Do not make custom
   views depend on adopting oxplow's task provider or code-centric UI.
7. **Move optional opinions into extensions.** Core owns evidence,
   analysis mechanisms, host guarantees and safe operations. A particular
   inference prompt, repair-task response, review threshold or workflow
   is a replaceable policy. No mandatory task filing is introduced.
8. **Migrate incrementally.** Keep stable IDs, references, event history,
   provider adapters and documented contracts usable. Prefer one real
   vertical slice before widening an abstraction. Do not pre-build every
   possible extension point or rewrite all existing implementations.

## Lessons from other systems

The patterns below inform the plan; they do not prescribe an IDE product
model or a copy of another platform's permissions.

- VS Code's separate extension hosts and declared execution locations
  provide useful runtime separation and discoverability. Its ordinary
  extension host has the application's permissions, so process separation
  alone is insufficient for this project's untrusted-extension goal.
  [Extension host](https://code.visualstudio.com/api/advanced-topics/extension-host),
  [runtime security](https://code.visualstudio.com/docs/configure/extensions/extension-runtime-security).
- IntelliJ's disposable ownership tree is a useful model for tying
  listeners, services and resources to plugin lifetimes. Use that lesson
  for generations and unload; do not inherit unrestricted plugin authority.
  [Disposable ownership](https://plugins.jetbrains.com/docs/intellij/disposers.html),
  [plugin security](https://plugins.jetbrains.com/docs/marketplace/understanding-plugin-security.html).
- Grafana's backend plugin system combines a supported SDK, crash
  separation and built-in instrumentation. Adopt host-owned execution and
  operational evidence so authors can focus on their data or workflow.
  [Backend plugin system](https://grafana.com/developers/plugin-tools/key-concepts/backend-plugins).
- BI semantic systems provide reusable entities, join relationships,
  metrics and dimensions independently of any particular chart. Protect
  and extend that separation in oxplow rather than making each custom
  page collect and interpret its own disconnected data.
  [dbt semantic models](https://docs.getdbt.com/docs/build/semantic-models).
- Obsidian demonstrates the appeal of personal knowledge customization,
  but documents that it cannot reliably restrict plugins to particular
  permissions. Keep linked understanding and personal authoring while
  enforcing sensitive access through oxplow's host services.
  [Obsidian plugin security](https://obsidian.md/help/plugin-security).

## Nine findings and their implementation homes

| Accepted finding | Planned response |
|---|---|
| Timeouts detach workers; CPU/memory are not contained | Bounded execution and inspectable extension permissions: stop and reap workers, then enforce authority |
| SQL reads can silently truncate | Coherent contracts: completeness/freshness result semantics |
| Kind-specific hosts prevent service composition | Shared RunContext, composable services and durable jobs |
| AI contract is narrow and roles are fixed | Typed capabilities, streaming/tool jobs and extension AI profiles |
| Event projections and fixtures differ | One versioned envelope across runtime and authoring |
| Reload has no coherent generation or complete ownership | Staged activation and owned disposal |
| Dependencies and ordering lack public versioned contracts | Explicit imports and contribution-ID ordering |
| Optional inference/repair/review policy remains in core | Public-API bundled policy extraction |
| Custom UI parity, state and provider independence are limited | Shared component runtime, state/config, general mounts and preview |

The reload finding is an architectural consistency risk, not a reproduced
user-facing race. Its task includes failure injection to establish and
verify the required boundary.

## Sequencing and review boundaries

Seven epics contain 27 child tasks. Four existing backlog tasks are
expanded and reparented, preserving their original requests. The
completed documentation task is recorded under the contracts epic.
Work-item dependency links express the task graph; the entire epic list
is not a single serial queue.

| Stage | Deliverable and work that can proceed |
|---|---|
| Start now | Enforceable script containment; visible SQL truncation; descriptor/terminology inventory. Documentation alignment is already done. |
| Shared contracts | Event envelope, scoped grant contract, RunContext, declared imports. Review these together so names and ownership agree. |
| Common services | Compose services and enforce grants; stage coherent generations; establish contribution ordering. Then add durable jobs, state/config and consistent run health. |
| Lifecycle and AI | Dispose owned generations; add typed AI capabilities, profiles and cancellable/tool-capable jobs. Preview work builds on the same lifecycle. |
| Prove the product model | Extract optional policies, make custom UI provider-independent, test authored collection/metrics/dimensions and exercise diverse non-bundled extensions. |

The first runtime changes should be independently reviewable containment
and truncation fixes. Do not hold them for a broad registry redesign.
A descriptor/context design owner should keep the public contracts
coherent while implementation tasks proceed in separate streams. A task
starts when its actual prerequisites are met, not merely when its epic
appears next in this table.

Each implementation task must update its subsystem documentation and
public examples as applicable. Validation should target its behavior and
failure modes using the repository's report-emitting test commands. The
roadmap is not permission to bypass the normal command bus, approvals or
write guards.

## Scope and completion

This is foundation work, not a marketplace launch, remote-agent host,
new autonomous coding driver, universal UI replacement, or full analysis
rewrite. Public distribution will need a later product/distribution
plan; enforceable execution and versioned contracts should precede it.
No architecture can promise that arbitrary opted-in code is harmless.
The restricted tiers must have concrete enforceable guarantees, and any
broader tier must expose the real authority a user is accepting.

Completion means that agent-authored extensions can collect and connect
new data, analyze it, use authorized AI, keep workflow state and present
custom understanding through public services, with bounded execution,
coherent reload and useful diagnostics. The final trials must demonstrate
that authors can do this without copying the host's plumbing or editing
core. Packaging more extension points without those properties is not
completion.

## Epics and implementation tasks

The details below mirror the work-item backlog by title. Repository docs
intentionally omit local task IDs. Dependencies identify prerequisite
work by title, so this plan is usable outside this project's database.

### Coherent extension contracts and understanding-first documentation

Make oxplow’s purpose and extension vocabulary precise, with shared contribution, event, and query contracts that agent authors can discover.

#### Align README and current guides with understanding-first oxplow

Status: documentation completed in this change.

Scope: Clarify that developers start from descriptions and connected data while agents implement; code is an evidence drill-down. Put collection, metrics, dimensions, personal customization and sharing at the center. Correct stale mandatory filing, task-to-effort, fixed-agent/thread, wiki, and history descriptions. Distinguish shipped guards from future targets.

Acceptance: README, landing copy, architecture vision, onboarding and affected guides agree with current code and the clarified purpose. Current security limitations are factual; target plans are explicitly future. Historical release notes retain their historical meaning.

Validation: Run the repository Poetry MkDocs build with --strict, inspect changed links and anchors, and run git diff --check. No application behavior change is included.

Starting points: `README.md`, `docs/philosophy/architecture-vision.md`, `docs/getting-started/concepts.md`, `.context/architecture.md`.

Prerequisites: none; can start independently.

#### Normalize contribution descriptors and public extension terminology

Status: done. Every kind is described (`oxplow_domain::contribution`,
`extensions::descriptors`): one id grammar and namespace rule, one
collision policy (two extensions can no longer hold one global name),
`version:` required to share, `v_contribution` published on every catalog
change, `list_extensions` counting every kind, change review's impact
listing every kind, SDK scaffolds for advisories, skills, pages and
panels, and the reference's kinds table generated.

Scope: Inventory every extension surface and normalize its metadata: stable identity, owner, contract version/stability, input/output schema, trigger, required services, ordering and lifetime. Keep contribution kinds useful, without independent copies of host rules. Distinguish a project capability, its implementation/provider, a contribution, a required service and an approved grant.

Acceptance: One authoritative descriptor/registry drives loader validation, discovery, generated reference/SDK descriptions and permission requirements. Existing manifests migrate through adapters; stable contribution IDs and semantic references remain usable. Stable versus experimental contracts are visible and documented. This task defines metadata, not a full new manifest language.

Validation: Exercise a representative collector, effect, provider and component through discovery/validation; reject inconsistent descriptors; verify legacy manifest fixtures and generated references.

Starting points: `crates/oxplow-app/src/extensions/manifest_v2.rs`, `crates/oxplow-app/src/extensions.rs`, `crates/oxplow-sdk/src/lib.rs`, `.context/extensions.md`.

Prerequisites: none; can start independently.

#### Use one versioned event envelope across runtime and authoring

Status: done. `EventView` (`oxplow_domain::events::view`) is the one
event an extension sees: every field (`id, type, v, seq, at, source,
subject, anchors, payload, cause`), `null` where absent, at the type's
newest version. Collectors (derived, fact, and exec on stdin), effects,
the effort policy, a provider's `react`, paced runs, review dry runs,
`extension test` examples and `oxplow_preview_collector` all build it the same
way; fixtures go through `EventView::fixture`, which refuses an event
that couldn't happen and carries an older `v` forward. There is no
legacy adapter: the envelope is a superset of every earlier shape, so
shipped scripts read the same fields. Stored events and payload
versioning are unchanged.

Scope: Introduce a common extension-facing projection of event identity, type/version, sequence, timestamp, source, subject, anchors and payload. Adapt collectors, effects, effort policy, fixtures and development tools to it. Preserve event storage semantics and payload versioning.

Acceptance: The same event produces equivalent envelopes in every supported execution and test path. Optional fields are explicit; absent values are not fabricated. Version mismatch and invalid anchor/subject handling are discoverable. Stored historical events and legacy scripts have documented compatibility adapters.

Validation: Use a shared fixture with provenance and anchors across collector/effect/policy/dev-host paths and replay historical versions. Compare exact supported fields and failure behavior.

Starting points: `crates/oxplow-app/src/effects.rs`, `crates/oxplow-app/src/collector_runner.rs`, `crates/oxplow-app/src/effort_policy_script.rs`, `crates/oxplow-sdk/src/extension_test.rs`.

Prerequisites: Normalize contribution descriptors and public extension terminology.

#### Make SQL completeness and collection freshness explicit

Status: done. `sql.read` refuses gateway truncation across commands,
effects, effort policies and provider host calls, and pages on request
(`page: { limit, offset }` → `{ rows, more, next_offset }`); a
collector's `input` is complete or declares its `limit` and reads
`input.complete`. Every gateway result carries `generation` and the
observed `coverage` of the collected models it read, which MCP/RPC
`oxplow_query_sql` and lens runs return and the lens view shows
(semantic-layer.md "Completeness and coverage").

Scope: First stop sql.read from returning an apparently complete result when its 1000-row limit truncates data. Then define a compatible result contract for completeness, limits/pagination, generation and available collection freshness. Keep SQL read-only and distinguish complete query results from complete collection coverage.

Acceptance: An unhandled row cap fails visibly rather than silently changing totals or AI inputs. Extensions can explicitly handle bounded/paged results where supported. Collectors, effects and component queries expose consistent completeness semantics; freshness/coverage use observed metadata and can be unknown.

Validation: Query more than 1000 rows through the affected host paths; assert clear failure or explicit truncation and correct full aggregation/pagination. Cover unsynced/failed sources separately from row limits.

Starting points: `crates/oxplow-app/src/scope_calls.rs`, `crates/oxplow-app/src/collector_runner.rs`, `crates/oxplow-app/src/semantic_catalog.rs`.

Prerequisites: none; can start independently.

### Bounded execution and inspectable extension permissions

Contain untrusted work and enforce scoped authority regardless of who authored or shared the extension. Publish safe usage evidence for inspection and policy.

#### Contain script CPU and memory and reap timed-out workers

Status: done. Every Starlark and jaq run executes in a killable
`oxplow-script-worker` process with a memory cap and host-wide
concurrency bound (`scripts:`, person-only); timeout, cancel and memory
overruns stop and reap it. Measured on realistic collectors
(`.context/performance.md` "What running scripts in workers costs"):
~42 ms per run, so workers are spawned per run.

Scope: The existing timeout returns to the caller while a worker continues. Evaluate interpreter instruction/allocation limits versus a killable worker process, then implement enforceable containment for Starlark and jaq across execution hosts. Measure cold/warm overhead with realistic collectors before selecting the implementation.

Acceptance: Timeout/cancellation actually stops execution; repeated overruns do not grow worker/thread counts. Pathological CPU and allocation inputs cannot exhaust the daemon. Limits and concurrency are bounded, diagnosed, and configurable within host policy. Trusted escape paths, if needed, require explicit consent and accurate labeling; accepting detached workers is not completion.

Validation: Run bounded regressions for repeated infinite/pathological scripts, allocation exhaustion, cancellation and concurrency. Check worker/process cleanup and host responsiveness. Benchmark a realistic AST/fact collector to document the cost.

Starting points: `crates/oxplow-script/src/runtime.rs`, `crates/oxplow-script/src/scope.rs`, `crates/oxplow-app/src/collector_runner.rs`.

Prerequisites: none; can start independently.

#### Define scoped extension grants and usage audit contracts

Status: done. `oxplow_domain::grant` defines the request (derived from a
contribution's declaration by `grants.rs`), the grant (a person's
approval of exactly that request; narrowing asks nothing new), and each
run's decisions (`allowed`, `denied`, `unenforced`), recorded on
`v_extension_run` — the usage record, with actor, cause, anchors, cost,
scopes and model calls, never values or contents. A collector's approval
covers its whole declaration; approvals can be revoked. Per-model reads,
file confinement and a model-call limit are left to the enforcement
item.

Scope: Extend the existing approval work into one grant model: extension/version identity, granted services, semantic model/entity scopes, path scopes/read-write, destinations, program execution, named credential use, AI invocation and resource budgets. Separate declared requirements from user-granted authority, and simple program approval from actual enforcement.

Acceptance: Authority defaults to denied for sensitive services, is derived per call, and changes requiring more authority need renewed approval. Grants can be revoked. The review UI explains requested access and enforcement limitations. Usage records expose actor/cause, scope, result and budget in the semantic layer, without credential values or unnecessary prompt/file contents. Existing permissions have explicit migration behavior.

Validation: Test grant matching, version changes, revocation, scope narrowing, redaction and unsupported enforcement states. Review a pure script, AI collector, external program and component action against the same model.

Starting points: `crates/oxplow-app/src/exec_consent.rs`, `crates/oxplow-app/src/extensions/manifest_v2.rs`, `crates/oxplow-app/src/ai_service.rs`, `.context/ai-providers.md`.

Prerequisites: Normalize contribution descriptors and public extension terminology.

#### Enforce scoped grants across hosts and external execution

Status: in progress. Model/path bounds, live consent and pinned component
authority are implemented; adversarial runtime validation and remaining host
integration are still required. External-process confinement is shelved by
the owner: a plan for later, not pursued now. Today every external program
runs with the person's authority, the approval says so, and oxplow only cleans
up its processes (Unix process groups, approved-code copies, revocation
watches). The plan text below stays as the design for when real OS
confinement is taken up.

Scope: Apply approved authority at host-service calls and semantic reads, external process launch, AI-provider network/credential access and component command bridges. Raw external processes need OS confinement or an explicitly consented broader-authority tier; a declaration alone cannot constrain them. Keep supported-platform enforcement explicit.

Acceptance: Path traversal/symlinks, redirects, subprocesses, stale component authority and changed scripts do not silently widen grants. Unsupported confinement fails closed for a restricted tier; an opt-in alternative honestly declares its broader access. Revocation stops new sensitive calls and handles in-flight work by a defined policy. Secret references are resolved by the host and not returned as extension-readable values.

Validation: Use adversarial fixtures for unauthorized semantic reads (including derived models), paths, destinations, redirects, credentials, child processes and bridge messages. Test the supported OS enforcement matrix and reapproval/revocation during execution.

Starting points: `crates/oxplow-app/src/exec_consent.rs`, `crates/oxplow-app/src/collector_runner.rs`, `crates/oxplow-ai-providers/src/scripted.rs`, `apps/desktop/src/lens/CustomComponentViz.tsx`.

Prerequisites: Define scoped extension grants and usage audit contracts.

#### Expose consistent extension run health and safe usage evidence

Status: planned.

Scope: Unify run identity, status, errors, cancellation, resource usage, grant decisions and data freshness across contributions. Publish semantic views and a small reusable inspector for extension authors and developers. Keep high-volume activity bounded and distinguish retryable execution failure from contract or permission rejection.

Acceptance: A developer can identify extension/contribution version, cause, active generation, used services and outcome without reading logs or secrets. Retry/backoff behavior is consistent and failures remain linked to their data effects. Evidence supports custom lenses and policy queries. Retention and sampling preserve audit significance without unbounded storage.

Validation: Run successful, rejected, cancelled and retrying contributions through the common views; assert ownership/cause linkage, no sensitive values, bounded retention and accurate freshness after failure.

Starting points: `crates/oxplow-app/src/extension_catalog.rs`, `crates/oxplow-db/src/effect_run_store.rs`, `crates/oxplow-app/src/collector_runner.rs`, `crates/oxplow-db/models`.

Prerequisites: Introduce a shared extension RunContext; Compose declared host services through the shared context; Define scoped extension grants and usage audit contracts.

### Composable host services and durable extension state

Give authors reusable data, analysis, AI, actions, state, identity, and job services without exposing raw database writes or requiring their own infrastructure.

#### Introduce a shared extension RunContext

Status: done. `oxplow_domain::run::RunContext` (built by `run_context.rs`)
carries identity, version and generation, actor, anchors, subject,
cause, mode, budget, cancel, trace, usage and authority through the bus,
scope calls, collectors, effects, the effort policy and provider invokes
(on the wire). Upcasts, report parsers, scripted AI providers and the MCP
adapter's mappings still run on their own fixed budgets.

Scope: Provide one execution context for extension/contribution/version and generation identity, actor, project/stream/thread, subject/anchors, cause/trace, cancellation and budgets. Fields depend on actual invocation context; absence is explicit. This is the basis for consistent host adapters, not permission to expose all services.

Acceptance: All migrated hosts preserve provenance and cancellation through nested calls. Tests can construct the same context as production. Context handles cannot outlive their owning activation or escalate grants. Deterministic/transactional execution modes remain explicit and documented.

Validation: Compare provenance from representative collector, effect, policy, provider and UI actions; cancel nested work; test missing context and expired handles.

Starting points: `crates/oxplow-script/src/lib.rs`, `crates/oxplow-script/src/scope.rs`, `crates/oxplow-app/src/scope_calls.rs`, `crates/oxplow-app/src/effort_policy_script.rs`.

Prerequisites: Normalize contribution descriptors and public extension terminology.

#### Compose declared host services through the shared context

Status: done. A script's services are scopes it declares in `needs:`
(`sql.read`, `tree.read`, `ai.compute`; `oxplow_domain::service::SERVICES`).
One router (`crates/oxplow-app/src/services.rs`) decides each one a script
uses on its `RunContext` (the mode first, so a pure run never asks a model;
then the declaration; then the authority; all recorded) and builds the
worker's host set. Every host goes through it: entity and fact collectors,
commands, effects and policies. A script that uses a service it doesn't
declare is a load error. Discovery is `list_services` (MCP), `oxplow
extension check --services` and the generated reference table.
`examples/extensions/triage` combines all three and runs end to end
(`crates/oxplow-sdk/tests/examples.rs`). Not here:
- A command's script reads the semantic layer, metric functions
  included, but not the tree: it decides what runs, and an effect or a
  collector reads the tree.
- The tree is shipped whole; a lazily read tree is a follow-up.
- References and audited commands stay where they were: commands compose
  through the bus.

Scope: Replace kind-exclusive Host access with a service router governed by RunContext, grants and execution mode. Expose read-only semantic queries, captured trees/AST/metrics, recorded AI, references and audited commands where allowed. Reuse implementations rather than adding a separate built-in API per extension point.

Acceptance: An extension can combine SQL, captured code analysis and recorded AI through supported composition. Pure/transactional code cannot perform blocking external I/O or raw writes; state-changing commands still pass the command bus and its policy. Semantic reads respect the approved data scope; declaring a cross-extension import does not itself grant access. Unsupported services fail with actionable diagnostics. SDK discovery lists availability, authority and input/output contracts.

Validation: Author a non-bundled contribution that combines services; compare behavior across hosts, test denied and unavailable services, and verify command auditing and transaction boundaries.

Starting points: `crates/oxplow-script/src/lib.rs`, `crates/oxplow-script/src/scope.rs`, `crates/oxplow-app/src/scope_calls.rs`, `crates/oxplow-app/src/ai_compute.rs`, `crates/oxplow-app/src/commands`.

Prerequisites: Introduce a shared extension RunContext; Define scoped extension grants and usage audit contracts.

#### Provide durable cancellable extension jobs outside database transactions

Status: in progress. The lifecycle contract now distinguishes durable intent,
attempt fencing, lease loss, observed cancellation and actual completion.
A bounded pure domain module is implemented, including cancellation of uncertain
retry-queued jobs and validation of persisted identity/state/counters. Dedicated
private persistence now integrates intent dedupe, revision/lifecycle CAS,
per-dispatch history and append-only late observations. History pages have row/
byte cursors; recovery and queued-work pages omit request payload. Explicit
savepoint rollback errors reach callers, and thin callers cannot bypass the host.
Strict private restart invocation context and atomic intent/context persistence
are implemented; duplicate deliveries retain original scheduling provenance.
The first public path now declares typed AI jobs, enqueues/cancels intent through
native commands, reads owned results, publishes exact operations with the active
generation and starts a bounded scheduler after coherent boot readiness. Fenced
completion emits a metadata-only event. Restart, duplicate and cancellation
behavior is pinned at each persistence boundary (`extension_job_hardening_tests.rs`,
`extension_job_crash_tests.rs`): a crash after the claim, before the publish
transaction and inside it; two dispatchers racing for one claim; concurrent
duplicate intents; the per-extension and project-wide caps and a changed
duplicate at capacity; a late result after recovery (kept once, readable in
`v_job_run_observation`); revocation between the worker and the publish; and
no write transaction held while a model streams. The replay policy is the
decision: `idempotent` jobs run again up to three attempts, `never` jobs end
`interrupted`. Status: done for the dispatcher; migrating a job's declaration
across versions and runtime resource limits beyond the caps remain pending.

Scope: Add a public asynchronous job path for AI/external computation, connected to events and commands by durable run identity. Define restart recovery, idempotency, bounded concurrency, timeouts, cancellation, result publication and generation ownership. Keep network/process work outside SQLite transaction closures.

Acceptance: Authors schedule work through a host API rather than spawning workers or inventing retry storage. Restart and duplicate delivery have specified behavior; commands/effects can receive recorded job results safely. Cancellation reaches actual execution; no stale job publishes under a new extension version. Non-idempotent external effects are not advertised as exactly-once.

Validation: Exercise restart at each persistence boundary, duplicate triggers, cancellation, resource limits and version replacement. Assert no DB transaction is held while waiting on external work.

Starting points: `crates/oxplow-app/src/effects.rs`, `crates/oxplow-app/src/event_pump.rs`, `crates/oxplow-app/src/ai_service.rs`, `crates/oxplow-db/src/effect_run_store.rs`.

Prerequisites: Compose declared host services through the shared context; Contain script CPU and memory and reap timed-out workers.

#### Provide namespaced extension state and configuration services

Status: in progress. Private namespace snapshots and fenced CAS persistence are
implemented. Versioned manifest configuration declarations validate with an
offline schema adapter and value-free diagnostics. Pure layer resolution and provenance are integrated with strict final validation.
State host ownership/live authority checks, transactional CAS publication and a
native snapshot read operation are wired. Shared script and provider routing now
serve typed private state reads with host-derived ownership; script waits stay
outside DB transactions. Native state CAS writes now return only a stamp and
withhold private values from audit/proposal/child metadata. Private configuration
override storage and authorized effective host services are integrated. Metadata
inspection and an atomic two-layer CAS support explicit schema migration; reset
retains revisions and disable/uninstall retain inaccessible rows. Declared
`migrations:` steps run at activation in the generation's transaction (a failing
step holds the extension back with its layers untouched), layers report
`needs_migration`, and a stale layer's read error is distinct from a stamp
conflict. Host-owned versioned secret references
now have human keychain ingress and a trusted remote MCP transport consumer;
references convey identity only and imported configuration cannot approve them.
Generation reader coordination is integrated. Configuration migration, restart,
isolation, race and host-denial behaviour is covered by `extension_config_tests.rs`;
state schema migration and secret-revocation runtime validation remain pending.

Scope: Add host-managed durable workflow state and typed extension configuration with migrations and personal/project scope. Distinguish mutable state, shared domain entities, rebuildable caches and preferences. Provide atomic updates, namespacing, access controls and host-owned secret references.

Acceptance: An extension can persist a workflow and user settings without a fake external provider or raw SQLite access. State and config have version/migration and uninstall policies; one extension cannot read or write another namespace without an explicit contract/grant. Secret values stay out of repo, SQL views and UI payloads. Imported team config cannot grant authority.

Validation: Cover concurrent updates, restart, migration failure/recovery, namespace isolation, disabled/uninstalled ownership and personal versus project precedence. Demonstrate a small triage workflow.

Starting points: `crates/oxplow-app/src/effective_config.rs`, `crates/oxplow-app/src/commands/config_commands.rs`, `crates/oxplow-app/src/extensions.rs`, `crates/oxplow-db`.

Prerequisites: Compose declared host services through the shared context.

### Consistent extension activation and public dependencies

Make activation, update, unload, and cross-extension integration coherent, versioned, owned, and diagnosable.

#### Stage and activate one coherent extension generation

Status: in progress. Import-aware authoring candidates are isolated. Installation
now stages and revalidates complete copies before replacement, with restoration
or retained backups on publication failure. Catalog metadata snapshots now pair
load identity with declarations; command replacement swaps one registry atomically
and retains prior handlers on registration failure with their original provenance.
The bus also supplies atomic multi-source replacement, and the DB supplies
strict model-set replacement with structured rejection and savepoint rollback.
Command reconciliation now stages all enabled replacements from one catalog
snapshot, publishes with a registry identity fence, and rejects enabled load
errors instead of exposing a partial command set. Explicit disable/removal and
live owner guards remain independent of candidate acceptance. Publication races
restage up to three times. A prepared database candidate now composes strict
model and vocabulary replacement from one catalog snapshot; rejection of either
restores both inside the caller's transaction. Vocabulary candidates also reject
enabled load errors and invalid host recognizers. Live model/vocabulary reactors
remain partial until shared coordinator adoption. Capability reads now resolve
one captured state; prepared capability replacements fence registry/choice
changes. The database candidate also includes contribution descriptors and
optional planned capability rows/switch events, with vocabulary recognition from
that planned selection. Provider preparation, native/external adapters, immutable component bundles,
strict database/metric publication and pinned runtime reads now join one boot
activation owner. A required-bundled repair baseline precedes full activation;
rejection retains the baseline/last coherent generation. Raw candidates and
runtime metadata are separate, with immediate disable/removal revocation.
Reader coverage and runtime acceptance validation remain incomplete.
Startup now scans bounded, marked install staging before the first catalog read:
it restores a missing target from a retained prior copy, never publishes an
unconfirmed first-install candidate and preserves ambiguous backups. Plain
directory checks, file/directory sync barriers and an in-process publication lock
are integrated; cross-process filesystem races and crash injection need review.
Composed command routing now retains each child's own contribution context,
authority and generation through transactional and external execution, including
the implicit backing scope of native operations.

Scope: Build and validate a candidate catalog including semantic models, vocabulary, commands, providers, capabilities and UI contribution metadata. Activate it through one coherent generation boundary; pinned runs and queries retain the generation that owns them. Define coordination across DB-backed and in-memory registries.

Acceptance: A failed update leaves the last good activation usable and reports candidate errors. New contributions are not partially exposed. A query or invocation sees compatible model/command/provider/UI metadata from one generation. Disabled initial candidates expose no runnable authority. Existing diagnostic partial-load behavior has an explicit compatibility decision. Decision: an extension that can't load is held at its last working version (or left out when new) and the rest activate together; only a required bundled extension's errors reject the candidate.

Validation: Inject failure into each registration step and test hot update while queries/actions run. Assert rollback/last-good behavior and no mixed generation or leaked registration.

Starting points: `crates/oxplow-app/src/extension_catalog.rs`, `crates/oxplow-app/src/extension_models.rs`, `crates/oxplow-app/src/config_reactors.rs`, `crates/oxplow-app/src/extensions/custom_components.rs`.

Prerequisites: Normalize contribution descriptors and public extension terminology; Introduce a shared extension RunContext.

#### Own and dispose extension resources on disable and update

Status: planned.

Scope: Create a lifetime owner per active extension generation for jobs, providers, listeners, timers, handles and caches. Define whether in-flight operations finish under a pinned generation or cancel, including revocation. Dispose owned children predictably on disable, replacement and project close.

Acceptance: No new calls use disposed handles; old contributions cannot keep running secretly after disable. Cleanup is idempotent even after partial activation failure. Bounded completion/cancellation policies are inspectable, and completed history remains attributable to its original version.

Validation: Repeat enable/update/disable cycles with long jobs, subscriptions and UI bridges. Check worker/resource counts, cleanup after errors and stale-handle rejection.

Starting points: `crates/oxplow-app/src/extension_catalog.rs`, `crates/oxplow-app/src/config_reactors.rs`, `crates/oxplow-app/src/providers`, `crates/oxplow-app/src/effects.rs`.

Prerequisites: Stage and activate one coherent extension generation; Provide durable cancellable extension jobs outside database transactions.

#### Declare versioned public imports and cross-extension contracts

Status: in progress. Canonical identity, visibility, exact contract versions,
required/optional outcomes and deterministic dependency validation are fixed.
The pure validator is integrated with manifest declarations, public model SQL
bindings, optional contribution guards, export withdrawal and dependency
diagnostics. Authoring/impact overlays and install preflight resolve fresh
candidate catalogs and reject downstream contract regressions. Coherent runtime
activation and behavioral validation remain; no runtime tests were added or run.

Scope: Add explicit dependencies and optional dependencies for exported semantic entities, events, commands and services. Distinguish extension package versions, engine compatibility and individual public contract versions. Diagnose missing/incompatible imports and cycles before activation. Evaluate a minimal typed exported extension point only where ordinary contracts cannot express the integration.

Acceptance: Two independent extensions can integrate without relying on internal names or a core patch. Disabled optional dependencies degrade by contract; required dependencies block the affected activation with an actionable explanation. Existing foreign-event graph support is retained. Private and experimental exports do not silently become stable APIs.

Validation: Use producer/consumer fixtures with compatible, breaking, optional, missing and cyclic imports. Test disable/update compatibility and confirm the loader rejects dependency ambiguity.

Starting points: `crates/oxplow-app/src/extensions/manifest_v2.rs`, `crates/oxplow-app/src/extension_models.rs`, `crates/oxplow-app/src/extension_event_types.rs`, `crates/oxplow-app/src/extension_commands.rs`.

Prerequisites: Normalize contribution descriptors and public extension terminology.

#### Order contributions by public IDs with predictable event delivery

Status: planned.

Scope: Normalize before/after ordering on contribution IDs rather than internal aggregate-consumer names. Compile dependency/order graphs and explain conflicts. Preserve delivery checkpoints, retry, idempotency and existing cross-extension event edges; document the difference between ordering, dependency and causality.

Acceptance: Unknown ordering targets and cycles produce errors rather than being ignored. Separate extensions can order against public contributions. Replay/retry retains specified delivery behavior, and an author can inspect the compiled ordering and explain why a contribution ran.

Validation: Test multiple effects in one extension and across extensions, missing/disabled targets, cycles, duplicate deliveries and restart replay; compare existing event graph fixtures.

Starting points: `crates/oxplow-app/src/event_pump.rs`, `crates/oxplow-app/src/effect_triggers.rs`, `crates/oxplow-app/src/extensions/manifest_v2.rs`.

Prerequisites: Use one versioned event envelope across runtime and authoring; Declare versioned public imports and cross-extension contracts.

### AI services that support new model capabilities

Support evolving AI functionality through discoverable typed services and durable cancellable jobs, retaining simple helpers and auditable usage.

#### Add AI capability discovery and typed request/response parts

Status: in progress. Versioned neutral capability/part envelopes and conservative
legacy adapters are implemented. Collectors can discover declared metadata
without keychain access and use recorded typed completion through configured
roles; scripted providers can opt into typed transport. Authorized image
asset resolution and runtime validation remain. Shipped Anthropic and
OpenAI-compatible mappings now support typed history/tools, finish categories
and optional observed usage; unsupported transport shapes fail explicitly.

Scope: Extend the provider contract beyond one prompt-string/JSON POST: discover models and capability metadata, typed text/image/tool parts, supported limits and usage/token estimation where available. Keep adapters for existing providers and simple complete/decide callers.

Acceptance: Extensions discover supported features before requesting them. Unsupported modalities/tool operations fail clearly; estimates are distinguished from actual usage. Request/response and capability schemas are versioned and independent of a particular vendor. Discovery does not expose credentials or automatically invoke paid calls.

Validation: Use fake providers with distinct capabilities plus existing scripted provider fixtures. Cover mixed parts, unsupported features, capability changes and backward-compatible text helpers.

Starting points: `crates/oxplow-ai/src/client.rs`, `crates/oxplow-ai/src/config.rs`, `crates/oxplow-ai-providers/src/scripted.rs`, `crates/oxplow-app/src/ai_service.rs`.

Prerequisites: Compose declared host services through the shared context.

#### Expose cancellable AI streams and tool lifecycles through jobs

Status: working end to end. A job's model rounds stream (`stream_typed`,
SSE through a script's `delta`, the whole reply at once from a provider
that doesn't stream) into its attempt's output (`v_job_run_output`, `v_job_run`);
cancelling drops the model's connection and the job ends `cancelled`
with what it had, its call recorded as cancelled; a job's declared
`tools` (its extension's read-only commands admitting `invokers.job`, and
`query_sql` with `sql.read`) run under the job's run and answer the model,
for at most `max_tool_rounds`; `timeout` fails it; its end is
`contribution.job.completed` with every outcome. A person cancels any job
(`oxplow.job.cancel`, the bundled Jobs lens). Still to do: the vendor
streaming mappings (Anthropic, OpenAI) and the hardening tests (lease
expiry, revocation and generation change mid-run, restart, capacity).
An `idempotent` job cut off by a restart or a lost lease runs again, up
to three attempts, its end announced once. See [extensions.md](./extensions.md)
"Jobs" and [ai-providers.md](./ai-providers.md) "Streaming and cancelling".

Scope: Build AI streaming on durable job identity with cancellation, incremental output and completion/error events. Route model-requested tool operations through explicitly granted host commands/services with bounded recursion and usage. Keep interactive sessions and their input authority separate from model inference.

Acceptance: An extension can consume partial output and cancel actual work without holding a DB transaction. Tool calls have typed arguments/results, provenance and permission checks; a model cannot acquire authority by requesting a tool. Restart and partial-stream handling are specified. No implicit ability to drive a human agent session is added.

Validation: Test interleaved text/tool chunks, cancellation, malformed tool arguments, denial, budget exhaustion, disconnect/restart and final usage recording with a fake streaming provider.

Starting points: `crates/oxplow-ai/src/client.rs`, `crates/oxplow-app/src/ai_service.rs`, `crates/oxplow-app/src/ai_compute.rs`, `crates/oxplow-ai-providers/src/scripted.rs`.

Prerequisites: Add AI capability discovery and typed request/response parts; Provide durable cancellable extension jobs outside database transactions; Enforce scoped grants across hosts and external execution.

#### Add extension AI profiles with compatible high-level helpers

Status: planned.

Scope: Let extensions declare named AI operations/profiles beyond the fixed main/summarize/decide roles, with schemas, binding preferences and host-granted invocation. Retain simple helpers and project/user binding rules. Pin model, provider, operation version and relevant inputs to a recorded run; define cache compatibility and failures.

Acceptance: Different extensions can select appropriate configured capabilities without global role proliferation. Existing role calls remain usable. Provider access and usage remain host-owned and auditable. Profile declarations do not authorize spending, inject credentials or bypass restricted bindings. Cached results retain original provenance.

Validation: Test profile discovery, overrides, missing/incompatible models, old-role adapters, cache keys after model/profile updates, and recorded call attribution.

Starting points: `crates/oxplow-ai/src/config.rs`, `crates/oxplow-app/src/ai_compute.rs`, `crates/oxplow-app/src/ai_service.rs`, `crates/oxplow-app/src/effective_config.rs`.

Prerequisites: Add AI capability discovery and typed request/response parts; Provide durable cancellable extension jobs outside database transactions.

### Built-in policies through public extension APIs

Move optional opinions out of core while retaining common evidence, analysis mechanisms, safe operations, and host guarantees.

#### Extract inferred-decision policy into a bundled extension

Status: planned.

Scope: Move the fixed effort-close inference prompt and scheduling policy out of the unconditional Rust reactor into a replaceable bundled contribution using public jobs, AI profiles, event inputs and recorded outputs. Retain safe decision storage/review mechanisms and separation of inferred versus confirmed evidence.

Acceptance: Default behavior remains available when selected and configured; disable/replace/customize needs no core patch. Missing models and rejected calls are diagnosable. Inferred decisions are not silently confirmed or fed back as agent truth. Extraction uses only services available to non-bundled extensions.

Validation: Run the current inference fixtures through the public extension path; test disabled/unconfigured/replaced policy, duplicate close events, cancellation and confirm/dismiss behavior.

Starting points: `crates/oxplow-app/src/inferred_decisions.rs`, `crates/oxplow-app/src/effort_reactors.rs`, `crates/oxplow-app/src/commands/review.rs`, `extensions/oxplow-bundled/extension.yaml`.

Prerequisites: Provide durable cancellable extension jobs outside database transactions; Add extension AI profiles with compatible high-level helpers; Use one versioned event envelope across runtime and authoring.

#### Extract work-item repair response from core contribution health

Status: done. Core keeps detection and the health report; the bundled
effect `oxplow-bundled/repair-disabled` files (or comments on) the repair
item through `oxplow.work_item.create` / `comment` on the active work list,
deduplicating by the open item's `Repair <extension> <contribution>:` title
prefix (no stable work-item key exists yet), once a person approves it. An
effect whose event carries no thread is an anchorless effect (no writer
gate). `contribution_repair.rs`, the `contribution.repair` consumer and
`contribution_health.repair_item`/`repair_seq` are gone (V58).

Scope: Keep core contribution validation and health reporting, but move automatic repair-task creation and its wording/deduplication policy into a bundled effect. Use the public event envelope, command bus and selected work-list provider.

Acceptance: Core detects/report errors even when no work list or repair policy is enabled. The selected extension can create or link repair items using any compatible provider, without directly reading task tables. Repeated failures deduplicate by stable cause/contribution identity; disabling the policy prevents new repair items.

Validation: Exercise healthy/failing contributions, repeated events, no work list, a fake alternate provider, disabled policy and recovered contribution. Verify audit and links.

Starting points: `crates/oxplow-app/src/contribution_repair.rs`, `crates/oxplow-app/src/extension_catalog.rs`, `extensions/oxplow-bundled/extension.yaml`, `crates/oxplow-app/src/commands`.

Prerequisites: Use one versioned event envelope across runtime and authoring; Compose declared host services through the shared context.

#### Separate reusable analysis evidence from default review policy

Status: done. Core keeps evidence; the judgements it still made moved to
the default extensions: an effort's verdict is the `effort.verdict`
replacement (oxplow-bundled's lens; core shows none), claim backing is
oxplow-bundled's `claim_status`, struggle its `struggle`, and per-effort
test outcome an oxplow-foundation collector. A value at a metric
threshold hasn't crossed it, on the server and the Metrics page alike.
`examples/extensions/schema-review` is the alternative review. The
inventory, what stays in core and why, and the gaps are in
[work-tracking.md](./work-tracking.md) "Review: evidence in core, policy
in extensions".

Scope: Inventory core test/risk/review heuristics and distinguish common collection/AST/metrics/change-analysis mechanisms from optional interpretation and workflow policy. Extract the highest-value hard-coded policies through public contracts; publish the remaining boundaries and rationale rather than attempting a wholesale analysis rewrite.

Acceptance: At least one alternative review policy can combine test evidence, custom business metrics/dimensions and code analysis without changing core. Defaults remain usable as bundled configuration/contributions. Attribution, evidence freshness, schema validation and safe operations remain host responsibilities.

Validation: Compare default analysis results before/after extraction on a representative change; build an alternate domain-specific review and ensure it joins existing evidence with custom collected facts.

Starting points: `crates/oxplow-app/src/change_reactor.rs`, `crates/oxplow-app/src/collector_runner.rs`, `crates/oxplow-db/models`, `extensions/oxplow-bundled/extension.yaml`.

Prerequisites: Normalize contribution descriptors and public extension terminology; Compose declared host services through the shared context.

### Composable presentation and agent authoring

Make personal and shared extensions practical to author, validate, preview, and combine through the same public APIs available to bundled extensions.

#### Give bundled and project custom components the same public runtime

Status: planned.

Scope: Allow bundled component files to use the same sandboxed component bridge as project files. Resolve assets from their actual origin and bind immutable content, commands and approved grants to an activation generation. Avoid bundling-only privileges.

Acceptance: The same component can run bundled or from a project with equivalent props/query/command behavior and authority. Modified code cannot keep an approval tied to another version. UI-token, storage/network and message-origin restrictions hold for both origins.

Validation: Run the same component fixture in both origins, hot update and revoke it, and test rejected bridge messages and undeclared commands.

Starting points: `crates/oxplow-app/src/extensions/custom_components.rs`, `crates/oxplow-app/src/bundled_extensions.rs`, `apps/desktop/src/lens/CustomComponentViz.tsx`.

Prerequisites: Stage and activate one coherent extension generation; Enforce scoped grants across hosts and external execution.

#### Provide provider-independent custom views, forms and workflow UI

Status: in progress. The Board, the Tasks list and a work item's editor
are unbound replacement targets any extension may replace with no
provider (`work_item.board`, `work_item.list`, `work_item.detail`); only
the state control stays bound to its item's provider. Contracts are
versioned, published as `v_replaceable` and generated into the
reference. The person picks whose view shows (`replacements`, Settings →
Integrations). Lens actions open their command's form (`form: true`,
`fixed`), form lenses take `fixed` and `fields`, a refused field shows
under it in every form, and component bundles may hold forms (bridge
protocol 4). The triage example keeps workflow decisions in private
state through its own commands. Still to do: the task-views example and
its browser spec. See [extensions.md](./extensions.md) "Replacements".

Scope: Expand the existing task-screen follow-up into general presentation primitives: detail/list/board mounting, forms, editing, reorder and navigation over references and host actions. Decouple choosing a presentation from installing a work-list provider. Integrate durable state/config instead of making authors impersonate a provider.

Acceptance: A custom task view works with the existing provider, and a non-task triage view can persist workflow state. Commands, link navigation, accessibility and validation use shared primitives. Default core inspection remains reachable. Public mount/props contracts are discoverable and versioned; the scope is a useful set of mounts, not arbitrary replacement of every app surface.

Validation: Build a custom task detail/board using two providers and a separate triage form using namespaced state. Exercise editing, reorder, navigation, invalid input and permission denial.

Starting points: `crates/oxplow-app/src/extensions/replacements.rs`, `apps/desktop/src/lens/useReplacement.ts`, `apps/desktop/src/lens/CustomComponentViz.tsx`, `crates/oxplow-app/src/extensions/schema.rs`.

Prerequisites: Provide namespaced extension state and configuration services.

#### Preview all contribution kinds in a consistent development host

Status: planned.

Scope: Unify validation, fixture execution and preview against an isolated throwaway host with the same staged generation and service contracts as production. Include events, models, providers, effects, jobs and custom UI; show requirements and diagnostics before any external action.

Acceptance: An agent can validate and preview a whole extension without publishing/installing it or touching production data. External/paid access remains consented or mocked; no copied credentials or accidental agent-terminal authority. Preview exposes the generation, contributed APIs and unsupported surfaces. Runtime/fixture schemas agree.

Validation: Preview an extension combining a model, event, effect, job and component; compare with a temporary production host, inject invalid changes, and assert isolation and cleanup.

Starting points: `crates/oxplow-sdk/src/throwaway.rs`, `crates/oxplow-sdk/src/extension_test.rs`, `crates/oxplow-sdk/src/conformance.rs`, `crates/oxplow-app/src/extensions.rs`.

Prerequisites: Stage and activate one coherent extension generation; Own and dispose extension resources on disable and update.

#### Test agent-authored collection, metrics and dimensions end to end

Status: done. `oxplow extension new metric` and `oxplow.metric.scaffold`
are one template (`oxplow_app::metric_scaffold`): a private extension with
a per-path measure, a Starlark fact collector over `source_files()` +
`code_metrics()`, the `<ns>.area` dimension, the metric and a lens linking
`file` refs, with a fixture `extension test` runs. `metrics_authoring_tests`
(`oxplow-app`) takes it from the scaffold through activation, a baseline
capture, a delta capture that carries untouched files forward, slicing by
area and a dimension filter, the lens rows' file and line refs, the
documented refusals (bare dimension key, fact outside `facts:`, undefined
measure) and a run that reports the oversize and binary files it left out
(`BaselineReport.excluded`, `SweepReport.excluded`). `check` now dry-runs a
lens reading `metric_grid()` / `metric_findings()`. Not exercised: joining
the facts with another entity in the lens, and a stale-input case beyond
the fingerprint re-baseline. The old `metric_scaffold` project-config path,
`entries_yaml` and the gauge wording are gone.

Scope: Update the old gauge authoring task to the current collector/fact/measure/metric/dimension model. Exercise files, AST queries and code metrics through the public authoring path, then join the resulting data with another entity and present it in a lens with reference drill-down.

Acceptance: A non-bundled fixture created through current SDK/agent tools produces correct facts and dimensioned aggregate values across changes. A developer can connect custom collection to existing effort/test/code evidence without a new core command. Stale inputs, truncation and invalid dimensions report clear errors. Obsolete scaffold_metric/gauges assumptions are removed.

Validation: Use a small source corpus, edit it between captures, validate/run the collector and lens, and assert semantic query totals, dimension filtering, freshness and drill-down refs.

Starting points: `crates/oxplow-sdk/src/lib.rs`, `crates/oxplow-script/src/helpers.rs`, `crates/oxplow-app/src/collector_runner.rs`, `docs/guide/metrics.md`.

Prerequisites: Compose declared host services through the shared context.

#### Prove public APIs with diverse agent-authored extension trials

Status: done. Three trials, tested through public commands and the SDK
only (`examples/extensions/README.md` lists them, the tests and the
friction found): effort-review (a job started by an effect on
`effort.finished`, its tool reading an effort's evidence, streamed,
cancelled through `oxplow.job.cancel`, commented on the work item and
exported as a model); triage's workflow state surviving restart,
revocation and re-approval (`crates/oxplow-sdk/tests/trials.rs`, the
browser spec); review-badges importing effort-review's model and
degrading when it is disabled, missing or exports another version.
Shared services added for them: a job's intent key as its subject
(snapshot `intentKey`, `v_job_run.intent_key`), a command fixture's `expect:
{ result }`, and `extension check`/`test` printing how each import
resolved. Still open (in the README's friction list): event-started jobs,
AI profiles, diff text, job fixtures over seeded evidence, the
builtin-shadowing scanner, withheld-contribution diagnostics, metric
functions in command scripts.

Scope: Build three small non-bundled trials: cancellable AI review over collected evidence, a durable triage workflow with a custom UI, and a producer/consumer integration between independent extensions. Reuse the custom metrics trial as the data-first baseline. Improve shared services when a trial otherwise needs copied infrastructure.

Acceptance: All trials validate and preview through the SDK, have documented requirements/grants, share without machine credentials, and survive reload/revocation with useful diagnostics. They use no private core paths. Document remaining limitations and any measured authoring friction. The tests prove composition and guardrails rather than adding a marketplace.

Validation: Run SDK/conformance and focused application tests for end-to-end trials, plus UI verification for the triage view. Review API discovery, ordinary agent authoring steps, failure repair and public-contract compatibility.

Starting points: `examples/extensions`, `crates/oxplow-sdk/src/conformance.rs`, `crates/oxplow-agent-text/assets/skills/oxplow-extension/SKILL.md`, `docs/reference/extensions.md`.

Prerequisites: Give bundled and project custom components the same public runtime; Provide provider-independent custom views, forms and workflow UI; Preview all contribution kinds in a consistent development host; Expose cancellable AI streams and tool lifecycles through jobs; Extract inferred-decision policy into a bundled extension; Declare versioned public imports and cross-extension contracts; Test agent-authored collection, metrics and dimensions end to end.
