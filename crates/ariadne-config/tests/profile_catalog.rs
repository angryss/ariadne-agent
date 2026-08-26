use ariadne_config::{ProfileCatalog, ProviderKind, ResolvedCapability};

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
capabilities = ["workspace"]

[capabilities.workspace]
kind = "filesystem"
root = "."
read_only = true
denied_patterns = ["private/**"]
max_traversal_files = 200
max_traversal_depth = 12
max_search_bytes = 4096

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
    assert_eq!(profile.profile.capabilities, ["workspace"]);
    let ResolvedCapability::FileSystem(filesystem) = &profile.capabilities[0];
    assert_eq!(filesystem.root.to_string_lossy(), ".");
    assert!(filesystem.read_only);
    assert_eq!(
        filesystem.denied_patterns.as_deref(),
        Some(&["private/**".to_owned()][..])
    );
    assert_eq!(filesystem.max_traversal_files, Some(200));
    assert_eq!(filesystem.max_traversal_depth, Some(12));
    assert_eq!(filesystem.max_search_bytes, Some(4096));
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
fn rejects_profiles_that_reference_unknown_capabilities() {
    let error = ProfileCatalog::from_toml(
        r#"
version = 1
default_profile = "broken"

[providers.local]
kind = "openai-compatible"
api_base = "http://127.0.0.1:11434/v1"

[profiles.broken]
provider = "local"
model = "model"
capabilities = ["missing"]
"#,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("references unknown capability `missing`")
    );
}

#[test]
fn filesystem_capabilities_are_unique_per_profile_not_globally() {
    let valid = ProfileCatalog::from_toml(
        r#"
version = 1
default_profile = "one"

[providers.local]
kind = "openai-compatible"
api_base = "http://127.0.0.1:11434/v1"

[profiles.one]
provider = "local"
model = "model"
capabilities = ["workspace-one"]

[profiles.two]
provider = "local"
model = "model"
capabilities = ["workspace-two"]

[capabilities.workspace-one]
kind = "filesystem"
root = "."

[capabilities.workspace-two]
kind = "filesystem"
root = ".."
"#,
    )
    .unwrap();
    assert_eq!(valid.resolve_all().unwrap().len(), 2);

    let error = ProfileCatalog::from_toml(
        r#"
version = 1
default_profile = "broken"

[providers.local]
kind = "openai-compatible"
api_base = "http://127.0.0.1:11434/v1"

[profiles.broken]
provider = "local"
model = "model"
capabilities = ["workspace-one", "workspace-two"]

[capabilities.workspace-one]
kind = "filesystem"
root = "."

[capabilities.workspace-two]
kind = "filesystem"
root = ".."
"#,
    )
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "profile `broken` activates multiple filesystem capabilities, whose tool names would conflict"
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
