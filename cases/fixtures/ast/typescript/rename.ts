// Retry settings.
export const MAX_TRIES = 5;

export interface RetryOptions {
  tries: number;
}

export function retry(maxTries: number, mode: string = "strict"): number {
  // try a few times
  return maxTries * 2;
}
