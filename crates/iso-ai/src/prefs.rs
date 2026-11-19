//! The user's AI settings: a model and an optional prompt per task, and Economy. Kept as
//! JSON beside the AI cache (`<data>/ai_settings.json`), so they apply to every repo.

use std::collections::HashMap;
use std::path::Path;

use iso_core::ipc::{AiConfig, AiConfigUpdate, AiModel, AiTask, AiTaskConfig};
use serde::{Deserialize, Serialize};

use crate::engine::AiSettings;
use crate::{ModelInfo, prompts};

pub const TASKS: [AiTask; 4] = [
    AiTask::Summaries,
    AiTask::Modules,
    AiTask::Architecture,
    AiTask::Review,
];

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AiPrefs {
    pub models: HashMap<AiTask, String>,
    pub prompts: HashMap<AiTask, String>,
    pub economy: Option<bool>,
}

impl AiPrefs {
    /// The saved settings; none when the file is missing or cannot be read.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(tmp, path)
    }

    /// The settings with these choices applied.
    pub fn apply(&self, mut s: AiSettings) -> AiSettings {
        for (task, m) in &self.models {
            if !m.trim().is_empty() {
                s.set_model(*task, m.trim().to_string());
            }
        }
        s.prompts = self.prompts.clone();
        if let Some(e) = self.economy {
            s.economy = e;
        }
        s
    }

    /// The settings after an update, and what to save. A prompt equal to the default is no
    /// change: the task goes back to the default prompt.
    pub fn update(s: AiSettings, u: &AiConfigUpdate) -> (AiSettings, AiPrefs) {
        let mut p = AiPrefs {
            economy: Some(u.economy),
            ..Default::default()
        };
        for t in &u.tasks {
            let m = t.model.trim();
            if !m.is_empty() {
                p.models.insert(t.task, m.to_string());
            }
            if let Some(text) = &t.prompt
                && !text.trim().is_empty()
                && text.trim() != prompts::default_system(t.task).trim()
            {
                p.prompts.insert(t.task, text.clone());
            }
        }
        (p.apply(s), p)
    }
}

/// What the settings dialog shows: each task's model and prompt, and the models to pick.
pub fn config(s: &AiSettings, models: &[ModelInfo], listed: bool) -> AiConfig {
    let fitted = AiSettings::default().fitted_to(models);
    AiConfig {
        tasks: TASKS
            .iter()
            .map(|&task| {
                let default_prompt = prompts::default_system(task);
                let custom = s.prompts.get(&task);
                AiTaskConfig {
                    task,
                    model: s.model_for(task).to_string(),
                    default_model: fitted.model_for(task).to_string(),
                    prompt: custom.cloned().unwrap_or_else(|| default_prompt.clone()),
                    default_prompt,
                    custom_prompt: custom.is_some(),
                }
            })
            .collect(),
        economy: s.economy,
        models: models
            .iter()
            .filter(|m| m.id != "auto")
            .map(|m| AiModel {
                id: m.id.clone(),
                name: m.name.clone(),
                input_per_m: m.input_per_m,
                output_per_m: m.output_per_m,
            })
            .collect(),
        models_listed: listed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iso_core::ipc::AiTaskUpdate;

    #[test]
    fn update_saves_models_and_only_changed_prompts() {
        let def = prompts::default_system(AiTask::Review);
        let u = AiConfigUpdate {
            economy: false,
            tasks: vec![
                AiTaskUpdate {
                    task: AiTask::Summaries,
                    model: "small".into(),
                    prompt: Some("Task: leaf\nBe brief.".into()),
                },
                AiTaskUpdate {
                    task: AiTask::Review,
                    model: "big".into(),
                    prompt: Some(format!("{def}\n")),
                },
            ],
        };
        let (s, p) = AiPrefs::update(AiSettings::default(), &u);
        assert_eq!(s.leaf_model, "small");
        assert_eq!(s.review_model, "big");
        assert!(!s.economy);
        assert_eq!(s.system(AiTask::Summaries), "Task: leaf\nBe brief.");
        assert_eq!(s.system(AiTask::Review), def);
        assert_eq!(p.prompts.len(), 1);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ai_settings.json");
        p.save(&path).unwrap();
        let back = AiPrefs::load(&path);
        assert_eq!(back, p);
        let s2 = back.apply(AiSettings::default());
        assert_eq!(s2.leaf_model, "small");
        assert!(
            AiPrefs::load(&dir.path().join("none.json"))
                .models
                .is_empty()
        );

        let c = config(&s2, &crate::default_models(), false);
        let sum = &c.tasks[0];
        assert!(sum.custom_prompt);
        assert_eq!(sum.model, "small");
        assert_eq!(sum.default_model, crate::pricing::DEFAULT_LEAF_MODEL);
        assert!(!c.tasks[3].custom_prompt);
    }
}
