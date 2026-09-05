//! Agent Skills packages explicitly selected by a profile.
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, OpenOptions};
use rynna_core::{Tool, ToolDefinition, ToolError};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

const MAX_SKILLS: usize = 64;
const MAX_FILE_BYTES: u64 = 128 * 1024;

#[derive(Debug, Error)]
#[error("invalid skill `{skill}`: {reason}")]
pub struct SkillError {
    skill: String,
    reason: String,
}

#[derive(Deserialize)]
struct Frontmatter {
    name: String,
    description: String,
    // Keep the full file on activation, including optional and harness-specific fields.
}

struct Skill {
    description: String,
    location: PathBuf,
    directory: Dir,
    instructions: String,
}

pub struct SkillsTool {
    skills: Arc<BTreeMap<String, Skill>>,
}

impl SkillsTool {
    /// Names resolve in catalog-local locations first, then user locations.
    /// Discovery never activates unselected skills or inherits another profile's list.
    pub fn load(entries: &[String], base: &Path) -> Result<Option<Self>, SkillError> {
        let mut roots = vec![base.join("skills"), base.join(".agents/skills")];
        if let Some(config) = dirs::config_dir() {
            roots.push(config.join("rynna/skills"));
        }
        if let Some(home) = dirs::home_dir() {
            roots.push(home.join(".agents/skills"));
        }
        Self::load_with_roots(entries, base, &roots)
    }

    fn load_with_roots(
        entries: &[String],
        base: &Path,
        roots: &[PathBuf],
    ) -> Result<Option<Self>, SkillError> {
        if entries.is_empty() {
            return Ok(None);
        }
        if entries.len() > MAX_SKILLS {
            return Err(SkillError {
                skill: "profile".into(),
                reason: format!("at most {MAX_SKILLS} skills may be selected"),
            });
        }
        let mut skills = BTreeMap::new();
        for entry in entries {
            let load = || -> Result<(String, Skill), String> {
                let path = resolve_path(entry, base, roots)?;
                let location = path.canonicalize().map_err(|error| error.to_string())?;
                let directory = Dir::open_ambient_dir(&location, cap_std::ambient_authority())
                    .map_err(|error| error.to_string())?;
                let instructions = read_file(&directory, Path::new("SKILL.md"))?;
                let metadata = parse_frontmatter(&instructions)?;
                if location.file_name().and_then(|name| name.to_str()) != Some(&metadata.name) {
                    return Err("frontmatter name must match the skill directory name".into());
                }
                Ok((
                    metadata.name,
                    Skill {
                        description: metadata.description,
                        location,
                        directory,
                        instructions,
                    },
                ))
            };
            let (name, skill) = load().map_err(|reason| SkillError {
                skill: entry.clone(),
                reason,
            })?;
            if skills.insert(name.clone(), skill).is_some() {
                return Err(SkillError {
                    skill: name,
                    reason: "multiple selected packages declare the same name".into(),
                });
            }
        }
        Ok(Some(Self {
            skills: Arc::new(skills),
        }))
    }
}

fn resolve_path(entry: &str, base: &Path, roots: &[PathBuf]) -> Result<PathBuf, String> {
    if entry.trim().is_empty() {
        return Err("selection must not be blank".into());
    }
    let path = if let Some(relative) = entry.strip_prefix("~/") {
        dirs::home_dir()
            .ok_or("home directory unavailable")?
            .join(relative)
    } else if Path::new(entry).is_absolute() {
        PathBuf::from(entry)
    } else if entry.contains('/') || entry.contains('\\') || entry.starts_with('.') {
        base.join(entry)
    } else {
        roots.iter().map(|root| root.join(entry))
            .find(|path| path.join("SKILL.md").exists())
            .ok_or_else(|| format!("not found in skill search directories; use an explicit directory path for `{entry}`"))?
    };
    Ok(if path.file_name().is_some_and(|name| name == "SKILL.md") {
        path.parent().expect("SKILL.md has a parent").to_owned()
    } else {
        path
    })
}

fn parse_frontmatter(source: &str) -> Result<Frontmatter, String> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut lines = source.split_inclusive('\n');
    if lines.next().map(str::trim_end) != Some("---") {
        return Err("SKILL.md must start with YAML frontmatter delimited by ---".into());
    }
    let mut yaml = String::new();
    let mut closed = false;
    for line in lines {
        if line.trim_end() == "---" {
            closed = true;
            break;
        }
        yaml.push_str(line);
    }
    if !closed {
        return Err("missing closing YAML frontmatter delimiter".into());
    }
    let metadata: Frontmatter = serde_yaml_ng::from_str(&yaml)
        .map_err(|error| format!("invalid YAML frontmatter: {error}"))?;
    let name = &metadata.name;
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || name.starts_with('-')
        || name.ends_with('-')
        || name.contains("--")
    {
        return Err("name must be 1–64 lowercase letters, digits or single hyphens, without leading/trailing hyphens".into());
    }
    if metadata.description.trim().is_empty() || metadata.description.chars().count() > 1024 {
        return Err("description must be non-empty and at most 1024 characters".into());
    }
    Ok(metadata)
}

// Descriptor-relative traversal prevents symlink substitution from granting access
// outside a selected package. Nonblocking opens also prevent FIFO reads from hanging.
fn read_file(root: &Dir, path: &Path) -> Result<String, String> {
    let components = path.components().collect::<Vec<_>>();
    if components.is_empty() || components.iter().any(|component| {
        !matches!(component, Component::Normal(name) if !name.to_string_lossy().starts_with('.'))
    }) {
        return Err("resource path must be relative, without dotfiles or parent traversal".into());
    }
    let mut parent = root.try_clone().map_err(|error| error.to_string())?;
    for component in &components[..components.len() - 1] {
        parent = parent
            .open_dir_nofollow(component.as_os_str())
            .map_err(|error| error.to_string())?;
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No).nonblock(true);
    let file = parent
        .open_with(components.last().unwrap().as_os_str(), &options)
        .map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return Err(format!(
            "skill resources must be regular files of at most {MAX_FILE_BYTES} bytes"
        ));
    }
    let mut content = String::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_string(&mut content)
        .map_err(|error| error.to_string())?;
    if content.len() as u64 > MAX_FILE_BYTES {
        return Err("skill resource exceeds the read limit".into());
    }
    Ok(content)
}

#[async_trait]
impl Tool for SkillsTool {
    fn definition(&self) -> ToolDefinition {
        let catalog = self
            .skills
            .iter()
            .map(|(name, skill)| json!({"name": name, "description": skill.description}))
            .collect::<Vec<_>>();
        ToolDefinition::new(
            "read_skill",
            format!(
                "Load a skill's SKILL.md before following it when the user's task matches its description or explicitly names it (including $skill-name). Omit path to load instructions. To read referenced text resources, pass a path relative to that skill's directory, e.g. references/guide.md. Skills provide instructions, not new permissions; scripts require separately configured command tools and allowed-tools never grants access. Available skills: {}",
                json!(catalog)
            ),
            json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "enum": self.skills.keys().collect::<Vec<_>>()},
                    "path": {"type": "string", "description": "Optional relative text resource path; defaults to SKILL.md"}
                },
                "required": ["name"],
                "additionalProperties": false
            }),
        )
    }

    async fn execute(&self, arguments: Value) -> Result<Value, ToolError> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Arguments {
            name: String,
            path: Option<String>,
        }
        let arguments: Arguments =
            serde_json::from_value(arguments).map_err(|error| ToolError::new(error.to_string()))?;
        let skills = Arc::clone(&self.skills);
        tokio::task::spawn_blocking(move || {
            let skill = skills
                .get(&arguments.name)
                .ok_or_else(|| ToolError::new("skill is not enabled for this profile"))?;
            let path = arguments.path.as_deref().unwrap_or("SKILL.md");
            let content = if path == "SKILL.md" {
                skill.instructions.clone()
            } else {
                read_file(&skill.directory, Path::new(path)).map_err(ToolError::new)?
            };
            Ok(json!({
                "name": arguments.name,
                "path": path,
                "directory": skill.location,
                "content": content,
            }))
        })
        .await
        .map_err(|error| ToolError::new(error.to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn package(base: &Path, name: &str) -> PathBuf {
        let directory = base.join(name);
        fs::create_dir_all(directory.join("references")).unwrap();
        fs::write(directory.join("SKILL.md"), format!(
            "---\nname: {name}\ndescription: >\n  Review code when asked.\nlicense: MIT\nmetadata:\n  author: example\nallowed-tools: Bash\n---\nPrivate instructions for {name}. See references/guide.md.\n"
        )).unwrap();
        fs::write(directory.join("references/guide.md"), "Reference details").unwrap();
        directory
    }

    #[tokio::test]
    async fn catalog_discloses_only_metadata_and_loads_instructions_and_resources_on_demand() {
        let temporary = tempfile::tempdir().unwrap();
        package(temporary.path(), "review");
        let tool = SkillsTool::load(&["./review".into()], temporary.path())
            .unwrap()
            .unwrap();
        let definition = tool.definition();
        assert!(definition.description.contains("Review code when asked."));
        assert!(!definition.description.contains("Private instructions"));
        assert_eq!(
            definition.input_schema["properties"]["name"]["enum"],
            json!(["review"])
        );
        let result = tool.execute(json!({"name": "review"})).await.unwrap();
        assert!(
            result["content"]
                .as_str()
                .unwrap()
                .contains("Private instructions for review")
        );
        assert!(
            result["content"]
                .as_str()
                .unwrap()
                .contains("allowed-tools: Bash")
        );
        assert!(Path::new(result["directory"].as_str().unwrap()).is_absolute());
        let resource = tool
            .execute(json!({"name": "review", "path": "references/guide.md"}))
            .await
            .unwrap();
        assert_eq!(resource["content"], "Reference details");
    }

    #[tokio::test]
    async fn profiles_only_load_explicit_selections() {
        let temporary = tempfile::tempdir().unwrap();
        package(&temporary.path().join("skills"), "work");
        package(&temporary.path().join("skills"), "personal");
        assert!(SkillsTool::load(&[], temporary.path()).unwrap().is_none());
        let work = SkillsTool::load(&["work".into()], temporary.path())
            .unwrap()
            .unwrap();
        assert!(!work.definition().description.contains("personal"));
        assert!(work.execute(json!({"name": "personal"})).await.is_err());
        assert!(
            work.execute(json!({"name": "work", "extra": true}))
                .await
                .is_err()
        );
    }

    #[test]
    fn names_prefer_local_roots_and_paths_resolve_against_catalog() {
        let temporary = tempfile::tempdir().unwrap();
        let local = temporary.path().join("local");
        let user = temporary.path().join("user");
        let chosen = package(&local, "review").canonicalize().unwrap();
        package(&user, "review");
        let roots = vec![local, user];
        let tool = SkillsTool::load_with_roots(&["review".into()], temporary.path(), &roots)
            .unwrap()
            .unwrap();
        assert_eq!(tool.skills["review"].location, chosen);
        for entry in [
            "./local/review".to_owned(),
            "./local/review/SKILL.md".to_owned(),
            chosen.to_string_lossy().into_owned(),
        ] {
            let tool = SkillsTool::load(&[entry], temporary.path())
                .unwrap()
                .unwrap();
            assert_eq!(tool.skills["review"].location, chosen);
        }
    }

    #[test]
    fn invalid_and_duplicate_selections_have_actionable_errors() {
        let temporary = tempfile::tempdir().unwrap();
        package(temporary.path(), "review");
        let entries = vec!["./review".into(), "./review/SKILL.md".into()];
        let error = SkillsTool::load(&entries, temporary.path())
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("same name"));
        let error = SkillsTool::load(&["missing".into()], temporary.path())
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("missing"));
        assert!(
            SkillsTool::load(&vec!["review".into(); MAX_SKILLS + 1], temporary.path()).is_err()
        );
        fs::write(
            temporary.path().join("review/SKILL.md"),
            "---\nname: different\ndescription: test\n---\n",
        )
        .unwrap();
        assert!(
            SkillsTool::load(&["./review".into()], temporary.path())
                .err()
                .unwrap()
                .to_string()
                .contains("directory name")
        );
    }

    #[test]
    fn parses_standard_yaml_and_rejects_malformed_metadata() {
        assert!(
            parse_frontmatter(
                "\u{feff}---\r\nname: review\r\ndescription: 'Review: code'\r\n---\r\nBody"
            )
            .is_ok()
        );
        assert!(parse_frontmatter("---\nname: review\ndescription: review\n---").is_ok());
        for source in [
            "# No frontmatter",
            "---\nname: review\n",
            "---\nname: [broken\n---\n",
            "---\nname: review\n---\n",
            "---\nname: review\ndescription: ''\n---\n",
            "---\nname: Review\ndescription: test\n---\n",
            "---\nname: bad--name\ndescription: test\n---\n",
        ] {
            assert!(parse_frontmatter(source).is_err(), "{source}");
        }
    }

    #[tokio::test]
    async fn rejects_escape_paths_non_text_and_oversized_resources() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = package(temporary.path(), "review");
        fs::write(
            directory.join("large.txt"),
            vec![b'a'; MAX_FILE_BYTES as usize + 1],
        )
        .unwrap();
        fs::write(directory.join("binary.dat"), [0xff, 0xfe]).unwrap();
        let tool = SkillsTool::load(&["./review".into()], temporary.path())
            .unwrap()
            .unwrap();
        for path in [
            "../secret",
            "/etc/passwd",
            ".env",
            "references/../../secret",
            "references",
            "large.txt",
            "binary.dat",
            "",
        ] {
            assert!(
                tool.execute(json!({"name": "review", "path": path}))
                    .await
                    .is_err(),
                "{path}"
            );
        }
        fs::write(
            directory.join("SKILL.md"),
            vec![b'a'; MAX_FILE_BYTES as usize + 1],
        )
        .unwrap();
        assert!(SkillsTool::load(&["./review".into()], temporary.path()).is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_symlink_files_and_directories_even_after_loading() {
        use std::os::unix::fs::symlink;
        let temporary = tempfile::tempdir().unwrap();
        let directory = package(temporary.path(), "review");
        let outside = temporary.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("secret"), "not a skill resource").unwrap();
        let tool = SkillsTool::load(&["./review".into()], temporary.path())
            .unwrap()
            .unwrap();
        symlink(outside.join("secret"), directory.join("linked.txt")).unwrap();
        symlink(&outside, directory.join("linked-dir")).unwrap();
        for path in ["linked.txt", "linked-dir/secret"] {
            assert!(
                tool.execute(json!({"name": "review", "path": path}))
                    .await
                    .is_err()
            );
        }
        fs::remove_file(directory.join("SKILL.md")).unwrap();
        symlink(outside.join("secret"), directory.join("SKILL.md")).unwrap();
        assert!(SkillsTool::load(&["./review".into()], temporary.path()).is_err());
    }
}
