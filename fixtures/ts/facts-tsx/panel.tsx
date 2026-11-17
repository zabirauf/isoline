import React, { useState, type ReactNode } from "react";
import { Row } from "./row";
import type { Item } from "./types";

interface PanelProps {
  title: string;
  items: Item[];
  footer?: ReactNode;
}

/** A titled list. */
export function Panel({ title, items, footer }: PanelProps) {
  const [open, setOpen] = useState(true);
  return (
    <section className="panel">
      <h2 onClick={() => setOpen(!open)}>{title}</h2>
      {open && items.map((it) => <Row key={it.id} item={it} />)}
      {footer}
    </section>
  );
}

export const List = <T,>({ rows }: { rows: T[] }) => <ul>{rows.length}</ul>;

export default function App() {
  return <Panel title="Items" items={[]} />;
}
