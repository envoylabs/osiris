# The `--json` contract

Every `ris` command takes `--json`. The output is for programs: a GUI,
a daemon, a script. Format 1 is described here. Within a format, keys
may be added to `data`; a key is never removed, renamed or given a new
shape without a new format number, so a client ignores keys it does
not know.

## Envelope

Every outcome is one JSON document on stdout, success or failure:

```json
{"format": 1, "ok": true, "data": { ... }}
```

```json
{"format": 1, "ok": false, "error": {
  "kind": "refused",
  "message": "refused: no item I-9",
  "next": "list items with `ris ready` or `ris report`",
  "refusal": {"unknown_item": "I-9"}
}}
```

- `ok` always agrees with the exit status, which keeps its meaning:
  0 success, 1 an error, 2 a refusal or a leak, 3 a failed check.
- Nothing is written to stderr in JSON mode.
- `error.kind` is one of `refused`, `leak`, `role_forbids`,
  `check_failed`, `checks_failed`, `not_a_project`,
  `already_a_project`, `config`, `unknown_agent`, `no_actor`, `contended`, `store`,
  `io`, `other`. `next` is the suggested next step, or null.
- Some kinds carry details: `refused` has `refusal` (the core's reason,
  as `{"<reason>": ...}`); `leak` has `hits`, each with `file`, `line`
  and `category` and never the matched text; `checks_failed` has
  `item`, `stage` and `failed` (`check` and `evidence`); `role_forbids`
  has `seat` and `role`.

## Streams

`ris log` and `ris watch` print one envelope per line, each with one
event as `data`. Without `--json` they print bare event lines.

`ris logs <seat> --follow` prints one envelope per line of the run's
output as it is written, each with `seat` and `line` as `data`. Without
`--follow`, `ris logs` is one document with `seat` and `lines`. With no
seat, `ris logs --follow` streams every seat's output the same way, one
line of output per envelope, and ends once no seat is running; without
`--follow` it is one document whose `lines` are each `seat` and `line`.

## Values

- Ids are strings in display form: `"I-7"`, `"D-5"`, `"A-2"`, `"P-1"`,
  `"M-3"`, and evidence as `"E-<64 hex>"`.
- What an item is blocked on is typed: `{"kind": "decision", "id": "D-5"}`,
  `{"kind": "item", "id": "I-3"}` or `{"kind": "action", "id": "A-2"}`.
- An item's `state` is its stored state. A `ready` item with an
  unmerged dependency also has `waiting_on`, the ids it waits on; the
  text output shows it as `waiting`, and `ris ready` leaves it out.
- Graph edges (`ris summary`'s `graph`) have `from`, `to` and `kind`:
  `blocks` (the target waits), `informs` (it proceeds) or `met` (the
  target depended on an item that has merged). Graph nodes have `id`
  and `state`, and an item node has its `phase`.
- Hashes are 64 lowercase hex characters.
- Actors are typed: `{"kind": "person"}` or
  `{"kind": "agent", "seat": "heron"}`.
- Events are the stored form (event format 3): `structure.event` is a
  snake_case name such as `"decision_raised"`.

## What a client reads most

- `ris summary`: `inbox`, `report`, `ready`, `graph` and `checks`
  (what running the person's action checks did, often empty). By
  default these cover the open phase alone. The `graph` is then the
  phase's picture, which is all the text output draws: every item of
  the phase in whatever state, each unmerged item of an earlier phase
  that one of them waits on, and the open decisions and actions
  attached to any of those, whatever phase raised them; `unlinked`
  counts the nodes with no edge, which are listed all the same.
  `carried_over` counts the unmerged `items` and open `decisions` from
  earlier phases that are left out. `all` says whether `--all` was
  given, in which case nothing is left out and the `graph` is the work
  in motion in every phase: open decisions, open actions and unmerged
  items, where only nodes with an edge are listed.
- `ris report` (and `phase` in `ris phase close`'s data): `tally`,
  the phase's `planned` items, the `merged` count (merges made while it
  was open, carried-in items included) and `carried_over` (items, from
  any phase up to it, neither merged nor superseded; for an open phase,
  what would carry over if it closed now).
- `ris inbox` as the person: `decisions` waiting, `actions` for the
  person, and `checks`.
- `ris audit`: `queue`, the open phase's done and merged items the
  caller has still to review, each with its `id`, `phase`, `goal`,
  `state` and `checks`; with `--all`, earlier phases' too.
- `ris status`: `seats`, each with its role, place, branch, claim,
  `paused`, `running` and `activity`: for a running seat with a run
  file, `last` (what it last ran or said, or null), `quiet_seconds`
  since its output last changed, and `stuck` (quiet for longer than
  `[runner] quiet_after_seconds`); null otherwise; and `faults`, the
  setup faults the seat reported with `ris seat fault` since its latest
  run started, oldest first.
- `ris phase open`: `phase`, `branch`, `docs_item` and `preflight`, what
  the writing seats would lack to run the checks, each
  `{"kind": "program", "check", "program"}`, `{"kind": "write", "tool",
  "path"}` or `{"kind": "network", "tool", "host"}`; empty when nothing
  is missing or `[runner] preflight` is off. `ris up` refuses with the
  same findings, as a `config` error, before it starts any seat.
- `ris show A-n`: a person action, with `blocks`, `informs`, `state`
  and, when it has one, its `check`.
- `ris decide` with no decision: `queue`, the open substantive
  decisions oldest first, whatever phase raised them, each entry the
  `data` that `ris eli5 D-n` gives (`decision`, and `explanation` or
  null). A client shows each and answers with
  `ris decide D-n --choose <k>`; `--rationale` is optional.
- `ris watch`: every event as it is appended, including `run_started`
  and `run_stopped` when the runner starts and stops a seat's agent,
  and `setup_fault` (with `seat`) when a seat reports a fault in its
  setup.

`crates/ris/tests/json_contract.rs` pins the envelope and the top-level
keys of the most-read commands' `data`.

## The map document

`ris map` gives the phase's change set as a graph over the source
code: the open phase's, or with none open the last closed one's, or
that of the phase named (`ris map P-3`). Its `data` is the graph
document. The document is the contract and holds everything there is
to draw: `ris map --html <file>` writes a page that embeds the same
document, as JSON in the element `<script id="map-data"
type="application/json">`, and draws nothing that is not in it, so any
other renderer draws the same map from either source. It follows the
rule of the rest of format 1: keys are added, never removed, renamed
or reshaped.

```json
{
  "phase": "P-3",
  "base": "<commit>",
  "tip": "<commit>",
  "tip_ref": "phase/P-3",
  "totals": {"files": 64, "changed": 12, "added": 371, "removed": 48, "outside": 43},
  "nodes": [
    {"path": "src/store.rs", "language": "rust", "status": "modified",
     "added": 12, "removed": 3, "external": ["filesystem"],
     "contract": false, "weight": 5.77}
  ],
  "edges": [
    {"from": "src/store.rs", "to": "src/wire.rs", "kind": "use"}
  ]
}
```

- `base` and `tip` are the commits compared: where the phase left the
  main line, and the phase's tip, whose files are the nodes. `tip_ref`
  says what named the tip: the phase's branch while it exists, `HEAD`
  for an open phase with no branch of its own, or the closing merge
  for a closed phase. `base` is null when where the work left the main
  line is not known, and then no node carries a diff. `phase` is null
  in a project that has never opened one.
- `nodes` are the repository's source files, sorted by `path`, which
  is relative to the repository root and is the node's identity. A
  file is source by its extension; `language` names it. A file that
  matches a `[map] contracts` pattern in `.osiris/config.toml` is a
  node as well, with `language` null when it is not source code.
  Files that are neither are not on the map, whatever the phase did to
  them.
- `status` is `added`, `modified`, `deleted` or `unchanged` in the
  phase, and `added` and `removed` count the lines the phase added and
  removed (`git diff --numstat` of `base` against `tip`, renames not
  followed). A deleted file is a node with no edges.
- `external` says how the file touches the outside, found from what it
  imports: any of `database`, `filesystem`, `network` and `process`,
  sorted; empty when it does not. `contract` is whether a `[map]
  contracts` pattern names it.
- `weight` is how much the node should draw the eye. It is 1 for a
  file nothing happened to, grows with the size of the diff (on a log
  scale) and is 2 higher for a file that is external or a contract. A
  renderer may size by it; the exact formula is not part of the
  contract.
- `edges` are how the code connects, each from the file that names the
  other, sorted. `kind` is the kind of line that makes the edge. For
  Rust: `mod` (a `mod name;` line, to the module's file) and `use` (a
  `use` line or a qualified path through `crate`, `self`, `super` or a
  package of the repository, to the deepest module file it names).
  Edges are found by scanning the text, per language; a language with
  no scanner yet has nodes and no edges, and gaining one adds edges
  and `kind` values without changing the document's shape.
- `totals` counts the nodes (`files`), those not `unchanged`
  (`changed`), their lines `added` and `removed`, and the nodes that
  are external or a contract (`outside`).

`crates/ris/tests/map.rs` pins these keys.
