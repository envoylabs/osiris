# The `--json` contract

Every `ris` command takes `--json`. The output is for programs: a GUI,
a daemon, a script. Format 1 is described here; any change to a shape
below is a new format number.

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

## Values

- Ids are strings in display form: `"I-7"`, `"D-5"`, `"P-1"`, `"M-3"`,
  and evidence as `"E-<64 hex>"`.
- Hashes are 64 lowercase hex characters.
- Actors are typed: `{"kind": "person"}` or
  `{"kind": "agent", "seat": "heron"}`.
- Events are the stored form (event format 3): `structure.event` is a
  snake_case name such as `"decision_raised"`.

`crates/ris/tests/json_contract.rs` pins the envelope and the top-level
keys of the most-read commands' `data`.
