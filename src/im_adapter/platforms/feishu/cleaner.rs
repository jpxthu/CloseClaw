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
