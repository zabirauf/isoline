// Plain JavaScript: CommonJS and ESM mixed, no types.
import { readConfig, writeConfig } from "./config.js";
import chalk from "chalk";

const fs = require("node:fs");
const { join } = require("node:path");

/** Merge user settings over the defaults. */
export function merge(defaults, user = {}, ...rest) {
  const out = Object.assign({}, defaults);
  for (const [k, v] of Object.entries(user)) {
    out[k] = v === undefined ? defaults[k] : v;
  }
  rest.forEach(function (extra) {
    Object.assign(out, extra);
  });
  return out;
}

export async function main(argv) {
  const file = join(process.cwd(), argv[2] || "config.json");
  if (!fs.existsSync(file)) {
    console.error(chalk.red(`missing ${file}`));
    return 1;
  }
  let cfg;
  try {
    cfg = merge(await readConfig(file), { verbose: argv.includes("-v") });
  } catch (e) {
    console.error(e);
    cfg = merge({});
  }
  const write = () => writeConfig(file, cfg);
  await write();
  const lazy = import("./plugins.js");
  lazy.then((m) => m.default(cfg));
  return cfg.verbose ? report(cfg) : 0;
}

function report(cfg) {
  const lines = Object.keys(cfg).map((k) => `${k}=${cfg[k]}`);
  lines.length && console.log(lines.join("\n"));
  return lines.length;
}

module.exports = { merge, main };
