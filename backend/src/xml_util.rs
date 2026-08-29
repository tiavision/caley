//! Small helpers for `quick-xml` 0.42.
//!
//! Two API changes in quick-xml affect our hand-rolled DAV/autodiscover parsers:
//!
//! * Name types (`QName`, `LocalName`) now wrap `&str` instead of `&[u8]`.
//! * Entity and character references inside text (`&amp;`, `&#38;`, ...) are no
//!   longer inlined into `Event::Text`; they arrive as separate
//!   `Event::GeneralRef` events. [`resolve_ref`] turns one back into its text.

use quick_xml::events::BytesRef;

/// Resolve a single `Event::GeneralRef` payload to the text it represents.
///
/// Handles numeric character references (`&#38;`, `&#x26;`) and the five
/// predefined XML entities. An unknown named entity is returned verbatim
/// (wrapped back in `&...;`) rather than dropped.
pub(crate) fn resolve_ref(r: &BytesRef) -> String {
    if let Ok(Some(c)) = r.resolve_char_ref() {
        return c.to_string();
    }
    match &**r {
        "amp" => "&".to_string(),
        "lt" => "<".to_string(),
        "gt" => ">".to_string(),
        "quot" => "\"".to_string(),
        "apos" => "'".to_string(),
        other => format!("&{other};"),
    }
}
