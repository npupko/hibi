import { describe, expect, test } from "bun:test";
import { Engine } from "../src/index.ts";
import { makeRepo } from "./helpers.ts";

/**
 * `hibi ignore` suppression (ADR-002 D14). Acknowledging a behavioral at-risk
 * records the `{path → hash}` map + reason; while the acknowledged evidence
 * stands the at-risk is non-gating (`suppressed: true`) and contributes nothing
 * to exit codes; it lapses automatically when the evidence moves again.
 */
describe("ignore / suppression (D14)", () => {
  async function atRisk() {
    const repo = await makeRepo();
    await repo.write("src/dep.ts", "export const RATE = 5;\n");
    await repo.write(
      "src/main.ts",
      'import { RATE } from "./dep.ts";\nexport function apply() {\n  return RATE;\n}\n',
    );
    await repo.write(
      "doc.md",
      "# Doc\n\nThe apply helper retries on failure.\n",
    );
    const engine = await Engine.open(repo.root);
    const rec = await engine.record({
      docPath: "doc.md",
      docQuote: "The apply helper retries on failure.",
      code: [
        {
          file: "src/main.ts",
          quote: "export function apply() {\n  return RATE;\n}",
        },
      ],
      behavioral: true,
      authoredTrust: "verified",
      ref: "PR-1",
      enforcement: "enforced",
    });
    // Drift the imported dep → the claim goes behavior:at-risk.
    await repo.write("src/dep.ts", "export const RATE = 50;\n");
    return { repo, engine, claimId: rec.assertion.id };
  }

  test("ignore requires a reason", async () => {
    const { repo, engine, claimId } = await atRisk();
    await expect(engine.ignore(claimId, "")).rejects.toThrow(/reason/);
    await repo.cleanup();
  });

  test("ignore records the acknowledged {path→hash} map + reason", async () => {
    const { repo, engine, claimId } = await atRisk();
    const result = await engine.ignore(claimId, "re-checked by hand vs PR-2");
    expect(result.reason).toBe("re-checked by hand vs PR-2");
    expect(Object.keys(result.paths)).toContain("src/dep.ts");
    const a = await engine.store.getAssertion(claimId);
    expect(a?.suppressed?.reason).toBe("re-checked by hand vs PR-2");
    expect(a?.suppressed?.paths["src/dep.ts"]).toBeDefined();
    await repo.cleanup();
  });

  test("an active suppression neutralizes the at-risk (suppressed:true, no warn)", async () => {
    const { repo, engine, claimId } = await atRisk();
    await engine.ignore(claimId, "acknowledged");
    const report = await engine.check({ failOn: "warn" });
    const v = report.verdicts.find((x) => x.assertionId === claimId);
    expect(v?.behavior).toBe("at-risk");
    expect(v?.suppressed).toBe(true);
    // Contributes nothing to exit codes, even under --fail-on warn.
    expect(report.summary.warning).toBe(0);
    expect(report.exitCode).toBe(0);
    await repo.cleanup();
  });

  test("the suppression lapses when the evidence moves again", async () => {
    const { repo, engine, claimId } = await atRisk();
    await engine.ignore(claimId, "acknowledged");
    // The acknowledged path moves past its acknowledged hash → lapsed.
    await repo.write("src/dep.ts", "export const RATE = 999;\n");
    const report = await engine.check({ failOn: "warn" });
    const v = report.verdicts.find((x) => x.assertionId === claimId);
    expect(v?.behavior).toBe("at-risk");
    expect(v?.suppressed).toBe(false);
    expect(report.exitCode).toBe(2); // warn now fails under --fail-on warn
    await repo.cleanup();
  });
});
