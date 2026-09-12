//! XML content extraction for feishu file/audio messages.
//!
//! Handles `<file key="..." name="..."/>` and `<audio key="..." name="..."/>`
//! tag formats used in CLI-format events.

use closeclaw_common::{MediaRef, MediaType};
use regex::Regex;
use std::sync::OnceLock;

static XML_FILE_AUDIO_RE: OnceLock<Regex> = OnceLock::new();

/// Extract an XML attribute value from a raw attribute string.
/// E.g. `key="file_v3_001" name="report.pdf"` → extracts `file_v3_001` for key.
pub(crate) fn extract_xml_attr(attrs: &str, attr_name: &str) -> Option<String> {
    let pattern = format!(r#"{attr_name}\s*=\s*["']([^"']*)["']"#);
    let re = Regex::new(&pattern).ok()?;
    let caps = re.captures(attrs)?;
    Some(caps.get(1)?.as_str().to_string())
}

/// Extract file/audio info from XML content like `<file key="..." name="..."/>`.
///
/// Returns `(text, media_refs, original_name)` on success, `None` on failure.
pub(crate) fn extract_file_audio_from_xml(
    message_type: &str,
    content: &str,
) -> Option<(String, Vec<MediaRef>, Option<String>)> {
    if !matches!(message_type, "file" | "audio") {
        return None;
    }
    let re: &Regex = XML_FILE_AUDIO_RE
        .get_or_init(|| Regex::new(r#"<(file|audio)\s+([^>]*?)/?>"#).expect("valid regex"));
    let caps = re.captures(content)?;
    let attrs = caps.get(2)?.as_str();
    let key = extract_xml_attr(attrs, "key")?;
    let name = extract_xml_attr(attrs, "name");
    let media_type = MediaType::from(message_type);
    let path = key.clone();
    let media_ref = MediaRef {
        key,
        path,
        media_type,
        size: 0,
        mime: String::new(),
    };
    Some((String::new(), vec![media_ref], name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_xml_attr_double_quotes() {
        let result = extract_xml_attr(r#"key="value""#, "key");
        assert_eq!(result.as_deref(), Some("value"));
    }

    #[test]
    fn test_extract_xml_attr_single_quotes() {
        let result = extract_xml_attr("key='value'", "key");
        assert_eq!(result.as_deref(), Some("value"));
    }

    #[test]
    fn test_extract_xml_attr_missing() {
        let result = extract_xml_attr(r#"key="fk""#, "name");
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_xml_attr_empty_value() {
        let result = extract_xml_attr(r#"name="""#, "name");
        assert_eq!(result.as_deref(), Some(""));
    }

    #[test]
    fn test_extract_file_audio_from_xml_file() {
        let (text, refs, name) =
            extract_file_audio_from_xml("file", r#"<file key="fk" name="doc.pdf"/>"#).unwrap();
        assert!(text.is_empty());
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].key, "fk");
        assert_eq!(refs[0].media_type, MediaType::File);
        assert_eq!(name.as_deref(), Some("doc.pdf"));
    }

    #[test]
    fn test_extract_file_audio_from_xml_audio() {
        let (text, refs, name) =
            extract_file_audio_from_xml("audio", r#"<audio key="ak" name="v.ogg"/>"#).unwrap();
        assert!(text.is_empty());
        assert_eq!(refs[0].media_type, MediaType::Audio);
        assert_eq!(name.as_deref(), Some("v.ogg"));
    }

    #[test]
    fn test_extract_file_audio_from_xml_no_name() {
        let (_, refs, name) = extract_file_audio_from_xml("file", r#"<file key="fk"/>"#).unwrap();
        assert_eq!(refs[0].key, "fk");
        assert!(name.is_none());
    }

    #[test]
    fn test_extract_file_audio_from_xml_non_file_type() {
        assert!(extract_file_audio_from_xml("image", r#"<file key="fk"/>"#).is_none());
    }

    #[test]
    fn test_extract_file_audio_from_xml_malformed() {
        assert!(extract_file_audio_from_xml("file", "<file key=\"bad").is_none());
    }

    #[test]
    fn test_extract_file_audio_from_xml_no_tag() {
        assert!(extract_file_audio_from_xml("file", "plain text").is_none());
    }
}
