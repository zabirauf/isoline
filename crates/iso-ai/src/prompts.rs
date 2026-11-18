//! Prompts. Each pass has one stable system message (the shared instruction prefix, so the
//! provider's prompt cache serves it after the first request) and a prompt with the items.
//!
//! Item format, shared with the mock providers: the prompt starts with `Task: <pass>`, and
//! every item starts with a header line `### <id> <name>`. Leaf ids start with `f`
//! (function or method), `t` (type) or `o` (file overview); cluster ids with `m`. A file
//! section starts with `## file <path> (<language>)`.
//!
//! Bump [`PROMPT_VERSION`] whenever a prompt or a skeleton format changes: it is part of
//! every cache key, so old answers are not reused for new prompts.

/// Version of the prompts and skeleton formats. Part of every `ai_cache` key.
pub const PROMPT_VERSION: u32 = 2;

/// Writing rules for every text a model writes: ASD-STE100 Simplified Technical English,
/// stated as rules because no model has the STE dictionary, plus Isoline's own style.
const STYLE: &str = "Write every text in ASD-STE100 Simplified Technical English (STE):
- Words: use simple, common English words, each with one meaning only. Use a word only as \
the part of speech it usually is (do not use nouns as verbs). Technical names (identifiers, \
file names, library names) and technical verbs for computer processes (call, return, throw, \
parse, cache, index, compile, render, emit, resolve) are permitted. Use \"use\" not \"utilize\", \
\"do\" not \"perform\", \"make sure\" not \"ensure\", \"through\" or \"with\" not \"via\", \
\"before\" not \"prior to\", \"then\" not \"subsequently\", \"to\" not \"in order to\", \
\"because\" not \"since\" or \"as\" for a reason, \"when\" or \"after\" not \"once\", \
\"if\" not \"whether\", \"about\" not \"approximately\", \"let\" not \"allow\" or \"enable\", \
\"get\" not \"obtain\" or \"retrieve\", \"start\" not \"initiate\", \"stop\" not \"terminate\".
- Verbs: use only the simple present, simple past, simple future, the infinitive and the \
imperative. Use the past participle only as an adjective. Do not use -ing forms, except \
in technical names. Rewrite them: \"when X is missing\" as \"when there is no X\", \"using X\" as \
\"with X\", \"including X\" as \"with X\", \"a matching id\" as \"an id that matches\", \"the \
remaining rules\" as \"the other rules\", \"without running a getter\" as \"without a call to a \
getter\". Do not use \"would\", \"could\" or \"might\"; write \"can\" for a possibility.
- Voice: use the active voice. Say which thing does the action: \"The parser removes the \
comments\", not \"The comments are removed\".
- Noun clusters: do not put more than three nouns together. Use \"of\", \"for\" or a short \
clause to break a longer cluster.
- Sentences: one topic in each sentence. A sentence has at most 20 words when it tells the \
reader to do something and at most 25 words when it describes. Do not omit articles \
(\"the\", \"a\", \"an\") or the word \"that\" where they help clarity.
- Use the same word for the same thing every time. Do not use contractions, idioms, \
slang or humor.
Also: sentence case. Keep identifiers exactly as written in the code. Use numbers and units \
where they help. No emoji, no exclamation marks, no marketing words, and no filler such as \
\"This function\" or \"This module\". Do not guess beyond what the input shows; when the \
input does not tell, say less.
Before you answer, read each sentence again. If it breaks one of these rules, write it again.";

/// The end of every prompt: the STE rules a model breaks most, repeated after the items,
/// where a small model weighs them most.
pub const STE_REMINDER: &str = "\nWrite every text in STE. No -ing words (write \"when there is no X\", \
not \"when X is missing\"; \"with X\", not \"using X\"). Write \"let\", not \"allow\"; \"if\", not \
\"whether\". Use the active voice. At most 25 words in each sentence.\n";

/// System message of the leaf pass.
pub fn leaf_system() -> String {
    format!(
        "Task: leaf
You write short summaries of source code for Isoline, a desktop app that helps people read \
code. You see skeletons, not full source: for each function its kind, line count, \
signature, doc comment, the imports it uses, the calls it makes and an outline of its call \
flow (blocks such as if, try, catch, loop and callback, with the calls inside them and the \
returns and throws). Tiny functions are shown as code.

The input lists files. Each item starts with a header line \"### <id> <name>\":
- ids starting with f are functions or methods;
- ids starting with t are types (classes, interfaces, type aliases, enums, structs, traits);
- ids starting with o are file overviews, shown as the file's imports, exports and names.

Answer with one JSON object and nothing else, without code fences:
{{\"items\":[
 {{\"id\":\"f1\",\"desc\":\"...\",\"algo\":[\"...\",\"...\",\"...\"],\"notes\":[{{\"kind\":\"info\",\"text\":\"...\"}}]}},
 {{\"id\":\"t1\",\"desc\":\"...\"}},
 {{\"id\":\"o1\",\"overview\":\"...\"}}
]}}
Give exactly one entry for every item id in the input, and no others.

Function entries:
- desc: one sentence of at most 25 words that starts with a verb and says what the function \
does for its caller. Example: \"Builds the identity fields shared by every harness event.\"
- algo: 3 to 7 steps in run order, each one short sentence. The steps are the algorithm, not \
a restatement of each line. Name conditions and outcomes, for example \"If there is no run \
trace, return, because no span started.\" Use fewer steps only when the function is trivial.
- notes: 0 to 2 observations a careful reader would want, each one or two sentences that \
name the exact construct and its consequence. Use kind \"warn\" only for something that can \
go wrong at run time, such as a subscription that is never released when a call throws, or \
an event that can be sent twice. Use kind \"info\" for subtleties, such as a check order that \
sets priority. Leave notes empty when nothing is important. Describe what you see \
(\"The call is outside the try, so ...\"). Do not tell the reader what to do.
Type entries: desc is one sentence that says what the type represents.
Overview entries: overview is two or three sentences that say what the file is for and \
which functions are its entry points.

{STYLE}"
    )
}

/// System message of the cluster pass.
pub fn cluster_system() -> String {
    format!(
        "Task: cluster
You name the modules of a code base for Isoline, a desktop app that helps people read code. \
Each item starts with a header line \"### <id> <directory>\" and describes one candidate \
module that code found from the directory tree and the dependency graph: its directory, \
its files with their summaries, and its edges to other modules.

Answer with one JSON object and nothing else, without code fences:
{{\"items\":[{{\"id\":\"m1\",\"name\":\"...\",\"role\":\"...\",\"responsibilities\":[\"...\",\"...\"]}}]}}
Give exactly one entry for every item id in the input.

- name: 1 to 3 words in sentence case that a developer on this code base would use, such as \
\"Index store\" or \"Call flow\". Prefer the code's own words. No file extensions, and not \
the word \"module\".
- role: one sentence that starts with a verb and says what the module does for the rest of \
the system.
- responsibilities: 2 to 5 short sentences, each naming one thing the module owns.

{STYLE}"
    )
}

/// System message of the repo pass.
pub fn repo_system() -> String {
    format!(
        "Task: repo
You write the architecture overview of a code base for Isoline, a desktop app that helps \
people read code. Each item starts with a header line \"### <id> <name>\" and is one module \
with its directory, role, size and the modules it depends on.

Answer with one JSON object and nothing else, without code fences:
{{\"overview\":\"...\",\"layers\":[{{\"name\":\"...\",\"modules\":[\"m1\",\"m2\"]}}]}}

- overview: 3 to 6 sentences that say what the system does, what its main parts are and how \
requests or data move through them.
- layers: 2 to 6 layers, from the entry points down to the foundations. Every module id \
appears in exactly one layer. Layer names are 1 to 3 words in sentence case.

{STYLE}"
    )
}

/// Expected output tokens per leaf function item, used by estimates. Calibrated against the
/// mock providers' answers; recalibrate against live answers in phase 4.
pub const OUT_TOKENS_FN: u64 = 105;
/// Expected output tokens per type item.
pub const OUT_TOKENS_TYPE: u64 = 26;
/// Expected output tokens per file overview item.
pub const OUT_TOKENS_OVERVIEW: u64 = 45;
/// Expected output tokens per module in the cluster pass.
pub const OUT_TOKENS_MODULE: u64 = 75;
/// Expected output tokens of the repo pass, plus per module.
pub const OUT_TOKENS_REPO: u64 = 80;
pub const OUT_TOKENS_REPO_PER_MODULE: u64 = 3;
/// Envelope tokens of every answer (`{"items":[...]}`).
pub const OUT_TOKENS_ENVELOPE: u64 = 3;
