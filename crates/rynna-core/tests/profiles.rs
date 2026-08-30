use std::sync::Arc;

use async_trait::async_trait;
use rynna_core::{
    Agent, AgentProfiles, Completion, CompletionRequest, Message, ModelProvider, Profile,
    ProviderError,
};

struct FixedProvider(&'static str);

#[async_trait]
impl ModelProvider for FixedProvider {
    async fn complete(&self, _request: CompletionRequest) -> Result<Completion, ProviderError> {
        Ok(Completion::new(Message::assistant(self.0)))
    }
}

fn profile(name: &str, reply: &'static str) -> (Profile, Agent) {
    (
        Profile {
            name: name.to_owned(),
            provider: "ollama".to_owned(),
            model: format!("{name}-model"),
            active_skills: Vec::new(),
            mcp_servers: Vec::new(),
            capabilities: Vec::new(),
        },
        Agent::new(Arc::new(FixedProvider(reply)), "Profile policy"),
    )
}

#[tokio::test]
async fn profile_catalog_dispatches_to_the_default_and_requested_profiles() {
    let profiles = AgentProfiles::new(
        "local",
        vec![
            profile("local", "Local reply"),
            profile("work", "Work reply"),
        ],
    )
    .unwrap();

    let default_reply = profiles.respond(None, &[], "Hello").await.unwrap();
    let work_reply = profiles.respond(Some("work"), &[], "Hello").await.unwrap();

    assert_eq!(default_reply, Message::assistant("Local reply"));
    assert_eq!(work_reply, Message::assistant("Work reply"));
}

#[test]
fn profile_catalog_upserts_and_removes_profiles() {
    let mut profiles = AgentProfiles::new("local", vec![profile("local", "Local reply")]).unwrap();
    profiles
        .upsert(
            profile("work", "Work reply").0,
            profile("work", "Work reply").1,
        )
        .unwrap();
    assert_eq!(
        profiles
            .profiles()
            .iter()
            .map(|profile| profile.name.as_str())
            .collect::<Vec<_>>(),
        vec!["local", "work"]
    );

    profiles.remove("work").unwrap();
    assert_eq!(
        profiles
            .profiles()
            .iter()
            .map(|profile| profile.name.as_str())
            .collect::<Vec<_>>(),
        vec!["local"]
    );
    assert!(profiles.remove("local").is_err());
}

#[test]
fn profile_catalog_exposes_sorted_metadata_and_the_default_profile() {
    let profiles = AgentProfiles::new(
        "local",
        vec![
            profile("work", "Work reply"),
            profile("local", "Local reply"),
        ],
    )
    .unwrap();

    assert_eq!(profiles.default_profile(), "local");
    assert_eq!(
        profiles
            .profiles()
            .iter()
            .map(|profile| profile.name.as_str())
            .collect::<Vec<_>>(),
        vec!["local", "work"]
    );
}
