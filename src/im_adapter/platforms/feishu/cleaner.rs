//! Feishu message cleaner — removes `<at>` mention tags from raw text.

/// Remove Feishu `<at user_id="...">name</at>` tags from raw text.
pub fn clean_feishu_content(raw: &str) -> String {
    let mut result = raw.to_string();
    loop {
        let open_tag = "<at ";
        let Some(start) = result.find(open_tag) else {
            break;
        };
        let Some(gt_offset) = result[start..].find('>') else {
            break;
        };
        let after_open = start + gt_offset + 1;
        if let Some(close_pos) = result[after_open..].find("</at>") {
            let end = after_open + close_pos + "</at>".len();
            result.replace_range(start..end, "");
        } else {
            result.replace_range(start..=start + gt_offset, "");
        }
    }
    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_single_at_tag() {
        assert_eq!(
            clean_feishu_content("hello <at user_id=\"ou_x\">Alice</at> world"),
            "hello  world"
        );
    }

    #[test]
    fn test_clean_multiple_at_tags() {
        assert_eq!(
            clean_feishu_content(
                "<at user_id=\"ou_a\">Bob</at> and <at user_id=\"ou_b\">Carol</at>"
            ),
            "and"
        );
    }

    #[test]
    fn test_clean_no_at_tags() {
        assert_eq!(clean_feishu_content("hello world"), "hello world");
    }

    #[test]
    fn test_clean_unclosed_at_tag() {
        // Unclosed <at> tag — only the opening tag is removed, content stays.
        assert_eq!(
            clean_feishu_content("hello <at user_id=\"ou_x\">Alice world"),
            "hello Alice world"
        );
    }

    #[test]
    fn test_clean_empty_input() {
        assert_eq!(clean_feishu_content(""), "");
    }
}
