use async_trait::async_trait;
use rynna_core::{
    Agent, Completion, CompletionRequest, MemoryConversation, MemoryError, MemoryProvider, Message,
    ModelProvider, ProviderError, flush_memory_writes,
};
use std::sync::{Arc, Mutex};

struct Model;
#[async_trait]
impl ModelProvider for Model {
    async fn complete(&self, _: CompletionRequest) -> Result<Completion, ProviderError> {
        Ok(Completion::new(Message::assistant("answer")))
    }
}
struct Memory {
    saved: Mutex<Vec<String>>,
    hang: bool,
}
#[async_trait]
impl MemoryProvider for Memory {
    async fn recall(&self, _: &str) -> Result<Vec<String>, MemoryError> {
        Ok(vec![])
    }
    async fn retain(&self, conversation: &MemoryConversation) -> Result<(), MemoryError> {
        if self.hang {
            std::future::pending::<()>().await;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        self.saved
            .lock()
            .unwrap()
            .push(conversation.messages[0].content.clone());
        Ok(())
    }
}

// Separate test binary: shutdown drains the process-wide registry.
#[tokio::test(start_paused = true)]
async fn shutdown_drains_dropped_provider_snapshots_and_has_a_deadline() {
    let memory = Arc::new(Memory {
        saved: Default::default(),
        hang: false,
    });
    let agent = Agent::new(Arc::new(Model), "policy").with_memory_provider(Some(memory.clone()));
    for prompt in ["one", "two"] {
        agent.respond(&[], prompt).await.unwrap();
    }
    drop(agent);
    flush_memory_writes().await;
    assert_eq!(*memory.saved.lock().unwrap(), ["one", "two"]);
    let hanging = Arc::new(Memory {
        saved: Default::default(),
        hang: true,
    });
    Agent::new(Arc::new(Model), "policy")
        .with_memory_provider(Some(hanging.clone()))
        .respond(&[], "pending")
        .await
        .unwrap();
    let start = tokio::time::Instant::now();
    flush_memory_writes().await;
    assert!(start.elapsed() <= std::time::Duration::from_secs(10));
    assert!(hanging.saved.lock().unwrap().is_empty());
}
