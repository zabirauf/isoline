import { add } from "./math.js";

export function greet(name: string): string {
  return `hello ${name} #${add(1, 2)}`;
}
