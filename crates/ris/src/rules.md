# Working with osiris

This project's work goes through `ris`, the osiris command, not through
status documents. The work runs in four steps, all the person's: open
a phase, `ris up`, review, close the phase. Between up and review the
agents build and test, and never wait for the person: nothing raised
while the phase runs stops it, and there is no check-in in between.

## Your seat

Your seat is a durable name in `.osiris/config.toml`, set in your
environment:

```
export RIS_AGENT=<your seat>
```

With it unset, `ris` refuses every command. Use only your own seat;
`RIS_AGENT=person` is the person's, never yours. A seat started by
`ris up` has it set already and finds `ris` on its PATH: run `ris ...`
as it is, with no `RIS_AGENT=` or path in front, since the seat's
allow rules match `ris ...`.

## The loop

```
ris ready                       # what you may claim
ris claim I-7                   # take one; the output is your brief
ris show I-7                    # the item in full, whenever you need it
...work...
ris done I-7                    # runs the item's checks; output becomes evidence
git commit -m "feat(scope): summary" -m "Item: I-7
Agent: <your seat>"
ris merge I-7                   # runs every project check and merges
ris inbox                       # messages to you; read between items
# A claim refused with "paused by the person" means stop and wait.
```

- An item lands as one commit, carrying exactly one `Item:` trailer and
  exactly one `Agent:` trailer naming your seat, unless the project has
  turned trailers off (`ris done` says which). `ris merge` refuses
  anything else.
- `ris done` and `ris merge` exit 3 when a check fails. The output is
  evidence under `.osiris/evidence/`, named in the message. Fix and run
  again.
- If you cannot finish an item, `ris release I-7` returns it to ready.

## Working beside other builders

Each writing seat, the lead included, has its own git worktree on its
own branch; the store is shared. The phase branch is checked out
nowhere, and the main checkout is the person's. Commit your item on
your branch and run `ris merge` from your worktree: it rebases your one
commit onto the phase branch, runs every check on the result, and moves
the phase branch forward by a ref update, one merge at a time. If your
change conflicts with work merged before it, or the combined tree fails
a check, the merge is refused and nothing moves: rebase onto the phase
branch yourself, fix, and run `ris merge` again. That is how a later
item is kept from undoing an earlier one.

## Decisions

The test is the cost of reversing, not whether reversal is possible.

- **Mechanical** (cheap to reverse: a blocker found, something adjacent
  discovered, the shape an implementation takes, or an addition to a
  contract or format, such as a new event or a new JSON key): decide
  and log it.
  ```
  ris decision raise --about I-7 --question "..." --option a --option b --recommend 0 --rationale "..."
  ```
- **Substantive** (expensive to reverse: removing, renaming or changing
  the meaning of something in a contract or a data or wire format, a
  dependency, the scope, a design direction others build on): queue it,
  and take another item. Raising it sets the item aside; the phase goes
  on, and the person takes every queued decision once, at review, with
  `ris decide`.
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
  Adding a dependency is substantive. Prefer an existing library to
  writing the thing yourself; raise the dependency rather than working
  around it. Never take a substantive decision yourself; `ris` refuses.

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
(paths are relative to the repository root). `ris` scans every text
field, and `ris merge` scans every line a commit adds and its message,
and refuses on a hit without echoing it. There are no exceptions. `ris
scan` runs the same scan over the tree before you commit. A test that
needs a planted secret or path assembles it at runtime from pieces,
with a comment saying why.

## Messages

Typed and targeted; no broadcasts.

```
ris send <seat> --kind question --about I-7 "is the interface settled?"
```

Kinds: `handoff`, `interface-changed`, `question`, `finding`.

## An auditor's seat

An auditor never writes to the tree, and `ris` refuses an auditor seat
that tries to claim, finish or merge an item, or to get a worktree.
`ris audit` is its queue: the open phase's done and merged items it did
not write and has not reviewed; earlier phases' are not its work, and
`ris audit --all` only shows them. For each, it reads
the diff and the evidence against the goal, the decisions that touch it
and these rules, and checks the item's mechanical decisions for any
that should have been substantive. Findings go to the author with
`ris send <author> --kind finding --about I-n "..."`; a finding the
author disputes is raised as a substantive decision. `ris audit I-n`
records the review. Merges never wait for it: findings are fixed
forward.

## An advisor's seat

An advisor works beside the person and never writes to the tree. It
reads, adds items (`ris item add`), sends messages (`ris send`), and
puts judgements to the person (`ris decision raise --substantive`, then
`ris decision explain`). `ris` refuses it everything else: claiming,
finishing, merging, releasing, deciding, a mechanical decision (which
is taken as it is raised), recording a review, running checks, and any
command that changes the harness.

## Onboarding a project

To load an existing project's backlog and standing decisions, follow
the osiris-onboard skill (in the osiris repository under
`skills/osiris-onboard`): checks first, then what the log already
holds, then an import file kept in `.osiris/`, checked with `ris import
--dry-run`, approved by the person, then imported. Never record a
decision the project has not made; import an undecided question open,
for the person to answer.
