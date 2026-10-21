export function add(a: number, b: number): number;
export function add(a: string, b: string): string;
export function add(a: any, b: any): any {
  return a + b;
}

export const mul = (a: number, b: number) => a * b;

function internal() {
  return 1;
}

export class Calc<T> {
  value: T;
  constructor(v: T) {
    this.value = v;
  }
  compute(): number {
    return add(1, 2);
  }
  static create() {
    return new Calc(0);
  }
}
