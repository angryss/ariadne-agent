use rynna_config::{
    AnthropicAuthentication, ConfiguredProvider, OpenAiAuthentication, ProfileCatalog,
    ProviderKind, ProviderSettingsStore, ResolvedCapability, secure_private_directory,
};
use rynna_core::Profile;

#[test]
fn parses_anthropic_api_and_subscription_profiles() {
    let catalog = ProfileCatalog::from_toml(
        r#"
version = 1
default_profile = "api"
[providers.anthropic_api]
kind = "anthropic-messages"
api_base = "https://api.anthropic.com"
api_key_env = "ANTHROPIC_API_KEY"
[providers.claude_subscription]
kind = "claude-subscription"
claude_program = "/usr/local/bin/claude"
[profiles.api]
provider = "anthropic_api"
model = "claude-sonnet-4-5"
[profiles.subscription]
provider = "claude_subscription"
model = "sonnet"
"#,
    )
    .unwrap();

    let api = catalog.resolve("api").unwrap();
    assert_eq!(api.provider_kind, ProviderKind::AnthropicMessages);
    assert_eq!(api.api_key_env.as_deref(), Some("ANTHROPIC_API_KEY"));
    let subscription = catalog.resolve("subscription").unwrap();
    assert_eq!(subscription.provider_kind, ProviderKind::ClaudeSubscription);
    assert_eq!(
        subscription.claude_program.to_string_lossy(),
        "/usr/local/bin/claude"
    );
}

#[test]
fn anthropic_provider_settings_never_serialize_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("providers.toml");
    let mut store = ProviderSettingsStore::load(&path).unwrap();
    store
        .add(ConfiguredProvider::Anthropic {
            authentication: AnthropicAuthentication::Subscription,
        })
        .unwrap();
    let encoded = std::fs::read_to_string(path).unwrap();
    assert!(encoded.contains("kind = \"anthropic\""));
    assert!(!encoded.contains("token"));
    assert!(!encoded.contains("api_key"));
}

#[test]
fn claude_subscription_profiles_reject_rynna_context() {
    let error = ProfileCatalog::from_toml(
        r#"
version = 1
default_profile = "subscription"

[providers.claude_subscription]
kind = "claude-subscription"

[profiles.subscription]
provider = "claude_subscription"
model = "sonnet"
active_skills = ["rust"]
"#,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("cannot declare skills, MCP servers, or capabilities")
    );
}

#[test]
fn claude_subscription_provider_rejects_api_credentials_and_endpoint() {
    let error = ProfileCatalog::from_toml(
        r#"
version = 1
default_profile = "claude"

[providers.claude]
kind = "claude-subscription"
api_base = "https://api.anthropic.com"
api_key_env = "ANTHROPIC_API_KEY"

[profiles.claude]
provider = "claude"
model = "sonnet"
"#,
    )
    .expect_err("subscription and direct API configuration must remain separate");

    assert!(
        error
            .to_string()
            .contains("cannot declare an API base URL or API key"),
        "unexpected error: {error}"
    );
}

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
system_prompt = "You are Rynna at work."
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
    let ResolvedCapability::FileSystem(filesystem) = &profile.capabilities[0] else {
        panic!("expected filesystem capability");
    };
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
    assert_eq!(profile.system_prompt, "You are Rynna at work.");
    assert_eq!(catalog.resolve_all().unwrap(), vec![profile]);
    assert_eq!(
        catalog.mcp_server("filesystem").unwrap()["command"].as_str(),
        Some("mcp-filesystem")
    );
}

#[test]
fn parses_a_bounded_command_capability() {
    let catalog = ProfileCatalog::from_toml(
        r#"
version = 1
default_profile = "local"

[providers.local]
kind = "openai-compatible"
api_base = "http://127.0.0.1:11434/v1"

[profiles.local]
provider = "local"
model = "test-model"
capabilities = ["host-commands"]

[capabilities.host-commands]
kind = "command"
working_directory = "."
programs = { uname = "/usr/bin/uname", sw_vers = "/usr/bin/sw_vers" }
timeout_seconds = 5
max_output_bytes = 8192
"#,
    )
    .unwrap();

    let profile = catalog.resolve("local").unwrap();
    let ResolvedCapability::Command(command) = &profile.capabilities[0] else {
        panic!("expected command capability");
    };
    assert_eq!(command.working_directory.to_string_lossy(), ".");
    assert_eq!(
        command.programs["uname"].to_string_lossy(),
        "/usr/bin/uname"
    );
    assert_eq!(
        command.programs["sw_vers"].to_string_lossy(),
        "/usr/bin/sw_vers"
    );
    assert_eq!(command.timeout_seconds, 5);
    assert_eq!(command.max_output_bytes, 8192);
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

    assert!(path.ends_with("rynna/config.toml"));
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
fn command_capabilities_are_unique_per_profile() {
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
capabilities = ["commands-one", "commands-two"]

[capabilities.commands-one]
kind = "command"
working_directory = "."
programs = { uname = "/usr/bin/uname" }
timeout_seconds = 5
max_output_bytes = 8192

[capabilities.commands-two]
kind = "command"
working_directory = "."
programs = { sw_vers = "/usr/bin/sw_vers" }
timeout_seconds = 5
max_output_bytes = 8192
"#,
    )
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "profile `broken` activates multiple command capabilities, whose tool names would conflict"
    );
}

#[test]
fn rejects_structurally_invalid_command_capabilities() {
    let invalid = [
        (
            "programs = {}\ntimeout_seconds = 5\nmax_output_bytes = 8192",
            "at least one program",
        ),
        (
            "programs = { uname = \"uname\" }\ntimeout_seconds = 5\nmax_output_bytes = 8192",
            "absolute path",
        ),
        (
            "programs = { uname = \"/usr/bin/uname\" }\ntimeout_seconds = 0\nmax_output_bytes = 8192",
            "greater than zero",
        ),
    ];

    for (command_config, expected) in invalid {
        let error = ProfileCatalog::from_toml(&format!(
            r#"
version = 1
default_profile = "broken"

[providers.local]
kind = "openai-compatible"
api_base = "http://127.0.0.1:11434/v1"

[profiles.broken]
provider = "local"
model = "model"
capabilities = ["commands"]

[capabilities.commands]
kind = "command"
working_directory = "."
{command_config}
"#
        ))
        .unwrap_err();

        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn rejects_command_limits_above_the_safe_hard_maximum() {
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
capabilities = ["commands"]

[capabilities.commands]
kind = "command"
working_directory = "."
programs = { uname = "/usr/bin/uname" }
timeout_seconds = 301
max_output_bytes = 8388609
"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("safe maximum"));
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

#[test]
fn provider_settings_start_blank_and_support_add_update_and_delete() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("providers.toml");
    let mut store = ProviderSettingsStore::load(&path).unwrap();

    assert!(store.list().is_empty());
    assert!(!path.exists());

    store
        .add(ConfiguredProvider::Ollama {
            api_base: "http://127.0.0.1:11434/v1".to_owned(),
        })
        .unwrap();
    store
        .add(ConfiguredProvider::OpenAi {
            authentication: OpenAiAuthentication::Chatgpt,
            reuse_existing: true,
        })
        .unwrap();

    assert_eq!(
        ProviderSettingsStore::load(&path).unwrap().list(),
        vec![
            ConfiguredProvider::Ollama {
                api_base: "http://127.0.0.1:11434/v1".to_owned(),
            },
            ConfiguredProvider::OpenAi {
                authentication: OpenAiAuthentication::Chatgpt,
                reuse_existing: true,
            },
        ]
    );

    store
        .update(ConfiguredProvider::OpenAi {
            authentication: OpenAiAuthentication::ApiKey,
            reuse_existing: false,
        })
        .unwrap();
    assert_eq!(
        store.get("openai"),
        Some(&ConfiguredProvider::OpenAi {
            authentication: OpenAiAuthentication::ApiKey,
            reuse_existing: false,
        })
    );

    store.delete("ollama").unwrap();
    assert_eq!(
        ProviderSettingsStore::load(path).unwrap().list(),
        vec![ConfiguredProvider::OpenAi {
            authentication: OpenAiAuthentication::ApiKey,
            reuse_existing: false,
        }]
    );
}

#[test]
fn provider_settings_reject_duplicates_unknown_updates_and_invalid_ollama_urls() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("providers.toml");
    let mut store = ProviderSettingsStore::load(path).unwrap();
    store
        .add(ConfiguredProvider::Ollama {
            api_base: "http://localhost:11434/v1".to_owned(),
        })
        .unwrap();

    assert!(
        store
            .add(ConfiguredProvider::Ollama {
                api_base: "http://localhost:11434/v1".to_owned(),
            })
            .unwrap_err()
            .to_string()
            .contains("already configured")
    );
    assert!(
        store
            .update(ConfiguredProvider::OpenAi {
                authentication: OpenAiAuthentication::Chatgpt,
                reuse_existing: false,
            })
            .unwrap_err()
            .to_string()
            .contains("is not configured")
    );
    assert!(
        ProviderSettingsStore::load(directory.path().join("invalid.toml"))
            .unwrap()
            .add(ConfiguredProvider::Ollama {
                api_base: "not a URL".to_owned(),
            })
            .unwrap_err()
            .to_string()
            .contains("URL is invalid")
    );
}

#[test]
fn provider_settings_do_not_change_in_memory_when_persistence_fails() {
    let directory = tempfile::tempdir().unwrap();
    let settings_path = directory.path().join("providers.toml");
    let mut store = ProviderSettingsStore::load(&settings_path).unwrap();
    std::fs::create_dir(&settings_path).unwrap();

    assert!(
        store
            .add(ConfiguredProvider::Ollama {
                api_base: "http://localhost:11434/v1".to_owned(),
            })
            .is_err()
    );
    assert!(store.list().is_empty());

    std::fs::remove_dir(&settings_path).unwrap();
    store
        .add(ConfiguredProvider::Ollama {
            api_base: "http://localhost:11434/v1".to_owned(),
        })
        .expect("a failed replacement must not poison future writes");
    assert_eq!(store.list().len(), 1);
}

#[cfg(unix)]
#[test]
fn provider_settings_ignore_stale_temporary_file_symlinks() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let settings_path = directory.path().join("providers.toml");
    let temporary_path = settings_path.with_extension("toml.tmp");
    let victim_path = directory.path().join("victim.txt");
    std::fs::write(&victim_path, "keep me").unwrap();
    symlink(&victim_path, temporary_path).unwrap();
    let mut store = ProviderSettingsStore::load(&settings_path).unwrap();

    store
        .add(ConfiguredProvider::Ollama {
            api_base: "http://localhost:11434/v1".to_owned(),
        })
        .unwrap();
    assert_eq!(std::fs::read_to_string(victim_path).unwrap(), "keep me");
    assert_eq!(store.list().len(), 1);
}

#[cfg(unix)]
#[test]
fn secure_private_directory_rejects_a_symlink_ancestor() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let real = directory.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let link = directory.path().join("link");
    symlink(&real, &link).unwrap();

    let error = secure_private_directory(link.join("codex")).unwrap_err();

    assert!(error.to_string().contains("symbolic link"));
    assert!(!real.join("codex").exists());
}

#[cfg(unix)]
#[test]
fn secure_private_directory_rejects_parent_components_and_root() {
    assert!(secure_private_directory("/").is_err());
    assert!(secure_private_directory("/tmp/rynna/../codex").is_err());
}

#[test]
fn provider_settings_merge_mutations_from_separate_store_instances() {
    let directory = tempfile::tempdir().unwrap();
    let settings_path = directory.path().join("providers.toml");
    let mut first = ProviderSettingsStore::load(&settings_path).unwrap();
    let mut second = ProviderSettingsStore::load(&settings_path).unwrap();

    first
        .add(ConfiguredProvider::Ollama {
            api_base: "http://localhost:11434/v1".to_owned(),
        })
        .unwrap();
    second
        .add(ConfiguredProvider::OpenAi {
            authentication: OpenAiAuthentication::Chatgpt,
            reuse_existing: false,
        })
        .unwrap();

    let reloaded = ProviderSettingsStore::load(settings_path).unwrap();
    assert_eq!(reloaded.list().len(), 2);
}

#[test]
fn provider_settings_support_a_bare_relative_filename() {
    let file_name = format!(".rynna-provider-settings-test-{}.toml", std::process::id());
    let lock_name = format!(
        ".rynna-provider-settings-test-{}.toml.lock",
        std::process::id()
    );
    let _ = std::fs::remove_file(&file_name);
    let _ = std::fs::remove_file(&lock_name);
    let mut store = ProviderSettingsStore::load(&file_name).unwrap();

    store
        .add(ConfiguredProvider::Ollama {
            api_base: "http://localhost:11434/v1".to_owned(),
        })
        .unwrap();

    assert_eq!(
        ProviderSettingsStore::load(&file_name)
            .unwrap()
            .list()
            .len(),
        1
    );
    std::fs::remove_file(file_name).unwrap();
    std::fs::remove_file(lock_name).unwrap();
}

fn editable_profile(name: &str, model: &str) -> Profile {
    Profile {
        name: name.to_owned(),
        provider: "ollama".to_owned(),
        model: model.to_owned(),
        active_skills: Vec::new(),
        mcp_servers: Vec::new(),
        capabilities: Vec::new(),
    }
}

#[test]
fn adds_updates_and_deletes_profiles_and_persists_them() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    std::fs::write(
        &path,
        r#"
version = 1
default_profile = "alpha"
[providers.ollama]
kind = "openai-compatible"
api_base = "http://127.0.0.1:11434/v1"
[profiles.alpha]
provider = "ollama"
model = "qwen3:8b"
"#,
    )
    .unwrap();

    let mut catalog = ProfileCatalog::load(&path).unwrap();
    catalog
        .add_profile(editable_profile("zeta", "gpt-5"))
        .unwrap();
    catalog
        .update_profile("zeta", editable_profile("work", "gpt-5.2"))
        .unwrap();
    catalog.delete_profile("work").unwrap();

    let reloaded = ProfileCatalog::load(&path).unwrap();
    let names: Vec<_> = reloaded
        .resolve_all()
        .unwrap()
        .into_iter()
        .map(|profile| profile.profile.name)
        .collect();
    assert_eq!(names, vec!["alpha".to_owned()]);
    assert_eq!(reloaded.resolve("alpha").unwrap().profile.model, "qwen3:8b");
}

#[test]
fn rejects_deleting_the_last_profile() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    std::fs::write(
        &path,
        r#"
version = 1
default_profile = "alpha"
[providers.ollama]
kind = "openai-compatible"
api_base = "http://127.0.0.1:11434/v1"
[profiles.alpha]
provider = "ollama"
model = "qwen3:8b"
"#,
    )
    .unwrap();

    let mut catalog = ProfileCatalog::load(&path).unwrap();
    let error = catalog.delete_profile("alpha").unwrap_err();
    assert!(error.to_string().contains("last profile"));
}
