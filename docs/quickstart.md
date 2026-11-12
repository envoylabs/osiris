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
network = ["*.crates.io"]              # hosts agents may reach, e.g. a registry
quiet_after_seconds = 600              # ris status marks a seat quiet for longer possibly stuck
preflight = true                       # ris up checks the seats can run the checks first

[runner.allow]
builder = ["Bash(cargo *)"]            # more tool permissions, in the agent tool's syntax
lead = ["Bash(cargo *)"]
```

`model` is passed to the agent tool as it is. Write a full name such as
`claude-opus-5-5` rather than an alias such as `opus`: the log records
what is written, and an alias moves to a newer model when one ships, so
runs on different days would not be comparable. Two seats may name the
same model.

A seat you run yourself, interactively beside the unattended ones, is
marked `start = false`. `ris up` gives it a worktree if it has none and
otherwise leaves it alone: it is not fast-forwarded, resumed or started,
and `ris down` does not pause it.

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

## The four steps

Once the harness is declared, the work runs in four steps, each yours:

1. **Open a phase**: `ris phase open`, and load its items.
2. **Bring it up**: `ris up`; the seats work until nothing is left.
3. **Review**: `ris summary`, then `ris decide` for every decision
   queued while the phase ran.
4. **Close the phase**: `ris phase close`.

There is no check-in in between. A seat that needs a judgement only you
can make queues it as a substantive decision: its item is set aside and
the seat takes another, so nothing raised while the phase runs stops
it. Every queued decision is put to you once, at review.

## 3. Step one: open a phase and load the work

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

## 4. Step two: bring it up

```sh
ris up                                 # make it so; safe to run again
ris status                             # where each seat is, what it holds, what it last did
ris logs rook --follow                 # what rook is doing
ris logs -f                            # every seat at once, each line prefixed with its seat
```

Before it starts any seat, `ris up` checks that the writing seats can
run the project's checks: each check's program is on your PATH, which
the seats inherit, and the build tools a check names (cargo, npm, pip,
uv, go) may write their caches and reach their registries under
`[runner] write` and `network`. Anything missing is named with the line
that allows it, and no seat starts until it is fixed. `ris phase open`
says the same, so you can fix it first. The check reads the
configuration and does not run the build; a tool it does not know is
not checked, and `preflight = false` turns it off for a build that
needs none of it (vendored dependencies, say).

A seat that meets a missing permission while it works reports it with
`ris seat fault "..."`, and takes another item. That is a fault in the
setup, not a decision: `ris status` shows it under the seat until its
next run, and you fix the configuration and run `ris up <seat>`.

`ris up` compares the file with what exists and does only the
difference: a worktree for each builder that has none, each idle
worktree behind the phase branch fast-forwarded to it, paused seats
resumed, and each seat whose agent is not running started in the
background. Run it again after any change to the file; with nothing to
do, it says so.

For each running seat, `ris status` shows what its agent last ran or
said and how long ago its output last changed. A seat quiet for longer
than `[runner] quiet_after_seconds` is marked possibly stuck; `ris logs
<seat>` shows where it stopped.

A worktree is brought up to the phase branch only when that loses
nothing: its branch has no commits of its own, its seat holds no claimed
item, and it has no uncommitted or untracked files. A worktree behind
that fails one of these is left where it is, and `ris up` names it and
says which.

One case is rebased instead. A seat can never write a rules file
(`AGENTS.md`, `CLAUDE.md`), so when the phase branch changes one, a seat
whose branch already holds a commit cannot rebase over the change: its
`ris merge` is refused before its worktree is touched, and names `ris
up <seat>`. That `ris up` rebases the seat's branch onto the phase
branch, keeping its commits, provided its worktree is clean, its agent
has stopped and nothing conflicts; otherwise it says what is in the way.

An unattended agent may do only what its role allows: reading and
`ris`; for a writing seat, edits, `git` and the project's check
commands; for an auditor, reading history. Anything else is refused
rather than asked, and the agent tool's sandbox keeps it to its own
directory and the shared store, with no network beyond `[runner]`.
It can never write the files that set its limits: the configuration,
the checks and the rules. Each run's output is kept under
`.osiris/runs/`, and its start and stop are in the log.

A fourth role, `advisor` (`ris seat add kite --role advisor`), is for
an assistant working beside you: it reads, adds items, sends messages
and raises decisions for you, and `ris` refuses it everything else.

To run an agent yourself instead, start it in its seat's directory with
its name set: `cd ../your-repo-rook && RIS_AGENT=rook claude`.

In a project that builds `ris` itself, `[install]` in the configuration
keeps your `ris` at the last build that passed every check. A merge
records its commit; your next command (or `ris install`) builds that
commit in a checkout outside the repository and installs it where `to`
says, so you never run a seat's work in progress. A build that fails is
reported once and leaves the last good program in place; `ris install`
tries it again.

## 5. Step three: review

```sh
ris summary                            # the phase in one picture: its items and what they wait on
ris decide                             # each queued decision's case, then pick with one key, or o for your own answer
ris map --html map.html                # the phase's change set drawn over the source files, as one page
ris eli5 D-3                           # one decision's full case
ris decide D-3 --choose 1 --rationale "..."
ris action done A-2 --note "what happened"
```

Review comes once the seats have run out of work, or whenever you
choose to look; the seats never wait for it. `ris decide` walks every
queued decision in turn. The options offered are never the only
answers: on every screen `o` asks for your own answer as text and
records it as `ris decide D-n --other "..."` does. Taking one returns the items it alone set
aside to ready: a seat still running takes them, and `ris up` starts
the seats that have stopped.

The summary is one picture of the open phase and nothing else: its
goal, its items counted by state, and a drawing of the items, each with
its state, under what it waits on. An item the phase waits on from an
earlier phase is drawn too, marked with that phase, and so is every
open decision or action attached to a drawn item, whatever phase raised
it. Hard edges (the item waits, or waited on something now merged) are
solid and soft ones (it proceeds) dashed. A footer counts what is left
out and names the command that shows it: `ris decide` for the open
decisions, `ris report` for the report, the decisions taken, failed
checks and hashes, and `ris summary --all` for the long view of every
phase (the decision queue, all work in motion, the report and what is
ready). With no phase open, the summary says so and how to open one.

Where the summary draws the phase's items, `ris map` draws what the
phase did to the code. Its nodes are the repository's source files,
each with the lines the phase added and removed, read from git as the
phase branch against the main line; unchanged files are there too,
faint. Its edges are how the code connects (for Rust, `mod` and `use`
lines, found by scanning). A file that touches the outside (a
database, the filesystem, the network, starting a process), found from
what it imports, is ringed, and so is one you list as a contract or
format:

```toml
[map]
contracts = ["docs/json.md", "crates/*/src/event.rs", "**/*.proto"]
```

A node is larger the more the phase changed it, and larger again for
touching the outside, so the eye lands first on large changes at the
edges of the system. `ris map --html map.html` writes one page that
needs no network (d3 is inlined): hover a file for its path and
numbers, drag it, scroll to zoom. `ris map` alone lists the changed
files, and `ris map --json` prints the graph document the page draws
(`docs/json.md`). After `ris phase close` the same commands show the
phase just closed, and `ris map P-n` an earlier one.

Person actions (A-n) are your own to-do list: only you raise them
(`ris action raise "..." --done-when "..." --blocks I-7`), linked to
the items they hold up. An agent that is blocked raises a setup fault,
an item or a decision instead, and never an action. An action that
names a check closes itself the next time you run `ris inbox`, `ris
summary` or `ris up` once the check passes. A check that runs past
`action_check_seconds` does not pass and is stopped with everything it
started; if stopping it fails, the command says so and why, since what
it started may still be running.

Only substantive decisions wait for you: those expensive to reverse,
such as removing, renaming or changing the meaning of something in a
contract or a format, a dependency, or the phase's scope. An agent takes
a mechanical one itself and logs it, which includes adding to a contract
(a new event, a new JSON key). The report lists the mechanical ones, one
line each, so you can see any that should have come to you.

The auditor's queue is `ris audit`: the open phase's done and merged
items it did not write and has not reviewed. Earlier phases' unreviewed
items are not its work; `ris audit --all` shows them.

## 6. Step four: close the phase

```sh
ris down                               # each seat finishes its item, then stops
ris phase close                        # merges phase/P-1 into main
```

`ris phase close` merges what is merged into the main line and carries
the rest over: an item not merged stays open and is offered in the next
phase. The report it prints says how many items the phase planned, how
many were merged while it was open, and how many carry over; `ris
report` shows the same line for the open phase, as it would read if
closed now.

`ris down` kills nothing: each agent stops at its next claim, and
worktrees and the log are kept. `ris up` after the next `ris phase
open` brings everything back.

`ris` makes commits of its own: the closing merge, the rebase inside a
seat's `ris merge`, and the rebase `ris up` does over a change to the
rules file. Each needs a git identity. Where git has none, the command
is refused before anything is touched and names what to run (`git
config user.name` and `git config user.email`), and the phase or the
item stays as it was.

After the close, `ris map` and `ris map --html map.html` show the
phase just closed, and `ris report` its report.

`ris show <id>` prints any item, decision, action, phase or message in
full.
