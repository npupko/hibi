# Judge fixtures

The `supports` and `conflict` responses were recorded from live System One calls on 2026-10-23 (model `jev-1.13.0`), and the `triage` response on 2026-10-23 (model `jev-1.14.0`), after the triage questions were rewritten. Each was recorded by POSTing the matching `*.request.json` file in this directory unchanged.

To re-record, POST each request file to `https://api.typesafe.ai/v1/systemone` with the `TYPESAFE_API_KEY` bearer token (the key is available via `fnox exec -- <cmd>`) and save the response body as the matching `*.response.json`.

Per judgment (`triage`, `supports`, `conflict`):

- `inputs.json` holds the judge inputs for all three.
- `<judgment>.request.json` is the exact body `JevJudge` sends for those inputs (model `jev-1.13.0`; `jev-1.14.0` for `triage`). `test/judge-jev.test.ts` fails when the built request differs, so a change to `src/judge/questions/*.json` or to request building must update these files on purpose.
- `<judgment>.response.json` is the recorded System One response in the documented shape (https://docs.typesafe.ai/api.md). `test/judge-jev.test.ts` asserts values derived from it, so re-recording requires updating those expected values.
