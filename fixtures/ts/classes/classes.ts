import { connect, type Conn } from "./db.js";

export class Repo {
  private conn?: Conn;
  constructor(private readonly url: string) {}

  async open(): Promise<void> {
    this.conn = await connect(this.url);
  }

  find = async (id: string) => {
    if (!this.conn) await this.open();
    return this.conn!.query(id);
  };

  get ready() {
    return Boolean(this.conn);
  }
}

export const makeRepo = (url: string) => {
  const repo = new Repo(url);
  return {
    load: (id: string) => repo.find(id),
  };
};

function boot() {
  const r = makeRepo("db://x");
  r.load("1");
}
