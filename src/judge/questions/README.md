# Judge questions

These files are the questions hibi asks the judge model (Jev, via TypeSafe's System One API). When a verdict looks wrong, read these first: the wording here is what the model judged, and most wrong answers trace back to a question that says something other than what was meant.

One file per judgment (`triage`, `supports`, `conflict`), each with:

- `state`: the state field names the request sends, each with a description for reviewers. The descriptions are not sent.
- `questions`: the TypeSafe question objects, sent as written. Keys are the question IDs (the model never sees them); `instructions` holds the complete question and refers to state fields by backticked path; `criteria` lists Choice options in their fixed canonical order, or a Noul's `true`/`false` descriptions.
- `placeholder` (triage only): `{{i}}` is replaced by the region's index in that request's `state.regions`, in keys and text, once per region. `idPlaceholder` (triage only): `{{id}}` is replaced by the region's `id`, so each question names its region both ways; with the index alone, the model answered about a neighboring region from about the fifth entry on. A long document is split across requests; each request's state holds `doc` and only the regions it asks about.
- `relationNames` (supports only): maps the citation-check cookbook's option names to hibi's result names.

Thresholds and code rules that act on the answers live in `src/judge/jev.ts`, not here. Changing a question changes cached request bytes, so every cached verdict for it is asked again.
