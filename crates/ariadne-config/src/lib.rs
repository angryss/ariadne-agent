use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use ariadne_core::Profile;
use serde::Deserialize;
use thiserror::Error;
use url::Url;

pub const DEFAULT_API_BASE: &str = "http://127.0.0.1:11434/v1";
pub const DEFAULT_MODEL: &str = "qwen3:8b";
pub const DEFAULT_PROFILE: &str = "default";
pub const DEFAULT_PROVIDER: &str = "ollama";
pub const DEFAULT_SYSTEM_PROMPT: &str = "You are Ariadne, a careful and capable AI software agent.";
const CONFIG_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub enum ProviderKind {
    #[serde(rename = "openai-compatible")]
    OpenAiCompatible,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedProfile {
    pub profile: Profile,
    pub provider_kind: ProviderKind,
    pub api_base: String,
    pub api_key_env: Option<String>,
    pub system_prompt: String,
    pub capabilities: Vec<ResolvedCapability>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolvedCapability {
    FileSystem(FileSystemCapability),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileSystemCapability {
    pub root: PathBuf,
    pub read_only: bool,
    pub allowed_patterns: Vec<String>,
    pub denied_patterns: Option<Vec<String>>,
    pub protected_patterns: Option<Vec<String>>,
    pub max_read_bytes: Option<usize>,
    pub max_results: Option<usize>,
    pub max_traversal_files: Option<usize>,
    pub max_traversal_depth: Option<usize>,
    pub max_search_bytes: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct ProfileCatalog {
    default_profile: String,
    providers: BTreeMap<String, ProviderConfig>,
    profiles: BTreeMap<String, ProfileConfig>,
    mcp_servers: BTreeMap<String, toml::Table>,
    capabilities: BTreeMap<String, CapabilityConfig>,
}

impl ProfileCatalog {
    pub fn built_in() -> Self {
        Self {
            default_profile: DEFAULT_PROFILE.to_owned(),
            providers: BTreeMap::from([(
                DEFAULT_PROVIDER.to_owned(),
                ProviderConfig {
                    kind: ProviderKind::OpenAiCompatible,
                    api_base: DEFAULT_API_BASE.to_owned(),
                    api_key_env: None,
                },
            )]),
            profiles: BTreeMap::from([(
                DEFAULT_PROFILE.to_owned(),
                ProfileConfig {
                    provider: DEFAULT_PROVIDER.to_owned(),
                    model: DEFAULT_MODEL.to_owned(),
                    system_prompt: Some(DEFAULT_SYSTEM_PROMPT.to_owned()),
                    active_skills: Vec::new(),
                    mcp_servers: Vec::new(),
                    capabilities: Vec::new(),
                },
            )]),
            mcp_servers: BTreeMap::new(),
            capabilities: BTreeMap::new(),
        }
    }

    pub fn from_toml(source: &str) -> Result<Self, ConfigError> {
        let file: ConfigFile = toml::from_str(source)?;
        Self::from_file(file)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let source = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_owned(),
            source,
        })?;
        Self::from_toml(&source)
    }

    pub fn default_path() -> Result<PathBuf, ConfigError> {
        dirs::config_dir()
            .map(|path| path.join("ariadne").join("config.toml"))
            .ok_or(ConfigError::ConfigDirectoryUnavailable)
    }

    pub fn load_default() -> Result<Self, ConfigError> {
        let path = Self::default_path()?;
        Self::load_default_from(&path)
    }

    fn load_default_from(path: &Path) -> Result<Self, ConfigError> {
        match path.try_exists() {
            Ok(true) => Self::load(path),
            Ok(false) => Ok(Self::built_in()),
            Err(source) => Err(ConfigError::Inspect {
                path: path.to_owned(),
                source,
            }),
        }
    }

    pub fn default_profile(&self) -> &str {
        &self.default_profile
    }

    pub fn resolve_all(&self) -> Result<Vec<ResolvedProfile>, ConfigError> {
        self.profiles
            .keys()
            .map(|name| self.resolve(name))
            .collect()
    }

    pub fn mcp_server(&self, name: &str) -> Option<&toml::Table> {
        self.mcp_servers.get(name)
    }

    pub fn resolve(&self, name: &str) -> Result<ResolvedProfile, ConfigError> {
        let profile = self
            .profiles
            .get(name)
            .ok_or_else(|| ConfigError::UnknownProfile(name.to_owned()))?;
        let provider =
            self.providers
                .get(&profile.provider)
                .ok_or_else(|| ConfigError::UnknownProvider {
                    profile: name.to_owned(),
                    provider: profile.provider.clone(),
                })?;

        Ok(ResolvedProfile {
            profile: Profile {
                name: name.to_owned(),
                provider: profile.provider.clone(),
                model: profile.model.clone(),
                active_skills: profile.active_skills.clone(),
                mcp_servers: profile.mcp_servers.clone(),
                capabilities: profile.capabilities.clone(),
            },
            provider_kind: provider.kind,
            api_base: provider.api_base.clone(),
            api_key_env: provider.api_key_env.clone(),
            system_prompt: profile
                .system_prompt
                .clone()
                .unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_owned()),
            capabilities: profile
                .capabilities
                .iter()
                .map(|name| match &self.capabilities[name] {
                    CapabilityConfig::FileSystem(config) => {
                        ResolvedCapability::FileSystem(config.clone().into())
                    }
                })
                .collect(),
        })
    }

    fn from_file(file: ConfigFile) -> Result<Self, ConfigError> {
        if file.version != CONFIG_VERSION {
            return Err(ConfigError::UnsupportedVersion(file.version));
        }
        if !file.profiles.contains_key(&file.default_profile) {
            return Err(ConfigError::UnknownDefault(file.default_profile));
        }

        for (name, provider) in &file.providers {
            ensure_not_blank("provider name", name)?;
            ensure_not_blank("provider API base URL", &provider.api_base)?;
            let api_base = Url::parse(&provider.api_base).map_err(|source| {
                ConfigError::InvalidProviderUrl {
                    provider: name.clone(),
                    source,
                }
            })?;
            if !api_base.username().is_empty() || api_base.password().is_some() {
                return Err(ConfigError::EmbeddedProviderCredentials {
                    provider: name.clone(),
                });
            }
            if let Some(api_key_env) = &provider.api_key_env {
                ensure_not_blank("provider API key environment variable", api_key_env)?;
            }
        }

        for (name, profile) in &file.profiles {
            ensure_not_blank("profile name", name)?;
            ensure_not_blank("profile provider", &profile.provider)?;
            ensure_not_blank("profile model", &profile.model)?;
            if !file.providers.contains_key(&profile.provider) {
                return Err(ConfigError::UnknownProvider {
                    profile: name.clone(),
                    provider: profile.provider.clone(),
                });
            }
            ensure_unique("active skill", &profile.active_skills)?;
            ensure_unique("MCP server", &profile.mcp_servers)?;
            ensure_unique("capability", &profile.capabilities)?;
            for server in &profile.mcp_servers {
                if !file.mcp_servers.contains_key(server) {
                    return Err(ConfigError::UnknownMcpServer {
                        profile: name.clone(),
                        server: server.clone(),
                    });
                }
            }
            for capability in &profile.capabilities {
                if !file.capabilities.contains_key(capability) {
                    return Err(ConfigError::UnknownCapability {
                        profile: name.clone(),
                        capability: capability.clone(),
                    });
                }
            }
            let filesystem_capabilities = profile
                .capabilities
                .iter()
                .filter(|capability| {
                    matches!(
                        file.capabilities.get(*capability),
                        Some(CapabilityConfig::FileSystem(_))
                    )
                })
                .count();
            if filesystem_capabilities > 1 {
                return Err(ConfigError::ConflictingFileSystemCapabilities {
                    profile: name.clone(),
                });
            }
        }

        Ok(Self {
            default_profile: file.default_profile,
            providers: file.providers,
            profiles: file.profiles,
            mcp_servers: file.mcp_servers,
            capabilities: file.capabilities,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    version: u32,
    default_profile: String,
    providers: BTreeMap<String, ProviderConfig>,
    profiles: BTreeMap<String, ProfileConfig>,
    #[serde(default)]
    mcp_servers: BTreeMap<String, toml::Table>,
    #[serde(default)]
    capabilities: BTreeMap<String, CapabilityConfig>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderConfig {
    kind: ProviderKind,
    api_base: String,
    api_key_env: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileConfig {
    provider: String,
    model: String,
    system_prompt: Option<String>,
    #[serde(default)]
    active_skills: Vec<String>,
    #[serde(default)]
    mcp_servers: Vec<String>,
    #[serde(default)]
    capabilities: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum CapabilityConfig {
    #[serde(rename = "filesystem")]
    FileSystem(FileSystemCapabilityConfig),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileSystemCapabilityConfig {
    root: PathBuf,
    #[serde(default)]
    read_only: bool,
    #[serde(default)]
    allowed_patterns: Vec<String>,
    denied_patterns: Option<Vec<String>>,
    protected_patterns: Option<Vec<String>>,
    max_read_bytes: Option<usize>,
    max_results: Option<usize>,
    max_traversal_files: Option<usize>,
    max_traversal_depth: Option<usize>,
    max_search_bytes: Option<usize>,
}

impl From<FileSystemCapabilityConfig> for FileSystemCapability {
    fn from(config: FileSystemCapabilityConfig) -> Self {
        Self {
            root: config.root,
            read_only: config.read_only,
            allowed_patterns: config.allowed_patterns,
            denied_patterns: config.denied_patterns,
            protected_patterns: config.protected_patterns,
            max_read_bytes: config.max_read_bytes,
            max_results: config.max_results,
            max_traversal_files: config.max_traversal_files,
            max_traversal_depth: config.max_traversal_depth,
            max_search_bytes: config.max_search_bytes,
        }
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Ariadne configuration is not valid TOML: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("Ariadne configuration version {0} is not supported")]
    UnsupportedVersion(u32),
    #[error("default profile `{0}` is not defined")]
    UnknownDefault(String),
    #[error("profile `{0}` is not defined")]
    UnknownProfile(String),
    #[error("profile `{profile}` references unknown provider `{provider}`")]
    UnknownProvider { profile: String, provider: String },
    #[error("provider `{provider}` base URL is invalid: {source}")]
    InvalidProviderUrl {
        provider: String,
        source: url::ParseError,
    },
    #[error("provider `{provider}` base URL must not contain embedded credentials")]
    EmbeddedProviderCredentials { provider: String },
    #[error("profile `{profile}` references unknown MCP server `{server}`")]
    UnknownMcpServer { profile: String, server: String },
    #[error("profile `{profile}` references unknown capability `{capability}`")]
    UnknownCapability { profile: String, capability: String },
    #[error(
        "profile `{profile}` activates multiple filesystem capabilities, whose tool names would conflict"
    )]
    ConflictingFileSystemCapabilities { profile: String },
    #[error("{0} must not be blank")]
    BlankValue(&'static str),
    #[error("{kind} `{name}` is listed more than once")]
    DuplicateReference { kind: &'static str, name: String },
    #[error("failed to read Ariadne configuration at {}: {source}", path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to inspect Ariadne configuration path {}: {source}", path.display())]
    Inspect {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("the platform configuration directory is unavailable")]
    ConfigDirectoryUnavailable,
}

fn ensure_not_blank(kind: &'static str, value: &str) -> Result<(), ConfigError> {
    if value.trim().is_empty() {
        return Err(ConfigError::BlankValue(kind));
    }
    Ok(())
}

fn ensure_unique(kind: &'static str, values: &[String]) -> Result<(), ConfigError> {
    let mut seen = BTreeSet::new();
    for value in values {
        ensure_not_blank(kind, value)?;
        if !seen.insert(value) {
            return Err(ConfigError::DuplicateReference {
                kind,
                name: value.clone(),
            });
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::symlink;

    use super::ProfileCatalog;

    #[test]
    fn default_loading_propagates_filesystem_lookup_errors() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        symlink("config.toml", &path).unwrap();

        assert!(ProfileCatalog::load_default_from(&path).is_err());
    }
}
