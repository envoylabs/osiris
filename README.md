# osiris

```
                                      osirisosirisosir
                              risosiris              isosi
                         risosi                          ris
                    risosi                                 o
                 sosi                                      s
              siri                                        ri
           riso                                          si
         osi                                           iso
       ris                                           sir
      si                                          riso
     so                                        sosi
     i                                      siri
     r                                 siriso
     iso                          siriso
       siris              isosiriso
           osirisosirisosir
```

A harness for directing several coding agents: you decide at check-ins,
agents build and test in between. Design: `PLAN.md`.

## Quickstart

```
cargo install --path crates/ris
export RIS_AGENT=person                  # you; agents get their seat's name
cd your-repo && ris init --local         # nothing tracked changes
# .osiris/config.toml: the denylist, the seats, what agents may do
ris phase open "the goal"               # branches phase/P-1 from main
ris item add "a task"                    # or: ris import backlog.toml
ris up                                   # worktrees, then every seat's agent
ris summary                              # check in: decisions, actions, the graph
ris down && ris phase close              # review; merges the phase into main
```

A lead, a builder and an auditor, step by step: `docs/quickstart.md`.
Building a client on `ris --json`: `docs/json.md`.
