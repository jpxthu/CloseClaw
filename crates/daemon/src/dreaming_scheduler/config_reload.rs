//! Config-reload handlers for the DreamingScheduler.
//!
//! Houses the Memory-section reload handlers shared by the scheduling
//! loops: [`DreamingScheduler::handle_config_change`] (apply the new
//! Memory config to the dreaming pipeline, memory miner and cron
//! schedule) and [`DreamingScheduler::handle_cron_config_reload`] (the
//! cron loop's variant, which additionally re-parses the schedule and
//! recomputes the next fire time).

use closeclaw_config::providers::MemoryConfigData;
use closeclaw_config::ConfigSection;
use tokio::time::Instant;
use tracing::{error, info};

use super::cron::{cron_next_fire, parse_cron_schedule};
use super::DreamingScheduler;

impl DreamingScheduler {
    /// Handle a config change for the Memory section.
    pub(super) async fn handle_config_change(&mut self) {
        match self.config_manager.section(ConfigSection::Memory) {
            Some(value) => {
                let content = match serde_json::to_string(&value) {
                    Ok(s) => s,
                    Err(e) => {
                        error!(%e, "failed to serialize memory config");
                        return;
                    }
                };
                let memory_config = match MemoryConfigData::from_json_str(&content) {
                    Ok(c) => c,
                    Err(e) => {
                        error!(%e, "failed to parse memory config");
                        return;
                    }
                };
                self.dreaming_pipeline.update_config(
                    crate::memory_params_adapter::dreaming_params_from_config(
                        &memory_config.config.dreaming,
                    ),
                );
                self.memory_miner.update_config(
                    crate::memory_params_adapter::miner_config_from_memory(&memory_config.config),
                );
                self.schedule = memory_config.config.dreaming.schedule.clone();
                info!("dreaming config reloaded via config manager");
            }
            None => {
                error!("Memory config section not available");
            }
        }
    }

    /// Handle a config reload within the cron loop: apply new config,
    /// re-parse the schedule, and recompute next fire time.
    pub(crate) async fn handle_cron_config_reload(&mut self, next_fire: &mut Instant) {
        self.handle_config_change().await;
        let new_sched = self.schedule.as_deref().and_then(parse_cron_schedule);
        if let Some(ref new_sched) = new_sched {
            *next_fire = cron_next_fire(new_sched);
            info!(
                schedule = %new_sched.display(),
                "schedule updated after config reload"
            );
        } else {
            let interval_secs = self.config.dreaming_interval_secs();
            info!(
                "schedule became invalid after config \
                 reload, falling back to fixed \
                 interval {}s",
                interval_secs
            );
            *next_fire = self.fallback_next_fire();
        }
    }
}
