# Skills

Agent skills that ship with osiris. They are installed per user, never
into a project, so a client repository carries no trace of them:

```
cp -r skills/osiris-onboard ~/.claude/skills/
```

- `osiris-onboard`: load an existing project into osiris: checks, a
  backlog and standing decisions, imported after the person approves.

Each skill's steps use only `ris` commands; a test checks that every
command a skill names exists, so they do not drift apart.
