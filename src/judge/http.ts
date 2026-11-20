/**
 * Minimal client for TypeSafe's System One endpoint
 * (`POST /v1/systemone`, https://docs.typesafe.ai/api.md).
 *
 * The official `@typesafe-ai/sdk` is not used: under Bun it reads the
 * response body twice and every request fails (see
 * design/evidence/typesafe-research-2026-10-22.md section 6). Retry numbers
 * below copy the SDK's documented defaults from the same section.
 */
import { JudgeError, JudgeUnavailable } from "./types.ts";

export const DEFAULT_BASE_URL = "https://api.typesafe.ai";
export const API_KEY_ENV = "TYPESAFE_API_KEY";
export const BASE_URL_ENV = "TYPESAFE_BASE_URL";

export const MAX_RETRIES = 2;
export const BACKOFF_INITIAL_MS = 500;
export const BACKOFF_MAX_MS = 5_000;
export const BACKOFF_JITTER = 0.25;
export const MAX_RETRY_AFTER_MS = 60_000;
export const ATTEMPT_TIMEOUT_MS = 10_000;
export const MAX_IN_FLIGHT = 8;

export interface SystemOneRequest {
  model: string;
  state: unknown;
  questions: Record<string, unknown>;
}

export interface ChoiceAnswer {
  type?: "choice";
  choice: string;
  confidence: number;
  probabilities: Record<string, number>;
}

export interface NoulAnswer {
  type?: "noul";
  noul: number;
}

export type Answer = ChoiceAnswer | NoulAnswer;

export interface SystemOneResponse {
  model: string;
  answers: Record<string, Answer>;
  usage: { input_tokens: number; output_tokens: number };
}

/** What the judge needs from a client; tests may substitute their own. */
export interface SystemOneClient {
  systemOne(body: SystemOneRequest): Promise<SystemOneResponse>;
}

export interface TypeSafeHttpOptions {
  /** Environment to read `TYPESAFE_API_KEY` and `TYPESAFE_BASE_URL` from. Defaults to `process.env`. */
  env?: Record<string, string | undefined>;
  fetch?: typeof fetch;
  sleep?: (ms: number) => Promise<void>;
  random?: () => number;
}

// In-module semaphore: at most MAX_IN_FLIGHT requests across all clients.
let inFlight = 0;
const waiting: Array<() => void> = [];

async function acquire(): Promise<void> {
  if (inFlight < MAX_IN_FLIGHT) {
    inFlight++;
    return;
  }
  await new Promise<void>((resolve) => waiting.push(resolve));
}

function release(): void {
  const next = waiting.shift();
  if (next) next();
  else inFlight--;
}

function isRetryableStatus(status: number): boolean {
  return status === 408 || status === 429 || (status >= 500 && status <= 599);
}

/**
 * Parses `retry-after` (seconds or HTTP date). Returns ms, clamped to
 * `MAX_RETRY_AFTER_MS`, or null when absent or invalid.
 */
export function parseRetryAfter(
  value: string | null,
  now = Date.now(),
): number | null {
  if (value === null || value.trim() === "") return null;
  const seconds = Number(value);
  const ms = Number.isFinite(seconds)
    ? seconds * 1000
    : Date.parse(value) - now;
  if (!Number.isFinite(ms)) return null;
  return Math.min(Math.max(ms, 0), MAX_RETRY_AFTER_MS);
}

/**
 * Parse a 200 body. Throws `JudgeError` when it is not JSON or lacks an
 * `answers` object and a `model` string.
 */
export function parseResponseBody(text: string): SystemOneResponse {
  let body: unknown;
  try {
    body = JSON.parse(text);
  } catch (e) {
    throw new JudgeError(
      `TypeSafe returned a body that is not JSON (${(e as Error).message}): ${text.slice(0, 200)}`,
    );
  }
  const o = body as Record<string, unknown> | null;
  if (
    o === null ||
    typeof o !== "object" ||
    Array.isArray(o) ||
    typeof o.model !== "string" ||
    o.answers === null ||
    typeof o.answers !== "object" ||
    Array.isArray(o.answers)
  ) {
    throw new JudgeError(
      `TypeSafe returned JSON without an "answers" object and a "model" string: ${text.slice(0, 200)}`,
    );
  }
  return o as unknown as SystemOneResponse;
}

export class TypeSafeHttp implements SystemOneClient {
  private readonly apiKey: string;
  private readonly url: string;
  private readonly fetchImpl: typeof fetch;
  private readonly sleep: (ms: number) => Promise<void>;
  private readonly random: () => number;

  constructor(options: TypeSafeHttpOptions = {}) {
    const env = options.env ?? process.env;
    this.apiKey = (env[API_KEY_ENV] ?? "").trim();
    const base = (env[BASE_URL_ENV] ?? "").trim() || DEFAULT_BASE_URL;
    this.url = `${base.replace(/\/+$/, "")}/v1/systemone`;
    this.fetchImpl = options.fetch ?? fetch;
    this.sleep =
      options.sleep ??
      ((ms) => new Promise((resolve) => setTimeout(resolve, ms)));
    this.random = options.random ?? Math.random;
  }

  async systemOne(body: SystemOneRequest): Promise<SystemOneResponse> {
    if (this.apiKey === "") {
      throw new JudgeUnavailable(
        `${API_KEY_ENV} is not set; the judge cannot answer.`,
      );
    }
    await acquire();
    try {
      return await this.send(JSON.stringify(body));
    } finally {
      release();
    }
  }

  private backoffMs(retry: number): number {
    const base = Math.min(BACKOFF_INITIAL_MS * 2 ** retry, BACKOFF_MAX_MS);
    return base * (1 - BACKOFF_JITTER * this.random());
  }

  private async send(payload: string): Promise<SystemOneResponse> {
    let lastFailure = "";
    for (let retry = 0; ; retry++) {
      let response: Response;
      let text: string;
      try {
        response = await this.fetchImpl(this.url, {
          method: "POST",
          headers: {
            authorization: `Bearer ${this.apiKey}`,
            "content-type": "application/json",
            accept: "application/json",
          },
          body: payload,
          signal: AbortSignal.timeout(ATTEMPT_TIMEOUT_MS),
        });
        // Read the body once, inside the timeout, then parse it.
        text = await response.text();
      } catch (error) {
        // Connection error or the per-attempt timeout.
        lastFailure = error instanceof Error ? error.message : String(error);
        if (retry >= MAX_RETRIES) break;
        await this.sleep(this.backoffMs(retry));
        continue;
      }

      if (response.ok) return parseResponseBody(text);

      const { status } = response;
      if (status === 401 || status === 403) {
        throw new JudgeUnavailable(
          `TypeSafe rejected the API key (HTTP ${status}); check ${API_KEY_ENV}. ${text}`,
        );
      }
      if (!isRetryableStatus(status)) {
        // 400, 422 and other 4xx: the request is wrong, not the service.
        throw new JudgeError(`TypeSafe returned HTTP ${status}: ${text}`);
      }
      lastFailure = `HTTP ${status}: ${text}`;
      if (retry >= MAX_RETRIES) break;
      const retryAfter = parseRetryAfter(response.headers.get("retry-after"));
      await this.sleep(retryAfter ?? this.backoffMs(retry));
    }
    throw new JudgeUnavailable(
      `TypeSafe did not answer after ${MAX_RETRIES + 1} attempts: ${lastFailure}`,
    );
  }
}
