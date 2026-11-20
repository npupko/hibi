interface Shape {
  kind: string;
  area(): number;
  area(scale: number): number;
  (x: number): void;
  [key: string]: unknown;
}

enum Color {
  Red,
  Green = 2,
  "Blue-ish" = 3,
}

abstract class Box {
  @observable size = 1;
  handler = () => 1;
  get width() { return 1; }
  abstract run(): void;
  abstract count: number;
  static { init(); }
  @log
  // between
  @trace open() {}
}

const shape = { area() { return 1; } };
type Point = { x: number };
