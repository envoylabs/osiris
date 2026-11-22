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
  `carried_over` counts the unmerged `items` and open `decisions`
  outside the open phase that are left out. `next_phase` is `null`
  with a phase open; with none open it is the `phase` that opens next
  and the `items` tagged for it. `phase_state` is where the open
  phase stands, `null` with no phase open: `line` (the one line the
  text output opens with), `items` (the phase's items, superseded and
  retired ones left out), `merged`, `in_progress` (claimed, or done and not yet
  merged; each an `item` and the `seat` on it), `set_aside` (the
  blocked items' ids), `not_started`, `open_decisions` (the phase's
  open substantive decisions, which wait on the person), `faults` (the
  seats with a setup fault), `running` (the seats whose agent is
  running; a run whose process is gone is not counted) and
  `ready_to_close` (every item merged, and at least one) and
  `reviewed` (the person's review with `ris phase review` stands at
  the tip of the phase's work now, as `ris phase close` asks: nothing
  has reached the phase's branch since). `merged`,
  `in_progress`, `set_aside` and `not_started` add up to `items`. It
  is the open phase's whether or not `--all` is given, though only the
  default text output shows the line. `seats` is every configured
  seat, as the text shows them under that line: `seat`, `role`,
  `running` (its agent is running now; a run whose process is gone is
  not), `claim` (the item it holds, or `null`), `activity` (for a
  running seat, as in `ris status`), `stopped` (for a seat that has
  run and is not running, why, as in `ris status`), `attended` (the
  person runs it, and `ris up` does not start it) and `next` (the
  command that brings a seat that is not running back; `null` while
  it runs and for an attended seat). `all` says whether `--all` was
  given, in which case nothing is left out and the `graph` is the work
  in motion in every phase: open decisions, open actions and unmerged
  items, where only nodes with an edge are listed.
- `ris phase review`: `phase`, `tip` (the commit at the head of the
  phase's work that was reviewed; `null` when the project is not in
  git), `taken` (decisions taken during the review's walk of the
  queue; with `--json` there is no walk, so none), `open` (substantive
  decisions still open) and `map` (the change set, as `ris map --json`
  gives it; `null` when it cannot be read). The review is recorded as a
  `phase_reviewed` event carrying `phase` and `tip`. `ris phase close`
  refuses unless the latest review stands at the tip of the phase's
  work now; `--unreviewed` closes without one.
- `ris report` (and `phase` in `ris phase close`'s data): `tally`,
  the items the phase `opened_with` (those tagged for it before it
  opened that `ris phase open` brought in, not those it moved on), the
  phase's `planned` items (added while it was open), the `merged` count (merges made while it
  was open, carried-in items included) and `carried_over` (items, from
  any phase up to it, neither merged, superseded nor retired; for an
  open phase, what would carry over if it closed now). `retired` counts
  the items the person has retired in the whole project; the report
  lists none of them, and its text gives the count on a `retired:` line
  when there is at least one.
- `ris ready`: for a seat, the open phase's items it may claim and no
  other. For the person, those first and then every other item that
  could be claimed were it in the phase: the backlog, and what is
  tagged for later phases.
- An item's `phase`, wherever an item record appears, is its tag: the
  phase it is in, a later phase it is planned for, or `null` when it
  has none, which is the backlog. `ris item add` gives `item` and the
  `phase` it was tagged for: the open phase, or what `--phase P-n` or
  `--phase none` named, or `null` with no phase open. `ris item phase
  I-n <P-n|none>` gives the same two keys for an item nobody has
  started. The event `item_added` carries the phase open when the item
  was added, `null` when none was; every later change of tag is an
  `item_scoped` event (with `item` and `phase`, `null` for none): one
  for a tag given at `ris item add` that is not the open phase, one
  for `ris item phase` and `ris item bring-in`, one per item whose tag
  `ris phase open --move-on` moves to the next phase, and one per
  unmerged item of a phase when it closes, after its `phase_closed`
  event. An item's current phase is the phase of the last of its
  `item_added` and `item_scoped` events; the phase it was added in does
  not change, and a phase's `tally` counts by that. The refusals:
  `{"phase_is_closed": "P-n"}` for a tag naming a closed phase,
  `{"already_tagged": {"item", "phase"}}`,
  `{"scope_is_for_the_person": "I-n"}` for a seat moving an item into
  or out of the open phase, and `role_forbids` for a seat that is not
  an advisor tagging for a phase that is not open.
- `ris item bring-in I-n`, the person's only: `item` and the open
  `phase` it is now in. A claim of an item outside the open phase is
  refused with `{"not_in_open_phase": {"item", "phase"}}`, `phase`
  being `null` when the item has none, and an agent's bring-in with
  `{"bring_in_is_for_the_person": "I-n"}`.
- `ris item retire I-n [I-m ...] --reason "<why>"`, the person's only:
  `items`, those retired, each once, in the order named. One
  `items_retired` event records them all, its text part carrying the
  one `reason`. A retired item's `state` is `retired` and its record
  carries `retired_reason`; it is final, never offered, claimed,
  brought in or tagged, and `ris summary --all` leaves it out. It takes
  an item that is ready, blocked or done and never merged; a claimed,
  merged, superseded or retired one is refused with
  `illegal_item_transition`, and one refusal refuses the whole command.
  The other refusals: `retire_is_for_the_person`, `retire_needs_reason`
  (an empty reason), `nothing_to_retire`,
  `{"retire_waited_on": {"item", "waiting"}}` when an item that is not
  merged, superseded or retired, and not named in the same command,
  waits on it (directly, or through an item it replaced), and
  `{"depends_on_retired": "I-n"}` for `ris item add --depends` naming a
  retired item.
- `ris inbox` as the person: `decisions` waiting, `actions` for the
  person, and `checks`.
- `ris audit`: `queue`, the open phase's done and merged items the
  caller has still to review, each with its `id`, `phase`, `goal`,
  `state` and `checks`; with `--all`, earlier phases' too. With an
  item, the review recorded: `item` and `by`. With `--file <path>
  --risk <low|medium|high> --why "<one line>"`, an auditor's risk
  judgement of a file the open phase changed, recorded: `phase`,
  `path`, `level` and `seat`. The map document carries it (below).
- `ris status`: `seats`, each with its role, place, branch, claim,
  `paused`, `running` and `activity`: for a running seat with a run
  file, `last` (what it last ran or said, or null), `quiet_seconds`
  since its output last changed, and `stuck` (quiet for longer than
  `[runner] quiet_after_seconds`); null otherwise; and `faults`, the
  setup faults the seat reported with `ris seat fault` since its latest
  run started, oldest first; and `stopped`, for a seat not running that
  has run, why it stopped (null otherwise): `reason`, the first of
  `usage_limit` (the agent tool's last result says it hit its usage
  limit), `setup_fault` (it reported one in that run), `paused` (by the
  person), `nothing_left` (its run finished normally holding no item)
  and `ended` (anything else: killed, an error, or a stop while holding
  an item); and `resets`, for `usage_limit`, when the limit resets in
  the tool's own words, or null.
- `ris phase open`: `brought_in`, the items that were tagged for the
  phase and are in it, or `moved_on`, the same items when `--move-on`
  moved their tags to the phase after (the other is empty); and
  `phase`, `branch`, `docs_item` and `preflight`, what
  the writing seats would lack to run the checks, each
  `{"kind": "program", "check", "program"}`, `{"kind": "write", "tool",
  "path"}` or `{"kind": "network", "tool", "host"}`; empty when nothing
  is missing or `[runner] preflight` is off. `ris up` refuses with the
  same findings, as a `config` error, before it starts any seat.
- `ris show A-n`: a person action, with `blocks`, `informs`, `state`
  and, when it has one, its `check`.
- `ris rules`: `rules`, the agents' rules as text. With `--render
  <project part> --to <rules file>`, the person's only, it writes the
  rules file (those rules, then the project's part) and gives `source`
  and `rendered`, the two paths as given, and `changed`, whether the
  file differed from what was there. With `--check` added, anyone's, it
  writes nothing and gives `source`, `checked` (the rules file) and
  `current`, which is true: a file that is not what the two render is
  the error `check_failed`, exit status 3.
- `ris decide` with no decision: `queue`, the open substantive
  decisions oldest first, whatever phase raised them, each entry the
  `data` that `ris eli5 D-n` gives (`decision`, and `explanation` or
  null). A client shows each and answers with
  `ris decide D-n --choose <k>`, or with
  `ris decide D-n --other "<an answer that is none of the options>"`,
  which it should always offer; `--rationale` is optional. Either
  gives `decision` and `unblocked`, the items the decision alone had
  set aside. An answer outside the options is recorded as one more
  option: `chosen` is its number, one past those offered, and `other`
  its text.
- `ris watch`: every event as it is appended, including `run_started`
  and `run_stopped` when the runner starts and stops a seat's agent,
  `setup_fault` (with `seat`) when a seat reports a fault in its
  setup, and `risk_judged` (with `phase`, `seat` and `level`) when an
  auditor records a risk judgement.

`crates/ris/tests/json_contract.rs` pins the envelope and the top-level
keys of the most-read commands' `data`.

## The schema

`docs/json.schema.json` is the JSON Schema (draft 2020-12) of format 1,
for a client to validate against and to generate its types from.
`ris schema` prints the same document, from any directory.

- The document itself is the envelope: a success or a failure, each
  with `format` 1.
- `#/$defs/envelope.failure` holds the error: its `kind` (the list
  above, as an enumeration), `message`, `next`, and the details each
  kind carries, a refusal's reason included.
- `#/$defs/data.<command>` is the `data` of one command, with `_` for
  each space in the command: `data.ready`, `data.phase_open`,
  `data.decision_raise`. For a stream (`log`, `watch`, a followed
  `logs`) it is the data of one line. A command whose data depends on
  what it was asked (`show`, `inbox`, `decide`, `audit`, `logs`,
  `report`) is an `anyOf` of its forms.
- Ids, hashes and evidence ids are strings with a `pattern`; actors,
  blockers and events are the typed objects described under Values.

A client validates an envelope against the document and then its
`data` against the definition of the command it ran; the envelope alone
accepts the data of any command.

The schema is generated from the Rust types that produce the output
(`crates/ris/src/schema.rs`, and the core's event and refusal types):
no command can build its output from anything else, so the two cannot
drift apart. Tests hold the rest: the committed file is regenerated and
compared, every command of the CLI must have a definition, and every
command's real output, and every error kind a run can reach, is
validated against the committed file (`crates/ris/tests/schema.rs`).

The compatibility rule in schema terms. Within format 1:

- A property may be added to an object, and a definition to `$defs`.
  The schema's objects are open (no `additionalProperties: false`),
  so a client holding an older copy of the schema still accepts what a
  newer `ris` writes. Three definitions are closed, each an object
  that is one of several named forms: `Actor`, `Refusal` and
  `Structure` (a stored event's structure, whose form is the event
  format's). The tests validate with every object closed, so a key
  that `ris` writes and no type declares fails there.
- A property is never removed from `properties` or from `required`,
  never renamed, and never given another `type`, `pattern` or shape;
  a value is never removed from an `enum`. A change of that kind is a
  new format: the `format` constant changes, and with it the schema.
- A property that is in `properties` and not in `required` is present
  only in some states, as the `evidence` of an item is once it is done.
  A client reads it as optional.
- Adding a value to an `enum`, or a form to an `anyOf`, is an addition:
  a new error `kind`, a new event. A client treats a value it does not
  know as it treats a key it does not know.

## The map document

`ris map` gives the phase's change set as a graph over the source
code: the open phase's, or with none open the last closed one's, or
that of the phase named (`ris map P-3`). Given a git range in place of
a phase, `ris map A..B` maps the files at `B` against `A`, and `ris map
A...B` against where the two met, as `git diff` reads them; either end
left out is `HEAD`. A range is read in any git repository, a project or
not (outside one, with the default `[map]` configuration, and the page
is written only with `--html`); its document has `phase` null, the
range as `tip_ref`, and no auditor's judgements. Its `data` is the
graph document. The document is the contract and holds everything there is
to draw: `ris map --html <file>` writes a page that embeds the same
document, as JSON in the element `<script id="map-data"
type="application/json">`, and draws nothing that is not in it, so any
other renderer draws the same map from either source. `ris map` with
no flags, run by the person at a terminal, writes that page to
`.osiris/map.html` and opens it in the browser; with `--json` or
`--list`, run by a seat, or with output that is not a terminal,
nothing is written or opened. `ris map --list --json` gives the same
document as `ris map --json`. It follows the
rule of the rest of format 1: keys are added, never removed, renamed
or reshaped.

```json
{
  "phase": "P-3",
  "base": "<commit>",
  "tip": "<commit>",
  "tip_ref": "phase/P-3",
  "totals": {"files": 64, "changed": 12, "added": 371, "removed": 48, "outside": 43,
             "tests": {"files": 30, "changed": 4, "added": 210, "removed": 6}},
  "nodes": [
    {"path": "src/store.rs", "language": "rust", "status": "modified",
     "added": 12, "removed": 3, "external": ["filesystem"],
     "contract": false, "weight": 5.77, "dependents": 4,
     "risk": {
       "level": "high",
       "computed": {"level": "medium", "signals": [
         {"signal": "outside", "reason": "touches the outside: filesystem"},
         {"signal": "dependents", "reason": "4 other files depend on it"}]},
       "judged": {"level": "high", "why": "writes the store with no lock", "seat": "wren"}
     },
     "functions": [
       {"name": "Store::save", "start": 40, "end": 58, "added": 6, "removed": 1,
        "effects": ["filesystem"], "direct": true,
        "risk": {"level": "low", "signals": [
          {"signal": "outside", "reason": "touches the outside: filesystem"}]}}
     ],
     "marks": [
       {"line": 47, "kind": "filesystem", "function": "Store::save", "risk": "low"}
     ]}
  ],
  "ranked": ["src/store.rs"],
  "edges": [
    {"from": "src/store.rs", "to": "src/wire.rs", "kind": "use"}
  ],
  "diffs": {
    "src/store.rs": "diff --git a/src/store.rs b/src/store.rs\nindex ...\n--- a/src/store.rs\n+++ b/src/store.rs\n@@ -1,4 +1,5 @@\n use std::fs;\n+use std::io;\n ..."
  }
}
```

- `base` and `tip` are the commits compared: where the phase left the
  main line, and the phase's tip, whose files are the nodes. `tip_ref`
  says what named the tip: the phase's branch while it exists, `HEAD`
  for an open phase with no branch of its own, or the closing merge
  for a closed phase, or the range as given. `base` is null when where the work left the main
  line is not known, and then no node carries a diff. `phase` is null
  in a project that has never opened one.
- `nodes` are the repository's source files, sorted by `path`, which
  is relative to the repository root and is the node's identity. A
  file is source by its extension; `language` names it. A file that
  matches a `[map] contracts` pattern in `.osiris/config.toml` is a
  node as well, with `language` null when it is not source code.
  Files that are neither are not on the map, whatever the phase did to
  them.
- Test files are not on the map either: a file that would be a node
  and matches a `[map] tests` pattern has no node and no edge, unless a
  `contracts` pattern also names it. By default the patterns are each
  language's convention: every file under a `tests`, `test` or
  `__tests__` directory, and every file named as a test beside its
  source (`*_test.go`; `*.test.ts`, `*.spec.ts` and the same for
  `.tsx`, `.js` and `.jsx`; `*_test.clj`, `.cljs` and `.cljc`). The
  tests verify the behaviour
  that lands in the source; `totals.tests` says what the phase did to
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
- `dependents` is how many other nodes have an edge to this one: the
  files that name it, each counted once whatever kinds of line do.
- `risk` says how closely a changed file should be read, so the person
  sees where to look first; it is null for an `unchanged` node. It has
  two layers. `computed` is what `ris` works out with no model: a
  `level` of `low`, `medium` or `high`, and the `signals` that count
  toward it, each with its name in `signal` and the same in words, with
  its numbers, in `reason`. The signals are `outside` (the file is
  `external`), `contract`, `diff` (the lines added plus removed are 50
  or more, and count double from 200) and `dependents` (3 or more, and
  double from 8), in that order; `signals` is empty when none counts.
  A contract counts double and touching the outside once; two points
  are `medium` and four `high`, so one mild signal alone stays `low`
  and `high` takes signals together. The thresholds are not part of
  the contract; the level, and the reasons given for it, are. `judged`
  is what an auditor recorded for the file in this phase with
  `ris audit --file`: its `level`, one line of `why`, and the `seat`;
  the latest, if more than one was recorded, and null when none was.
  `risk.level` is the level that stands, and the one to draw: the
  judgement's where there is one, and the computed one otherwise. A
  closed phase's map keeps its judgements.
- `functions` and `marks` look inside a changed file of Rust,
  TypeScript, Go or Clojure, read from its syntax tree at `tip` and
  its diff; they are null for any other language, an unchanged or
  deleted file, and when `base` is null. `functions` are the
  functions the phase changed (a line added in one, or removed from
  it), in the order they start: `name` as a reader finds it (a method
  with its type in front), `start` and `end` lines at `tip`, the lines
  `added` in it and `removed` from it, the kinds of `effects` on its
  lines, whether it is `direct` (holds an effect site itself), the
  functions of the repository it `calls` (`path` and `function`), its
  `steps` one step from an effect, whether it is `wide` itself, and
  its `risk`, computed from the same signals as a file's (its effects
  and the size of its change), with one point more and the signal
  `reach` when it has a step that counts, as `level` and `signals`. A
  step is a direct function it calls (`direction` `triggers`) or one
  that calls it (`feeds`), with that function's `path`, `function` and
  `effects`, and whether that function is `wide`. A function is wide
  when the functions it calls, or those that call it, lie in more than
  half of the files that hold functions, and in at least eight: a
  command's dispatcher, a helper everything uses. Most functions are
  one step from such a function, so a step to one does not count
  toward the level, and no step of a function that is wide itself
  does. Calls are resolved over every file on the map of those
  languages, and only where the text says which function is meant: a
  call into a module the repository does not hold (a standard library,
  a package, another crate) is left out; a call that names a module
  means the functions of the file or directory named as the module's
  last segment is; a name with a type in front means that type's
  method; and a name with neither means a function of the caller's own
  file, or in Go, where a package is a directory, failing that its
  directory. A method called on a value is a
  call only where the text says the value's type (`self` or `this`, a
  Go method's receiver, a parameter or a `let` with its type, a `let`
  set to `Type::new(..)`, `Type::default()` or a struct written out),
  and then means that type's method. A name the function binds more
  than once (shadowed by another `let`, a loop or a pattern) says
  nothing, since which binding a call sees takes the scopes. On any
  other value it is not
  resolved, so a call is never matched to a method by its name alone
  and the hint errs toward missing a step. `marks` are the
  lines that touch the outside (an effect site), on a line the phase
  changed or in a function it changed, by line: `line` at `tip`,
  `kind` (`database`, `filesystem`, `network` or `process`), the
  innermost `function` it sits in, and `risk`, that function's level,
  which is the line's (null outside any function, where the file's
  stands). A file's own `risk` does not change with them. An effect
  site is a name the code uses, its imports and aliases resolved
  (`std::fs::read`, `fs.readFile`, `os.ReadFile`,
  `clojure.java.io/file`), that is one of the language's patterns or
  lies under one: a list shipped per language, extended per project in
  `[map.effects.<language>]` of `.osiris/config.toml`. A method called
  on a value names no module and is not read.
- `call_graph` holds the calls around the phase's changed functions,
  for a view of one function: `functions`, sorted by `path` and then
  `start`, are every changed function and every function one call from
  one, in full, and the functions one call further, by name only. Each
  has an `id` (`path:start`), its `path`, `name`, `start` and `end`,
  whether it is `direct` and its `effects`, its effect `sites` (`line`
  and `kind`), the lines of it the phase added (`changed`), its text at
  `tip` as `lines` (one entry a line from `start`), and the ids of the
  functions it `calls` and its `callers`. A function given by name only
  has `lines` null and no sites, calls or callers. Empty when no file is
  read inside.
- `ranked` is the path of every changed node, each once, in the order
  to read them: the highest `risk.level` first, then the greatest
  `weight`, then by `path`. It is the order `ris map --list` prints
  and the page lists, so neither ranks the files itself.
- `edges` are how the code connects, each from the file that names the
  other, sorted. `kind` is the kind of line that makes the edge. For
  Rust: `mod` (a `mod name;` line, to the module's file) and `use` (a
  `use` line or a qualified path through `crate`, `self`, `super` or a
  package of the repository, to the deepest module file it names).
  For TypeScript: `import` (an `import` or `export ... from`, a
  `require` or an `import()` of a relative specifier, to the file it
  names: with or without its extension, `./wire.js` for `wire.ts`, or
  a directory's `index` file). For Go: `import` (an import path under
  a module a `go.mod` in the repository declares, to every Go file in
  that package's directory). For Clojure: `require` (a namespace an
  `ns` form's `:require` or `:use` names, to every file whose `ns` form
  declares it). Rust's edges are found by scanning the text, the
  others' from their syntax trees; a language with no scanner yet has
  nodes and no edges, and gaining one adds edges and `kind` values
  without changing the document's shape.
- `totals` counts the nodes (`files`), those not `unchanged`
  (`changed`), their lines `added` and `removed`, and the nodes that
  are external or a contract (`outside`). `tests` counts the test files
  left off the map in the same terms: all of them (`files`), those the
  phase changed (`changed`), and the lines `added` and `removed` in
  them. A page shows it as one line of text.

- `diffs` holds each changed node's unified diff between `base` and
  `tip`, by `path`: the text `git diff` writes for that one file, with
  its file header (`diff --git`, `index`, `---`, `+++`), then each hunk
  header (`@@ -old,count +new,count @@`) and its lines, marked in the
  first column as added (`+`), removed (`-`) or context (a space),
  with three lines of context, the prefixes `a/` and `b/`, and renames
  not followed. An added file's old side is `/dev/null`, and a deleted
  file's new side. A node that is `unchanged` has no entry, and
  neither has a file that is not a node; the object is empty when
  `base` is null. A binary file's entry has the header and git's one
  line saying the files differ, and no hunk. Bytes that are not UTF-8
  are replaced. The page draws a clicked node's entry in a pane beside
  the graph, so another renderer can draw the same pane.

`crates/ris/tests/map.rs` pins these keys.
