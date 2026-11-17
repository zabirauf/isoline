import { fetchAll, makeClient, emit } from "./api.js";
import type { Item, Options } from "./types.js";

/**
 * Loads items and renders them.
 */
export async function render(items: Item[], opts: Options = {}, notify = (msg: string) => emit(msg)) {
  const format = (item: Item) => item.name.toUpperCase().trim();
  const unused = () => emit("never called here");
  const visible = items.filter((i) => i.visible).map(format);
  items.forEach((item) => {
    if (item.children) {
      item.children.map((child) => render([child], opts));
    }
  });
  const sorted = [...visible].sort((a, b) => a.localeCompare(b));
  setTimeout(() => notify("rendered"), 10);
  queueMicrotask(format);
  notify(`count ${sorted.length}`);
  const all = await fetchAll(opts.url ?? "/items", { retries: opts.retries || 3 });
  return all.reduce((acc, x) => acc + x.size, 0);
}

export function createWorker(name: string, run = async () => fetchAll(name)) {
  const client = makeClient(name);
  let busy = false;
  const start = async () => {
    busy = true;
    await run();
    busy = false;
  };
  function stop() {
    client.close();
  }
  return {
    name,
    client,
    busy: () => busy,
    start,
    stop,
    async restart() {
      stop();
      await start();
    },
    status(): string {
      return busy ? "busy" : "idle";
    },
    get label() {
      return name;
    },
  };
}

export async function supervise(names: string[]) {
  const workers = names.map((n) => createWorker(n));
  const first = createWorker(names[0]);
  await first.start();
  first.restart();
  first.client.send("hi");
  first.status();
  first.label;
  const inline = createWorker("x", async () => {
    emit("custom");
    return [];
  });
  inline.stop();
  Object.keys(workers).forEach(function (k) {
    JSON.stringify(k);
  });
  return Promise.all(workers.map((w) => w.start()));
}

export const helper = (x: number) => x * 2;
