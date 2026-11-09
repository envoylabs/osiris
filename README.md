# osiris

A harness for directing several coding agents: you decide at check-ins,
agents build and test in between. Design: `PLAN.md`.

## Quickstart

```
cargo install --path crates/ris
cd your-repo && ris init                 # .osiris/ and OSIRIS.md
# add @OSIRIS.md to CLAUDE.md; seats and denylist in .osiris/config.toml,
# checks in .osiris/checks.toml
ris phase open "the goal"               # branches phase/P-1 from main
ris item add "a task" --check test
git worktree add ../repo-kovacs -b kovacs        # one per agent
cd ../repo-kovacs && RIS_AGENT=kovacs claude     # likewise for each
ris summary          # check in; answer with ris decide
ris phase close      # review; merges the phase branch into main
```
