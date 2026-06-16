/**
 * Identity helpers. Entity ids are authored/explicit; the Proposition
 * fingerprint is a content hash (the dedup unit, §5) — never similarity-computed.
 */
import { randomUUID } from "node:crypto";
import { normalizeText } from "../algo/normalize.ts";

/** A fresh opaque id with a short typed prefix (e.g. `prop_…`, `asrt_…`). */
export function newId(prefix: string): string {
  return `${prefix}_${randomUUID().replace(/-/g, "").slice(0, 16)}`;
}

/**
 * Content fingerprint of a Proposition's text (§5). Identity is by normalized
 * content so two authors writing the same claim dedup, but it is *explicit
 * content*, not a similarity score. xxHash64 hex (§16).
 */
export function propositionFingerprint(text: string): string {
  return Bun.hash.xxHash64(normalizeText(text)).toString(16).padStart(16, "0");
}
