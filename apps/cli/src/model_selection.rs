use rynna_core::{AgentProfiles, ModelSelection, ThinkingLevel};

pub fn is_command(input: &str) -> bool {
    matches!(
        input.split_whitespace().next(),
        Some("/model" | "/provider" | "/thinking")
    )
}

/// Return feedback without sending selector commands to the model or persisting defaults.
pub fn apply(
    profiles: &AgentProfiles,
    profile: &str,
    selection: &mut Option<ModelSelection>,
    input: &str,
) -> Result<String, String> {
    let metadata = profiles
        .profiles()
        .into_iter()
        .find(|p| p.name == profile)
        .ok_or("unknown profile")?;
    let pairs: Vec<_> = metadata.providers.iter().filter(|p| p.enabled).collect();
    let words: Vec<_> = input.split_whitespace().collect();
    let mut next = selection.clone();
    match words.as_slice() {
        ["/model"] | ["/provider"] => {
            let options = pairs
                .iter()
                .enumerate()
                .map(|(i, p)| format!("{}: {} / {}", i + 1, p.provider, p.model))
                .collect::<Vec<_>>()
                .join("\n");
            return Ok(format!(
                "{options}\n/model <number> or /model <provider> <model>\n/provider <provider> · /thinking default|low|medium|high\n/model default restores profile defaults"
            ));
        }
        ["/model", "default"] => next = None,
        ["/model", index] => {
            let pair = index
                .parse::<usize>()
                .ok()
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| pairs.get(i))
                .ok_or("use /model to list numbered provider/model choices")?;
            next = Some(ModelSelection {
                provider: pair.provider.clone(),
                model: pair.model.clone(),
                thinking: ThinkingLevel::Default,
            });
        }
        ["/model", provider, model] => {
            next = Some(ModelSelection {
                provider: (*provider).to_owned(),
                model: (*model).to_owned(),
                thinking: ThinkingLevel::Default,
            })
        }
        ["/provider", provider] => {
            let pair = pairs
                .iter()
                .find(|p| p.provider == *provider)
                .ok_or("provider is not enabled in this profile")?;
            next = Some(ModelSelection {
                provider: pair.provider.clone(),
                model: pair.model.clone(),
                thinking: ThinkingLevel::Default,
            });
        }
        ["/thinking"] => {
            return Ok(format!(
                "Thinking: {}. Use /thinking default|low|medium|high",
                selection
                    .as_ref()
                    .map_or("default", |s| s.thinking.as_str())
            ));
        }
        ["/thinking", level] => {
            let level = match *level {
                "default" => ThinkingLevel::Default,
                "low" => ThinkingLevel::Low,
                "medium" => ThinkingLevel::Medium,
                "high" => ThinkingLevel::High,
                _ => return Err("thinking level must be default, low, medium, or high".to_owned()),
            };
            if next.is_none() {
                let pair = pairs
                    .iter()
                    .find(|p| p.is_default)
                    .or_else(|| pairs.first())
                    .ok_or("no enabled models")?;
                next = Some(ModelSelection {
                    provider: pair.provider.clone(),
                    model: pair.model.clone(),
                    thinking: level,
                });
            } else if let Some(selected) = &mut next {
                selected.thinking = level;
            }
        }
        _ => return Err("use /model, /provider <provider>, or /thinking <level>".to_owned()),
    }
    profiles
        .clone()
        .with_model_selection(Some(profile), next.as_ref())
        .map_err(|e| e.to_string())?;
    let feedback = summary(next.as_ref());
    *selection = next;
    Ok(feedback)
}

pub fn summary(selection: Option<&ModelSelection>) -> String {
    selection.map_or_else(
        || "Profile default".to_owned(),
        |s| {
            format!(
                "{} / {} · thinking {}",
                s.provider,
                s.model,
                s.thinking.as_str()
            )
        },
    )
}
