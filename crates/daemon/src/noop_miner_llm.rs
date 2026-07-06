use closeclaw_memory::miner::MiningEntity;
use closeclaw_memory::miner::MiningEvent;
use closeclaw_memory::miner_llm::MinerLlmCaller;
use closeclaw_memory::miner_llm::MinerLlmError;

/// No-op LLM caller for the memory miner.
///
/// Returns empty events so mining is a no-op. Used during daemon startup
/// until a real LLM caller is wired up.
pub(crate) struct NoopMinerLlmCaller;

#[async_trait::async_trait]
impl MinerLlmCaller for NoopMinerLlmCaller {
    async fn extract_events(
        &self,
        _transcript: &str,
        _existing_events: &str,
        _existing_memory: &str,
    ) -> Result<Vec<MiningEvent>, MinerLlmError> {
        Ok(Vec::new())
    }

    async fn assign_entities(
        &self,
        events: &[MiningEvent],
        _entity_catalog: &str,
    ) -> Result<Vec<Vec<MiningEntity>>, MinerLlmError> {
        Ok(events.iter().map(|_| Vec::new()).collect())
    }
}
