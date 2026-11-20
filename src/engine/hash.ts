/**
 * Sentence hash: the identity of a sentence's text for acknowledgements
 * (SPEC 9.4). An `ack` on a conflict pair stores the hashes of both sentences
 * and stops suppressing the pair once either hash changes.
 *
 * Definition, shared with the evidence and attest code that writes acks:
 * sha256 hex of the text after NFC normalization, collapsing every run of
 * whitespace to one space, and trimming.
 */
import { createHash } from "node:crypto";

export function normalizeSentence(text: string): string {
  return text.normalize("NFC").replace(/\s+/g, " ").trim();
}

export function sentenceHash(text: string): string {
  return createHash("sha256").update(normalizeSentence(text)).digest("hex");
}
