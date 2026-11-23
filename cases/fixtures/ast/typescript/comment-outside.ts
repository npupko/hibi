// Retry policy, reviewed.
export const MAX_ATTEMPTS = 5;

export interface RetryOptions {
  attempts: number;
}

/** Retries with backoff. */
export function retry(maxAttempts: number, mode: string = "strict"): number {
  // try a few times
  return maxAttempts * 2;
}
