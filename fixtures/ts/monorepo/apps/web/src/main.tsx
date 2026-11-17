import { Button } from "@acme/ui";
import { Card } from "@acme/ui/card";
import { Icon } from "@acme/ui/button/icon";
import { add, greet } from "@acme/core";
import { routes } from "@/routes";
import { config } from "./config.js";
import { createRoot } from "react-dom/client";
import fs from "node:fs";
import { missing } from "./nope";
import { gone } from "@/gone";
import { hidden } from "@acme/ui/hidden";

export async function main() {
  const lazy = await import("./lazy");
  createRoot(fs.existsSync(config.root) ? null : null);
  return [Button, Card, Icon, add, greet, routes, missing, gone, hidden, lazy];
}
