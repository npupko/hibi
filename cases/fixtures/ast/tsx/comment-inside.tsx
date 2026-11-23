// Retry settings.
export const LIMIT = 99;

export function Badge({ count }: { count: number }) {
  // try several times, then give up
  return <span className="strict" data-label="yes">{count > LIMIT ? "many" : count}</span>;
}
