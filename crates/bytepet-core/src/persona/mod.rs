//! Personas: system prompt, style traits, sampling, model binding, memory and
//! TTS settings. A persona is the unit of "customisation" the user edits.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::pet::manifest::validate_pet_id;

pub const DEFAULT_PERSONA_ID: &str = "default";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct PersonaTraits {
    /// Free-form tone description, e.g. "毒舌但温柔".
    pub tone: String,
    /// `short` | `normal` | `detailed`.
    pub verbosity: String,
    /// BCP-47-ish language tag, e.g. `zh-CN`.
    pub language: String,
    pub emoji: bool,
}

impl Default for PersonaTraits {
    fn default() -> Self {
        Self {
            tone: "friendly and concise".to_string(),
            verbosity: "normal".to_string(),
            language: "zh-CN".to_string(),
            emoji: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct SamplingConfig {
    pub temperature: f32,
    pub max_tokens: u32,
}

impl Default for SamplingConfig {
    fn default() -> Self {
        Self {
            temperature: 0.8,
            max_tokens: 800,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelRef {
    pub provider: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct PersonaMemoryConfig {
    pub enabled: bool,
    /// How many recent turns are sent verbatim.
    pub window_turns: u32,
    pub long_term: bool,
    /// Generate a summary once this many turns are unsummarized.
    pub summarize_after_turns: u32,
}

impl Default for PersonaMemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            window_turns: 12,
            long_term: true,
            summarize_after_turns: 20,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct PersonaTtsConfig {
    pub enabled: bool,
    pub voice: Option<String>,
    pub rate: f32,
}

impl Default for PersonaTtsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            voice: None,
            rate: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ProactiveConfig {
    pub enabled: bool,
    pub idle_minutes: u32,
}

impl Default for ProactiveConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            idle_minutes: 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Persona {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// Pet skin bound to this persona (pet id in the library).
    pub avatar_pet: Option<String>,
    pub system_prompt: String,
    pub greeting: Option<String>,
    pub traits: PersonaTraits,
    pub sampling: SamplingConfig,
    pub model: Option<ModelRef>,
    pub memory: PersonaMemoryConfig,
    pub tts: PersonaTtsConfig,
    pub proactive: ProactiveConfig,
    /// Built-in personas can be edited but not deleted.
    pub builtin: bool,
}

impl Default for Persona {
    fn default() -> Self {
        Self {
            id: DEFAULT_PERSONA_ID.to_string(),
            name: "小助手".to_string(),
            description: Some("默认人格：友好、简洁、乐于帮忙。".to_string()),
            avatar_pet: None,
            system_prompt: "你是一只住在用户桌面上的宠物伙伴。你友好、好奇、说话简洁。\
                你可以陪用户聊天、帮忙梳理思路，但不要编造事实。\
                回答时优先使用用户使用的语言。"
                .to_string(),
            greeting: Some("我在这儿呢，需要我陪你聊聊吗？".to_string()),
            traits: PersonaTraits::default(),
            sampling: SamplingConfig::default(),
            model: None,
            memory: PersonaMemoryConfig::default(),
            tts: PersonaTtsConfig::default(),
            proactive: ProactiveConfig::default(),
            builtin: true,
        }
    }
}

impl Persona {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            builtin: false,
            ..Self::default()
        }
    }

    /// The system prompt actually sent to the model.
    pub fn effective_system_prompt(&self, pet_name: Option<&str>, state: Option<&str>) -> String {
        let mut parts = Vec::new();
        parts.push(self.system_prompt.trim().to_string());

        let mut style = Vec::new();
        if !self.traits.tone.trim().is_empty() {
            style.push(format!("语气：{}", self.traits.tone.trim()));
        }
        if !self.traits.language.trim().is_empty() {
            style.push(format!("默认语言：{}", self.traits.language.trim()));
        }
        match self.traits.verbosity.as_str() {
            "short" => style.push("回答保持简短，通常不超过三句话。".to_string()),
            "detailed" => style.push("回答可以详细一些，必要时分点说明。".to_string()),
            _ => style.push("回答长度适中，避免冗长。".to_string()),
        }
        if !self.traits.emoji {
            style.push("不要使用 emoji。".to_string());
        }
        if !style.is_empty() {
            parts.push(style.join("\n"));
        }

        if let Some(pet) = pet_name {
            parts.push(format!(
                "你现在的桌宠形象是「{pet}」{}",
                state
                    .map(|s| format!("，当前状态：{s}"))
                    .unwrap_or_default()
            ));
        }
        parts.push(
            "只输出你要对用户说的话，不要描述自己的动作或心理活动，不要使用 Markdown 标题。"
                .to_string(),
        );
        parts.join("\n\n")
    }

    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty() {
            return Err(Error::config("persona id must not be empty"));
        }
        if self.name.trim().is_empty() {
            return Err(Error::config("persona name must not be empty"));
        }
        if self.system_prompt.trim().is_empty() {
            return Err(Error::config("persona system prompt must not be empty"));
        }
        if self.sampling.temperature < 0.0 || self.sampling.temperature > 2.0 {
            return Err(Error::config("temperature must be between 0 and 2"));
        }
        if let Some(pet) = &self.avatar_pet {
            validate_pet_id(pet).map_err(|e| Error::config(format!("invalid avatar pet: {e}")))?;
        }
        Ok(())
    }

    fn file_name(&self) -> String {
        // Ids are already validated to be filesystem safe.
        format!("{}.json", self.id)
    }
}

/// Persona storage on disk (`personas/<id>.json`).
pub struct PersonaStore {
    dir: PathBuf,
}

impl PersonaStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn ensure(&self) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        if self.list()?.is_empty() {
            self.save(&Persona::default())?;
        }
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<Persona>> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Ok(out);
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| serde_json::from_str::<Persona>(&t).ok())
            {
                Some(persona) => out.push(persona),
                None => tracing::warn!(path = %path.display(), "skipping invalid persona file"),
            }
        }
        out.sort_by_key(|p| p.name.to_lowercase());
        Ok(out)
    }

    pub fn get(&self, id: &str) -> Result<Option<Persona>> {
        Ok(self.list()?.into_iter().find(|p| p.id == id))
    }

    pub fn save(&self, persona: &Persona) -> Result<()> {
        persona.validate()?;
        std::fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(persona.file_name());
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(persona)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        if id == DEFAULT_PERSONA_ID {
            return Err(Error::config("the default persona cannot be deleted"));
        }
        let path = self.dir.join(format!("{id}.json"));
        if !path.is_file() {
            return Err(Error::NotFound(format!("persona '{id}'")));
        }
        std::fs::remove_file(path)?;
        Ok(())
    }

    pub fn duplicate(&self, id: &str, new_id: &str, new_name: &str) -> Result<Persona> {
        let mut persona = self
            .get(id)?
            .ok_or_else(|| Error::NotFound(format!("persona '{id}'")))?;
        persona.id = new_id.to_string();
        persona.name = new_name.to_string();
        persona.builtin = false;
        self.save(&persona)?;
        Ok(persona)
    }

    /// Import a persona from a JSON file.
    pub fn import_file(&self, path: &Path, overwrite: bool) -> Result<Persona> {
        let text = std::fs::read_to_string(path)?;
        let mut persona: Persona = serde_json::from_str(&text)
            .map_err(|e| Error::config(format!("cannot parse persona file: {e}")))?;
        persona.validate()?;
        if !overwrite && self.get(&persona.id)?.is_some() {
            return Err(Error::config(format!(
                "persona '{}' already exists",
                persona.id
            )));
        }
        persona.builtin = false;
        self.save(&persona)?;
        Ok(persona)
    }

    pub fn export_file(&self, id: &str, out: &Path) -> Result<()> {
        let persona = self
            .get(id)?
            .ok_or_else(|| Error::NotFound(format!("persona '{id}'")))?;
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(out, serde_json::to_string_pretty(&persona)?)?;
        Ok(())
    }
}

/// Starter personas offered in the UI.
pub fn templates() -> BTreeMap<String, Persona> {
    let mut out = BTreeMap::new();

    let mut genki = Persona::new("genki", "元气助手");
    genki.description = Some("活力满满，鼓励式回应，适合日常陪伴。".into());
    genki.system_prompt = "你是一只元气满满的桌面宠物。你热情、爱鼓励人，喜欢用轻快的语气回应，\
        但在用户需要认真帮助时会立刻切换到靠谱模式。"
        .into();
    genki.traits.tone = "元气、鼓励、轻快".into();
    genki.traits.verbosity = "short".into();
    genki.greeting = Some("今天也要一起加油呀！".into());
    genki.builtin = true;
    out.insert(genki.id.clone(), genki);

    let mut snark = Persona::new("snark", "毒舌吐槽");
    snark.description = Some("嘴上不饶人，实际很关心你。".into());
    snark.system_prompt =
        "你是一只嘴很毒的桌面宠物。你爱吐槽，但吐槽背后是真的关心用户，\
        绝不进行人身攻击，也不会贬低用户的努力。用户认真提问时要给出准确答案。"
            .into();
    snark.traits.tone = "毒舌但温柔".into();
    snark.traits.verbosity = "short".into();
    snark.greeting = Some("又见面了，今天又想偷懒多久？".into());
    snark.builtin = true;
    out.insert(snark.id.clone(), snark);

    let mut advisor = Persona::new("advisor", "沉稳顾问");
    advisor.description = Some("冷静、结构化，适合梳理思路和做决策。".into());
    advisor.system_prompt = "你是一只沉稳的桌面宠物顾问。你冷静、结构化，习惯先澄清问题再给建议，\
        会指出风险，也会给出可执行的下一步。"
        .into();
    advisor.traits.tone = "沉稳、克制".into();
    advisor.traits.verbosity = "detailed".into();
    advisor.sampling.temperature = 0.4;
    advisor.greeting = Some("需要我帮你理一理思路吗？".into());
    advisor.builtin = true;
    out.insert(advisor.id.clone(), advisor);

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_round_trips_and_seeds_default() {
        let tmp = tempfile::tempdir().unwrap();
        let store = PersonaStore::new(tmp.path().join("personas"));
        store.ensure().unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
        let mut p = Persona::new("snarky", "毒舌");
        p.system_prompt = "be snarky".into();
        store.save(&p).unwrap();
        assert!(store.get("snarky").unwrap().is_some());
        store.delete("snarky").unwrap();
        assert!(store.get("snarky").unwrap().is_none());
        assert!(store.delete(DEFAULT_PERSONA_ID).is_err());
    }

    #[test]
    fn duplicate_and_templates() {
        let tmp = tempfile::tempdir().unwrap();
        let store = PersonaStore::new(tmp.path().to_path_buf());
        store.ensure().unwrap();
        let copy = store.duplicate(DEFAULT_PERSONA_ID, "copy", "副本").unwrap();
        assert!(!copy.builtin);
        assert_eq!(store.list().unwrap().len(), 2);
        assert_eq!(templates().len(), 3);
    }

    #[test]
    fn effective_prompt_includes_traits_and_pet() {
        let mut p = Persona::default();
        p.traits.verbosity = "short".into();
        let prompt = p.effective_system_prompt(Some("珍珠小子"), Some("工作中"));
        assert!(prompt.contains("珍珠小子"));
        assert!(prompt.contains("工作中"));
        assert!(prompt.contains("简短"));
        assert!(prompt.contains("不要使用 emoji"));
    }

    #[test]
    fn validation_rejects_bad_values() {
        let mut p = Persona::default();
        p.sampling.temperature = 5.0;
        assert!(p.validate().is_err());
        p = Persona::default();
        p.avatar_pet = Some("../evil".into());
        assert!(p.validate().is_err());
    }

    #[test]
    fn export_then_import() {
        let tmp = tempfile::tempdir().unwrap();
        let store = PersonaStore::new(tmp.path().join("a"));
        store.ensure().unwrap();
        let mut custom = Persona::new("custom", "自定义");
        custom.system_prompt = "be custom".into();
        store.save(&custom).unwrap();

        let out = tmp.path().join("out.json");
        store.export_file("custom", &out).unwrap();

        let store2 = PersonaStore::new(tmp.path().join("b"));
        store2.ensure().unwrap();
        let imported = store2.import_file(&out, false).unwrap();
        assert_eq!(imported.id, "custom");
        assert_eq!(imported.name, "自定义");
    }
}
