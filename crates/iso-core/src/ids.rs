//! Stable ids. They are hashes, so they survive reindexing.
//!
//! On the wire (JSON / IPC) ids are 16-character hex strings, because JavaScript
//! numbers cannot hold a u64.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

use crate::{SymbolKind, hash64};

macro_rules! hex_id {
    ($name:ident) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
        pub struct $name(pub u64);

        impl $name {
            pub fn to_hex(self) -> String {
                format!("{:016x}", self.0)
            }
            pub fn from_hex(s: &str) -> Option<Self> {
                u64::from_str_radix(s, 16).ok().map($name)
            }
            /// SQLite stores INTEGER as i64; ids round-trip through this bit cast.
            pub fn as_i64(self) -> i64 {
                self.0 as i64
            }
            pub fn from_i64(v: i64) -> Self {
                $name(v as u64)
            }
        }
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({:016x})", stringify!($name), self.0)
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{:016x}", self.0)
            }
        }
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.to_hex())
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                $name::from_hex(&s).ok_or_else(|| serde::de::Error::custom("bad id"))
            }
        }
        #[cfg(feature = "ts")]
        impl ts_rs::TS for $name {
            type WithoutGenerics = Self;
            type OptionInnerType = Self;
            fn name(_: &ts_rs::Config) -> String {
                "string".into()
            }
            fn inline(_: &ts_rs::Config) -> String {
                "string".into()
            }
            fn decl(_: &ts_rs::Config) -> String {
                String::new()
            }
            fn decl_concrete(_: &ts_rs::Config) -> String {
                String::new()
            }
        }
    };
}

hex_id!(FileId);
hex_id!(SymbolId);

impl FileId {
    /// Hash of the repo-relative path, with `/` separators on every OS.
    pub fn of_path(rel_path: &str) -> Self {
        FileId(hash64(normalize_rel(rel_path).as_bytes()))
    }
}

impl SymbolId {
    /// Hash of file path + qualified name + kind.
    pub fn new(rel_path: &str, qualified_name: &str, kind: SymbolKind) -> Self {
        let key = format!(
            "{}\u{0}{}\u{0}{}",
            normalize_rel(rel_path),
            qualified_name,
            kind.as_str()
        );
        SymbolId(hash64(key.as_bytes()))
    }
}

/// Repo-relative paths always use `/` and never start with `./`.
pub fn normalize_rel(p: &str) -> String {
    let s = p.replace('\\', "/");
    s.trim_start_matches("./").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids_are_stable_and_hex_roundtrip() {
        let a = FileId::of_path("src/a.ts");
        assert_eq!(a, FileId::of_path("./src/a.ts"));
        assert_eq!(a, FileId::of_path("src\\a.ts"));
        assert_eq!(FileId::from_hex(&a.to_hex()), Some(a));
        assert_eq!(FileId::from_i64(a.as_i64()), a);
        let j = serde_json::to_string(&a).unwrap();
        assert_eq!(serde_json::from_str::<FileId>(&j).unwrap(), a);
        let s1 = SymbolId::new("a.ts", "f", SymbolKind::Function);
        let s2 = SymbolId::new("a.ts", "f", SymbolKind::Method);
        assert_ne!(s1, s2);
    }
}
