use super::*;

#[test]
fn test_context_default_state() {
    let ctx = WizardContext::default();
    assert_eq!(ctx.current_state, WizardState::SelectProvider);
    assert!(ctx.selected_provider.is_none());
    assert!(ctx.credential.is_none());
    assert!(ctx.fetched_models.is_empty());
    assert!(ctx.selected_models.is_empty());
}

#[test]
fn test_state_equality() {
    assert_eq!(WizardState::SelectProvider, WizardState::SelectProvider);
    assert_eq!(WizardState::InputCredential, WizardState::InputCredential);
    assert_ne!(WizardState::SelectProvider, WizardState::InputCredential);
}
