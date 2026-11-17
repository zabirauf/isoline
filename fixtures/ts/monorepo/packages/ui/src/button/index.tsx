import { greet } from "@acme/core";
import { add } from "@acme/core/math";
import type { Theme } from "../theme";

export function Button({ label, theme }: { label: string; theme?: Theme }) {
  return <button title={greet(label)}>{add(1, 2)}</button>;
}
