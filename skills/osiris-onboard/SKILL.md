---
name: osiris-onboard
description: Load an existing project into osiris (the ris harness) so a lead and an auditor can start work - read the project, draft its backlog and standing decisions as an import file, check it, get the person's approval, and import it. Use when the person asks to set up, onboard or load a project into osiris or ris.
---

# Onboard a project into osiris

You are setting up `ris` on a repository that is usually not yours. Two
rules hold throughout:

- **No residue.** Nothing tracked in the repository changes. Every file
  you write goes inside `.osiris/`, which is excluded from git.
- **Confidentiality first.** Nothing secret, and nothing that identifies
  a person, client or machine, goes into an item, a decision or a
  commit. `ris` scans every text field and refuses on a hit.

## 1. Preconditions

1. `ris --version` works. If not, stop: the person installs it from the
   osiris repository with `cargo install --path crates/ris`.
2. In the repository root, if there is no `.osiris/`, run
   `ris init --local`. Then `git status --porcelain` must print
   nothing; if it prints anything, stop and tell the person.
3. Ask the person to fill the denylist in `.osiris/config.toml`: their
   name, the client's name, product and codenames, their home
   directory. Do not suggest the terms yourself and do not repeat them
   back. Then ask them to open a phase: `ris phase open "<goal>"`. That
   command refuses while the denylist is empty, which is the check that
   it was done.
4. Ask the person for the agents' seat names (pseudonyms, one lead and
   one auditor at least) and the role of each, and give them the
   commands to run: `ris seat add <name> --role lead` (or `builder`,
   `auditor`). Seats are the person's to add; `ris` refuses an agent.

## 2. Checks

Read how the project builds and tests: `Makefile`, `package.json`
scripts, `Cargo.toml`, `pyproject.toml`, CI configuration, the README.
Propose `.osiris/checks.toml` with the commands that are the project's
real gates, and show it to the person before writing it. Then run
`ris check`. If a check cannot run on this machine (no Docker, no
service, no network), say so plainly: an agent that cannot run the real
checks puts the person back in the loop.

## 3. Read the project

Read, in this order, and note file and line for everything you take:

- the README and anything under `docs/`;
- contribution guides, architecture or decision records;
- `TODO`, `FIXME` and backlog files, and any backlog the person points
  you at (an issue export, a planning document);
- `git log --oneline -50` for the current direction.

## 4. Read what is already recorded

On a project that already uses `ris`, the log holds items and decisions
from earlier phases. Read them before drafting, so nothing is recorded
twice:

- `ris --json report`: every decision ever recorded, and the current
  phase's items plus every item not yet merged;
- `ris show P-n` for each earlier phase: the items merged in it.

Leave out any item or decision already there, however it is worded. If
a recorded decision looks stale, do not record it again: draft an open
question that names it (`Revisit D-2: ...`) and let the person decide.
`ris import` refuses an item goal or decision question that matches
one in the log exactly, but a reworded duplicate gets past it; reading
the log is the real check.

## 5. Draft the import file

Write `.osiris/import.toml`:

```toml
[[item]]
key = "short-name"            # optional; for depends
goal = "One sentence, the outcome, in the project's own terms"
checks = ["test"]             # optional: absent means every check in
                              # .osiris/checks.toml; [] means none
                              # (done on attached evidence)
depends = ["other-key"]       # earlier keys in this file

[[decision]]
question = "The question as it was decided"
options = ["what was chosen", "the main alternative"]
answer = 0                    # or: other = "..."
rationale = "Why, citing the document: docs/adr/0003.md"

[[decision]]                  # no answer: an open question
question = "A question the documents leave open"
options = ["one way", "another"]
recommend = 0                 # your recommendation, required
rationale = "Why it is open and why you recommend that option, citing the document"
```

- Items: work the documents say is outstanding, one outcome each,
  small enough for one commit. Prefer fewer, clearer items.
- Decisions: only ones the project has actually made and written down,
  or the person tells you. Never invent a standing decision; a guess
  recorded as decided is worse than none.
- Open questions: a question the documents name as open, or that you
  find undecided and that blocks or shapes outstanding work, goes in
  without an answer. It is raised as a substantive decision and waits
  for the person. When unsure whether something was decided, import it
  open.
- Paths are relative to the repository root. No names of people, no
  client or product names the denylist would refuse, no URLs to
  private systems.

## 6. Check, show, import

1. `ris import --dry-run .osiris/import.toml`. Fix every problem it
   names; it never echoes the text that tripped the scan, so look at
   the field it names.
2. Show the person a summary: the items with their checks and
   dependencies, and each decision with its source. Wait for approval;
   change what they ask.
3. Run `ris import .osiris/import.toml` with `RIS_AGENT` unset: the
   decisions are recorded as the person's, which their approval makes
   true.
4. `ris summary` to show the result.

## 7. Hand over

For each builder seat, `ris worktree <seat>` creates its worktree beside
the repository with the rules loaded. Tell the person the command to
start each agent there with `RIS_AGENT=<seat>`. The lead at width one
works in the repository itself on the phase branch. The auditor needs
no worktree: it reads and never writes.
