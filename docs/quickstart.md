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
phase's work; `ris phase close` merges it into the main line. Where
your project's convention needs another name, `ris phase open "<goal>"
--branch <name>` creates the phase's branch under it: the name is
recorded with the phase's opening, so merges, `ris up`, `ris status`,
`ris map` and the closing merge all use it, and `ris summary` and
`ris report` show it. The open is refused, with nothing created, when
the name is not a valid branch name, is the main line or is a branch
that already exists, and when `phase_branches` is off.

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

A repository that commits its agents' rules file, with rules of its own
beside those `ris` gives every project, keeps its own part in a file
and renders the rules file from the two:

```sh
ris rules --render docs/project-rules.md --to AGENTS.md
```

The file is the rules `ris rules` prints, then the project's part as
written, under a first line naming the command. Only you run it, since
a seat never writes its own rules, and you run it once: after that the
file is kept current for you. A seat merges a change to the project's
part (or, in the osiris repository, to the rules themselves) and
leaves the rules file alone, and `ris phase close` renders the file
again into the closing merge when it is no longer what the two render,
and says so in a line of its own. It finds the file by that first
line, so nothing is configured, and it renders only where it makes the
merge: with `merge_on_close = false`, or with the phase's work already
on the main line, you render and commit the file yourself.

Until the phase closes, the phase branch may hold a rules file that is
behind its sources: no check a seat runs reads it, so a change to the
rules never waits for you. To hold the main line to it, run the check
there:

```sh
ris rules --render docs/project-rules.md --to AGENTS.md --check
```

It writes nothing, anyone may run it, and it exits 3 with the first
line that differs when the file is not what the command would render.
The osiris repository's own `AGENTS.md` is kept this way, and its CI
runs the check on the main line.

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
`merge_on_close = false` and, if the project's commit convention has no
room for them, `trailers = false`. The closing merge is then the
server's, and the phase stays open until it is made, so a failed build
or a reviewer's change there still has an open phase to take the fix:

```sh
ris phase review                       # names the branch to push
git push origin phase/P-1              # yours to run: ris pushes and fetches nothing
# ...the pull or merge request is merged on the server...
git pull                               # on the main line
ris phase close                        # closes once the phase's work is on the main line
```

`ris phase close` reads only your local repository. It finds the
phase's work on the main line whether the server made a merge commit
or moved the main line forward (the branch's tip is in the main line's
history), rebased the commits (each has a commit there making the same
change) or squashed them (one commit there makes the branch's whole
change). Found, it closes the phase, records the commits on the main
line where the work starts and ends, and deletes the local branch.
Not found, it refuses, the phase stays open, and it says what to do
next. A merge it cannot recognise, such as a squash resolved by hand
on the server, you name: `ris phase close --merged-at <commit>`, which
is refused for a commit that is not on the main line. `ris map P-n`
for a phase closed this way reads the recorded commits, so it does not
depend on the branch or on what the server called its merge.

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
3. **Review**: `ris phase review`. It shows the phase's state as `ris
   summary` does, walks every decision queued while the phase ran as
   `ris decide` does, then lists the change set ranked by risk, as
   `ris map --list` does, and records the review at the tip of the
   phase's work. It may be run at any point while the phase is open,
   on what has merged so far.
4. **Close the phase**: `ris phase close`. It refuses a phase whose
   work has moved since it was reviewed (an item merged, or a commit
   landed on its branch), and names the review; `ris phase close
   --unreviewed` closes without one. Once closed, the phase's change
   set stays readable with `ris map P-n`.

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
ris item bring-in I-7                  # an unstarted item from the backlog, into this phase
ris item add "later" --phase P-2       # planned for a phase not open yet
ris item phase I-7 P-2                 # the same for an item there already; `none` untags it
ris item retire I-7 I-8 --reason "..." # out of the work for good, with nothing to replace them
```

Every phase gets an item to bring the docs up to date, and `ris phase
close` waits for it (turn it off with `docs_item = false`). The
osiris-onboard skill (`skills/osiris-onboard`) drafts the import file
from an existing project's backlog.

Seats work only the open phase's items. An item's phase is a tag: the
open phase, a later one by its number, or none, which is the backlog.
An item added while a phase is open is that phase's unless `--phase`
says otherwise; one added with no phase open has none; and one left
unmerged when its phase closes loses its tag. A seat's `ris ready`
shows the open phase's items and nothing else, and `ris claim` refuses
any other, naming the command that brings it in. Your own `ris ready`
lists the open phase's items first and the rest after them.

Planning needs no open phase. `ris item add --phase P-n` and `ris item
phase I-n <P-n|none>` tag an item nobody has started, and `ris
summary` with no phase open lists what is tagged for the next one.
`ris phase open` opens with those items in it and lists them; with
`--move-on` it moves their tags to the phase after instead. Tagging
for a later phase is yours and an advisor seat's; a lead, builder or
auditor adds only to the open phase or to the backlog, and moving an
item into or out of the open phase (`ris item bring-in`, or `ris item
phase`) is yours alone. The item keeps the phase it was added in for
that phase's report, so a phase's tally counts the items added while
it was open and does not change when one of them is tagged elsewhere.

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
<seat>` shows where it stopped. For a seat that has run and is not
running, it says why it stopped, beside the seat: nothing left to
claim, paused by the person, a setup fault, the agent tool's usage
limit (with the time it resets, when the tool says), or ended for any
other ending, read from how its last run ended.

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
branch, keeping its commits, provided its worktree is clean and its
agent has stopped; otherwise it says what is in the way. When the
seat's commits conflict with the phase branch outside the rules files,
`ris up` does the part only it can: it keeps the commits on a branch
named `<seat>-kept-<commit>`, moves the seat's worktree to the phase
branch, and sends the seat a handoff naming the `git cherry-pick` that
brings the commits back, so the seat resolves the conflict in its own
worktree. A conflict in a rules file itself is left for the person.

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
tries it again. A phase is closed by that build and no older one: when
`ris phase close` is itself the command that installs it, the close is
refused with nothing touched and asks to be run again, since the
closing merge carries what `ris` renders (the rules file). For the
same reason a close run by any other `ris` than the one at `to` (an
older one earlier on your `PATH`, one built by hand) is refused, and
names the installed one.

## 5. Step three: review

```sh
ris summary                            # the phase in one picture: its state, each seat, its items and what they wait on
ris decide                             # each queued decision's case, then pick with one key, or o for your own answer
ris map                                # the phase's change set drawn over the source files, as one page in a window or the browser
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

The summary opens with the phase's state in one line, for a glance:
that all of its items are merged and it is ready to review with `ris
phase review`, then once reviewed that it is ready to close with `ris
phase close`, or how many of the phase's items are merged, how many are in progress and
with which seats, how many are set aside and how many not started,
then whatever waits on you with the command for it (open decisions:
`ris decide`; a seat's setup fault: `ris status`, then `ris up
<seat>`), and, when items remain and no seat is running, that `ris up`
starts them. Superseded and retired items are not counted.

Below that line the summary is one picture of the open phase and
nothing else: its
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
ready). With no phase open, the summary says so and how to open one,
and lists the items tagged for the phase that opens next.

Where the summary draws the phase's items, `ris map` draws what the
phase did to the code. Its nodes are the repository's source files,
each with the lines the phase added and removed, read from git as the
phase branch against the main line; unchanged files are there too,
faint. Its edges are how the code connects: for Rust, `mod` and `use`
lines, found by scanning; for TypeScript, relative imports; for Go,
imports of the repository's own packages; for Clojure, required
namespaces. A file that touches the outside (a
database, the filesystem, the network, starting a process), found from
what it imports, is ringed, and so is one you list as a contract or
format:

```toml
[map]
contracts = ["docs/json.md", "crates/*/src/event.rs", "**/*.proto"]
tests = ["**/tests/**", "**/*_test.go"]
```

Test files are left off the map, since the behaviour lands in the
source and the tests only verify it; one line counts the test files
the phase changed and the lines added and removed in them. `tests`
names them, and by default it is each language's convention: every
file under a `tests`, `test` or `__tests__` directory, and every file
named as a test (`*_test.go`, `*.test.ts`, `*.spec.ts`, `*_test.clj`).
A node is larger the more the phase changed it, so the eye lands first
on the large changes, and the ring picks out those at the edges of the
system. An amber diamond beside a changed file is its risk: hollow for
medium, filled for high, none for low. `ris` computes the level from
what it can count (the file touches the outside, is a contract, the
size of its diff, how many files depend on it), and an auditor seat
records its own judgement as it reviews, with one line of why
(`ris audit --file <path> --risk <level> --why "..."`); the judgement
stands where there is one, and hovering shows both and the reasons.
`ris map`, run by you at a terminal, writes one page to
`.osiris/map.html` (which git ignores), replacing the last one, opens
it in your browser and prints where it is; if the browser does not
open, it says so and you open the file yourself. A `ris` built with
its `window` feature (`cargo build --release -p ris --features window`;
put the same command in `build` under `[install]` and the installed
`ris` has it) opens the page in a native window of its own instead:
`ris map` returns once the page is shown, and the window stays until
you close it. That build needs the system's web view: on macOS it is
part of the system, and on Linux it is the package
`libwebkit2gtk-4.1-dev`. The page needs no
network (d3 is inlined) and has two views of the same files, switched
in its header. It opens on the list: the changed files ranked by risk,
the highest first, each with its lines added and removed and the
reasons for its level. The graph is the other view: hover a file for
its path and numbers, drag it, scroll to zoom. In either, click a file
to read its diff for the phase in a pane beside it (click another file
to swap it, Esc or the pane's button to close it). `ris map` also
lists the changed files with their risk, the highest first; that list
is all a seat gets, and all you get when the output is piped, since
nothing is opened then. `ris map --list` prints the same list as the
page's, with each file's reasons, and opens nothing.

Inside a changed Rust, TypeScript, Go or Clojure file the map also
finds the functions the phase changed and marks each line that touches
a database, the filesystem, the network or a process, with the function
it sits in and that function's level. It also follows calls one step:
a changed function that calls a function touching the outside triggers
that effect, and one called by such a function feeds it; each is
listed with the function and effect it reaches. A call is followed only
where the text says which function it means: by its path, by its own
file, or for a method by the type of the value it is called on (`self`,
a parameter or a `let` with its type). A method on any other value is
not followed, so a step can be missed and a common method name flags
nothing. A function most of the repository calls or is called by (a
command's dispatcher, say) is said once for a file, and a step to it
does not raise a function's level. `ris map --list` prints a file's
marked lines under its reasons and then the count of its one-step
functions; `ris map --list --steps` names each. On the page a file
with any expands to show its marked lines, with the one-step functions
folded under their count.
Click one of those lines, or the page's calls view, to see one function
laid out left to right as execution flows: what calls it, the function,
what it calls, and the effects reached, each a box with its own lines,
the phase's changes marked and its effect sites labelled. Click a box
beside it to move there. The page carries the functions one call from
a changed function in full; one call further, it names them only. What counts is a list of names per language that ships
with `ris`; add your own (a module of your project that wraps the
database, say) under `[map.effects.<language>]` in
`.osiris/config.toml`. Files of other languages stay at file level.
`ris map --html map.html` writes the page to a file you name and opens
nothing, and `ris map --json` prints the graph document the page draws
(`docs/json.md`). After `ris phase close` the same commands show the
phase just closed, and `ris map P-n` an earlier one. `ris map A..B`
maps a git range instead (`ris map main..feature`, `ris map
v1.2..v1.3`), in any repository's history, a project or not.

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
the rest over: an item not merged loses its phase tag and stays open,
in the backlog, until you tag it for a later phase with `ris item
phase` or bring it into the open one with `ris item bring-in`. An
item that will not be built, and that nothing replaces, you retire
with `ris item retire I-n --reason "<why>"`: it leaves the work for
good, is never carried over again, and `ris show I-n` gives the
reason. An item still in the work that waits on it is retired in the
same command, or the retirement is refused. The report it prints says how many items the phase opened with (those
tagged for it before it opened), how many were added while it was
open (planned), how many were merged while it was open, and how many
carry over; `ris report` shows the same line for the open phase, as
it would read if closed now.

`ris down` kills nothing: each agent stops at its next claim, and
worktrees and the log are kept. `ris up` after the next `ris phase
open` brings everything back.

`ris` makes commits of its own: the closing merge (which carries the
rules file rendered again, when the phase changed what it is rendered
from), the rebase inside a
seat's `ris merge`, and the rebase `ris up` does over a change to the
rules file. Each needs a git identity. Where git has none, the command
is refused before anything is touched and names what to run (`git
config user.name` and `git config user.email`), and the phase or the
item stays as it was.

After the close, `ris map` and `ris map --html map.html` still show the
phase just closed, and `ris report` its report.

`ris show <id>` prints any item, decision, action, phase or message in
full.
