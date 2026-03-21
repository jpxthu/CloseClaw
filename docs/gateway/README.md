# Gateway Module

> Message routing, session management, and IM protocol adapters.

## Components

### Gateway
Central hub connecting IM platforms to agents.

```rust
pub struct Gateway {
    config: GatewayConfig,
    adapters: RwLock<HashMap<String, Arc<dyn IMAdapter>>>,
    sessions: RwLock<HashMap<String, Session>>,
}
```

### IMAdapter Trait
```rust
#[async_trait]
pub trait IMAdapter: Send + Sync {
    fn name(&self) -> &str;
    async fn handle_webhook(&self, payload: &[u8]) -> Result<Message, AdapterError>;
    async fn send_message(&self, message: &Message) -> Result<(), AdapterError>;
    async fn validate_signature(&self, signature: &str, payload: &[u8]) -> bool;
}
```

### Message
Internal message representation.
```rust
pub struct Message {
    pub id: String,
    pub from: String,
    pub to: String,
    pub content: String,
    pub channel: String,
    pub timestamp: i64,
    pub metadata: HashMap<String, String>,
}
```

### Implemented Adapters
- `FeishuAdapter` - Feishu/Lark webhook handling
