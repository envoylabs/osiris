# Working in this repository

This repository is osiris, the harness itself, and it is built with
itself. What there is to do, and what has been decided, is only in
`ris`. `PLAN.md` describes the design as built; read its Principles,
Working model and Confidentiality sections for background, but never
take work from it.

What follows is this repository's own, beside the rules above. The file
the seats read is rendered from both when a phase closes (`ris phase
close` renders it into the closing merge); to change a rule, change the
rules in `crates/ris/src/rules.md` or this file, and merge that. Leave
the rendered file as it is: no check of yours reads it.

## The seats and the harness

- The seats are named in `.osiris/config.toml`. Outside `ris up`, in a
  tool that starts a fresh shell per command, set your seat on each
  command (`RIS_AGENT=<your seat> ris ...`) or launch the tool with it
  set.
- Run the loop with the harness's `ris`, the one on a seat's PATH,
  never a binary built in your worktree from code you are changing.
  `cargo build -p ris` builds the code under work, at
  `target/debug/ris`, for its tests.
- The `Agent:` trailer is how a commit records co-authorship: by alias,
  with no email address and no model name (the model behind a seat is
  in its binding). Add no other trailer that carries an address; the
  exit scan refuses them and has no exceptions.
- A claim refused with "paused by the person" means stop: finish
  nothing new, and end the session. The person brings seats back with
  `ris up`.
- Read your inbox before starting an item and after a check runs, not
  mid-task.
- Adding a crate is substantive. Prefer an existing crate to writing
  the thing yourself; raise the dependency rather than working around
  it.

## Writing for the repository

Anything committed is self-contained: readable by someone with no
knowledge of the conversation that produced it. State decisions,
alternatives, reasons and evidence as facts about the system; no
session narrative, no references to chat or memory. The confidentiality
rule applies to what you write in code, comments, commit messages and
docs alike.

Commit messages are conventional: `type(scope): summary`, imperative,
with the `Item:` and `Agent:` trailers. Types in use: `feat`, `fix`,
`refactor`, `docs`, `test`, `chore`.

## Checks

The standing checks, declared in `.osiris/checks.toml` and run by
`ris done` and `ris merge`:

- `cargo build --workspace`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo fmt --all --check`
