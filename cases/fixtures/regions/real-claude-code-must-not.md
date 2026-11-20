The skill also states what an agent must not do:

- vouch as a human. An agent attests as `agent:<name>`. Its vouch is recorded and does not reset a ttl. Only judged evidence or a `human:` vouch does (see [Attestations](/attestations)).
- reanchor a claim without re-reading the sentence against its evidence.
- delete a file under `.claims/` instead of running `hibi retire <id>`.