# Judge fixtures

Synthetic. No `TYPESAFE_API_KEY` was available when these were written, so no file here came from a live call.

Per judgment (`triage`, `supports`, `conflict`):

- `inputs.json` holds the judge inputs for all three.
- `<judgment>.request.json` is the exact body `JevJudge` sends for those inputs (model `jev-1.13.0`). `test/judge-jev.test.ts` fails when the built request differs, so a change to `src/judge/questions/*.json` or to request building must update these files on purpose.
- `<judgment>.response.json` is a plausible System One response in the documented shape (https://docs.typesafe.ai/api.md). The probabilities are made up; they are not evidence of how Jev answers.

Replace the responses with recorded ones once a key is available.
