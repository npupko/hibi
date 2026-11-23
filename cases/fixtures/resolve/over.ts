export function f(a: string): void;
export function f(a: number): void;
export function f(a: any) {
  return a;
}

function g(a /* first */, b) {
  return a + b;
}
