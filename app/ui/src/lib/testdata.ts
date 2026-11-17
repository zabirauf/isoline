// Fixture access for unit tests: the mock data built from the prototype and the TS golden.
import type { FileView } from "../ipc/bindings/FileView";
import type { FlowUnit } from "../ipc/bindings/FlowUnit";
import type { ModuleMap } from "../ipc/bindings/ModuleMap";
import data from "../ipc/mock/data/mock.json";

const D = data as unknown as { files: Record<string, { view: FileView; flows: Record<string, FlowUnit> }>; moduleMap: ModuleMap };

export const LIFE = "src/agents/harness/lifecycle.ts";
export const lifecycle = D.files[LIFE].view;
export const lifecycleFlows = D.files[LIFE].flows;
export const mockModuleMap = D.moduleMap;
