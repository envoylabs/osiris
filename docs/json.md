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
`--follow`, `ris logs` is one document with `seat` and `lines`.

## Values

- Ids are strings in display form: `"I-7"`, `"D-5"`, `"A-2"`, `"P-1"`,
  `"M-3"`, and evidence as `"E-<64 hex>"`.
- What an item is blocked on is typed: `{"kind": "decision", "id": "D-5"}`,
  `{"kind": "item", "id": "I-3"}` or `{"kind": "action", "id": "A-2"}`.
- An item's `state` is its stored state. A `ready` item with an
  unmerged dependency also has `waiting_on`, the ids it waits on; the
  text output shows it as `waiting`, and `ris ready` leaves it out.
- Graph edges (`ris summary`'s `graph`) have `from`, `to` and `kind`,
  either `blocks` (the target waits) or `informs` (it proceeds).
- Hashes are 64 lowercase hex characters.
- Actors are typed: `{"kind": "person"}` or
  `{"kind": "agent", "seat": "heron"}`.
- Events are the stored form (event format 3): `structure.event` is a
  snake_case name such as `"decision_raised"`.

## What a client reads most

- `ris summary`: `inbox`, `report`, `ready`, `graph` (the nodes and
  edges of work in motion) and `checks` (what running the person's
  action checks did, often empty). By default these cover the open
  phase alone, and `carried_over` counts the `items` and open
  `decisions` from earlier phases left out; `all` says whether
  `--all` was given, in which case nothing is left out.
- `ris inbox` as the person: `decisions` waiting, `actions` for the
  person, and `checks`.
- `ris status`: `seats`, each with its role, place, branch, claim,
  `paused` and `running`.
- `ris show A-n`: a person action, with `blocks`, `informs`, `state`
  and, when it has one, its `check`.
- `ris watch`: every event as it is appended, including `run_started`
  and `run_stopped` when the runner starts and stops a seat's agent.

`crates/ris/tests/json_contract.rs` pins the envelope and the top-level
keys of the most-read commands' `data`.
