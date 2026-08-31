use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rynna_core::{
    Completion, CompletionDelta, CompletionRequest, ContextPlan, ContextSize, FallbackProvider,
    Message, ModelProvider, ProviderError,
};

struct RecordingProvider {
    name: &'static str,
    result: Result<&'static str, &'static str>,
    calls: Arc<Mutex<Vec<&'static str>>>,
}

#[async_trait]
impl ModelProvider for RecordingProvider {
    async fn complete(&self, _request: CompletionRequest) -> Result<Completion, ProviderError> {
        self.calls.lock().unwrap().push(self.name);
        match self.result {
            Ok(reply) => Ok(Completion::new(Message::assistant(reply))),
            Err(message) => Err(ProviderError::new(message)),
        }
    }
}

struct StreamingProvider {
    delta: &'static str,
    result: Result<&'static str, &'static str>,
}

#[async_trait]
impl ModelProvider for StreamingProvider {
    async fn complete(&self, _request: CompletionRequest) -> Result<Completion, ProviderError> {
        match self.result {
            Ok(reply) => Ok(Completion::new(Message::assistant(reply))),
            Err(message) => Err(ProviderError::new(message)),
        }
    }

    async fn complete_stream(
        &self,
        _request: CompletionRequest,
        on_delta: &mut (dyn for<'delta> FnMut(&'delta CompletionDelta) + Send),
    ) -> Result<Completion, ProviderError> {
        on_delta(&CompletionDelta::Content(self.delta.to_owned()));
        match self.result {
            Ok(reply) => Ok(Completion::new(Message::assistant(reply))),
            Err(message) => Err(ProviderError::new(message)),
        }
    }
}

struct ManagedOnlyProvider;

#[async_trait]
impl ModelProvider for ManagedOnlyProvider {
    async fn complete(&self, _request: CompletionRequest) -> Result<Completion, ProviderError> {
        Err(ProviderError::new("plain completion path used"))
    }

    async fn complete_managed(&self, _plan: ContextPlan) -> Result<Completion, ProviderError> {
        Ok(Completion::new(Message::assistant("managed reply")))
    }

    async fn complete_stream(
        &self,
        _request: CompletionRequest,
        _on_delta: &mut (dyn for<'delta> FnMut(&'delta CompletionDelta) + Send),
    ) -> Result<Completion, ProviderError> {
        Err(ProviderError::new("plain streaming path used"))
    }

    async fn complete_stream_managed(
        &self,
        _plan: ContextPlan,
        on_delta: &mut (dyn for<'delta> FnMut(&'delta CompletionDelta) + Send),
    ) -> Result<Completion, ProviderError> {
        on_delta(&CompletionDelta::Content("managed stream".to_owned()));
        Ok(Completion::new(Message::assistant("managed stream")))
    }
}

fn request() -> CompletionRequest {
    CompletionRequest {
        messages: vec![Message::user("hello")],
        tools: Vec::new(),
    }
}

fn plan() -> ContextPlan {
    ContextPlan {
        request: request(),
        size: ContextSize {
            current_tokens: 1,
            max_tokens: 10,
        },
        server_compaction_threshold: None,
        compacted: false,
    }
}

#[tokio::test]
async fn fallback_provider_tries_configured_providers_in_order() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider = FallbackProvider::new(vec![
        Arc::new(RecordingProvider {
            name: "primary",
            result: Err("primary unavailable"),
            calls: calls.clone(),
        }),
        Arc::new(RecordingProvider {
            name: "secondary",
            result: Ok("secondary reply"),
            calls: calls.clone(),
        }),
        Arc::new(RecordingProvider {
            name: "unused",
            result: Ok("unused reply"),
            calls: calls.clone(),
        }),
    ])
    .unwrap();

    let completion = provider.complete(request()).await.unwrap();

    assert_eq!(completion.message, Message::assistant("secondary reply"));
    assert_eq!(*calls.lock().unwrap(), vec!["primary", "secondary"]);
}

#[tokio::test]
async fn fallback_provider_does_not_leak_deltas_from_failed_attempts() {
    let provider = FallbackProvider::new(vec![
        Arc::new(StreamingProvider {
            delta: "discarded",
            result: Err("stream failed"),
        }),
        Arc::new(StreamingProvider {
            delta: "kept",
            result: Ok("kept"),
        }),
    ])
    .unwrap();
    let mut deltas = Vec::new();

    let completion = provider
        .complete_stream(request(), &mut |delta| deltas.push(delta.clone()))
        .await
        .unwrap();

    assert_eq!(completion.message, Message::assistant("kept"));
    assert_eq!(deltas, vec![CompletionDelta::Content("kept".to_owned())]);
}

#[tokio::test]
async fn fallback_provider_preserves_managed_completion_paths() {
    let provider = FallbackProvider::new(vec![Arc::new(ManagedOnlyProvider)]).unwrap();
    let mut deltas = Vec::new();

    let completion = provider.complete_managed(plan()).await.unwrap();
    let streamed = provider
        .complete_stream_managed(plan(), &mut |delta| deltas.push(delta.clone()))
        .await
        .unwrap();

    assert_eq!(completion.message, Message::assistant("managed reply"));
    assert_eq!(streamed.message, Message::assistant("managed stream"));
    assert_eq!(
        deltas,
        vec![CompletionDelta::Content("managed stream".to_owned())]
    );
}

#[test]
fn fallback_provider_requires_at_least_one_provider() {
    assert!(FallbackProvider::new(Vec::new()).is_err());
}
