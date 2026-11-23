// Retry settings.
export const MAX_SHOWN = 99;

export function Badge({ total }: { total: number }) {
  // try a few times
  return <span className="strict" data-title="yes">{total > MAX_SHOWN ? "many" : total}</span>;
}
