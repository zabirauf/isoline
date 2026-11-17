import fs, { readFile as read, type Stats } from "node:fs";
import * as path from "node:path";
import type { Config } from "./config.js";
import "./polyfill.js";
export { helper } from "./helper.js";
export * from "./all.js";
export type { Shape } from "./shapes.js";

/** Default timeout in ms. */
export const TIMEOUT = 5000;
const cache = new Map<string, Stats>();

export interface Options extends Config {
  root: string;
}

export type Mode = "fast" | "slow";

export enum Level {
  Low,
  High,
}

/**
 * Loads a file.
 * @param name file name
 */
export async function load(name: string, opts: Options = { root: "." }): Promise<string> {
  const full = path.join(opts.root, name);
  const text = await read(full, "utf8");
  cache.set(full, fs.statSync(full));
  return text.trim();
}

export const parse = (text: string, mode: Mode) => JSON.parse(text) as Config;

export class Loader {
  private seen = 0;
  constructor(private readonly opts: Options) {}

  async run(name: string) {
    this.seen++;
    const plugin = await import("./plugin.js");
    return plugin.apply(await load(name, this.opts));
  }

  onDone = () => console.log(TIMEOUT);
}
