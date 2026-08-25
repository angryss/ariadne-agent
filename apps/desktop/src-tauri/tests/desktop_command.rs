use std::sync::{Arc, Mutex};

use ariadne_core::{
    Agent, AgentProfiles, Completion, CompletionRequest, Message, ModelProvider, Profile,
    ProviderError,
};
use ariadne_desktop::{RespondRequest, list_profiles, respond_with_agent, respond_with_profiles};
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
            profile: None,
            prompt: "Continue".to_owned(),
            history: vec![Message::user("Start")],
        },
    )
    .await
    .unwrap();

    assert_eq!(response.message, Message::assistant("Desktop reply"));
    assert_eq!(provider.requests.lock().unwrap().len(), 1);
}

fn profile(name: &str, reply: &'static str) -> (Profile, Agent) {
    struct FixedProvider(&'static str);

    #[async_trait]
    impl ModelProvider for FixedProvider {
        async fn complete(&self, _request: CompletionRequest) -> Result<Completion, ProviderError> {
            Ok(Completion::new(Message::assistant(self.0)))
        }
    }

    (
        Profile {
            name: name.to_owned(),
            provider: format!("{name}-provider"),
            model: format!("{name}-model"),
            active_skills: vec![format!("{name}-skill")],
            mcp_servers: vec![format!("{name}-mcp")],
        },
        Agent::new(Arc::new(FixedProvider(reply)), "Desktop policy"),
    )
}

#[tokio::test]
async fn desktop_profile_commands_list_and_dispatch_profiles() {
    let profiles = AgentProfiles::new(
        "local",
        vec![
            profile("local", "Local reply"),
            profile("work", "Work reply"),
        ],
    )
    .unwrap();

    let catalog = list_profiles(&profiles);
    let response = respond_with_profiles(
        &profiles,
        RespondRequest {
            profile: Some("work".to_owned()),
            prompt: "Continue".to_owned(),
            history: Vec::new(),
        },
    )
    .await
    .unwrap();

    assert_eq!(catalog.default_profile, "local");
    assert_eq!(catalog.profiles[1].name, "work");
    assert_eq!(response.message, Message::assistant("Work reply"));
}
