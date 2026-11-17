import json, re, sys

src_path, tpl_path, out_path = sys.argv[1], sys.argv[2], sys.argv[3]
src = open(src_path, encoding="utf-8").read().split("\n")
if src and src[-1] == "":
    src = src[:-1]


def sl(a, b):
    return "\n".join(src[a - 1 : b])


# ---------- imports ----------
imports_text = sl(1, 43)
modules = []
for m in re.finditer(r'import\s*(type\s+)?\{([^}]*)\}\s*from\s*"([^"]+)"', imports_text):
    all_type = bool(m.group(1))
    syms = []
    for part in m.group(2).split(","):
        p = part.strip()
        if not p:
            continue
        t = all_type or p.startswith("type ")
        name = p[5:].strip() if p.startswith("type ") else p
        syms.append({"name": name, "typeOnly": t})
    modules.append({"id": m.group(3), "path": m.group(3), "symbols": syms})
modules.append(
    {
        "id": "harness",
        "path": "harness (AgentHarness param)",
        "runtime": True,
        "symbols": [
            {"name": "runAttempt", "typeOnly": False, "member": True},
            {"name": "classify", "typeOnly": False, "member": True},
        ],
    }
)

# ---------- annotations ----------
FUNCS = [
    dict(
        name="agentRunMessageContent", start=59, end=71,
        desc="Builds the optional, privacy-gated prompt/answer text that rides along on completion events.",
        algo=[
            "Read the content-capture policy from the runtime config snapshot.",
            "If output capture is allowed, pull the assistant's final raw text and truncate it.",
            "If input capture is allowed and a prompt exists, truncate the prompt.",
            "Return whichever fields survived, or undefined when neither did.",
        ],
        notes=[("info", "Prompt and answer are gated independently, so a payload can carry one without the other.")],
    ),
    dict(
        name="assertAgentHarnessContextEngineSupport", start=73, end=89,
        desc="Fails fast when the harness cannot host the configured (non-legacy) context engine.",
        algo=[
            "No context engine, or the \"legacy\" one: return, it is always supported.",
            "Otherwise describe the host: id and label from the harness, capabilities from contextEngineHostCapabilities (default []).",
            "Call assertContextEngineHostSupport for operation \"agent-run\"; it throws on a mismatch.",
        ],
        notes=[],
    ),
    dict(
        name="agentHarnessDiagnosticBase", start=91, end=111,
        desc="Builds the identity fields shared by every harness.* diagnostic event.",
        algo=[
            "Use the trace passed in, else the active diagnostic trace.",
            "Channel is messageChannel, falling back to messageProvider.",
            "Always include runId, sessionId, provider, model and harnessId.",
            "Add agentId, pluginId, sessionKey, trigger, channel and a frozen trace only when present.",
        ],
        notes=[],
    ),
    dict(
        name="normalizeAgentHarnessAttemptResult", start=113, end=154,
        desc="Converts a harness result from the legacy or current contract into the canonical shape with a single terminal field.",
        algo=[
            "Strip the nine legacy terminal flags (aborted, timedOut, promptError and friends); keep the rest as canonical.",
            "Resolve attempt provenance: keep currentAttemptAssistant if the key exists, else fall back to lastAssistant for legacy harnesses.",
            "If the result already has terminal (current contract), return canonical plus provenance.",
            "Otherwise build terminal from the legacy flags with normalizeAgentRunAttemptTerminal and attach it.",
        ],
        notes=[("info", "Object.hasOwn separates an explicit undefined (\"no response this attempt\") from a missing key (legacy harness).")],
    ),
    dict(
        name="agentRunDiagnosticBase", start=156, end=169,
        desc="Builds the shared fields for run.* events on the child agent-run span.",
        algo=[
            "Channel is messageChannel, falling back to messageProvider.",
            "Always include runId, provider, model and the trace passed in.",
            "Add sessionKey, sessionId, agentId, trigger and channel only when present.",
        ],
        notes=[("info", "Differs from the harness base: no harnessId or pluginId, sessionId is optional, and the trace is required and not re-frozen.")],
    ),
    dict(
        name="emitAgentRunCompleted", start=171, end=200,
        desc="Emits run.completed for the child run span, if one was opened.",
        algo=[
            "No run trace: return (no run span was started).",
            "failed = outcome is \"error\" and there is an error, or classifyMissingError is set.",
            "Public fields: base, durationMs, outcome, plus blockedBy and errorCategory when relevant.",
            "Private data: errorMessage and messageContent, sent only if at least one exists.",
        ],
        notes=[],
    ),
    dict(
        name="agentRunCompletion", start=202, end=214,
        desc="Maps a normalized attempt terminal to a run outcome: aborted, blocked, error or completed.",
        algo=[
            "Project the terminal into flat flags.",
            "Timed out, externally aborted or aborted: \"aborted\".",
            "Prompt error from hook:before_agent_run: \"blocked\" by before_agent_run.",
            "Any other prompt error source: \"error\" carrying promptError.",
            "Otherwise: \"completed\".",
        ],
        notes=[("info", "Check order is the priority: an abort wins over a hook block, which wins over an error.")],
    ),
    dict(
        name="withFallbackDiagnosticTrace", start=216, end=227,
        desc="Stamps the lifecycle's trace onto a result that came back without one.",
        algo=[
            "Result already has a diagnosticTrace, or there is no trace to add: return it unchanged.",
            "Otherwise copy the result with a frozen trace, carrying TTS tool-result provenance over to the copy.",
        ],
        notes=[],
    ),
    dict(
        name="emitAgentHarnessRunCompleted", start=229, end=269,
        desc="Emits harness.run.completed with outcome, classification, item lifecycle and the gated final answer.",
        algo=[
            "Derive the run completion and project the terminal.",
            "Harness outcome: timed out becomes \"timed_out\", blocked becomes \"error\", otherwise the run outcome.",
            "On error, take the message from terminal.promptError (a classified, non-thrown failure).",
            "Build gated answer text (no prompt) from currentAttemptAssistant, else currentAttemptCompletedAssistant.",
            "Emit with resultClassification, yieldDetected (if boolean) and a copy of itemLifecycle.",
        ],
        notes=[("info", "The same timeout reads \"aborted\" on run.completed but \"timed_out\" here; a hook block reads \"blocked\" there and \"error\" here.")],
    ),
    dict(
        name="createAgentHarnessLifecycleDiagnostics", start=271, end=355,
        desc="Opens the harness span and returns a controller for phases, the child run span, failure and completion.",
        algo=[
            "Capture startedAt and the active trace; emit harness.run.started with the gated prompt.",
            "Subscribe to agent commentary diagnostics (unsubscribe is returned).",
            "createRunTrace makes a child trace only for non-\"openclaw\" harnesses with an active trace; finalizing mode creates it up front.",
            "startAgentRun: create the run trace if needed and emit run.started.",
            "run(execute): execute inside the run trace context when there is one.",
            "fail(error): record the preflight owner (attempts only), emit harness.run.error with the current phase, then run.completed as error.",
        ],
        notes=[
            ("info", "Phase starts at \"prepare\", so a throw before the send step is attributed to preparation."),
            ("info", "If a failure happens before startAgentRun, no run.* events go out; only harness.run.error does."),
        ],
    ),
    dict(
        name="runAgentHarnessLifecycleAttempt", start=357, end=423,
        desc="Entry point: runs one harness attempt inside full diagnostics, then classifies and normalizes the result.",
        algo=[
            "Create lifecycle diagnostics (opens the harness span).",
            "Phase prepare: assert context-engine support, then start the child run span.",
            "Phase send: run execute(params) in the run trace (default: harness.runAttempt).",
            "Phase resolve: without classify, just tag agentHarnessId; with it, drop any previous classification, reclassify, keep only non-\"ok\" labels.",
            "Normalize to the canonical terminal shape, keep TTS provenance from the raw result, add the fallback trace.",
            "On a throw: fail() emits the error events, then rethrow. Always unsubscribe.",
            "On success: emit run.completed (prompt and answer) then harness.run.completed, and return.",
        ],
        notes=[("info", "Completion events fire after the finally block, so a throwing emitter cannot trigger fail() and double-report.")],
    ),
    dict(
        name="runAgentHarnessLifecycleFinalization", start=425, end=486,
        desc="Entry point: runs a settled-turn finalization under diagnostics, treating an empty result as a normal outcome.",
        algo=[
            "Create diagnostics in finalizing mode (run trace up front) and emit run.started.",
            "Phase send: run execute() in the run trace.",
            "Phase resolve: validate with assertSettledTurnFinalizationResult, giving outcome \"answered\".",
            "EmptySettledTurnFinalizationError is caught and becomes outcome \"empty\" with its result.",
            "Add the fallback trace to the inner result if it has none.",
            "Emit run.completed and harness.run.completed (zeroed itemLifecycle), return the outcome.",
            "Any other error: fail() and rethrow. Always unsubscribe.",
        ],
        notes=[
            ("warn", "startAgentRun() sits outside the try, so if it throws, the commentary subscription is never unsubscribed."),
            ("warn", "Completion events are emitted inside the try. If one throws, fail() sends a second run.completed, this time as error."),
            ("info", "An \"empty\" finalization still reports outcome \"completed\" in both events."),
        ],
    ),
]

TYPES = [
    dict(name="AgentHarnessLifecycleFinalizationOutcome", start=45, end=48,
         desc="What the finalization entry point returns: answered or empty, plus the prepared result."),
    dict(name="AgentHarnessLifecyclePhase", start=50, end=50,
         desc="The step a failure is attributed to (prepare, send, resolve), borrowed from the harness.run.error event type."),
    dict(name="AgentRunCompletion", start=51, end=56,
         desc="Internal summary of how a run ended, fed to run.completed."),
    dict(name="AgentRunMessageContent", start=57, end=57,
         desc="Optional prompt and answer text attached to completion events as private data."),
]


def strip(code):
    code = re.sub(r"/\*.*?\*/", " ", code, flags=re.S)
    code = re.sub(r"//[^\n]*", " ", code)
    code = re.sub(r'"(?:[^"\\\n]|\\.)*"', '""', code)
    code = re.sub(r"'(?:[^'\\\n]|\\.)*'", "''", code)
    return code


fn_names = [f["name"] for f in FUNCS]
out_funcs = []
for f in FUNCS:
    code = sl(f["start"], f["end"])
    s = strip(code)
    calls = [n for n in fn_names if n != f["name"] and re.search(r"\b%s\b" % n, s)]
    uses, type_uses = [], []
    for mod in modules:
        for sym in mod["symbols"]:
            if mod.get("runtime"):
                hit = re.search(r"\bharness\.%s\b" % sym["name"], s)
            else:
                hit = re.search(r"(?<![\.\w$])%s\b" % re.escape(sym["name"]), s)
            if hit:
                (type_uses if sym["typeOnly"] else uses).append(sym["name"])
    exported = src[f["start"] - 1].startswith("export") or any(
        src[i].startswith("export") for i in range(f["start"] - 1, min(f["start"] + 1, f["end"]))
        if re.match(r"export\s+(async\s+)?function\s+%s" % f["name"], src[i])
    )
    out_funcs.append(dict(
        name=f["name"], start=f["start"], end=f["end"], exported=bool(exported),
        code=code, calls=calls, uses=uses, typeUses=type_uses,
        desc=f["desc"], algo=f["algo"], notes=[{"kind": k, "text": t} for k, t in f["notes"]],
    ))

out_types = [dict(t, code=sl(t["start"], t["end"])) for t in TYPES]

data = dict(
    file=dict(name="attachment.txt", lang="TypeScript", lines=len(src)),
    modules=modules,
    functions=out_funcs,
    types=out_types,
    imports=dict(start=1, end=43, code=imports_text),
)
import subprocess, os
data["flows"] = json.loads(subprocess.check_output(["node", os.path.join(os.path.dirname(os.path.abspath(__file__)), "analyze.js"), src_path]))
data["methodDescs"] = {
    "setPhase": "Records which phase (prepare, send, resolve) is running, so a failure can be attributed to it.",
    "startAgentRun": "Opens the child run span if one is needed and emits run.started.",
    "run": "Executes a callback inside the run trace context when there is one, otherwise runs it directly.",
    "fail": "Reports a failure: harness.run.error tagged with the phase, then run.completed as an error.",
    "complete": "Emits run.completed for this lifecycle's run span; does nothing if no span was opened.",
}

for f in out_funcs:
    print(f"{f['name']:42} exp={f['exported']!s:5} calls={f['calls']} uses={f['uses']} types={f['typeUses']}", file=sys.stderr)

blob = json.dumps(data, ensure_ascii=False).replace("</", "<\\/")
tpl = open(tpl_path, encoding="utf-8").read()
open(out_path, "w", encoding="utf-8").write(tpl.replace("__DATA__", blob))
