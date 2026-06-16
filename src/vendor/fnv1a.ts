/**
 * FNV-1a (32-bit) — the one canonical non-crypto checksum used wherever the spec
 * calls for one (the banner body checksum, §8/§17.5). Vendored and owned (§16).
 *
 * Offset basis `0x811c9dc5`, prime `0x01000193`. Operates on the UTF-8 byte
 * stream so it is independent of JS string internals.
 */

const OFFSET_BASIS = 0x811c9dc5;
const PRIME = 0x01000193;

export function fnv1a32(input: string | Uint8Array): number {
  const bytes = typeof input === "string" ? new TextEncoder().encode(input) : input;
  let hash = OFFSET_BASIS;
  for (let i = 0; i < bytes.length; i++) {
    hash ^= bytes[i]!;
    // 32-bit multiply (Math.imul) then force unsigned.
    hash = Math.imul(hash, PRIME) >>> 0;
  }
  return hash >>> 0;
}

/** 8 lowercase hex chars, as recorded on the banner END sentinel (§17.5). */
export function fnv1a32hex(input: string | Uint8Array): string {
  return fnv1a32(input).toString(16).padStart(8, "0");
}
