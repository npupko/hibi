// Retry settings.
export const LIMIT = 100;

export function Badge({ count }: { count: number }) {
  // try a few times
  return <span className="lenient" data-label="yes">{count > LIMIT ? "many" : count}</span>;
}
