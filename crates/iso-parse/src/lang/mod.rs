//! Language registry. The lead owns this file; each language agent owns its own module.

use iso_core::Language;

use crate::Extractor;

pub mod css;
pub mod generic;
pub mod html;
pub mod js;
pub mod python;
pub mod rust;
pub mod ts;

pub fn extractor(lang: Language) -> Option<&'static dyn Extractor> {
    use Language::*;
    Some(match lang {
        TypeScript => &ts::TS,
        Tsx => &ts::TSX,
        JavaScript => &js::JS,
        Jsx => &js::JSX,
        Python => &python::PYTHON,
        Rust => &rust::RUST,
        Html => &html::HTML,
        Css => &css::CSS,
        Go => &generic::GO,
        Java => &generic::JAVA,
        CSharp => &generic::CSHARP,
        _ => return None,
    })
}
