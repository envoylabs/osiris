# Osiris: a harness for parallel agent work

**Status:** draft, 2026-11-03; amended 2026-11-05; brought in line
with the built system 2026-11-11. The first slice is built and osiris
is developed with itself, three seats at once. This document is the
design; the work still to do is in the harness's own log (`ris ready`,
`ris report`), not here.

## Goal

Let one person direct 1–5 coding agents working in parallel on a
codebase, with the person deciding and the agents building and testing.
The person's attention goes to decisions and to phase-end review, not to
running tests, approving edits one at a time, or keeping status
documents in step.

The harness is portable and does not depend on any one agent vendor.
On a new machine the steps are: `git pull` the harness, let an agent set
it up, start work. It drives a working method rather than fitting
itself to the workflow of any one project; each codebase brings its own
assumptions through a manifest.

## Principles

1. **A decision-making harness as much as a coordination harness.** The
   aim is to amortise the cost of governance, deciding and with it
   coordinating, so the human's cost per phase stays close to constant
   rather than growing linearly with the number of items or agents. The
   mechanisms:
   - decisions are batched to phase boundaries rather than taken one by
     one as work hits them;
   - a recorded decision is memoized reasoning: decided once, reused by
     every item it touches, never re-derived or re-argued;
   - mechanical decisions never reach the human (the decision rule
     below);
   - review is one report per phase, not one approval per change.

2. **Centaur, not reverse centaur.** Each side does what it is best at.
   The human does design and decisions; the automation does the
   mechanical steps: running checks, sequencing, integrating, recording
   state, assembling reports. A human doing a mechanical step is a
   defect in the harness, not a working arrangement. The usual cause is
   an environment where the agents cannot run the real checks
   themselves; isolated per-agent environments are how the harness
   removes it.

3. **Verify, don't trust; settled facts are final.** Facts are reified
   in the harness as evidenced things. Trust in the moment is what lets
   work run uninterrupted: an agent acts on a mechanical decision, or
   on another agent's report, without waiting. But once something is
   settled (code written and integrated, a decision confirmed at
   review, a design record written), it becomes **final**: a fact the
   harness holds with its provenance, which can be
   - **looked up and introspected:** what was settled, when, by whom,
     on what evidence, and what depends on it; and
   - **verified or falsified at any time:** every final claim carries
     the check that would show it false, and re-running that check
     against the current tree either confirms it or reopens it.
   Final facts change only by an explicit new decision that supersedes
   them, never by being edited in place.

## The cost model

The model is an anchoring design's: one commitment per window covers
any number of transactions, so cost is a function of windows, not of
participants or volume.

| anchoring design | harness |
|---|---|
| transactions submitted in a window | agent output: mechanical decisions, check results, asserted claims |
| the window | the phase (about half a day by default) |
| closing the window's set | closing a phase: asserted claims become final |
| one anchoring per window, whatever the volume | one human review per phase, whatever the number of agents |
| one slot per key per window, last write wins | the human reviews an item's final state, not its churn |
| verifier cost proportional to windows crossed | catch-up and introspection cost proportional to phases crossed; finality means mostly only the head is needed |
| a hash chain with no gaps | each phase report carries the previous one's hash, so nothing is settled outside a phase |

**Claim:** the human's cost is proportional to the number of phases, that
is to elapsed time, not to agents × items. Adding an agent adds work
inside a window, not reviews.

Three conditions make it hold:

1. **The report grows sublinearly with the phase's work.** Substantive
   decisions and exceptions (failed checks, reclassified decisions,
   falsified claims) are read in full; mechanical work appears as counts
   with drill-down on request.
2. **A decision budget per phase.** The number of substantive decisions a
   phase may queue is capped to the human's attention at a check-in.
   Similar questions are merged; anything over budget waits for the next
   window, and agents keep working on what it does not block. More
   agents mean more surface and more genuine questions, so without the
   cap the cost per window grows with the agent count.
3. **The window is chosen by the budget, not by habit.** Longer phases
   mean fewer reviews but larger reports, later finality and more
   provisional work at risk when a decision is overturned; shorter
   phases mean more reviews. Pick the longest window whose decisions fit
   the budget and whose blast radius on an overturned decision is
   acceptable.

## What it fixes

These costs come from a long multi-phase project (a Rust workspace of
about a dozen crates, built over several months by one person directing
one agent at a time):

1. **The human was in the inner loop.** The agent's environment could not
   run the real test suite (no Docker, no port binding), so the person
   ran almost every gate, and every failure cost a round trip.
2. **Decisions arrived mid-implementation.** Each one stopped work,
   waited for the person and resumed with context rebuilt.
3. **State was recorded several times.** One placement decision meant
   edits to an item entry, a step table and a decisions list; resuming
   work relied on a growing free-text pick-up note.
4. **Ordering was serialised and kept being renegotiated,** because
   every item queued on the same human-run gate.
5. **Standing rules lived in three places** (global agent instructions,
   agent memory, a code-practice ADR), each phrased for a different
   reader.
6. **One shared backlog file** (about 4,800 lines) was the source of
   truth for status, so it was expensive to load and would conflict
   under parallel edits.

## Working model

### Phases

A phase fits a working day: by default at most two a day, with
check-ins at the start of the day (plan, answer decisions), midday
(review) and before close of play (a vibe check). When the check-ins
fall is configurable (Window schedule, below). Between check-ins the
agents never wait for the person.

1. **Plan.** From a goal, an agent drafts the items and the decisions
   they need. The person answers the decision batch in one sitting.
2. **Build.** Agents claim items and work in parallel, each in its own
   worktree and environment.
3. **Done means evidence.** An item is done when its checks pass and the
   evidence (test output, benchmark files, logs) is attached.
4. **Merge.** A finished item merges onto the phase's one branch as soon as
   it is done, not at the end of the window: it is rebased onto the
   head, and the full checks run again on the result (Commits).
5. **Review.** At phase end the person reads one generated report:
   decisions taken, diffs, evidence, deviations from the rules. Anything
   the person overturns becomes a new item.

### Window schedule

When the check-ins fall is configuration, not design. The default of
two decision check-ins a day (start of the morning, start of the
afternoon) and a close-out is a prior, to be tested against the data
below; some people and some projects will want more windows.

A schedule is a list of boundaries. It lives in the personal rule
layer, and a project or environment layer may override it. A boundary
takes one of two forms:

```toml
[windows]
boundaries = ["09:00", "13:30"]   # wall-clock, local time
close_out  = "17:30"

# or offsets from the start of the session:
# boundaries = ["+2h", "+4h", "+6h"]
# close_out  = "+8h"
```

- **Wall-clock or offset.** Wall-clock times suit a day with a fixed
  shape; offsets suit a day whose start moves. To the harness they are
  the same thing: a time at which the shell issues a command.
- **A session, not a calendar.** A session starts when the person does
  the first check-in of the day, which is always the person's act.
  Boundaries fire only inside a session, so the schedule needs times of
  day and none of cron's recurrence across weekdays or months.
- **A boundary is a target, not a close.** At a boundary the daemon
  assembles the report and the decision batch. The phase closes when
  the person completes the review, whether early, on time or late; the
  agents carry on meanwhile with whatever is not blocked. The person
  can also check in before a boundary. Checking in on demand is
  therefore the simplest schedule, and the first slice needs no timer.
- **Time enters the core as a command.** The pure core never reads the
  clock or the schedule; the shell turns a timer, or the person's
  check-in, into a command. Replay does not depend on the
  configuration, and the log records when check-ins happened rather
  than when they were meant to.

**What each window records,** so that schedules can be compared: its
scheduled and actual length; items done; substantive decisions queued
against the budget, and when the budget filled; agent time with no
unblocked item to take; the person's minutes at the check-in; the size
of the report; decisions overturned, and the provisional work discarded
with them. These make the cost model's third condition measurable. The
budget filling, or agents running out of unblocked work, well before
the boundary says the window is too long. Check-ins with little to
decide say it is too short.

### The decision rule

- **Mechanical decisions: the agent goes ahead and logs it.** Blockers
  found during an item, adjacent things discovered along the way, and
  the way an implementation unfolds while working an issue.
- **Substantive decisions: queued for the person.** The test is the cost
  of reversing, not whether reversal is possible: a decision can be
  reversible and still expensive to reverse. Substantive if it changes a
  contract (an API, a data or wire format, a public interface), adds or
  swaps a dependency, changes the phase's scope or the order of other
  agents' items, or sets a design direction other work will build on.
  The agent records it with options and a recommendation, sets that item
  aside and takes another.
- **Misclassification is caught at review.** The phase report lists the
  mechanical decisions too, one line each, so one that should have been
  substantive is seen at the next check-in.

### Checks, not gates

A check is a command and its pass condition, declared per project. For
most projects it is "the tests are green". A phase pauses when its items
are done or blocked and its substantive decisions are waiting; usually
nothing needs running at that point, only the decision queue draining.

### Commits

The item is the unit of change in git, as it is in the harness. Each
phase has one branch, and every item of the phase merges onto it.

- **One branch per phase.** It starts from the main line when the phase
  opens, and is checked out nowhere: every writer, the lead included,
  works in a worktree of its own, and a merge moves the phase branch by
  a compare-and-swap ref update, which needs only the git directory, so
  a sandboxed seat can merge without writing the person's checkout. The
  main checkout is the person's and stays on the main line. Nothing is reconciled at the end of the window: reconciling
  happens at each merge. Provided nothing else writes to the main
  line during the phase, the closing merge has no conflicts.
- **Merging is a command, and the daemon takes one at a time.**
  Merges therefore have a total order, the order of the log. To merge,
  an item is rebased onto the branch's head and the full checks run on
  the result. Those include the checks of every item merged before it,
  so the later one builds on the earlier: undoing an earlier change
  fails that change's falsifier. A merge that conflicts or fails the
  checks is refused and the item returns to its author, who has
  meanwhile taken another.
- **A change is protected by its falsifier, not by its branch.** At
  phase close `verify all` re-runs every claim asserted in the phase
  against the final tree. The protection is as strong as the checks: a
  change with weak checks can be undone unnoticed by a later rebase.
  Whether its lines are still present is no test, since later items
  rightly rewrite them.
- **Only finished items merge.** Half-done work on the shared branch
  would leave other agents building on a tree whose failures are not
  theirs.
- **Each writer has a working ref.** A writer's worktree is on a branch
  of its own, on which it commits, amends and rebases as it likes, one
  item at a time; the claim keeps other writers off. Only with phase
  branches off does a single writer commit straight onto the main line
  in the main checkout.
- **One commit per item.** A finished item merges as a single commit
  with the item's id in its message, so `why <file>` can go from a line
  to its commit, its item, the decisions that touched it and its
  evidence. A commit is as large as its item, so small commits come
  from decomposition at planning time. Inside an item, the logged
  mechanical decisions are the breakdown.
- **Fix forward once merged.** Other agents may have built on the
  phase's branch, so it is not rewritten: an accepted finding, or a
  decision overturned at review, merges as a further commit against the
  same item or a new one.
- **Nothing is pushed before the phase boundary.** The phase's branch
  is pushed at phase close, after the review, when state is exported
  too. Inside the window everything is local, so a confidentiality leak
  caught there can still be removed by rewriting local history. That is
  the one exception to fixing forward.
- **One merge commit per phase.** The phase's branch merges into the
  main line without fast-forward. `git log --first-parent` then shows
  one commit per phase, and the full log shows the items.
- **Commits carry the git identity configured on the machine.** That is
  the user's to set, and the harness does not override it for agents'
  commits. Which agent wrote an item is in the log, under the agent's
  name.
- **Commit messages are agent-written free text,** and pass the
  confidentiality scan with the diff. They follow the conventional
  style, `type(scope): summary`, so a log scans at a glance, with the
  item's id as a trailer.
- **Co-authorship is recorded by alias.** Each commit carries an
  `Agent: <name>` trailer naming the seat that did the item, and merge
  refuses one that does not match the item's claimant. The alias is
  what a reader needs; the model behind a seat is in the seat's logged
  binding. Trailers that carry an email address are not used, so the
  message scan needs no exceptions: a commit message is free text, and
  any rule that skips lines by their prefix would let anything through
  under that prefix.

### Arrangements: width and audit

A setup has two independent properties. The number of agents follows
from them; it does not define them.

- **Width:** how many items are being written at once. Each builder is
  a writer, with its own worktree and environment. Width above one
  needs items that do not collide, a working tree per builder and
  per-agent environments.
- **Audit:** whether an agent other than an item's author examines the
  item before review. The auditor never writes to the tree. The checks'
  output is already evidence; the auditor judges what a check cannot:
  whether the diff does what the item asked, whether the evidence
  supports the claim that it is done, whether the rules were kept, and
  whether any decision logged as mechanical should have been
  substantive.

| arrangement | width | audit | needs beyond the core |
|---|---|---|---|
| solo | 1 | off | nothing |
| pair: a lead and an auditor | 1 | on | messages |
| parallel | 2 to 5 | on or off | decomposition, a working tree per builder, per-agent environments, a runner |

A pair is not parallel operation with two agents. Two builders are two
writers: they need items that do not collide and a working tree each,
and what they buy is throughput. A lead and an auditor are one writer:
there is nothing to merge and no second environment, and what they buy
is independent verification, the claim that an item is done examined by
an agent that did not write it. The windows, the decision rule, the
budget and the review are the same in every arrangement, and audit can
be added at any width.

What audit adds to the harness:

- **Author and auditor differ.** An agent cannot audit an item it
  wrote; the core refuses it. Where two runners are available the
  auditor runs on a different model from the author, so the two are
  less likely to share a blind spot.
- **The author does not wait.** It marks the item done on its checks
  and takes the next. A finding the author accepts reopens the item. A
  finding the author disputes is queued for the person as a decision,
  inside the budget. Audit therefore costs agent time, and the person's
  only through disputed findings, which the budget caps.
- **A finding is evidence, not a falsifier.** It is a judgement and
  cannot be re-run, so finality still rests on checks. A finding worth
  keeping is turned into a check.
- **The report shows audit by exception:** counts of items audited and
  findings accepted, and disputed findings in full.

**The arrangement to start with is the pair.** It exercises claims,
decisions, messages, evidence and the report with two agents and none
of the machinery that width needs, and it puts a second reader on every
item while the checks are still few. Width follows once decomposition,
per-builder working trees and per-agent environments exist.

### Agent names

Each agent seat has a durable name, set in configuration beside the
window schedule, so that its record can be followed across sessions:
whose items reopen, whose findings are upheld, whose mechanical
decisions are reclassified at review.

```toml
[agents.heron]
role   = "lead"       # lead, builder or auditor
runner = "runner-a"   # the runner adapter, and through it the model

[agents.wren]
role   = "auditor"
runner = "runner-b"
```

- **A name is a seat, not a session.** Every session started in a seat
  acts under its name, and the log records the name.
- **A name is a pseudonym.** It appears in state and in exports, so it
  passes the confidentiality scan and is never a real person's name.
- **What is behind a name is on the record.** The binding of a name to
  its role, runner and model is logged when the name is first used and
  whenever it changes. Without that, a name would imply a continuity
  that is not there: a comparison over time has to be able to see that
  the model in a seat changed.
- **A retired name is not given to another seat.**

## Primitives

The harness's concepts are types, and illegal transitions are
unrepresentable in them.

- **Phase:** a set of items with an exit review.
- **Item:** goal, checks, dependencies, a claim (agent, and later a
  lease) and, at width above one, a working ref.
- **Decision:** question, options, a recommendation, class (mechanical
  or substantive), state, rationale, who decided.
- **Evidence:** an artefact attached to an item or decision, stored by
  content hash.
- **Check:** a command and its pass condition.
- **Rule:** standing practice in three layers: personal (in the harness
  repo), project (in the project repo), environment (for example "this
  client forbids X"). Above them sits one ground rule that no layer can
  relax (Confidentiality).
- **Actor:** who did something: the person, or an agent under its
  configured name (Agent names). Never a real person's name or an
  address.
- **Message:** a typed, targeted note between agents (below).
- **Claim:** a statement the harness holds as true, with its provenance
  (the decision, item and commit it came from), its evidence and its
  falsifier (a check). An item being done, a confirmed decision, a
  statement in a design record, a measured figure: each is a claim.
  A claim is **asserted** (trusted in the moment) until phase review
  makes it **final** with evidence attached and its falsifier passing.
  A final claim is reopened only by its falsifier failing or by a
  decision that supersedes it.

Sketch, as built (`crates/ris-core/src/primitives.rs`,
`crates/ris-core/src/event.rs`):

```rust
enum DecisionClass { Mechanical, Substantive }   // substantive = expensive to reverse
enum DecisionState { Open, Taken { by: Actor, chosen: usize, at: u64 }, Superseded(DecisionId) }

enum ItemState {
    Ready,
    Claimed { agent: AgentName },             // no lease yet: leases come with the daemon
    Blocked { on: Vec<Blocker> },             // a decision, another item
    Done { evidence: Vec<EvidenceId> },       // checks passed
    Merged { evidence: Vec<EvidenceId>, commit: CommitHash },
    Superseded { by: ItemId },                // replaced before it started; final
}

enum ClaimState {                             // the type exists; no events yet
    Asserted,
    Final { evidence: Vec<EvidenceId>, at: CommitHash },
    Falsified { by: EvidenceId },
    Superseded(DecisionId),
}

enum Structure {                              // an event's structural part
    PhaseOpened { phase }, PhaseClosed { phase, report: Hash },
    ItemAdded { item, phase, checks, depends_on }, ItemClaimed { item, agent },
    ItemBlocked { item, on }, ItemReady { item }, ItemDone { item, evidence },
    ItemMerged { item, commit }, ItemAudited { item, by }, ItemSuperseded { item, by },
    DecisionRaised { decision, class, by, about, options, recommended },
    DecisionTaken { decision, by, chosen },
    DecisionExplained { decision, by, pros, cons },
    CheckRan { item, check, passed, evidence },
    MessageSent { message, from, to, kind, about },
}
```

Free text (a goal, a question, a rationale) is never in the structural
part: it sits in the event's text part, which enters the chain by salted
hash (Confidentiality), and the fold never reads it. Records point at
the event that carries their text (`at`).

- **Event format 3:** the stored form is the shown form, snake_case
  event names, ids as `I-1`, hashes as hex, actors as
  `{"kind": "agent", "seat": "heron"}`, with a timestamp per event.
  Events are versioned from the first release; older logs are verified
  from their stored bytes and upgraded in place.
- **The report hash:** a phase report's hash is
  `H(previous report hash, log head at close)`, so the reports form a
  chain of their own over the log, and removing text later does not
  change it.

## Architecture

Functional core, imperative shell; hexagonal between components.

```text
 agent ─┐                                   ┌──────────── daemon ─────────────┐
 agent ─┼─ CLI / MCP ── local socket ──────▶│ commands → pure core → events   │
 human ─┘                                   │                 │               │
                                            │            Store trait          │
                                            │        ┌────────┴────────┐      │
                                            │     RocksDB          SQLite     │
                                            └─────────────────────────────────┘
```

- **Core (pure):** a command plus the current typed state gives events
  or a refusal. State is a fold over the event log.
- **Daemon (shell):** the only process that opens the store. It
  serialises commands, which is where atomicity comes from, appends
  events, updates materialised views (items by state, open decisions) in
  the same write, runs lease timers and routes messages. In the first
  slice there is no daemon: the CLI hosts the core and opens the store
  itself, one transaction per command, which SQLite serialises across
  processes. The daemon arrives with leases, timers and the MCP
  adapter. A view the daemon keeps instead of refolding (the reverse
  index of dependents, decisions about each item and replacements is
  the first) records the log head it was built at: when that differs
  from the store's head it is rebuilt from the log, and at phase close
  it is rebuilt and compared, the rebuilt copy winning and any
  difference reported. Until then the index is part of the fold,
  checked against a full scan by a property test, after every fold in
  debug builds, and by `ris verify` and `ris phase close`. Either way every command passes through the core, which is
  where refusals, the confidentiality scan and atomicity live, so the
  agent sees the same commands and the same answers in both.
- **`Store` trait:** the port that hides the engine. Swapping engines
  changes an adapter, not the core.
- **CLI first.** Every agent tool can run a shell command, and so can
  the person. The project is osiris and its store and configuration
  live in `.osiris/`; only the binary (the CLI, and later the daemon)
  is `ris`, three letters that fall under the fingers on a plain
  keyboard: `ris decide`, `ris claim`,
  `ris done --evidence …`, `ris inbox`, `ris report`, plus
  `why <claim | file | decision>` (provenance and dependants) and
  `verify [claim | all]` (re-run falsifiers against the current tree).
  The CLI is the contract. Agents read its output, so it is short and
  stable: plain text by default, `--json` on request, and a refusal
  says what to do next.
- **Tamper-evident log (cheap option):** each event carries the hash of
  the one before, so a final fact cannot be rewritten silently. Settled
  facts are then checkable against the log's head hash, as entries are
  against a chain's. An event's free text enters the chain by hash
  rather than directly, so it can be removed (Confidentiality).
- **MCP server:** a thin adapter over the same commands, for clients
  that prefer tools.
- **Agent runners:** one adapter per agent tool. Start an agent on an
  item with a worktree, an environment and the applicable rules; stream
  its output; report done or failed. Vendor independence lives here.
- **Rules, rendered per tool:** one source, rendered into whatever each
  agent tool reads (`CLAUDE.md`, `AGENTS.md`, and so on).
- **Human interface:** the CLI's `inbox` and `report` first, or a local
  web page served by the CLI. A native app comes later, only if the
  terminal is the bottleneck: the person works across machines, and a
  native app would tie the interface to one platform.
- **Language:** Rust for the core, daemon and CLI.

## Runtime and environments

Guaranteed on every machine: git and Docker. Laptops may be locked down;
the CLI on a dev machine is usually unrestricted.

- **Native first.** The first slice runs the core inside the CLI as one
  native Rust binary on the dev machine, with agents in their usual
  tools. Containers come in when isolation or cross-machine portability
  is needed, not before.
- **Then Docker as the runtime for agents.** The daemon and each agent
  run in containers, with the agent CLIs installed in the image. Setting
  up is `git pull` plus `docker compose up`; nothing installs on the
  host beyond what Docker allows. An agent's sandbox is its container,
  not a vendor's built-in one, and switching agent tools means switching
  an image. Costs to handle at that point:
  - a project whose checks need Docker needs Docker inside the agent's
    container: mounting the host socket is easy but gives the container
    root-equivalent access to the host, which defeats the isolation; a
    Docker-in-Docker sidecar keeps it, at more weight;
  - credentials (the model API key, git push rights) are injected per
    container;
  - repos bind-mounted into containers on macOS are slower than native
    file access;
  - interactive approvals are awkward from inside a container.
- **Project manifest:** how to build, how to test, the checks, and what
  the environment needs (Docker, ports, services). For a new project the
  setup agent reads it and provisions a disposable environment per
  agent where the full checks run. If an agent cannot run the real
  checks, setup says so before work starts.
- **Per-agent environments** are separate compose projects with their own
  name and network, so agents run full suites in parallel without
  fighting over ports.
- **Network:** the containers need to pull images and reach the model
  provider's API. This is the one dependency to confirm per client.

## Storage

### Alternatives considered

- **Bare filesystem** (one file per item, claims by atomic `mv` or
  `mkdir`). No dependencies, readable and diffable, and it suits agents'
  native tools. Against: no transactions across files, no leases, the
  ready queue is a hand-written scan, and atomic claims depend on agents
  following a convention rather than on enforcement. A good fallback,
  not the design.
- **beads** (a graph issue tracker for agents, MIT, Go, on Dolt). It has
  atomic claims, a ready queue, dependencies, messages and integrations
  for several agent tools. Against: embedded mode is single-writer, so
  parallel agents need `dolt sql-server`; there is no decision, evidence
  or phase primitive; the source of truth is a database rather than
  readable files; and it is young and changes quickly. Dolt's
  distinctive feature, branching and merging data edited offline on
  many machines, is for many writers without a coordinator, which this
  design does not have.
- **A Raft ledger as the substrate.** It would give signed per-agent
  attribution and a total order. Against: replication protects against
  hardware failure, which buys nothing on one machine; a ledger that
  holds one slot per submitter per block, with last write wins, loses
  concurrent claims through one node; sealing latency (minutes in a
  typical configuration) is far too slow for claims; and there is no
  state machine to query. Possible later as a soak test of a ledger,
  not as the store.
- **RocksDB** behind the daemon. The single-process lock does not matter
  when the daemon is the only writer. The log is a column family keyed
  by sequence number, with views in other column families updated in
  the same `WriteBatch`. Against: queries are hand-written code over
  views, and there is no built-in text search.
- **SQLite (WAL mode)** behind the daemon. The same role, plus ad-hoc
  SQL and built-in full-text search (FTS5), which the context features
  below want.

### Decision

The event log is the source of truth, behind the `Store` trait, opened
only by whichever shell hosts the core. Speed does not choose the
engine: five agents at their busiest produce a few events per second,
and either engine has orders of magnitude to spare. The first slice
uses SQLite: it needs no install, the `sqlite3` tool on a dev machine
can inspect the store, and FTS5 is there when search arrives. RocksDB
remains a possible second adapter, judged by the bench of the append
path if SQLite ever falls short.

### Sync across machines

A supported configuration, off by default; with it off, the store's
directory is ignored by git. The design assumes it may be on: state is
always written as if it will be committed (Confidentiality), so turning
export on later exposes nothing in the history that the ground rule
would have kept out.

With it on, what is exported is the event log, at phase close, as
append-only JSONL, one file per phase. That gives diffable history,
lets another machine pull and replay, and is the same whatever engine
sits behind the `Store` trait. A closed phase's file never changes, so
git has nothing to merge inside one. A dump of the database is not
exported: git cannot usefully diff or merge a binary one, and a dump of
either kind carries the materialised views, which are derivable, and
ties the history to one engine. Evidence blobs are not exported either
(Confidentiality): the other machine gets their hashes, and confirms a
claim by re-running its falsifier.

- **Nothing is exported before a check-in.** Phase close is the
  person's review, and the export passes the confidentiality scan
  first; the report shows the scan's result.
- **One writer at a time.** Another machine pulls, replays and
  continues from the head. Two machines extending the same head make a
  fork; the hash chain shows it, and the import is refused rather than
  merged. How a fork is resolved is open.

### Evidence

Large blobs (test logs, benchmark output, diffs) live on disk,
addressed by content hash; the store holds only the hashes.

## Coordination and context

The model reasons only over what is in its context, so the harness
spends its effort on loading less, and only what is relevant.

- **Coordinate through state first.** Claims, blockers, dependencies and
  decisions are visible to every agent without anyone sending anything.
  An agent finds an item is taken because its claim fails.
- **Targeted, typed messages second.** When an item changes something
  another depends on, the daemon notifies only the agents on dependent
  items, using the dependency graph. Messages are short: a handoff, "the
  interface you use changed (here is the diff)", a question to an item's
  owner. No broadcasts and no free chat. Agents read their inbox at
  natural points (before starting an item, after a check runs), not
  mid-task.
- **Answers instead of raw material.** `ris ready` returns a few lines;
  the agent never loads a status document to work out what is next.
- **Decisions are memoized reasoning.** A recorded decision with its
  rationale is not re-derived or re-argued. When an item is claimed, the
  daemon attaches only the decisions that touch it.
- **A brief at claim time:** goal, checks, relevant decisions, files
  involved, applicable rules, assembled by the daemon within a fixed
  size.
- **Finality bounds the working set.** What an agent carries is what is
  in motion: open items, open decisions, claims still asserted this
  phase. Final claims leave the working set and are held as one line
  and an id, loaded in full through `why` on demand; a superseded chain
  loads only its head. The brief selects the final claims that touch
  the item's files, contracts or dependencies. Selection can miss a
  constraint, and the falsifiers are the safety net: a violated final
  claim fails a check rather than passing silently, so checks stand in
  for loading every settled fact as a precaution. Finality does not
  shrink reading the code itself; cached summaries do that.
- **Summaries cached by content hash.** A cheaper model summarises a
  module or document once, keyed by the hash of its content at a commit;
  every agent reuses it until the hash changes. Each summary cites its
  sources (file and line, commit) so an agent can go to the original.
- **Search instead of preloading,** over decisions, rules, design records
  and summaries. Full-text search first; embeddings only if it falls
  short.
- **Tool results cached** by (command, tree hash), so a check result or
  a call graph at a commit is reused rather than recomputed.
- **Prompt order for provider-side caching:** rules first, then the
  project brief, then the item brief, so the unchanging prefix is
  cached. This cuts cost and latency, not context size.

## Confidentiality

Harness state is written by agents, may be exported and committed, and
is append-only. Something written into it is hard to take back, and
once pushed it cannot be taken back at all. So one rule ships with the
harness, sits above the three rule layers, and cannot be relaxed by any
of them:

**Ground rule: nothing secret, and nothing that identifies a person or
a machine, goes into harness state or into a commit.**

It covers:

- **secrets:** credentials, tokens, keys and connection strings, in any
  field and in any evidence;
- **personal data:** the names, addresses and account handles of real
  people;
- **machine identifiers:** hostnames, usernames and absolute paths (a
  home directory carries a username). Paths in state are relative to
  the repository root;
- **whatever a layer adds:** a project or environment layer can extend
  the list (a client's name, a codename). Layers add to it and never
  remove from it.

What follows for the design:

- **Actors are roles and pseudonyms.** Who decided is "the person" or
  an agent's configured name (Agent names). A commit's author fields
  belong to whoever configured git on that machine, and the harness
  does not set them.
- **State travels no further than the code it describes.** Each project
  has its own store in its own directory, and an export goes only where
  that project's code may go. The harness's own repository holds no
  other project's state.
- **Evidence blobs are never exported.** Test logs and command output
  are the likeliest carrier: environment dumps, absolute paths, a token
  echoed by a failing command. They stay on the local disk, and an
  export carries their hashes only.
- **Enforced in the command path, not left to the agent.** The rule is
  rendered first into every agent's rules, but an instruction lowers
  the rate of leaks and cannot guarantee none; a convention agents are
  trusted to follow is the weakness the bare-filesystem store was
  rejected for. Every command goes through the core, whichever shell
  hosts it, so every free-text field (a rationale, a question, a
  claim's statement, a message) passes one point. There it is scanned for secret patterns and against a local
  denylist: the person's name and addresses, client names, the home
  directory. A hit is a typed refusal that names the field and does not
  echo the match. The denylist is itself sensitive: it is local
  configuration and is never exported.
- **Scanned again at the two exits:** the export of state, and each
  item's diff and commit message, by a check that is on by default in
  every project. The first version scans with patterns built into the
  harness and the denylist; an off-the-shelf secret scanner can be
  added to the diff check later.
- **The auditor reads for what patterns miss,** where an audit is
  running.
- **Free text is removable without breaking the log.** Scanning is
  best effort: text that identifies someone without matching a pattern
  will get through sooner or later. So each event has two parts. The
  structural part (ids, states, the option chosen) enters the hash
  chain directly. The free text (a rationale, a question, a statement,
  a message) enters it by salted hash, as evidence enters by hash:

  ```text
  text_hash(n) = H(salt(n), text(n))
  h(n)         = H(h(n-1), structure(n), text_hash(n))
  ```

  Deleting an event's text and its salt leaves the chain verifiable,
  and the salt stops a short text being recovered from its hash by
  guessing. Three things follow:

  - the fold reads only the structural part, so state never depends on
    text that might be removed;
  - a phase report's hash is taken over the log's head at close, not
    over the rendered report, so removing text does not break the chain
    of reports;
  - the split is the scan's own: the fields classed as free text are
    the fields scanned.

  An event that is wrong is superseded, never removed. Only toxic text
  needs removing, so no event is ever cut out of the log with the rest
  re-applied. The format is in from the first version, because adding
  it later means rehashing the log. The `redact` command (the person's
  only, and leaving its own event, so that text missing without one is
  detectable) and the rebuild of views and the search index after it
  are built when first needed. Removal repairs the store and later
  exports, not history already pushed: that is treated as disclosed,
  and a secret is rotated.

## Risks

- **Unattended agents with full environments.** Disposable environments,
  scoped credentials, no production secrets.
- **A secret or a personal detail written into state or a commit.** The
  ground rule, enforced in the command path and at both exits; evidence kept
  local; free text removable from the log without breaking its chain
  (see Confidentiality).
- **Errors accumulate between reviews.** Checks on every item, phases no
  longer than half a day by default, substantive decisions always
  queued, and an auditor on each item where an audit is running.
- **Provisional work built on a decision later overturned.** The report
  shows which items depend on which decisions, so overturning one shows
  its blast radius.
- **Merge conflicts between parallel agents.** Decomposition quality
  matters: items carry file-ownership hints, and a merge that
  conflicts is refused and returns to its author.
- **Merges queue behind slow checks.** Merging is serialised and runs
  the full checks, so its rate is bounded by the suite's run time. The
  time items spend waiting to merge is recorded per window; if it grows,
  merges are batched.
- **Event format changes over time.** Versioned events with an upgrade
  step, from the first release.
- **The daemon is a single point of failure** for coordination. Fine on
  one machine; it is one container.
- **Building and maintaining the harness** replaces ready-made
  integrations. Keep the core small, and build the harness with itself
  from the second week.
- **Runner adapters need upkeep** as agent tools change.

## First slice

**Done.** Steps 1 to 3 are built, and osiris is developed with itself;
step 5's runner adapter is built (for Claude Code, sandboxed, with an
allowlist per role); the project manifest and steps 4 and 6 are items
in its log. Built beyond the slice: width above one with a worktree per
seat, phase branches, roles enforced (an auditor cannot write), the
exit scan on every merge, `ris import` with open decisions, `ris
status` and `ris watch`, `ris eli5` for a decision's full case, one
versioned `--json` envelope for every command (`docs/json.md`), a brief
at claim time, person actions with hard and soft edges, a graph of what
waits on what, a reverse index checked against a scan, and the harness
declared in one file and brought up and down with `ris up` and `ris
down` (`docs/quickstart.md`).

The aim was something minimal in the dev flow as soon as possible, then
dogfood it from there.

1. The primitives as types, the pure core, and the event log behind the
   `Store` trait with one engine. Events have the two-part format from
   the start (Confidentiality); the `redact` command waits.
2. The CLI, native, hosting the core with no daemon yet: `phase open`
   and `phase close`, `item add`, `claim`, `decision raise` and
   `decide`, `done`, `merge`, `inbox`, `report`. The confidentiality
   scan is in from the first version. Check-ins are on demand; no
   timer yet.
3. Use it at once to build the rest of the harness, as a pair: a lead
   and an auditor.
4. Then drive the next phases of an existing project with it: import
   that project's backlog and standing decisions as items and
   decisions, and work from the harness rather than from status
   documents. The import doubles as the first measurement of the
   context savings (see Falsifiers).
5. One runner adapter, and one project manifest with an environment
   where the full checks run, once parallel agents are in use.
6. Later, point it at a second codebase with different assumptions to
   test portability.

**Not yet: coordinating with other committers.** One harness
coordinates the agents inside it. Anyone else writing to the same
repository (a harness on another machine, an agent outside the harness,
a person) works on a branch of their own and meets the others at the
main line, as with any shared repository. When the main line has moved
during a phase, the closing merge runs as an item's merge does: the full
checks run on the merged result before it goes in. Passing messages
between harnesses, so that their agents coordinate, is not designed.

**Not yet: project namespacing.** When a second project arrives, it gets
its own store in its own directory, so namespacing needs no project
field in the events. If it ever does, events are versioned.

## Falsifiers

Each claim above, and the test that would show it false:

- **Governance cost near constant (principle 1, the cost model):**
  across phases, the human's time per phase (check-in minutes,
  decisions answered) against the number of items and agents in the
  phase. If it grows roughly in proportion, decisions are not being
  amortised. The report's size against the phase's work should grow
  sublinearly, and the queued substantive decisions should never exceed
  the budget.
- **Centaur (principle 2):** every human action in a phase is logged and
  classified as design/decision or mechanical. The mechanical count per
  phase should be zero; each one found is an item to automate.
- **Finality (principle 3):** every final claim has a falsifier, and
  `verify all` on the current tree either passes or names the claims it
  falsified. A final claim with no runnable falsifier is refused at
  review. Editing a final claim in place is impossible through the core
  (a test of the transition table), and, with the hash-chained log,
  detectable outside it.

- **Atomic claims:** N agents claim one item concurrently; exactly one
  succeeds, and the rest get a typed refusal.
- **Illegal transitions are unrepresentable:** a table-driven test over
  the core, covering every state and command, including the refusals.
- **State is a fold of the log:** replaying the log from empty gives the
  same views as the live daemon.
- **Storage headroom:** a bench of the append path on each engine shows
  at least 100 times the peak event rate of five agents.
- **Leases:** an agent killed mid-item has its claim expire and the item
  return to ready.
- **Context savings:** tokens for a claim-time brief against tokens for
  the raw material it replaces, measured on a real project.
- **The working set stays bounded:** brief size in tokens, plotted
  against the number of final claims as a project's history grows,
  stays roughly flat. Growth in proportion to history means relevance
  selection is loading the past rather than the present.
- **The safety net holds:** a seeded change that violates a final claim
  left out of the agent's brief is caught by that claim's falsifier at
  the item's checks.
- **Portability:** a fresh machine with only git and Docker reaches a
  first claimed item from `git pull` and setup alone.
- **Vendor independence:** the same phase runs with two different agent
  runners.
- **Decision rule:** across a phase, the count of mechanical decisions
  the person reclassified at review; if it is high, the rule's wording
  is wrong.
- **The window schedule:** the per-window record, compared across
  schedules. The default of two a day is wrong for a project if, over a
  run of windows, the budget fills or agents run out of unblocked work
  well before the boundary (too long), or check-ins arrive with little
  to decide (too short).
- **Schedule independence:** replaying a log under a different window
  configuration gives the same state.
- **Audit earns its place:** the share of findings upheld, by the author
  or by the person, out of findings raised; and a seeded defect (a
  weakened check, a contract change logged as mechanical) is found by
  the auditor. A low upheld share, or seeded defects passing, means the
  audit is noise. An agent auditing its own item is refused (the
  transition table).
- **Confidentiality:** canaries (a fake token, a denylisted name, an
  absolute path) are planted where an agent will meet them during an
  item: in a file, in a command's output, in the environment. After the
  phase none appears in the event log, in an export or in a diff, and
  each attempted write shows as a refusal. A test over the event types
  shows that no free-text field reaches the log without passing the
  scan.
- **The fold ignores text:** folding the log with every text part
  removed gives the same structural state as folding it whole.
- **Removed text leaves the chain intact:** with one event's text and
  salt deleted, the chain verifies, and verification names that event
  as missing its text.
- **Commits map to items:** every commit on a phase's branch names
  exactly one item, and every done item has at least one commit there.
- **Later merges cannot undo earlier ones:** a seeded item that
  reverts an earlier item's behaviour is refused at merge, by the
  earlier item's check. Two merges submitted at once merge in one
  order, and the second is checked against a tree that contains the
  first.
- **The closing merge is clean:** a phase's branch merges into the main
  line without conflict when nothing else has written to it during the
  phase.
- **Export round trip:** a second machine that pulls an export and
  replays it reaches the same state and the same head hash. A log that
  does not extend the local head is refused.
- **Agent names are durable:** two sessions in one seat appear under
  one name in the log, and a change of the runner or model behind a
  name appears as an event.

## Open questions

- Exported state: whether it lives in the project's repository or in
  one of its own, and how a fork between two machines is resolved.
- The default window schedule: two a day is the prior, settled by the
  per-window record.
- Which agent runner comes first, and which second, for the portability
  test.
- Client constraints: whether any client forbids running agents on its
  code, or restricts where code may be processed or sent.
