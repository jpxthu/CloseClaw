# LLM Module

> Abstract trait for multiple LLM providers.

## Components

### LLMProvider Trait
```rust
#[async_trait]
pub trait LLMProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LLMError>;
    fn models(&self) -> Vec<&str>;
}
```

### ChatRequest / ChatResponse
```rust
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub temperature: f32,
    pub max_tokens: Option<u32>,
}

pub struct ChatResponse {
    pub content: String,
    pub model: String,
    pub usage: Usage,
}
```

### LLMRegistry
```rust
pub struct LLMRegistry {
    providers: RwLock<HashMap<String, Arc<dyn LLMProvider>>>,
}
```

## Implemented Providers
- `OpenAIProvider` - GPT-4, GPT-3.5
- `AnthropicProvider` - Claude 3
- `MiniMaxProvider` - MiniMax M2 series

## Usage
```rust
let registry = LLMRegistry::new();
registry.register("openai".to_string(), Arc::new(OpenAIProvider::new(api_key)));
let provider = registry.get("openai").await;
```
