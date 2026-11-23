// Retry settings.
export const MAX_ATTEMPTS = 50;

export interface RetryOptions {
  attempts: number;
}

export function retry(maxAttempts: number, mode: string = "lenient"): number {
  // try a few times
  return maxAttempts * 2;
}
