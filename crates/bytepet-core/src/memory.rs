//! Lightweight pet memory.
//!
//! This is deliberately not a chat transcript. It stores a small set of stable
//! facts, recent interaction events and greeting state in one JSON file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

const MEMORY_VERSION: u32 = 1;
const MAX_EVENTS: usize = 200;
const MAX_FACTS: usize = 50;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Fact {
    pub id: String,
    pub key: String,
    pub value: String,
    pub confidence: f32,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    AppStart,
    UserClick,
    UserMessage,
    PetGreeting,
    PetReaction,
    PetChanged,
    CodexStatus,
    IdleReturn,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEvent {
    pub id: String,
    pub kind: EventKind,
    pub text: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct PersonaMemory {
    pub facts: Vec<Fact>,
    pub events: Vec<MemoryEvent>,
    pub last_seen_at: Option<i64>,
    pub last_greeting_at: Option<i64>,
    pub last_trigger: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryFile {
    pub version: u32,
    pub personas: BTreeMap<String, PersonaMemory>,
}

impl Default for MemoryFile {
    fn default() -> Self {
        Self {
            version: MEMORY_VERSION,
            personas: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct GreetingContext {
    pub facts: Vec<Fact>,
    pub recent_events: Vec<MemoryEvent>,
    pub last_seen_at: Option<i64>,
    pub last_greeting_at: Option<i64>,
}

impl GreetingContext {
    pub fn render(&self, now: i64) -> String {
        let mut sections = Vec::new();

        if !self.facts.is_empty() {
            let facts = self
                .facts
                .iter()
                .map(|fact| format!("- {}：{}", fact.key, fact.value))
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(format!("【你记得关于用户的事情】\n{facts}"));
        }

        if !self.recent_events.is_empty() {
            let events = self
                .recent_events
                .iter()
                .map(|event| {
                    let text = event.text.as_deref().unwrap_or_else(|| event.kind.label());
                    format!("- {}（{}）", text, humanize_ago(now, event.created_at))
                })
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(format!("【最近互动】\n{events}"));
        }

        if let Some(last_seen) = self.last_seen_at {
            sections.push(format!("【上次见面】{}", humanize_ago(now, last_seen)));
        }

        if let Some(last_greeting) = self.last_greeting_at {
            sections.push(format!(
                "【上次主动问候】{}",
                humanize_ago(now, last_greeting)
            ));
        }

        sections.join("\n\n")
    }
}

impl EventKind {
    pub fn label(self) -> &'static str {
        match self {
            EventKind::AppStart => "应用启动",
            EventKind::UserClick => "用户点击了宠物",
            EventKind::UserMessage => "用户说了一句话",
            EventKind::PetGreeting => "宠物主动问候",
            EventKind::PetReaction => "宠物做出了回应",
            EventKind::PetChanged => "更换了宠物",
            EventKind::CodexStatus => "Codex 状态变化",
            EventKind::IdleReturn => "用户离开后回来",
        }
    }
}

pub struct PetMemory {
    path: PathBuf,
    inner: Mutex<MemoryFile>,
}

impl PetMemory {
    pub fn open(path: &Path) -> Result<Self> {
        let inner = if path.is_file() {
            let text = std::fs::read_to_string(path)?;
            serde_json::from_str(&text).unwrap_or_default()
        } else {
            MemoryFile::default()
        };
        Ok(Self {
            path: path.to_path_buf(),
            inner: Mutex::new(inner),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn record_event(
        &self,
        persona_id: &str,
        kind: EventKind,
        text: Option<String>,
    ) -> Result<MemoryEvent> {
        let event = MemoryEvent {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            text,
            created_at: now_ms(),
        };
        let mut memory = self.inner.lock();
        let persona = memory.personas.entry(persona_id.to_string()).or_default();
        persona.last_seen_at = Some(event.created_at);
        persona.events.push(event.clone());
        trim_events(persona);
        self.save_locked(&memory)?;
        Ok(event)
    }

    pub fn remember_fact(
        &self,
        persona_id: &str,
        key: &str,
        value: &str,
        confidence: f32,
    ) -> Result<Fact> {
        let now = now_ms();
        let mut memory = self.inner.lock();
        let persona = memory.personas.entry(persona_id.to_string()).or_default();
        let existing = persona
            .facts
            .iter_mut()
            .find(|fact| fact.key.eq_ignore_ascii_case(key));
        let fact = match existing {
            Some(fact) => {
                fact.value = value.to_string();
                fact.confidence = confidence;
                fact.updated_at = now;
                fact.clone()
            }
            None => {
                let fact = Fact {
                    id: uuid::Uuid::new_v4().to_string(),
                    key: key.to_string(),
                    value: value.to_string(),
                    confidence,
                    created_at: now,
                    updated_at: now,
                };
                persona.facts.push(fact.clone());
                fact
            }
        };
        persona
            .facts
            .sort_by_key(|fact| std::cmp::Reverse(fact.updated_at));
        persona.facts.truncate(MAX_FACTS);
        self.save_locked(&memory)?;
        Ok(fact)
    }

    pub fn forget_fact(&self, persona_id: &str, fact_id: &str) -> Result<bool> {
        let mut memory = self.inner.lock();
        let Some(persona) = memory.personas.get_mut(persona_id) else {
            return Ok(false);
        };
        let before = persona.facts.len();
        persona.facts.retain(|fact| fact.id != fact_id);
        let removed = persona.facts.len() != before;
        if removed {
            self.save_locked(&memory)?;
        }
        Ok(removed)
    }

    pub fn list_facts(&self, persona_id: &str) -> Vec<Fact> {
        self.inner
            .lock()
            .personas
            .get(persona_id)
            .map(|persona| persona.facts.clone())
            .unwrap_or_default()
    }

    pub fn recent_events(&self, persona_id: &str, limit: usize) -> Vec<MemoryEvent> {
        let memory = self.inner.lock();
        let mut events = memory
            .personas
            .get(persona_id)
            .map(|persona| persona.events.clone())
            .unwrap_or_default();
        if events.len() > limit {
            events.drain(0..events.len() - limit);
        }
        events
    }

    pub fn mark_seen(&self, persona_id: &str) -> Result<()> {
        let mut memory = self.inner.lock();
        let persona = memory.personas.entry(persona_id.to_string()).or_default();
        persona.last_seen_at = Some(now_ms());
        self.save_locked(&memory)
    }

    pub fn mark_greeted(&self, persona_id: &str, trigger: &str, text: &str) -> Result<()> {
        let now = now_ms();
        let mut memory = self.inner.lock();
        let persona = memory.personas.entry(persona_id.to_string()).or_default();
        persona.last_greeting_at = Some(now);
        persona.last_trigger = Some(trigger.to_string());
        persona.events.push(MemoryEvent {
            id: uuid::Uuid::new_v4().to_string(),
            kind: EventKind::PetGreeting,
            text: Some(text.to_string()),
            created_at: now,
        });
        trim_events(persona);
        self.save_locked(&memory)
    }

    pub fn clear_persona(&self, persona_id: &str) -> Result<()> {
        let mut memory = self.inner.lock();
        memory.personas.remove(persona_id);
        self.save_locked(&memory)
    }

    pub fn build_greeting_context(
        &self,
        persona_id: &str,
        recent_events: usize,
        fact_limit: usize,
    ) -> GreetingContext {
        let memory = self.inner.lock();
        let Some(persona) = memory.personas.get(persona_id) else {
            return GreetingContext::default();
        };
        let mut facts = persona.facts.clone();
        facts.sort_by_key(|fact| std::cmp::Reverse(fact.updated_at));
        facts.truncate(fact_limit);

        let mut events = persona.events.clone();
        if events.len() > recent_events {
            events.drain(0..events.len() - recent_events);
        }

        GreetingContext {
            facts,
            recent_events: events,
            last_seen_at: persona.last_seen_at,
            last_greeting_at: persona.last_greeting_at,
        }
    }

    fn save_locked(&self, memory: &MemoryFile) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = self.path.with_extension("json.tmp");
        std::fs::write(&temp, serde_json::to_string_pretty(memory)?)?;
        std::fs::rename(temp, &self.path).map_err(Error::from)
    }
}

fn trim_events(persona: &mut PersonaMemory) {
    if persona.events.len() > MAX_EVENTS {
        persona.events.drain(0..persona.events.len() - MAX_EVENTS);
    }
}

fn humanize_ago(now: i64, then: i64) -> String {
    let seconds = ((now - then).max(0) / 1000) as u64;
    if seconds < 60 {
        "刚刚".to_string()
    } else if seconds < 3600 {
        format!("{} 分钟前", seconds / 60)
    } else if seconds < 86_400 {
        format!("{} 小时前", seconds / 3600)
    } else {
        format!("{} 天前", seconds / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facts_upsert_and_clear() {
        let dir = tempfile::tempdir().unwrap();
        let memory = PetMemory::open(&dir.path().join("memory.json")).unwrap();
        memory
            .remember_fact("default", "咖啡", "美式", 1.0)
            .unwrap();
        memory
            .remember_fact("default", "咖啡", "拿铁", 0.9)
            .unwrap();
        let facts = memory.list_facts("default");
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].value, "拿铁");
        memory.forget_fact("default", &facts[0].id).unwrap();
        assert!(memory.list_facts("default").is_empty());
    }

    #[test]
    fn events_and_context_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let memory = PetMemory::open(&dir.path().join("memory.json")).unwrap();
        memory
            .record_event("default", EventKind::AppStart, Some("启动".into()))
            .unwrap();
        memory
            .record_event("default", EventKind::UserClick, None)
            .unwrap();
        let context = memory.build_greeting_context("default", 5, 20);
        assert_eq!(context.recent_events.len(), 2);
        assert!(context.render(now_ms()).contains("最近互动"));
    }
}
