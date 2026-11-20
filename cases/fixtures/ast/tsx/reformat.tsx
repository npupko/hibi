// Retry settings.
export const LIMIT    =    99;

export function Badge({ count }:
                      { count: number })
{
    // try a few times
    return <span   className="strict"
                   data-label="yes">{count   >   LIMIT ? "many" : count}</span>;
}
