// Mock AI settings and request log: what the Settings dialog and the developer panel read.
// The prompts are short stand-ins for the engine's real system messages.
import type { AiConfig } from "../bindings/AiConfig";
import type { AiConfigUpdate } from "../bindings/AiConfigUpdate";
import type { AiModel } from "../bindings/AiModel";
import type { AiRequestDetail } from "../bindings/AiRequestDetail";
import type { AiRequestRow } from "../bindings/AiRequestRow";
import type { AiTask } from "../bindings/AiTask";

const MODELS: AiModel[] = [
  { id: "mock-small", name: "Mock small", inputPerM: 0.1, outputPerM: 0.5 },
  { id: "mock-medium", name: "Mock medium", inputPerM: 2, outputPerM: 10 },
  { id: "mock-large", name: "Mock large", inputPerM: 4, outputPerM: 20 },
];

const DEFAULTS: Record<AiTask, { model: string; prompt: string }> = {
  summaries: { model: "mock-small", prompt: 'Task: leaf\nYou write short summaries of source code for Isoline, a desktop app that helps people read code.\n\nAnswer with one JSON object and nothing else:\n{"items":[{"id":"f1","desc":"...","algo":["..."],"notes":[]}]}\n\nWrite every text in ASD-STE100 Simplified Technical English (STE).' },
  modules: { model: "mock-medium", prompt: 'Task: cluster\nYou name the modules of a code base for Isoline.\n\nAnswer with one JSON object and nothing else:\n{"items":[{"id":"m1","name":"...","role":"...","responsibilities":["..."]}]}' },
  architecture: { model: "mock-large", prompt: 'Task: repo\nYou write the architecture overview of a code base for Isoline.\n\nAnswer with one JSON object and nothing else:\n{"overview":"...","layers":[],"flows":[]}' },
  review: { model: "mock-large", prompt: 'Task: review\nYou explain a code change to a reviewer for Isoline.\n\nAnswer with one JSON object and nothing else:\n{"summary":"...","notes":[],"modules":[]}' },
};
const TASKS: AiTask[] = ["summaries", "modules", "architecture", "review"];

export function createMockAiDev(signedIn: () => boolean) {
  const models: Record<AiTask, string> = { summaries: DEFAULTS.summaries.model, modules: DEFAULTS.modules.model, architecture: DEFAULTS.architecture.model, review: DEFAULTS.review.model };
  const prompts: Partial<Record<AiTask, string>> = {};
  let economy = true;
  const log: AiRequestDetail[] = [];
  let next = 0;

  const config = (): AiConfig => ({
    tasks: TASKS.map((task) => ({ task, model: models[task], defaultModel: DEFAULTS[task].model, prompt: prompts[task] ?? DEFAULTS[task].prompt, defaultPrompt: DEFAULTS[task].prompt, customPrompt: prompts[task] !== undefined })),
    economy,
    models: MODELS,
    modelsListed: signedIn(),
  });

  return {
    config,
    setConfig(u: AiConfigUpdate): AiConfig {
      economy = u.economy;
      for (const t of u.tasks) {
        if (t.model.trim()) models[t.task] = t.model.trim();
        if (t.prompt && t.prompt.trim() && t.prompt.trim() !== DEFAULTS[t.task].prompt.trim()) prompts[t.task] = t.prompt;
        else delete prompts[t.task];
      }
      return config();
    },
    /** Logs one mock request; `done` finishes it after `ms`. */
    request(task: AiTask, prompt: string, answer: string, ms: number) {
      const model = task === "modules" && economy ? models.summaries : models[task];
      const price = MODELS.find((m) => m.id === model) ?? MODELS[0];
      const tokensIn = Math.round(prompt.length / 3.6) + 600;
      const tokensOut = Math.round(answer.length / 3.6);
      const system = prompts[task] ?? DEFAULTS[task].prompt;
      const row: AiRequestRow = { id: ++next, task, model, startedMs: Date.now(), durationMs: null, state: "running", tokensIn: 0, tokensOut: 0, cachedTokensIn: 0, costUsd: 0, maxTokens: 4000, systemChars: system.length, promptChars: prompt.length, answerChars: 0, error: null };
      const d: AiRequestDetail = { row, system, prompt, answer: "" };
      log.push(d);
      setTimeout(() => {
        Object.assign(row, { durationMs: ms, state: "done", tokensIn, tokensOut, cachedTokensIn: next > 1 ? 600 : 0, costUsd: (tokensIn * price.inputPerM + tokensOut * price.outputPerM) / 1e6, answerChars: answer.length });
        d.answer = answer;
      }, ms);
    },
    rows: (): AiRequestRow[] => log.map((d) => ({ ...d.row })).reverse(),
    detail: (id: number): AiRequestDetail | null => {
      const d = log.find((x) => x.row.id === id);
      return d ? { ...d, row: { ...d.row } } : null;
    },
    clear() {
      log.length = 0;
    },
  };
}
