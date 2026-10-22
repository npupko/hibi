import { expect, test } from "bun:test";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { renderReference } from "../scripts/gen-cli-reference.ts";

test("the exit-code line names what gates and what --fail-on warn does", () => {
  const line = renderReference()
    .split("\n")
    .find((l) => l.startsWith("Exit codes:"));
  expect(line).toBe(
    "Exit codes: `0` clean or warnings only, `2` gating (on an enforced claim: a `changed`, `orphaned`, or `ambiguous` side, a `refuted` judge verdict, a conflict, or `expired` under `--doc`; under `--fail-on warn`, any warning also exits `2`), `1` operational error (bad flag, no store, missing file, `check --doc` on a path with no registered claims).",
  );
});

test("the committed CLI reference matches the option tables", async () => {
  const committed = await readFile(
    join(import.meta.dir, "..", "docs", "cli-reference.mdx"),
    "utf8",
  );
  expect(committed.endsWith(renderReference())).toBe(true);
});
