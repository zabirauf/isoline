//! Call-flow trees. The JSON shape is exactly the prototype's `analyze.js` output, so the
//! golden files it produces compare with `serde_json::Value` equality.
//!
//! ```json
//! {"t":"call","label":"foo","target":{"kind":"fn","id":"foo"},"line":3,"await":false,
//!  "optional":false,"argOf":null,"inner":[]}
//! {"t":"block","kind":"cond","kw":"if","label":"(x)","line":4,"children":[...]}
//! {"t":"exit","kind":"return","label":"x","line":5,"early":false}
//! ```

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "lowercase")]
pub enum FlowTargetKind {
    /// A function declared at the top of this file.
    Fn,
    /// An imported binding.
    Ext,
    /// A method call, or a method on a controller object returned by a factory in this file.
    Method,
    /// A local function bound inside the unit (`const helper = () => …`).
    Local,
    /// A parameter of the unit.
    Param,
    /// A built-in (`Object.keys`, `JSON.parse`, `print`, `len`).
    Builtin,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct FlowTarget {
    pub kind: FlowTargetKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub id: Option<String>,
    /// Module specifier for `ext` targets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub module: Option<String>,
    /// Flow unit id to drill into (`factory.method`), for controller methods.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub unit: Option<String>,
    /// Factory function the receiver came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub note: Option<String>,
    /// Imported function that produced the method's value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub ext: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "lowercase")]
pub enum BlockKind {
    Cond,
    Try,
    Catch,
    Finally,
    Loop,
    Callback,
    Inline,
    Deferred,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "lowercase")]
pub enum ExitKind {
    Return,
    Throw,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ReturnedMethod {
    pub name: String,
    pub unit: Option<String>,
    pub from: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum FlowItem {
    Call {
        label: String,
        target: FlowTarget,
        line: u32,
        #[serde(rename = "await")]
        awaited: bool,
        optional: bool,
        #[serde(rename = "argOf")]
        arg_of: Option<String>,
        inner: Vec<FlowItem>,
    },
    Block {
        kind: BlockKind,
        /// Keyword tag as shown: `if`, `else`, `only if`, `unless`, `if nullish`, `case`,
        /// `default`, `try`, `catch`, `finally`, `loop`, `callback`, `body`, `later`.
        kw: String,
        label: String,
        line: u32,
        children: Vec<FlowItem>,
    },
    Exit {
        kind: ExitKind,
        label: String,
        line: u32,
        /// Present on returns only: the return sits under a condition.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        early: Option<bool>,
        /// Present when a factory returns its controller object.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        methods: Option<Vec<ReturnedMethod>>,
    },
}

/// One unit of call flow: a top-level function, or a method of a controller object a
/// factory function returns (`createX.run`), or a class method (`MyClass.run`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct FlowUnit {
    pub id: String,
    /// The top-level symbol that owns the unit (its card).
    pub owner: String,
    pub method: Option<String>,
    pub start: u32,
    pub end: u32,
    #[serde(rename = "async")]
    pub is_async: bool,
    pub params: Vec<String>,
    pub tree: Vec<FlowItem>,
}

/// Version of the call-flow walker. Bump when output changes so cached trees are recomputed.
pub const FLOW_WALKER_VERSION: u32 = 3;
