/**
 * Generate versioned JSON Schema (`schemas/*.v1.json`) from the canonical Zod
 * model (§5). The Zod model is the single source of truth; this is a derived
 * artifact, regenerated, never hand-edited.
 */
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import * as z from "zod";
import { MODEL_VERSION, SCHEMAS } from "../src/core/model.ts";
import { PROTOCOL_SCHEMAS } from "../src/resolver/protocol.ts";

const OUT_DIR = join(import.meta.dir, "..", "schemas");

export async function generateSchemas(outDir = OUT_DIR): Promise<string[]> {
  await mkdir(outDir, { recursive: true });
  const written: string[] = [];
  const all = { ...SCHEMAS, ...PROTOCOL_SCHEMAS };
  for (const [name, schema] of Object.entries(all)) {
    const jsonSchema = z.toJSONSchema(schema, {
      target: "draft-2020-12",
      reused: "ref",
    });
    const file = join(outDir, `${name}.${MODEL_VERSION}.json`);
    await writeFile(file, JSON.stringify(jsonSchema, null, 2) + "\n");
    written.push(file);
  }
  return written;
}

if (import.meta.main) {
  const written = await generateSchemas();
  console.log(`Wrote ${written.length} schema(s) to ${OUT_DIR}`);
  for (const f of written) console.log(`  ${f}`);
}
