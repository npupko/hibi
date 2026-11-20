// Retry settings.
export const MAX_ATTEMPTS = 5;

export interface RetryOptions {
  attempts: number;
}

export function retry(maxAttempts: number, mode: string = "strict"): number {
  // try several times, then give up
  return maxAttempts * 2;
}
