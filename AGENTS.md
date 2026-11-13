# Working in this repository

This repository is osiris, a harness for directing coding agents, and it
is built with itself. Work here goes through the `ris` command, not
through status documents: what there is to do, and what has been
decided, is only in `ris`. `PLAN.md` describes the design as built;
read its Principles, Working model and Confidentiality sections for
background, but never take work from it.

## Who you are

Your seat is named in `.osiris/config.toml` and set in your environment:

```
export RIS_AGENT=heron    # or rook, or wren
```

With it unset, `ris` refuses every command. Never set it to a name
that is not your seat; `RIS_AGENT=person` is the person's alone. A seat
started by `ris up` has it set already and finds the harness's `ris` on
its PATH: run `ris ...` as it is, with no `RIS_AGENT=` or path in front,
since the seat's allow rules match `ris ...`. Outside `ris up`, in a
tool that starts a fresh shell per command, set it on each command
(`RIS_AGENT=heron ris ...`) or launch the tool with it set.

## The loop

```
ris ready                       # what you may claim
ris claim I-7                   # take one; the output is your brief
...work on your own ref...
ris done I-7                    # runs the item's checks, attaches the output as evidence
git commit -m "feat(core): ..." -m "Item: I-7
Agent: heron"
ris merge I-7                   # runs every project check, records the commit
ris inbox                       # messages to you; read it between items
```

A claim refused with "paused by the person" means stop: finish nothing
new, and end the session. The person brings seats back with `ris up`.

- `done` fails with exit 3 if a check fails. The output is evidence
  under `.osiris/evidence/`, named in the message. Fix and run again.
- `merge` needs exactly one `Item: <id>` trailer and exactly one
  `Agent: <your seat>` trailer, naming the agent that claimed the item.
  That trailer is how a commit records co-authorship: by alias, with no
  email address and no model name (the model behind a seat is in its
  binding). Add no other trailer that carries an address; the exit scan
  refuses them and has no exceptions.
- Run the loop with the harness's `ris`, the one on a seat's PATH,
  never a binary built in your worktree from code you are changing.
  `cargo build -p ris` builds the code under work, at
  `target/debug/ris`, for its tests.
- You work only the open phase's items. `ris ready` shows you nothing
  else, and `ris claim` refuses an item in the backlog; the person
  brings one into the phase with `ris item bring-in I-7`.

## Decisions

The test is the cost of reversing, not whether reversal is possible.

- **Mechanical** (cheap to reverse: a blocker found, something adjacent
  discovered, the shape an implementation takes, or an addition to a
  contract or format, such as a new event or a new JSON key): decide,
  and log it.
  ```
  ris decision raise --about I-7 --question "..." --option a --option b --recommend 0 --rationale "..."
  ```
  It is taken at once under your name and appears in the phase report,
  where the person checks the class.
- **Substantive** (expensive to reverse: removing, renaming or changing
  the meaning of something in a contract or a data or wire format, a
  dependency, the phase's scope, a design direction others build on):
  queue it and take another item.
  ```
  ris decision raise --substantive --about I-7 --question "..." --option a --option b --recommend 1
  ```
  Then attach the full case, which the person reads with `ris eli5`:
  ```
  ris decision explain D-4 --summary "what is being decided, plainly" \
    --pro "0: ..." --con "0: ..." --pro "1: ..." --con "1: ..." \
    --why "why the recommendation is the recommendation"
  ```
  Every option gets at least one point for or against.
  Adding a crate is substantive. Prefer an existing crate to writing the
  thing yourself; raise the dependency rather than working around it.
- Never take a substantive decision; `ris` refuses it. The person does,
  at a check-in.

## Person actions

A person action is the person's own to-do, raised only by the person
and linked to the work it holds up; `ris` refuses an agent that raises
one. An item may wait on an action or proceed under one; its brief
names them.

When you are blocked, raise one of two things and take another item:

- **An item**, for a fault in the harness or the code:
  `ris item add "what is wrong and what fixes it"`.
- **A decision**, for a judgement, as under Decisions. When what you
  need is something only the person can do (grant access, add a seat,
  run a trial), queue it as a substantive decision about your item and
  say what is needed in the question.

Never mark an action done; `ris` refuses. Never do the person's part
of the work by another route.

## Confidentiality: the ground rule

Nothing secret, and nothing that identifies a person or a machine, goes
into harness state or into a commit: no credentials, no names or
addresses of real people, no hostnames or usernames, no absolute paths
(a home directory carries a username; paths are relative to the repo
root). `ris` scans every free-text field and refuses on a hit without
echoing it, and `ris merge` scans the commit's message and every line
its diff adds, so the same rule applies to what you write in code,
comments, commit messages and docs. `ris scan [paths]` runs that scan
over the tree before you commit. There is no allowance mechanism: a
test fixture that needs a planted secret or path assembles it at
runtime from pieces, with a comment saying why, so the literal is never
in the tree.

## Writing for the repository

Anything committed is self-contained: readable by someone with no
knowledge of the conversation that produced it. State decisions,
alternatives, reasons and evidence as facts about the system; no
session narrative, no references to chat or memory.

Commit messages are conventional: `type(scope): summary`, imperative,
with the `Item:` and `Agent:` trailers. Types in use: `feat`, `fix`, `refactor`,
`docs`, `test`, `chore`.

## Tests

Tests live in their own files, never inline in a source file. A unit
test's module goes in a file under a `tests` directory beside the
source it tests, declared from the source file by a `mod` line, so a
source file holds only behaviour and its diff shows only behaviour.

## Checks

The standing checks, declared in `.osiris/checks.toml` and run by
`ris done` and `ris merge`:

- `cargo build --workspace`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo fmt --all --check`

## Messages

Typed and targeted; no broadcasts.

```
ris send wren --kind question --about I-7 "is the interface settled?"
ris send heron --kind finding --about I-7 "the fold reads a text field at state.rs:120"
```

Kinds: `handoff`, `interface-changed`, `question`, `finding`. Read your
inbox before starting an item and after a check runs, not mid-task.

## The auditor's seat

An auditor never writes to the tree; `ris` refuses an auditor seat
that tries to claim, finish or merge an item. For each done item of
the open phase (`ris audit`; earlier phases' items are not its work,
and `ris audit --all` only shows them) it reads the diff and the
evidence against the goal, the decisions that touch it and these rules,
and reads the item's mechanical decisions for any that should have been
substantive. Findings go to the author as messages; a finding the
author disputes is raised as a substantive decision.

As it reads the phase's diffs, an auditor records how closely the
person should read each changed file, for the map (`ris map`):

```
ris audit --file src/store.rs --risk high --why "writes the store with no lock"
```

The level is `low`, `medium` or `high`, the reason one line, and the
path as `ris map` lists it. `ris` already computes a level for every
changed file from what it can count, and the map shows it where no
judgement is recorded. Record one where reading the change says
otherwise, up or down, and for every file the person should look at
first. Only an auditor records one.

## The advisor's seat

An advisor works beside the person and never writes to the tree. It
reads, adds items (`ris item add`), sends messages (`ris send`), and
puts judgements to the person (`ris decision raise --substantive`, then
`ris decision explain`). `ris` refuses it everything else: claiming,
finishing, merging, releasing, deciding, a mechanical decision (which
is taken as it is raised), recording a review, running checks, and any
command that changes the harness.
