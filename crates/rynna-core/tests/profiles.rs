use std::sync::Arc;

use async_trait::async_trait;
use rynna_core::{
    Agent, AgentProfiles, Completion, CompletionRequest, Message, ModelProvider, Profile,
    ProfileProvider, ProviderError,
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
            providers: vec![ProfileProvider {
                provider: "ollama".to_owned(),
                model: format!("{name}-model"),
                enabled: true,
                is_default: true,
            }],
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

#[tokio::test]
async fn model_selection_routes_both_modes_without_mutating_defaults() {
    let (mut metadata, default_agent) = profile("local", "default");
    let second = ProfileProvider {
        provider: "other".into(),
        model: "second".into(),
        enabled: true,
        is_default: false,
    };
    metadata.providers.push(second.clone());
    let agent =
        default_agent.with_model_options(vec![(second, Arc::new(FixedProvider("selected")))]);
    let profiles = AgentProfiles::new("local", vec![(metadata.clone(), agent)]).unwrap();
    let selection = rynna_core::ModelSelection {
        provider: "other".into(),
        model: "second".into(),
        thinking: rynna_core::ThinkingLevel::Default,
    };
    let selected = profiles
        .clone()
        .with_model_selection(None, Some(&selection))
        .unwrap();
    assert_eq!(
        selected
            .respond(
                None,
                &[Message::user("previous"), Message::assistant("answer")],
                "continue"
            )
            .await
            .unwrap()
            .content,
        "selected"
    );
    let mut deltas = Vec::new();
    assert_eq!(
        selected
            .respond_stream(None, &[], "hello", &mut |d| deltas.push(d.clone()))
            .await
            .unwrap()
            .content,
        "selected"
    );
    assert!(!deltas.is_empty());
    assert_eq!(
        profiles.respond(None, &[], "hello").await.unwrap().content,
        "default"
    );
    assert_eq!(profiles.profiles(), vec![metadata]);
    let unknown = rynna_core::ModelSelection {
        model: "unknown".into(),
        ..selection.clone()
    };
    assert!(
        profiles
            .clone()
            .with_model_selection(None, Some(&unknown))
            .is_err()
    );
    let unsupported = rynna_core::ModelSelection {
        thinking: rynna_core::ThinkingLevel::High,
        ..selection
    };
    assert!(
        profiles
            .with_model_selection(None, Some(&unsupported))
            .is_err()
    );
}

#[test]
fn disabled_models_and_unknown_thinking_are_rejected() {
    let (mut metadata, agent) = profile("local", "default");
    metadata.providers.push(ProfileProvider {
        provider: "other".into(),
        model: "disabled".into(),
        enabled: false,
        is_default: false,
    });
    let profiles = AgentProfiles::new("local", vec![(metadata, agent)]).unwrap();
    let selection = rynna_core::ModelSelection {
        provider: "other".into(),
        model: "disabled".into(),
        thinking: rynna_core::ThinkingLevel::Default,
    };
    assert!(
        profiles
            .with_model_selection(None, Some(&selection))
            .is_err()
    );
    assert!(
        serde_json::from_value::<rynna_core::ModelSelection>(
            serde_json::json!({"provider":"other","model":"disabled","thinking":"bogus"})
        )
        .is_err()
    );
}
