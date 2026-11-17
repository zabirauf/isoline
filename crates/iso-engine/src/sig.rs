//! Parameter lists and return types from declaration heads, for TypeScript, JavaScript,
//! Python and Rust. Text-based and bracket-aware: it never needs a parse tree, so it also
//! works on the signature stored in the index.

use iso_core::ipc::{ParamInfo, SigInfo};

/// The declaration head of a symbol: its source from the first line up to the body, at most
/// 60 lines.
pub fn declaration_head(lines: &[&str], start: u32, end: u32) -> String {
    let s = start.max(1) as usize - 1;
    let e = (end as usize).min(lines.len()).min(s + 60);
    if s >= e {
        return String::new();
    }
    lines[s..e].join("\n")
}

fn opens(c: char) -> bool {
    matches!(c, '(' | '[' | '{' | '<')
}
fn closes(c: char) -> bool {
    matches!(c, ')' | ']' | '}' | '>')
}

/// Characters with their bracket depth before them. `=>` and `->` are not brackets.
fn depths(text: &str) -> Vec<(usize, char, i32)> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = Vec::with_capacity(chars.len());
    let mut depth = 0i32;
    let mut in_str: Option<char> = None;
    for (k, &(i, c)) in chars.iter().enumerate() {
        let prev = if k > 0 { chars[k - 1].1 } else { ' ' };
        out.push((i, c, depth));
        if let Some(q) = in_str {
            if c == q && prev != '\\' {
                in_str = None;
            }
            continue;
        }
        match c {
            // `'a` is a Rust lifetime, not a string: an identifier after the quote that is not
            // closed right away, in a type or generic position.
            '\'' if is_lifetime(&chars, k) => {}
            '"' | '\'' | '`' => in_str = Some(c),
            '>' if prev == '=' || prev == '-' => {}
            c if opens(c) => depth += 1,
            c if closes(c) => depth = (depth - 1).max(0),
            _ => {}
        }
    }
    out
}

fn is_lifetime(chars: &[(usize, char)], k: usize) -> bool {
    let prev = chars[..k]
        .iter()
        .rev()
        .map(|x| x.1)
        .find(|c| !c.is_whitespace());
    let ident: Vec<char> = chars[k + 1..]
        .iter()
        .map(|x| x.1)
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    let after = chars.get(k + 1 + ident.len()).map(|x| x.1);
    !ident.is_empty()
        && after != Some('\'')
        && matches!(prev, Some('<' | ',' | '&' | '(' | '+' | ':'))
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Split at top-level commas.
fn split_top(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut last = 0;
    for (i, c, d) in depths(text) {
        if c == ',' && d == 0 {
            parts.push(text[last..i].to_string());
            last = i + 1;
        }
    }
    parts.push(text[last..].to_string());
    parts
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// Byte index of the first top-level `ch` that is not part of `::`, `==`, `=>`, `->`, `:=`.
fn find_top(text: &str, ch: char) -> Option<usize> {
    let b = text.as_bytes();
    for (i, c, d) in depths(text) {
        if c != ch || d != 0 {
            continue;
        }
        let next = b.get(i + c.len_utf8()).copied().unwrap_or(b' ');
        let prev = if i > 0 { b[i - 1] } else { b' ' };
        let bad = match ch {
            ':' => next == b':' || prev == b':' || next == b'=',
            '=' => {
                next == b'='
                    || next == b'>'
                    || prev == b'='
                    || prev == b'!'
                    || prev == b'<'
                    || prev == b'>'
            }
            _ => false,
        };
        if !bad {
            return Some(i);
        }
    }
    None
}

fn param(p: &str) -> ParamInfo {
    let mut text = p.trim();
    // Decorators and modifiers that do not name the parameter.
    for m in [
        "public ",
        "private ",
        "protected ",
        "readonly ",
        "override ",
        "mut ",
    ] {
        while let Some(r) = text.strip_prefix(m) {
            text = r.trim_start();
        }
    }
    while text.starts_with('@') {
        // Python/TS decorators on parameters: drop up to the first space at depth 0.
        match text.find(' ') {
            Some(i) => text = text[i..].trim_start(),
            None => break,
        }
    }
    let (head, default) = match find_top(text, '=') {
        Some(i) => (text[..i].trim(), Some(collapse(&text[i + 1..]))),
        None => (text, None),
    };
    let (name_part, ty) = match find_top(head, ':') {
        Some(i) => (head[..i].trim(), Some(collapse(&head[i + 1..]))),
        None => (head, None),
    };
    let mut name = name_part.to_string();
    let rest = name.starts_with("...") || name.starts_with('*');
    let mut optional = default.is_some();
    if let Some(n) = name.strip_suffix('?') {
        name = n.to_string();
        optional = true;
    }
    ParamInfo {
        name: collapse(&name),
        type_text: ty.filter(|t| !t.is_empty()),
        default,
        rest,
        optional,
    }
}

/// Parse a declaration head. Returns empty parameters when no parameter list is found.
pub fn parse(head: &str) -> SigInfo {
    let d = depths(head);
    // The parameter list is the first `(` at depth 0 (after any generic parameters, which
    // are deeper). For `const f = (a) => …` and `def f(a):` alike.
    let Some(&(open, _, _)) = d.iter().find(|(_, c, dep)| *c == '(' && *dep == 0) else {
        return SigInfo {
            params: Vec::new(),
            returns: None,
        };
    };
    let close = d
        .iter()
        .find(|(i, c, dep)| *i > open && *c == ')' && *dep == 1)
        .map(|x| x.0);
    let Some(close) = close else {
        return SigInfo {
            params: Vec::new(),
            returns: None,
        };
    };
    let params: Vec<ParamInfo> = split_top(&head[open + 1..close])
        .iter()
        .map(|p| param(p))
        .collect();
    // Return type: `-> T` (Rust, Python) or `: T` (TypeScript), up to the body.
    let after = head[close + 1..].trim_start();
    let ret_text = if let Some(r) = after.strip_prefix("->") {
        Some(r)
    } else if let Some(r) = after.strip_prefix(':') {
        // Python's trailing colon is the body, not a type.
        Some(r).filter(|r| !(r.trim().is_empty() || r.starts_with('\n')))
    } else {
        None
    };
    let returns = ret_text.and_then(|r| {
        let mut end = r.len();
        for (i, c, dep) in depths(r) {
            if dep != 0 {
                continue;
            }
            let tail = &r[i..];
            if c == '{'
                || tail.starts_with("=>")
                || tail.starts_with(" where ")
                || tail.starts_with("\nwhere")
                || (c == ':' && tail[1..].trim().is_empty())
            {
                end = i;
                break;
            }
            if c == ':' && tail.starts_with(":\n") {
                end = i;
                break;
            }
        }
        let t = collapse(&r[..end]);
        let t = t.trim_end_matches(':').trim().to_string();
        (!t.is_empty()).then_some(t)
    });
    SigInfo { params, returns }
}

/// Identifiers in a type text (`Promise<Map<string, User>>` gives Promise, Map, string, User).
pub fn type_names(t: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev_colon = false;
    for c in t.chars().chain(std::iter::once(' ')) {
        if c.is_alphanumeric() || c == '_' || c == '$' {
            cur.push(c);
            continue;
        }
        if !cur.is_empty() {
            // `a::B` keeps only the path head as a name to resolve; `x.Y` the head too.
            let skip = prev_colon || cur.chars().next().is_some_and(|ch| ch.is_ascii_digit());
            if !skip && !out.contains(&cur) {
                out.push(cur.clone());
            }
            cur.clear();
        }
        prev_colon = c == ':' || c == '.';
    }
    out
}

/// Builtin and primitive type names per language family.
pub fn is_builtin_type(name: &str) -> bool {
    const B: &[&str] = &[
        // TypeScript / JavaScript
        "string",
        "number",
        "boolean",
        "bigint",
        "symbol",
        "object",
        "void",
        "undefined",
        "null",
        "never",
        "unknown",
        "any",
        "Promise",
        "Array",
        "ReadonlyArray",
        "Record",
        "Partial",
        "Required",
        "Readonly",
        "Pick",
        "Omit",
        "Exclude",
        "Extract",
        "NonNullable",
        "ReturnType",
        "Parameters",
        "Awaited",
        "Map",
        "Set",
        "WeakMap",
        "WeakSet",
        "Date",
        "Error",
        "RegExp",
        "Function",
        "Object",
        "String",
        "Number",
        "Boolean",
        "Uint8Array",
        "ArrayBuffer",
        "Iterable",
        "AsyncIterable",
        "Iterator",
        "AsyncIterator",
        "Generator",
        "AsyncGenerator",
        "keyof",
        "typeof",
        "infer",
        "extends",
        "true",
        "false",
        "unique",
        "asserts",
        "is",
        "readonly",
        "in",
        // Python
        "int",
        "float",
        "str",
        "bool",
        "bytes",
        "None",
        "list",
        "dict",
        "set",
        "tuple",
        "type",
        "Any",
        "Optional",
        "Union",
        "List",
        "Dict",
        "Tuple",
        "Callable",
        "Iterable",
        "Sequence",
        "Mapping",
        "Self",
        "ClassVar",
        "Literal",
        // Rust
        "i8",
        "i16",
        "i32",
        "i64",
        "i128",
        "isize",
        "u8",
        "u16",
        "u32",
        "u64",
        "u128",
        "usize",
        "f32",
        "f64",
        "char",
        "str",
        "bool",
        "String",
        "Vec",
        "Option",
        "Result",
        "Box",
        "Rc",
        "Arc",
        "HashMap",
        "HashSet",
        "BTreeMap",
        "BTreeSet",
        "VecDeque",
        "Path",
        "PathBuf",
        "Cow",
        "Fn",
        "FnMut",
        "FnOnce",
        "impl",
        "dyn",
        "mut",
        "const",
        "self",
        "Self",
        "Sync",
        "Send",
        "Sized",
        "Iterator",
        "IntoIterator",
        "AsRef",
        "Into",
        "From",
        "Default",
        "Clone",
        "Debug",
    ];
    B.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(s: &SigInfo) -> Vec<(&str, Option<&str>, Option<&str>)> {
        s.params
            .iter()
            .map(|p| {
                (
                    p.name.as_str(),
                    p.type_text.as_deref(),
                    p.default.as_deref(),
                )
            })
            .collect()
    }

    #[test]
    fn typescript() {
        let h = "export async function runAgentHarnessLifecycleAttempt(\n  harness: AgentHarness,\n  params: AgentHarnessAttemptParamsV2,\n  execute: (params: AgentHarnessAttemptParamsV2) => Promise<AgentHarnessAttemptResult> = (\n    attemptParams,\n  ) => harness.runAttempt(attemptParams),\n): Promise<EmbeddedRunAttemptResult> {";
        let s = parse(h);
        assert_eq!(s.params.len(), 3);
        assert_eq!(s.params[0].name, "harness");
        assert_eq!(s.params[0].type_text.as_deref(), Some("AgentHarness"));
        assert_eq!(s.params[2].name, "execute");
        assert_eq!(
            s.params[2].type_text.as_deref(),
            Some("(params: AgentHarnessAttemptParamsV2) => Promise<AgentHarnessAttemptResult>")
        );
        assert!(s.params[2].optional);
        assert_eq!(
            s.returns.as_deref(),
            Some("Promise<EmbeddedRunAttemptResult>")
        );
        let s = parse("function f<T extends Base>(a?: T, ...rest: string[]): void {");
        assert_eq!(
            names(&s),
            vec![("a", Some("T"), None), ("...rest", Some("string[]"), None)]
        );
        assert!(s.params[0].optional && s.params[1].rest);
        let s = parse("const handle = async ({ a, b }: Props, n = 3) => {");
        assert_eq!(
            names(&s),
            vec![("{ a, b }", Some("Props"), None), ("n", None, Some("3"))]
        );
        assert_eq!(s.returns, None);
    }

    #[test]
    fn python_and_rust() {
        let s = parse(
            "@cached\ndef load(self, repo: Repo, n: int = 3, *args, **kw) -> dict[str, Item]:",
        );
        assert_eq!(
            names(&s),
            vec![
                ("self", None, None),
                ("repo", Some("Repo"), None),
                ("n", Some("int"), Some("3")),
                ("*args", None, None),
                ("**kw", None, None)
            ]
        );
        assert_eq!(s.returns.as_deref(), Some("dict[str, Item]"));
        let s = parse("def f(a):");
        assert_eq!(s.returns, None);
        let s = parse(
            "pub fn index_paths(self: &Arc<Self>, paths: &[String], f: impl Fn(u32) -> bool) -> Result<IndexReport> {",
        );
        assert_eq!(
            names(&s),
            vec![
                ("self", Some("&Arc<Self>"), None),
                ("paths", Some("&[String]"), None),
                ("f", Some("impl Fn(u32) -> bool"), None)
            ]
        );
        assert_eq!(s.returns.as_deref(), Some("Result<IndexReport>"));
        let s = parse("fn g<'a, T>(&mut self, x: x::Y<'a>) -> io::Result<()>\nwhere T: Clone {");
        assert_eq!(names(&s)[0].0, "&mut self");
        assert_eq!(s.returns.as_deref(), Some("io::Result<()>"));
        assert_eq!(
            type_names("Promise<Map<string, User>>"),
            vec!["Promise", "Map", "string", "User"]
        );
        assert_eq!(type_names("io::Result<()>"), vec!["io"]);
    }
}
