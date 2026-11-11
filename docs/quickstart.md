# Quickstart: a lead, a builder and an auditor

You declare the harness in one file and bring it up with one command,
as with docker compose. This sets up three agent seats on an existing
git repository:

| seat  | role      | works in                    |
|-------|-----------|-----------------------------|
| heron | `lead`    | its own worktree            |
| rook  | `builder` | its own worktree            |
| wren  | `auditor` | the main checkout, read-only |

The main checkout is yours and stays on the main line. Each phase's
work goes onto its branch, `phase/P-n`, which is checked out nowhere:
a seat's `ris merge` moves it by a ref update, which needs only the git
directory, so a seat in its sandbox can merge without writing your
checkout. `git log phase/P-1` and `git diff main...phase/P-1` show the
phase's work; `ris phase close` merges it into the main line.

The person runs `ris` with `RIS_AGENT=person`. `ris` refuses every
command when `RIS_AGENT` is unset, so an agent that loses its seat's
name is stopped rather than taken for the person. Set it once in your
shell profile:

```sh
export RIS_AGENT=person
```

## 1. Install and initialise

```sh
cargo install --path crates/ris        # from the osiris repository
cd your-repo
ris init --local
```

`--local` leaves nothing tracked: `.osiris/` and `CLAUDE.local.md` are
listed in `.git/info/exclude`, which git never commits, and the rules
the agents read go in `.osiris/OSIRIS.md`. `git status` is unchanged.

## 2. Declare the harness

`.osiris/config.toml` is the one file you write. It stays on this
machine and is never exported.

```toml
# Words that must never appear in harness state or commits: your name,
# the client's name, your machine's hostname. Case-insensitive.
denylist = ["your surname", "client name"]

[agents.heron]
role = "lead"

[agents.rook]
role = "builder"
model = "claude-sonnet-5-5"            # optional: a full model name, not an alias

[agents.wren]
role = "auditor"

[runner]
write = ["~/.cargo"]                   # more paths agents may write, e.g. a build cache
network = []                           # hosts agents may reach

[runner.allow]
builder = ["Bash(cargo *)"]            # more tool permissions, in the agent tool's syntax
lead = ["Bash(cargo *)"]
```

`model` is passed to the agent tool as it is. Write a full name such as
`claude-opus-5-5` rather than an alias such as `opus`: the log records
what is written, and an alias moves to a newer model when one ships, so
runs on different days would not be comparable. Two seats may name the
same model.

Top-level keys such as `denylist` go above the first table. `ris phase
open` refuses while the denylist is empty, because text is scanned as
it is written and the log cannot be redacted; set
`confirm_empty_denylist = true` if an empty list is deliberate. A seat
name is a pseudonym, never a real person's name, and `person` is
reserved.

For a repository merged through a git server's web interface, set
`merge_on_close = false` (phase close leaves the branch for a pull or
merge request) and, if the project's commit convention has no room for
them, `trailers = false`.

Each builder's worktree goes where `worktrees` says: a path template,
relative to the main checkout unless absolute, in which `{repo}` is the
checkout's directory name and `{seat}` the seat's name. The default,
`"../{repo}-{seat}"`, puts it beside the repository. The template must
contain `{seat}`. A worktree placed inside the main checkout, such as
`".seats/{seat}"`, is added to git's local exclude file so the main
checkout stays clean.

`.osiris/checks.toml` declares the project's checks, each a command that
passes on exit status zero:

```toml
[checks.test]
command = "cargo test --workspace"

[checks.lint]
command = "cargo clippy --workspace --all-targets -- -D warnings"
```

## 3. Open a phase and load the work

```sh
ris phase open "the goal"              # creates phase/P-1, and a docs item for it
ris item add "a task"                  # held to every check; --check names some
ris item add "a review" --no-checks    # done on attached evidence instead
ris import backlog.toml --dry-run      # a backlog file: validate,
ris import backlog.toml                # then import
```

Every phase gets an item to bring the docs up to date, and `ris phase
close` waits for it (turn it off with `docs_item = false`). The
osiris-onboard skill (`skills/osiris-onboard`) drafts the import file
from an existing project's backlog.

## 4. Bring it up

```sh
ris up                                 # make it so; safe to run again
ris status                             # where each seat is, and what it holds
ris logs rook --follow                 # what rook is doing
```

`ris up` compares the file with what exists and does only the
difference: a worktree for each builder that has none, paused seats
resumed, and each seat whose agent is not running started in the
background. Run it again after any change to the file; with nothing to
do, it says so.

An unattended agent may do only what its role allows: reading and
`ris`; for a writing seat, edits, `git` and the project's check
commands; for an auditor, reading history. Anything else is refused
rather than asked, and the agent tool's sandbox keeps it to its own
directory and the shared store, with no network beyond `[runner]`.
It can never write the files that set its limits: the configuration,
the checks and the rules. Each run's output is kept under
`.osiris/runs/`, and its start and stop are in the log.

To run an agent yourself instead, start it in its seat's directory with
its name set: `cd ../your-repo-rook && RIS_AGENT=rook claude`.

## 5. Check in

```sh
ris summary                            # decisions, your actions, what waits on what, the report
ris eli5 D-3                           # a decision's full case
ris decide D-3 --choose 1 --rationale "..."
ris action done A-2 --note "what happened"
```

The summary draws what waits on what: open decisions, actions and
unmerged items, with hard edges (the item waits) solid and soft ones
(it proceeds) dashed. Things only you can do arrive as person actions
(A-n) in your inbox; one that names a check closes itself at your next
check-in once the check passes.

## 6. Bring it down and close

```sh
ris down                               # each seat finishes its item, then stops
ris phase close                        # review; merges phase/P-1 into main
```

`ris down` kills nothing: each agent stops at its next claim, and
worktrees and the log are kept. `ris up` after the next `ris phase
open` brings everything back.

`ris show <id>` prints any item, decision, action, phase or message in
full.
