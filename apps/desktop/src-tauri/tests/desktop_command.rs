use std::sync::{Arc, Mutex};

use ariadne_core::{Agent, Completion, CompletionRequest, Message, ModelProvider, ProviderError};
use ariadne_desktop::{RespondRequest, respond_with_agent};
use async_trait::async_trait;

#[derive(Default)]
struct RecordingProvider {
    requests: Mutex<Vec<CompletionRequest>>,
}

#[async_trait]
impl ModelProvider for RecordingProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<Completion, ProviderError> {
        self.requests.lock().unwrap().push(request);
        Ok(Completion::new(Message::assistant("Desktop reply")))
    }
}

#[tokio::test]
async fn desktop_command_delegates_to_the_shared_agent_core() {
    let provider = Arc::new(RecordingProvider::default());
    let agent = Agent::new(
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
        "Desktop policy",
    );

    let response = respond_with_agent(
        &agent,
        RespondRequest {
            prompt: "Continue".to_owned(),
            history: vec![Message::user("Start")],
        },
    )
    .await
    .unwrap();

    assert_eq!(response.message, Message::assistant("Desktop reply"));
    assert_eq!(provider.requests.lock().unwrap().len(), 1);
}
