// A pretend shell for the mock backend: echoes what is typed, answers a few commands, and
// exits on `exit`. Lets the terminal panel be tried and tested in a plain browser.
import type { TermInfo } from "../api";

export function createMockTerm(onExit: (id: number, code: number) => void) {
  const enc = new TextEncoder();
  const sessions = new Map<number, { out: (s: string) => void; line: string; cwd: string }>();
  let next = 0;
  const prompt = (cwd: string) => `\x1b[36m${cwd.split("/").pop() || "/"}\x1b[0m \x1b[32m$\x1b[0m `;
  return {
    open(cwd: string | null, _cols: number, _rows: number, onData: (b: Uint8Array) => void): TermInfo {
      const id = ++next;
      const dir = cwd ?? "/home/mock";
      const out = (s: string) => setTimeout(() => onData(enc.encode(s)), 0);
      sessions.set(id, { out, line: "", cwd: dir });
      out(`\x1b[2mMock shell: it echoes what you type. Try \x1b[0mls\x1b[2m, \x1b[0mcolors\x1b[2m or \x1b[0mexit\x1b[2m.\x1b[0m\r\n${prompt(dir)}`);
      return { id, shell: "mock-sh", cwd: dir };
    },
    write(id: number, data: string) {
      const s = sessions.get(id);
      if (!s) throw new Error("The terminal has closed.");
      for (const ch of data) {
        if (ch === "\r") {
          const cmd = s.line.trim();
          s.line = "";
          let reply = "";
          if (cmd === "exit") {
            sessions.delete(id);
            s.out("\r\n");
            setTimeout(() => onExit(id, 0), 10);
            return;
          } else if (cmd === "ls") reply = "README.md  package.json  src  tools\r\n";
          else if (cmd === "pwd") reply = s.cwd + "\r\n";
          else if (cmd === "colors") reply = [31, 32, 33, 34, 35, 36].map((c) => `\x1b[${c}m■ ${c}\x1b[0m`).join("  ") + "\r\n";
          else if (cmd) reply = `mock-sh: ${cmd}: command not found\r\n`;
          s.out(`\r\n${reply}${prompt(s.cwd)}`);
        } else if (ch === "\x7f") {
          if (s.line) {
            s.line = s.line.slice(0, -1);
            s.out("\b \b");
          }
        } else if (ch === "\x03") {
          s.line = "";
          s.out(`^C\r\n${prompt(s.cwd)}`);
        } else if (ch >= " ") {
          s.line += ch;
          s.out(ch);
        }
      }
    },
    close(id: number) {
      if (sessions.delete(id)) setTimeout(() => onExit(id, 0), 10);
    },
  };
}
