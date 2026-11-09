# Working in this repository

This repository is osiris, a harness for directing coding agents, and it
is built with itself. Work here goes through the `ris` command, not
through status documents. The design is `PLAN.md`; read its Principles,
Working model and Confidentiality sections before the first item.

## Who you are

Your seat is named in `.osiris/config.toml` and set in your environment:

```
export RIS_AGENT=heron    # or wren
```

With it unset, `ris` acts as the person. Never set it to a name that is
not a seat.

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

- `done` fails with exit 3 if a check fails. The output is evidence
  under `.osiris/evidence/`, named in the message. Fix and run again.
- `merge` needs exactly one `Item: <id>` trailer and exactly one
  `Agent: <your seat>` trailer, naming the agent that claimed the item.
  That trailer is how a commit records co-authorship: by alias, with no
  email address and no model name (the model behind a seat is in its
  binding). Add no other trailer that carries an address; the exit scan
  refuses them and has no exceptions.
- Build `ris` with `cargo build -p ris`; the binary is
  `target/debug/ris`.

## Decisions

The test is the cost of reversing, not whether reversal is possible.

- **Mechanical** (cheap to reverse: a blocker found, something adjacent
  discovered, the shape an implementation takes): decide, and log it.
  ```
  ris decision raise --about I-7 --question "..." --option a --option b --recommend 0 --rationale "..."
  ```
  It is taken at once under your name and appears in the phase report,
  where the person checks the class.
- **Substantive** (expensive to reverse: a contract, a data or wire
  format, a dependency, the phase's scope, a design direction others
  build on): queue it and take another item.
  ```
  ris decision raise --substantive --about I-7 --question "..." --option a --option b --recommend 1
  ```
  Adding a crate is substantive. Prefer an existing crate to writing the
  thing yourself; raise the dependency rather than working around it.
- Never take a substantive decision; `ris` refuses it. The person does,
  at a check-in.

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

An auditor never writes to the tree. For each done item it reads the
diff and the evidence against the goal, the decisions that touch it and
these rules, and reads the item's mechanical decisions for any that
should have been substantive. Findings go to the author as messages; a
finding the author disputes is raised as a substantive decision.
