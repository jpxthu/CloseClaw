use super::parse_model_selection;

#[test]
fn test_single_number() {
    let result = parse_model_selection("3", 10).unwrap();
    assert_eq!(result, vec![2]);
}
#[test]
fn test_space_separated() {
    let result = parse_model_selection("1 3 5", 10).unwrap();
    assert_eq!(result, vec![0, 2, 4]);
}
#[test]
fn test_parse_range_expands_closed_interval() {
    let result = parse_model_selection("2-5", 10).unwrap();
    assert_eq!(result, vec![1, 2, 3, 4]);
}
#[test]
fn test_parse_mixed_ranges_and_single_values() {
    let result = parse_model_selection("1-3,5,7", 10).unwrap();
    assert_eq!(result, vec![0, 1, 2, 4, 6]);
}
#[test]
fn test_parse_all_keyword_selects_every_model() {
    let result = parse_model_selection("all", 5).unwrap();
    assert_eq!(result, vec![0, 1, 2, 3, 4]);
}
#[test]
fn test_parse_all_keyword_case_insensitive() {
    let result = parse_model_selection("ALL", 3).unwrap();
    assert_eq!(result, vec![0, 1, 2]);
}
#[test]
fn test_parse_deduplicates_repeated_tokens() {
    let result = parse_model_selection("1 1 1", 5).unwrap();
    assert_eq!(result, vec![0]);
}
#[test]
fn test_parse_sorts_out_of_order_input() {
    let result = parse_model_selection("5 3 1", 10).unwrap();
    assert_eq!(result, vec![0, 2, 4]);
}
#[test]
fn test_empty_input_error() {
    let result = parse_model_selection("", 5);
    assert!(result.is_err());
}
#[test]
fn test_zero_not_allowed() {
    let result = parse_model_selection("0", 5);
    assert!(result.is_err());
}
#[test]
fn test_out_of_range() {
    let result = parse_model_selection("11", 10);
    assert!(result.is_err());
}
#[test]
fn test_start_greater_than_end() {
    let result = parse_model_selection("5-2", 10);
    assert!(result.is_err());
}
#[test]
fn test_invalid_syntax() {
    let result = parse_model_selection("1-2-3", 10);
    assert!(result.is_err());
}
