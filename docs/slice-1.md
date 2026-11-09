# First slice: items

**Status:** draft, 2026-11-09. The backlog for the first slice of
`PLAN.md`, kept as a file until `ris` can hold it; the last item
imports this list into the harness and this file stops being the
source of truth.

The slice ends when a lead and an auditor can run a phase of work on
this repository through `ris` alone: open a phase, add items, claim,
raise and take decisions, mark items done with evidence, land, and
read one report at the close.

## Standing checks

Every item passes these on the tree it leaves behind, in addition to
its own checks:

- `cargo build --workspace`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo fmt --all --check`

## Decisions already taken

Substantive, answered before the slice (recorded in `PLAN.md`):

- SQLite is the first engine.
- No daemon in this slice; the CLI hosts the core.
- Dependencies, as one set: `rusqlite` (bundled), `serde` and
  `serde_json`, `clap`, and one of `sha2` or `blake3`.
- The confidentiality scan is built-in patterns plus a local denylist.

Mechanical, taken while drafting this list; each is one line, so a
wrong class is visible at review:

- SHA-256 (`sha2`) for the chain, so a chain can be checked with
  standard tools outside the harness.
- A workspace of three crates, `ris-core` (pure), `ris-store-sqlite`
  (adapter), `ris` (CLI), so the purity boundary is a crate boundary
  and `cargo tree` can prove it.
- Ids are short and typed, assigned in sequence per kind: `I-1`,
  `D-1`, `C-1`, `E-<hash prefix>`. One store per project means no
  namespace is needed.
- Configuration is TOML in `.osiris/config.toml`; the store is
  `.osiris/store.sqlite`; evidence blobs are `.osiris/evidence/<hash>`.
- An agent names itself with `RIS_AGENT=<name>`; the person is the
  default actor when it is unset.
- Free text is a `Text` type whose only constructor runs the scan, so
  no field reaches the log unscanned by construction.

## Items

Each item: goal; its own checks beyond the standing ones; what it
depends on.

### I-1 Workspace skeleton

Goal: the three crates, building and testing empty, with the
dependency set above declared where it belongs.

Checks: standing checks pass; `cargo tree -p ris-core` lists neither
`rusqlite` nor `clap`.

Depends on: nothing.

### I-2 Primitive types

Goal: the primitives of `PLAN.md` as types in `ris-core`: Phase, Item,
Decision, Evidence, Check, Claim, Message, Actor, and their states and
ids, with illegal transitions unrepresentable.

Checks: a table-driven test enumerates every state of every primitive
and the commands that may leave it; the table is the specification.

Depends on: I-1.

### I-3 Text and the confidentiality scan

Goal: the `Text` type and `scan(raw, &Policy) -> Result<Text, Refusal>`.
Patterns: key and token formats, private-key headers, email addresses,
absolute paths on the common platforms, and the denylist from the
policy. A refusal names the field and does not echo the match.

Checks: one canary test per category, each refused; a passing test for
ordinary prose containing a relative path; the refusal's rendering
contains no substring of the canary.

Depends on: I-1.

### I-4 Event envelope and hash chain

Goal: the versioned event envelope with its two parts, the salted text
hash, the chain hash, and `verify(&[Event])`.

Checks: a chain of generated events verifies; altering any structural
field or any text breaks it; deleting one event's text and salt leaves
it verifying and reports that event as missing text; a short text
cannot be recovered from its hash by trying the obvious candidates.

Depends on: I-2, I-3.

### I-5 Commands, transitions and fold

Goal: `apply(&State, Command) -> Result<Vec<Event>, Refusal>` and
`fold(&[Event]) -> State` for the slice's commands: open and close a
phase, add an item, claim, raise a decision, take a decision, done,
land, send a message. The fold reads only structural parts.

Checks: the table test from I-2 drives `apply` over every (state,
command) pair and asserts each result, refusals included; a second
claim on a claimed item is refused; `fold(log) == fold(strip_text(log))`
over generated logs; `fold` of any generated log equals the state the
commands were applied to.

Depends on: I-2, I-3, I-4.

### I-6 Store trait and SQLite adapter

Goal: `Store` in `ris-core` (append, read from a sequence, head hash,
views: items by state, open decisions, inbox by agent) and its SQLite
implementation in WAL mode, views updated in the appending
transaction, a schema version in the file.

Checks: replaying the log from empty gives the same views as the live
store, on generated logs; appenders in several processes produce one
total order with no gaps and no lost events; the bench of the append
path reports events per second and is at least a hundred times five
agents' peak (taken as ten events per second).

Depends on: I-4, I-5.

### I-7 Evidence blobs

Goal: content-addressed blobs under `.osiris/evidence/`, written by the
CLI when a check runs or `--evidence <file>` is given, referenced from
events by hash only.

Checks: a round trip returns identical bytes; the id is the hash of the
content; an export of the log contains hashes and no blob content.

Depends on: I-1.

### I-8 The `ris` CLI

Goal: `ris init`, `phase open`, `phase close`, `item add`, `ready`,
`claim`, `decision raise`, `decide`, `done`, `land`, `inbox`,
`report`, each a thin shell over `apply` and the store. Plain text by
default, `--json` on request; every refusal says what to do next.
Configuration read from `.osiris/config.toml`: agents, denylist.

Checks: an integration test drives the binary through a scripted
phase (two agents, three items, one substantive and one mechanical
decision, one landing) and compares output to a golden file; every
subcommand's `--help` fits one screen; a refused command exits
non-zero with the refusal on stderr.

Depends on: I-5, I-6, I-7.

### I-9 Checks and landing

Goal: project checks declared in `.osiris/checks.toml` as command and
pass condition (exit status by default); `ris done` runs the item's
checks and attaches their output as evidence; `ris land` at width one
records the commit at HEAD against the item, requires the item id as
a trailer in that commit's message, and runs the full checks.

Checks: a scripted check that fails leaves the item not done, with the
failure as evidence; a passing one marks it done; `land` refuses a
HEAD whose message lacks the trailer.

Depends on: I-7, I-8.

### I-10 The phase report

Goal: `ris report` renders the phase: substantive decisions in full,
mechanical ones one line each, items done with evidence hashes,
refusals and failed checks, the previous report's hash and this one's,
taken over the log head.

Checks: a golden test over the scripted phase from I-8; the report's
hash changes when the log does and not when the rendering does.

Depends on: I-8.

### I-11 Dogfood

Goal: `ris init` in this repository; this list imported as items; a
short agent rules file telling an agent to work through `ris`; the
first phase of further work run as a pair, lead and auditor, through
`ris` alone.

Checks: the first report of that phase exists and its hash matches the
log head; this file is marked superseded by the store.

Depends on: I-9, I-10.

## Order

I-1, then I-2 and I-3 and I-7 in any order, then I-4, I-5, I-6, I-8,
I-9 and I-10, then I-11. Until I-8 exists there is nothing for a pair
to work through, so items I-1 to I-8 are built by one agent; the pair
begins at I-11.
