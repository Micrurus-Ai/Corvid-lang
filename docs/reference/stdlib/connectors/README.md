# Stdlib connectors

Typed connector references. Each connector ships in three modes —
mock, replay, real — that share a single typed surface, with
adversarial coverage that fires a replay quarantine if a replay run
escapes into real mode.

## Available connectors

- [Gmail](gmail.md)
- [Microsoft 365](ms365.md)
- [Slack](slack.md)
- [Calendar](calendar.md)
- [Tasks](tasks.md)
- [Files](files.md)

## See also

- [Guide: Connectors](../../../guides/connectors.md) — task-focused
  walkthrough (auth, self-test, three-mode pattern).
- [Recipe: Personal Executive Agent](../../../recipes/personal-executive-agent.md) —
  example pulling several connectors together.
