//! Languages, support tiers and detection by extension or shebang.

use serde::{Deserialize, Serialize};

/// Every language Isoline knows by name. Anything else is `Other` and lands in the text tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "lowercase")]
pub enum Language {
    TypeScript,
    Tsx,
    JavaScript,
    Jsx,
    Python,
    Rust,
    Html,
    Css,
    Go,
    Java,
    CSharp,
    // Text tier languages that Monaco can still highlight.
    Json,
    Markdown,
    Yaml,
    Toml,
    Shell,
    Sql,
    C,
    Cpp,
    Ruby,
    Php,
    Kotlin,
    Swift,
    Scss,
    Xml,
    Other,
}

/// Support tier. The tier is always visible to the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Cards, dependency map, call flow, summaries.
    Full,
    /// Outline from a grammar's tags query; file-level imports; no call flow.
    Outline,
    /// Monaco highlighting only.
    Text,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Full => "full",
            Tier::Outline => "outline",
            Tier::Text => "text",
        }
    }
    pub fn parse(s: &str) -> Tier {
        match s {
            "full" => Tier::Full,
            "outline" => Tier::Outline,
            _ => Tier::Text,
        }
    }
}

impl Language {
    pub const ALL: &'static [Language] = &[
        Language::TypeScript,
        Language::Tsx,
        Language::JavaScript,
        Language::Jsx,
        Language::Python,
        Language::Rust,
        Language::Html,
        Language::Css,
        Language::Go,
        Language::Java,
        Language::CSharp,
        Language::Json,
        Language::Markdown,
        Language::Yaml,
        Language::Toml,
        Language::Shell,
        Language::Sql,
        Language::C,
        Language::Cpp,
        Language::Ruby,
        Language::Php,
        Language::Kotlin,
        Language::Swift,
        Language::Scss,
        Language::Xml,
        Language::Other,
    ];

    pub fn tier(self) -> Tier {
        use Language::*;
        match self {
            TypeScript | Tsx | JavaScript | Jsx | Python | Rust | Html | Css => Tier::Full,
            Go | Java | CSharp => Tier::Outline,
            _ => Tier::Text,
        }
    }

    /// Stable lowercase name, used in the database and on the wire.
    pub fn as_str(self) -> &'static str {
        use Language::*;
        match self {
            TypeScript => "typescript",
            Tsx => "tsx",
            JavaScript => "javascript",
            Jsx => "jsx",
            Python => "python",
            Rust => "rust",
            Html => "html",
            Css => "css",
            Go => "go",
            Java => "java",
            CSharp => "csharp",
            Json => "json",
            Markdown => "markdown",
            Yaml => "yaml",
            Toml => "toml",
            Shell => "shell",
            Sql => "sql",
            C => "c",
            Cpp => "cpp",
            Ruby => "ruby",
            Php => "php",
            Kotlin => "kotlin",
            Swift => "swift",
            Scss => "scss",
            Xml => "xml",
            Other => "other",
        }
    }

    pub fn parse(s: &str) -> Language {
        Language::ALL
            .iter()
            .copied()
            .find(|l| l.as_str() == s)
            .unwrap_or(Language::Other)
    }

    /// Human label, sentence case.
    pub fn label(self) -> &'static str {
        use Language::*;
        match self {
            TypeScript => "TypeScript",
            Tsx => "TSX",
            JavaScript => "JavaScript",
            Jsx => "JSX",
            Python => "Python",
            Rust => "Rust",
            Html => "HTML",
            Css => "CSS",
            Go => "Go",
            Java => "Java",
            CSharp => "C#",
            Json => "JSON",
            Markdown => "Markdown",
            Yaml => "YAML",
            Toml => "TOML",
            Shell => "Shell",
            Sql => "SQL",
            C => "C",
            Cpp => "C++",
            Ruby => "Ruby",
            Php => "PHP",
            Kotlin => "Kotlin",
            Swift => "Swift",
            Scss => "SCSS",
            Xml => "XML",
            Other => "Plain text",
        }
    }

    /// Monaco language id for the code view.
    pub fn monaco_id(self) -> &'static str {
        use Language::*;
        match self {
            TypeScript | Tsx => "typescript",
            JavaScript | Jsx => "javascript",
            Python => "python",
            Rust => "rust",
            Html => "html",
            Css => "css",
            Go => "go",
            Java => "java",
            CSharp => "csharp",
            Json => "json",
            Markdown => "markdown",
            Yaml => "yaml",
            Toml => "ini",
            Shell => "shell",
            Sql => "sql",
            C => "c",
            Cpp => "cpp",
            Ruby => "ruby",
            Php => "php",
            Kotlin => "kotlin",
            Swift => "swift",
            Scss => "scss",
            Xml => "xml",
            Other => "plaintext",
        }
    }

    /// Detect from the file name, falling back to a shebang on the first line.
    pub fn detect(path: &str, first_line: Option<&str>) -> Language {
        let name = path.rsplit('/').next().unwrap_or(path);
        let lower = name.to_ascii_lowercase();
        let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
        use Language::*;
        let by_ext = match ext {
            "ts" | "mts" | "cts" => TypeScript,
            "tsx" => Tsx,
            "js" | "mjs" | "cjs" => JavaScript,
            "jsx" => Jsx,
            "py" | "pyi" => Python,
            "rs" => Rust,
            "html" | "htm" => Html,
            "css" => Css,
            "go" => Go,
            "java" => Java,
            "cs" => CSharp,
            "json" | "jsonc" => Json,
            "md" | "mdx" | "markdown" => Markdown,
            "yml" | "yaml" => Yaml,
            "toml" => Toml,
            "sh" | "bash" | "zsh" => Shell,
            "sql" => Sql,
            "c" | "h" => C,
            "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => Cpp,
            "rb" => Ruby,
            "php" => Php,
            "kt" | "kts" => Kotlin,
            "swift" => Swift,
            "scss" | "sass" | "less" => Scss,
            "xml" | "svg" | "plist" => Xml,
            _ => Other,
        };
        if lower.ends_with(".d.ts") {
            return TypeScript;
        }
        if by_ext != Other {
            return by_ext;
        }
        match lower.as_str() {
            "dockerfile" | "makefile" => return Shell,
            _ => {}
        }
        if let Some(rest) = first_line.and_then(|l| l.strip_prefix("#!")) {
            {
                if rest.contains("python") {
                    return Python;
                }
                if rest.contains("node") || rest.contains("deno") || rest.contains("bun") {
                    return JavaScript;
                }
                if rest.contains("sh") {
                    return Shell;
                }
            }
        }
        Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects() {
        assert_eq!(Language::detect("src/a.ts", None), Language::TypeScript);
        assert_eq!(Language::detect("a.d.ts", None), Language::TypeScript);
        assert_eq!(Language::detect("x/App.TSX", None), Language::Tsx);
        assert_eq!(
            Language::detect("bin/tool", Some("#!/usr/bin/env python3")),
            Language::Python
        );
        assert_eq!(Language::detect("main.go", None).tier(), Tier::Outline);
        assert_eq!(Language::detect("weird.zzz", None).tier(), Tier::Text);
        for l in Language::ALL {
            assert_eq!(Language::parse(l.as_str()), *l);
        }
    }
}
