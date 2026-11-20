// Top-level comments are not units.
import { readFile } from "fs";

export const VERSION = "1.0";
export const lo = 1, hi = 2;
const { p, q } = { p: 1, q: 2 };
let counter = 0;
export const handler = (x: number): number => x * 2;
declare const DEBUG: boolean;

function f(a: string): void;
function f(a: number): void;
function f(a: any): void {}

declare function g(x: number): void;

export function* gen(): Generator<number> {
  yield 1;
}

export interface Shape {
  area(): number;
  name: string;
}

export abstract class Base {
  abstract size(): number;
  label: string = "café";
}

export class Cache<K> extends Base {
  private count: number = 0;
  get(key: K): string;
  get(key: K): string {
    return "";
  }
  size(): number {
    return this.count;
  }
}

export type Id = string | number;
export enum Color { Red, Green }

export namespace NS {
  export function inner(): void {}
  const hidden = 1;
}

declare module Legacy {
  function old(): void;
}

namespace Bare {
  function inside(): void {}
}

counter += 1;
counter += 1;
