//! Plan confirmation notification callback builder.
//!
//! Builds the owner notification callback for plan execution confirmations.
//! Mirrors the pattern in [`set_approval_flow`](closeclaw_gateway::Gateway::set_approval_flow):
//! iterates registered plugins and sends confirmation cards to the owner.

use std::collections::HashMap;
use std::sync::Arc;

use closeclaw_gateway::Gateway;

/// Build the owner notification callback for plan confirmation cards.
///
/// Returns a closure that iterates registered plugins and sends
/// confirmation cards to the owner. Called during daemon lifecycle to
/// install on [`PlanExecConfirmFlow`](closeclaw_tools::builtin::PlanExecConfirmFlow)
/// before passing the flow to the gateway as `Arc<dyn PlanConfirmationHandler>`.
pub(crate) async fn build_confirm_notify_callback(
    gateway: &Arc<Gateway>,
) -> Arc<dyn Fn(closeclaw_tools::builtin::PlanExecNotification) + Send + Sync> {
    let handle = tokio::runtime::Handle::current();
    let plugins = gateway.get_all_plugins().await;
    let plugin_clones: HashMap<String, Arc<dyn closeclaw_common::IMPlugin>> = plugins
        .into_iter()
        .map(|p| (p.platform().to_string(), p))
        .collect();

    Arc::new(
        move |notification: closeclaw_tools::builtin::PlanExecNotification| {
            let id = notification.confirmation_id;
            let plan_name = std::path::Path::new(&notification.plan_file_path)
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| notification.plan_file_path.clone());
            let new_session_hint = if notification.new_session {
                "（新 session 执行）"
            } else {
                ""
            };

            let text = format!(
                "📋 Plan 执行确认 [{}]\n\
             是否开始执行 plan {}{}？\n\
             回复 /confirm {} 开始执行，或 /cancel {} 取消。\n\
             此确认为执行启动确认，与危险操作审批无关。",
                id, plan_name, new_session_hint, id, id,
            );

            let plugins = plugin_clones.clone();
            let handle = handle.clone();
            handle.spawn(async move {
                for plugin in plugins.values() {
                    let output = closeclaw_common::im_plugin::RenderedOutput {
                        msg_type: "text".into(),
                        payload: serde_json::json!({"content": {"text": text}}),
                    };
                    match plugin.send(&output, "owner", None, None).await {
                        Ok(()) => break,
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "failed to send plan confirm notification"
                            );
                        }
                    }
                }
            });
        },
    )
}
