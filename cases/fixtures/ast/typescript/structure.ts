// Retry settings.
export const MAX_ATTEMPTS = 5;

export interface RetryOptions {
  attempts: number;
  jitter: boolean;
}

export function retry(maxAttempts: number, delay: number, mode: string = "strict"): number {
  // try a few times
  return maxAttempts * 2;
}
