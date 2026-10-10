//! Smart cleanup and transforms on a local LLM: Qwen3.5 GGUF served by the
//! bundled llama.cpp `llama-server`, spoken to over 127.0.0.1 only.

use super::EngineState;
use std::time::Duration;

pub trait LlmEngine: Send + Sync {
    /// One chat completion. `budget` is a hard deadline for the whole call,
    /// server start-up included.
    fn complete(&self, system: &str, user: &str, budget: Duration) -> Result<String, LlmError>;
    fn status(&self) -> EngineState;
}

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("Smart model not downloaded")]
    Missing,
    #[error("Smart cleanup timed out")]
    Timeout,
    #[error("Smart engine error: {0}")]
    Engine(String),
}
