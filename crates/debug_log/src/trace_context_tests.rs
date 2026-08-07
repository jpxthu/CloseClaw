use super::*;

#[test]
fn test_new_root_has_empty_parent_span_id() {
    let ctx = TraceContext::new_root("trace-123".into());
    assert!(ctx.parent_span_id.is_empty());
}

#[test]
fn test_new_root_has_trace_id() {
    let ctx = TraceContext::new_root("trace-456".into());
    assert_eq!(ctx.trace_id, "trace-456");
}

#[test]
fn test_new_root_has_span_id() {
    let ctx = TraceContext::new_root("trace-789".into());
    assert!(!ctx.span_id.is_empty());
}

#[test]
fn test_child_inherits_trace_id() {
    let parent = TraceContext::new_root("trace-abc".into());
    let child = parent.child();
    assert_eq!(child.trace_id, parent.trace_id);
}

#[test]
fn test_child_parent_span_id_points_to_parent() {
    let parent = TraceContext::new_root("trace-xyz".into());
    let child = parent.child();
    assert_eq!(child.parent_span_id, parent.span_id);
}

#[test]
fn test_child_has_own_span_id() {
    let parent = TraceContext::new_root("trace-123".into());
    let child = parent.child();
    assert_ne!(child.span_id, parent.span_id);
}

#[test]
fn test_child_empty_parent_span_id_not_empty() {
    let parent = TraceContext::new_root("trace-123".into());
    let child = parent.child();
    assert!(!child.parent_span_id.is_empty());
}

#[test]
fn test_nested_child_chain() {
    let root = TraceContext::new_root("trace-chain".into());
    let child1 = root.child();
    let child2 = child1.child();

    // All share the same trace_id
    assert_eq!(root.trace_id, child1.trace_id);
    assert_eq!(child1.trace_id, child2.trace_id);

    // parent chain is correct
    assert_eq!(child1.parent_span_id, root.span_id);
    assert_eq!(child2.parent_span_id, child1.span_id);

    // All span_ids are unique
    assert_ne!(root.span_id, child1.span_id);
    assert_ne!(child1.span_id, child2.span_id);
    assert_ne!(root.span_id, child2.span_id);
}

#[test]
fn test_span_id_uniqueness() {
    let ctx1 = TraceContext::new_root("trace-1".into());
    let ctx2 = TraceContext::new_root("trace-2".into());
    assert_ne!(ctx1.span_id, ctx2.span_id);
}

#[test]
fn test_child_span_ids_unique() {
    let parent = TraceContext::new_root("trace-1".into());
    let child1 = parent.child();
    let child2 = parent.child();
    assert_ne!(child1.span_id, child2.span_id);
}

#[test]
fn test_clone_preserves_values() {
    let ctx = TraceContext::new_root("trace-clone".into());
    let cloned = ctx.clone();
    assert_eq!(ctx.trace_id, cloned.trace_id);
    assert_eq!(ctx.span_id, cloned.span_id);
    assert_eq!(ctx.parent_span_id, cloned.parent_span_id);
}
