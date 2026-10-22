import { expect, test } from "bun:test";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { renderReference } from "../scripts/gen-cli-reference.ts";
import { COMMANDS, usage } from "../src/cli/options.ts";

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

test("the top-level exit-code line names each exit code and what exits 1", () => {
  const line = usage()
    .split("\n")
    .find((l) => l.startsWith("Exit codes:"));
  expect(line).toBe(
    "Exit codes: 0 clean or warnings only; 2 gating; 1 operational error (bad flag, no store, missing file, check --doc on a path with no registered claims)",
  );
});

/** True when a shell would expand a `$` in the command: outside single quotes. */
function expandsDollar(command: string): boolean {
  let single = false;
  let double = false;
  for (let i = 0; i < command.length; i++) {
    const c = command[i];
    if (c === "\\" && !single) {
      i++;
      continue;
    }
    if (c === "'" && !double) single = !single;
    else if (c === '"' && !single) double = !double;
    else if (c === "$" && !single && /[\w{(]/.test(command[i + 1] ?? ""))
      return true;
  }
  return false;
}

test("no example lets the shell expand a $ in its text", () => {
  for (const cmd of COMMANDS) {
    for (const example of cmd.examples ?? []) {
      expect({ example, expands: expandsDollar(example) }).toEqual({
        example,
        expands: false,
      });
    }
  }
});
