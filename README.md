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

A harness for directing several coding agents in four steps: you open a
phase, bring the agents up, review, and close the phase. Agents build
and test in between and never wait for you; what needs your judgement
is queued for the review. Design: `PLAN.md`.

## Quickstart

```
cargo install --path crates/ris
export RIS_AGENT=person                  # you; agents get their seat's name
cd your-repo && ris init --local         # nothing tracked changes
# .osiris/config.toml: the denylist, the seats, what agents may do
ris phase open "the goal"               # 1. branches phase/P-1 from main
ris item add "a task"                    #    or: ris import backlog.toml
ris up                                   # 2. worktrees, then every seat's agent
ris summary && ris decide                # 3. review: the graph, then each queued decision
ris down && ris phase close              # 4. merges the phase; the rest carries over
```

A lead, a builder and an auditor, step by step: `docs/quickstart.md`.
Building a client on `ris --json`: `docs/json.md`.
