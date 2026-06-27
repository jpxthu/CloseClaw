use super::cleaner::clean_feishu_content;

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
        clean_feishu_content("<at user_id=\"ou_a\">Bob</at> and <at user_id=\"ou_b\">Carol</at>"),
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
