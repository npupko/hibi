interface Ctor {
  new (x: number): Ctor;
  'a-b': number;
  1: string;
  [k]: number;
  m(): void;
}

class C {
  #count = 0;
  #step(): void {}
  'a-b' = 1;
  2 = 'two';
  [k] = 3;
}
