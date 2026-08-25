use ariadne_config::{ProfileCatalog, ProviderKind};

#[test]
fn parses_profile_provider_model_skills_and_mcp_servers() {
    let catalog = ProfileCatalog::from_toml(
        r#"
version = 1
default_profile = "work"

[providers.ollama]
kind = "openai-compatible"
api_base = "http://127.0.0.1:11434/v1"
api_key_env = "OLLAMA_API_KEY"

[profiles.work]
provider = "ollama"
model = "qwen3:14b"
system_prompt = "You are Ariadne at work."
active_skills = ["rust", "github"]
mcp_servers = ["filesystem"]

[mcp_servers.filesystem]
transport = "stdio"
command = "mcp-filesystem"
args = ["/workspace"]
"#,
    )
    .unwrap();

    let profile = catalog.resolve("work").unwrap();

    assert_eq!(catalog.default_profile(), "work");
    assert_eq!(profile.profile.name, "work");
    assert_eq!(profile.profile.provider, "ollama");
    assert_eq!(profile.profile.model, "qwen3:14b");
    assert_eq!(profile.profile.active_skills, ["rust", "github"]);
    assert_eq!(profile.profile.mcp_servers, ["filesystem"]);
    assert_eq!(profile.provider_kind, ProviderKind::OpenAiCompatible);
    assert_eq!(profile.api_base, "http://127.0.0.1:11434/v1");
    assert_eq!(profile.api_key_env.as_deref(), Some("OLLAMA_API_KEY"));
    assert_eq!(profile.system_prompt, "You are Ariadne at work.");
    assert_eq!(catalog.resolve_all().unwrap(), vec![profile]);
    assert_eq!(
        catalog.mcp_server("filesystem").unwrap()["command"].as_str(),
        Some("mcp-filesystem")
    );
}

#[test]
fn built_in_catalog_preserves_the_existing_local_ollama_defaults() {
    let catalog = ProfileCatalog::built_in();
    let profile = catalog.resolve(catalog.default_profile()).unwrap();

    assert_eq!(catalog.default_profile(), "default");
    assert_eq!(profile.profile.provider, "ollama");
    assert_eq!(profile.profile.model, "qwen3:8b");
    assert_eq!(profile.api_base, "http://127.0.0.1:11434/v1");
    assert!(profile.profile.active_skills.is_empty());
    assert!(profile.profile.mcp_servers.is_empty());
}

#[test]
fn loads_a_profile_catalog_from_an_explicit_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    std::fs::write(
        &path,
        r#"
version = 1
default_profile = "local"

[providers.local]
kind = "openai-compatible"
api_base = "http://localhost:1234/v1"

[profiles.local]
provider = "local"
model = "test-model"
"#,
    )
    .unwrap();

    let catalog = ProfileCatalog::load(&path).unwrap();

    assert_eq!(
        catalog.resolve("local").unwrap().profile.model,
        "test-model"
    );
}

#[test]
fn default_configuration_path_uses_the_platform_configuration_directory() {
    let path = ProfileCatalog::default_path().unwrap();

    assert!(path.ends_with("ariadne/config.toml"));
}

#[test]
fn rejects_profiles_that_reference_unknown_providers_or_mcp_servers() {
    let unknown_provider = ProfileCatalog::from_toml(
        r#"
version = 1
default_profile = "broken"
providers = {}

[profiles.broken]
provider = "missing"
model = "model"
"#,
    )
    .unwrap_err();
    assert!(
        unknown_provider
            .to_string()
            .contains("references unknown provider `missing`")
    );

    let unknown_mcp = ProfileCatalog::from_toml(
        r#"
version = 1
default_profile = "broken"

[providers.local]
kind = "openai-compatible"
api_base = "http://127.0.0.1:11434/v1"

[profiles.broken]
provider = "local"
model = "model"
mcp_servers = ["missing"]
"#,
    )
    .unwrap_err();
    assert!(
        unknown_mcp
            .to_string()
            .contains("references unknown MCP server `missing`")
    );
}

#[test]
fn rejects_provider_urls_with_embedded_credentials() {
    let error = ProfileCatalog::from_toml(
        r#"
version = 1
default_profile = "unsafe"

[providers.remote]
kind = "openai-compatible"
api_base = "http://user:password@remote.example/v1"

[profiles.unsafe]
provider = "remote"
model = "model"
"#,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("provider `remote` base URL must not contain embedded credentials")
    );
}
