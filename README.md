# osiris

A harness for directing several coding agents: you decide at check-ins,
agents build and test in between. Design: `PLAN.md`.

## Quickstart

```
cargo install --path crates/ris
cd your-repo && ris init --local         # nothing tracked changes
# .osiris/config.toml: denylist first, then seats; .osiris/checks.toml
ris phase open "the goal"               # branches phase/P-1 from main
ris item add "a task" --check test       # or: ris import backlog.toml
ris worktree heron                       # ../your-repo-heron, rules loaded
cd ../your-repo-heron && RIS_AGENT=heron claude
ris summary          # check in; answer with ris decide
ris phase close      # review; merges the phase branch into main
```

A lead, a builder and an auditor, step by step: `docs/quickstart.md`.
