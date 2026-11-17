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
pub const PROMPT_VERSION: u32 = 1;

const STYLE: &str = "Style for every text: plain, short sentences in sentence case. \
Keep identifiers exactly as written in the code. Use numbers and units where they help. \
No emoji, no exclamation marks, no marketing words, and no filler such as \"This function\" \
or \"This module\". Do not guess beyond what the input shows; when it cannot tell, say less.";

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
a restatement of each line. Name conditions and outcomes, for example \"No run trace: \
return, since no span was started.\" Use fewer steps only when the function is trivial.
- notes: 0 to 2 observations a careful reader would want, each one or two sentences that \
name the exact construct and its consequence. Use kind \"warn\" only for something that can \
go wrong at run time, such as a subscription that is never released when a call throws, or \
an event that can be sent twice. Use kind \"info\" for subtleties, such as a check order that \
sets priority. Leave notes empty when nothing stands out. Describe what you observe \
(\"sits outside the try, so ...\"); never tell the reader what to do.
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
