# Quickstart: a lead, a builder and an auditor

This sets up three agent seats on an existing git repository:

| seat  | role      | works in                    |
|-------|-----------|-----------------------------|
| heron | `lead`    | its own worktree            |
| rook  | `builder` | its own worktree            |
| wren  | `auditor` | the main checkout, read-only |

The person works in the main checkout with `RIS_AGENT` unset.

## 1. Install and initialise

```sh
cargo install --path crates/ris        # from the osiris repository
cd your-repo
ris init --local
```

`--local` leaves nothing tracked: `.osiris/` and `CLAUDE.local.md` are
listed in `.git/info/exclude`, which git never commits, and the rules
the agents read go in `.osiris/OSIRIS.md`. `git status` is unchanged.

## 2. Configure

`.osiris/config.toml` holds the denylist and the seats. It stays on this
machine and is never exported. Fill the denylist first:

```toml
# Words that must never appear in harness state or commits: your name,
# the client's name, your machine's hostname. Case-insensitive.
denylist = ["your surname", "client name"]
```

The template comes with two seats, heron (lead) and wren (auditor).
Add the builder:

```sh
ris seat add rook --role builder
```

A seat name is a pseudonym, refused if the scan or the denylist would
refuse it, and never reused once it is in the log.

Top-level keys such as `denylist` go above the first `[agents.*]` table.
`ris phase open` refuses while the denylist is empty, because text is
scanned as it is written and the log cannot be redacted; set
`confirm_empty_denylist = true` if an empty list is deliberate.

For a repository merged through a git server's web interface, set
`merge_on_close = false` (phase close leaves the branch for a pull or
merge request) and, if the project's commit convention has no room for
them, `trailers = false`.

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
ris phase open "the goal"              # creates phase/P-1 and switches to it
ris item add "a task" --check test     # one at a time, or
ris import backlog.toml --dry-run      # a backlog file: validate,
ris import backlog.toml                # then import
```

The osiris-onboard skill (`skills/osiris-onboard`) drafts the import
file from an existing project's backlog.

## 4. Give each writing seat a worktree

```sh
ris worktree heron                     # ../your-repo-heron, branch heron
ris worktree rook                      # ../your-repo-rook, branch rook
```

Run this after `ris phase open`: a seat's branch starts from whatever the
main checkout has checked out, which must be the phase branch. Each
worktree gets an excluded `CLAUDE.local.md` importing the rules, and all
worktrees share the one store.

The auditor needs no worktree. It never writes, and every `ris merge`
moves the main checkout's phase branch forward, so the main checkout
always shows what has been merged.

## 5. Start the agents

One terminal each:

```sh
cd ../your-repo-heron && RIS_AGENT=heron claude
cd ../your-repo-rook  && RIS_AGENT=rook  claude
cd your-repo          && RIS_AGENT=wren  claude
```

The builders loop on `ris ready`, `ris claim`, `ris done`, a commit and
`ris merge`. The auditor works through `ris audit`. The rules file
(`ris rules` prints it) tells each seat what its role does.

## 6. Check in and close

```sh
ris summary                            # decisions waiting, the report, what is ready
ris decide D-3 --choose 1 --rationale "..."
ris phase close                        # review; merges phase/P-1 into main
```

`ris show <id>` prints any item, decision, phase or message in full.
