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
cargo install --path crates/ris          # --features window: the map in a native window
export RIS_AGENT=person                  # you; agents get their seat's name
cd your-repo && ris init --local         # nothing tracked changes
# .osiris/config.toml: the denylist, the seats, what agents may do
ris phase open "the goal"               # 1. branches phase/P-1 from main; --branch names it
ris item add "a task"                    #    or: ris import backlog.toml
ris up                                   # 2. worktrees, then every seat's agent
ris phase review                         # 3. review: the phase, each queued decision, its changes ranked by risk
ris map                                  #    and what the phase did to the code, as one page in a window or the browser
ris map main..feature                    #    or any git range, in any repository: Rust, TypeScript, Go, Clojure
ris down && ris phase close              # 4. merges the phase; the rest carries over
```

Where a git server makes the merge (`merge_on_close = false`), step 4
waits for it: push the phase's branch, have it merged there, pull, and
`ris phase close` closes the phase once its work is on the main line.

A lead, a builder and an auditor, step by step: `docs/quickstart.md`.
Building a client on `ris --json`: `docs/json.md`, with its JSON Schema
in `docs/json.schema.json` (`ris schema` prints it).
