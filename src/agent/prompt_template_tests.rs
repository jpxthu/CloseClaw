use std::str::FromStr;

use super::prompt_template::PromptTemplate;

#[test]
fn test_parse_explore() {
    let template = PromptTemplate::from_str("explore").unwrap();
    assert_eq!(template, PromptTemplate::Explore);
}

#[test]
fn test_parse_validation() {
    let template = PromptTemplate::from_str("validation").unwrap();
    assert_eq!(template, PromptTemplate::Validation);
}

#[test]
fn test_parse_invalid() {
    let result = PromptTemplate::from_str("invalid");
    assert!(result.is_err());
}

#[test]
fn test_explore_prefix_non_empty() {
    let template = PromptTemplate::Explore;
    let prefix = template.prefix();
    assert!(!prefix.is_empty());
}

#[test]
fn test_validation_prefix_non_empty() {
    let template = PromptTemplate::Validation;
    let prefix = template.prefix();
    assert!(!prefix.is_empty());
}

#[test]
fn test_prefixes_differ() {
    let explore_prefix = PromptTemplate::Explore.prefix();
    let validation_prefix = PromptTemplate::Validation.prefix();
    assert_ne!(explore_prefix, validation_prefix);
}
