/**
 * The check engine (§6, §9): walk the store's claims → resolve drift against the
 * current working tree → merge with document lifecycle/supersession → optionally
 * stamp banners → emit a JSON report with a meaningful exit code.
 *
 * Verdicts are recomputed live and never persisted (§6).
 */

import { access, readFile, writeFile } from "node:fs/promises";
import { extname, join } from "node:path";
import { type AstAnalyzer, resolveAssertion } from "../algo/resolve.ts";
import {
  type BannerAction,
  type BannerEntry,
  DEFAULT_HEADLINE,
  removeBanner,
  stampBanner,
} from "../banner/banner.ts";
import { setFrontmatterStatus } from "../banner/frontmatter.ts";
import type {
  Assertion,
  ComputedState,
  Document,
  DocumentLifecycle,
  Proposition,
  Verdict,
} from "../core/model.ts";
import type { ResolverRegistry } from "../resolver/registry.ts";
import type { ClaimStore } from "../store/store.ts";

/** Status precedence for the single-valued frontmatter status (most severe first). */
const STATUS_PRECEDENCE = [
  "stale",
  "ghost",
  "expired",
  "retracted",
  "superseded",
  "archived",
  "amended",
  "moved",
];
const MARKDOWN_EXT = new Set([".md", ".markdown", ".mdx"]);

/** Computed states that put a document in the suspect set (exit 2). */
const SUSPECT_STATES: ReadonlySet<ComputedState> = new Set([
  "stale",
  "ghost",
  "expired",
]);

export type FailOn = "suspect" | "moved" | "tamper" | "never";

export interface CheckOptions {
  ast?: AstAnalyzer;
  /** Resolve through the registry (built-in + external + advisory) instead of inline. */
  registry?: ResolverRegistry;
  /** Apply banner stamps to documents (the write path). Default: read-only. */
  write?: boolean;
  /** Restrict the check to assertions whose anchored file is in this set. */
  onlyFiles?: Set<string>;
  failOn?: FailOn;
  now?: number;
  ref?: string;
}

export interface DocumentReport {
  id: string;
  path: string;
  lifecycle: DocumentLifecycle;
  suspect: { propositionId: string; state: string }[];
  bannerAction?: BannerAction;
  tampered?: boolean;
  /** The machine-readable frontmatter status written (markdown only, §8). */
  frontmatterStatus?: string;
}

export interface CheckReport {
  ref: string;
  verdicts: Verdict[];
  documents: DocumentReport[];
  summary: Record<ComputedState | "total", number>;
  exitCode: number;
}

async function exists(p: string): Promise<boolean> {
  try {
    await access(p);
    return true;
  } catch {
    return false;
  }
}

/** Banner entries contributed by a document's lifecycle (§6 remediation). */
function lifecycleEntries(
  doc: Document,
  propsById: Map<string, Proposition>,
): BannerEntry[] {
  const entries: BannerEntry[] = [];
  const amended = new Set<string>();
  for (const e of doc.edges) {
    if (e.type === "amended-by") for (const p of e.propositions) amended.add(p);
  }
  for (const propId of amended) {
    const p = propsById.get(propId);
    if (p) entries.push({ status: "amended", id: propId, text: p.text });
  }
  if (doc.lifecycle === "superseded") {
    entries.push({
      status: "superseded",
      id: doc.id,
      text: `This document has been superseded.`,
    });
  }
  if (doc.lifecycle === "retracted") {
    entries.push({
      status: "retracted",
      id: doc.id,
      text: `The author withdrew this document.`,
    });
  }
  return entries;
}

export async function runCheck(
  store: ClaimStore,
  options: CheckOptions = {},
): Promise<CheckReport> {
  const root = store.root;
  const ref = options.ref ?? "WORKTREE";
  const documents = await store.allDocuments();
  const propositions = await store.allPropositions();
  const assertions = await store.allAssertions();

  const propsById = new Map(propositions.map((p) => [p.id, p]));
  const docsById = new Map(documents.map((d) => [d.id, d]));

  // Cache file reads (a regenerable optimization; never affects the verdict).
  const fileCache = new Map<string, string | null>();
  const readFileText = async (rel: string): Promise<string | null> => {
    if (fileCache.has(rel)) return fileCache.get(rel)!;
    const abs = join(root, rel);
    const text = (await exists(abs)) ? await readFile(abs, "utf8") : null;
    fileCache.set(rel, text);
    return text;
  };

  const verdicts: Verdict[] = [];
  const summary: Record<string, number> = {
    fresh: 0,
    moved: 0,
    stale: 0,
    ghost: 0,
    expired: 0,
    total: 0,
  };

  for (const a of assertions) {
    if (options.onlyFiles && !options.onlyFiles.has(a.anchor.file)) continue;
    const text = await readFileText(a.anchor.file);
    let verdict: Verdict;
    if (options.registry) {
      verdict = await options.registry.resolve(
        a,
        text,
        propsById.get(a.propositionId),
      );
    } else if (text === null) {
      // Anchored file is gone → ghost (no selector can locate it).
      verdict = {
        assertionId: a.id,
        propositionId: a.propositionId,
        documentId: a.documentId,
        state: "ghost",
        confidence: 0,
        selectorScores: [],
        ref: a.ref,
        notes: [`anchored file ${a.anchor.file} not found`],
        advisories: [],
      };
    } else {
      verdict = resolveAssertion(a, text, {
        ast: options.ast,
        now: options.now,
      });
    }
    verdicts.push(verdict);
    summary[verdict.state] = (summary[verdict.state] ?? 0) + 1;
    summary.total!++;
  }

  // ── Per-document banner payloads & lifecycle ──
  const verdictsByDoc = new Map<string, Verdict[]>();
  for (const v of verdicts) {
    const list = verdictsByDoc.get(v.documentId) ?? [];
    list.push(v);
    verdictsByDoc.set(v.documentId, list);
  }

  const docReports: DocumentReport[] = [];
  let sawSuspect = false;
  let sawMoved = false;
  let sawTamper = false;

  for (const doc of documents) {
    const dv = verdictsByDoc.get(doc.id) ?? [];
    const suspectVerdicts = dv.filter((v) => SUSPECT_STATES.has(v.state));
    const suspectEntries: BannerEntry[] = suspectVerdicts.map((v) => ({
      status: v.state,
      id: v.propositionId,
      text: propsById.get(v.propositionId)?.text ?? "(unknown proposition)",
    }));
    const lcEntries = lifecycleEntries(doc, propsById);
    const allEntries = [...suspectEntries, ...lcEntries];

    if (dv.some((v) => v.state === "moved")) sawMoved = true;
    if (suspectVerdicts.length > 0) sawSuspect = true;

    const report: DocumentReport = {
      id: doc.id,
      path: doc.path,
      lifecycle: doc.lifecycle,
      suspect: suspectVerdicts.map((v) => ({
        propositionId: v.propositionId,
        state: v.state,
      })),
    };

    // Worst single status for the optional frontmatter field (§8).
    const severities = [
      ...suspectVerdicts.map((v) => v.state as string),
      ...(doc.lifecycle !== "active" ? [doc.lifecycle as string] : []),
    ];
    const statusValue = severities.length
      ? (STATUS_PRECEDENCE.find((s) => severities.includes(s)) ??
        severities[0]!)
      : null;

    if (options.write) {
      const abs = join(root, doc.path);
      if (await exists(abs)) {
        const original = await readFile(abs, "utf8");
        const nonce = (await store.config()).nonce;
        const headline =
          suspectEntries.length === 0 && lcEntries.length > 0
            ? `DOCUMENT STATUS — ${lcEntries.length} notice(s) — re-verify before trusting.`
            : DEFAULT_HEADLINE(allEntries.length);

        let content = original;
        if (allEntries.length === 0) {
          // Nothing suspect → ensure no stale banner lingers.
          const res = removeBanner(content, doc.path, nonce);
          content = res.content;
          report.bannerAction = res.action;
        } else {
          const res = stampBanner(
            content,
            doc.path,
            { headline, entries: allEntries },
            nonce,
            {
              failOnTamper: options.failOn === "tamper",
            },
          );
          if (res.tampered) {
            sawTamper = true;
            report.tampered = true;
          }
          report.bannerAction = res.action;
          // Honor --fail-on tamper: refuse to overwrite a hand-edited banner.
          content =
            res.tampered && options.failOn === "tamper"
              ? original
              : res.content;
        }

        // Optional markdown frontmatter status (§8): only where frontmatter exists.
        if (MARKDOWN_EXT.has(extname(doc.path).toLowerCase())) {
          content = setFrontmatterStatus(content, statusValue);
          report.frontmatterStatus = statusValue ?? undefined;
        }

        if (content !== original) await writeFile(abs, content);
      }
    }

    docReports.push(report);
  }

  void docsById; // documents already iterated directly

  const exitCode = computeExitCode({
    sawSuspect,
    sawMoved,
    sawTamper,
    failOn: options.failOn ?? "suspect",
  });

  return {
    ref,
    verdicts,
    documents: docReports,
    summary: summary as CheckReport["summary"],
    exitCode,
  };
}

/** Exit-code contract (§9): 0 clean · 2 suspect · 3 moved-only · 1 op error. */
export function computeExitCode(s: {
  sawSuspect: boolean;
  sawMoved: boolean;
  sawTamper: boolean;
  failOn: FailOn;
}): number {
  if (s.failOn === "tamper" && s.sawTamper) return 2;
  if (s.sawSuspect) return 2;
  if (s.sawMoved) {
    return s.failOn === "moved" ? 2 : 3;
  }
  return 0;
}
