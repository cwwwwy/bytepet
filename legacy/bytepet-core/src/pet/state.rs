//! Pet animation states, the official Codex timing table, and the priority
//! state machine that turns chat/agent/auto-walk events into a state.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::pet::manifest::{FrameSpec, PetManifest};

/// The Codex-compatible animation rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PetState {
    Idle,
    RunningRight,
    RunningLeft,
    Waving,
    Jumping,
    Failed,
    Waiting,
    Running,
    Review,
    LookRow9,
    LookRow10,
}

impl PetState {
    pub const ALL: [PetState; 11] = [
        PetState::Idle,
        PetState::RunningRight,
        PetState::RunningLeft,
        PetState::Waving,
        PetState::Jumping,
        PetState::Failed,
        PetState::Waiting,
        PetState::Running,
        PetState::Review,
        PetState::LookRow9,
        PetState::LookRow10,
    ];

    /// Canonical Codex row index.
    pub fn row(self) -> u32 {
        match self {
            PetState::Idle => 0,
            PetState::RunningRight => 1,
            PetState::RunningLeft => 2,
            PetState::Waving => 3,
            PetState::Jumping => 4,
            PetState::Failed => 5,
            PetState::Waiting => 6,
            PetState::Running => 7,
            PetState::Review => 8,
            PetState::LookRow9 => 9,
            PetState::LookRow10 => 10,
        }
    }

    /// Canonical Codex state name.
    pub fn name(self) -> &'static str {
        match self {
            PetState::Idle => "idle",
            PetState::RunningRight => "running-right",
            PetState::RunningLeft => "running-left",
            PetState::Waving => "waving",
            PetState::Jumping => "jumping",
            PetState::Failed => "failed",
            PetState::Waiting => "waiting",
            PetState::Running => "running",
            PetState::Review => "review",
            PetState::LookRow9 => "look-row-9",
            PetState::LookRow10 => "look-row-10",
        }
    }

    /// Parse a state name from the protocol / manifest, accepting common aliases.
    pub fn from_name(name: &str) -> Option<Self> {
        let normalized = name.trim().to_ascii_lowercase().replace(['_', ' '], "-");
        let normalized = normalized.trim_start_matches("look-row-");
        match normalized {
            "idle" | "still" | "default" => Some(PetState::Idle),
            "running-right" | "runningright" | "walk-right" | "walkright" | "right" => {
                Some(PetState::RunningRight)
            }
            "running-left" | "runningleft" | "walk-left" | "walkleft" | "left" => {
                Some(PetState::RunningLeft)
            }
            "waving" | "wave" | "hello" | "greeting" => Some(PetState::Waving),
            "jumping" | "jump" | "hop" => Some(PetState::Jumping),
            "failed" | "fail" | "error" | "sad" => Some(PetState::Failed),
            "waiting" | "wait" | "blocked" | "input" => Some(PetState::Waiting),
            "running" | "working" | "work" | "busy" | "thinking" => Some(PetState::Running),
            "review" | "success" | "done" | "complete" => Some(PetState::Review),
            "9" | "look9" => Some(PetState::LookRow9),
            "10" | "look10" => Some(PetState::LookRow10),
            _ => None,
        }
    }

    /// Higher wins. Used to arbitrate competing events.
    pub fn priority(self) -> u8 {
        match self {
            PetState::Failed => 90,
            PetState::Waiting => 80,
            PetState::Running => 70,
            PetState::Review => 60,
            PetState::Waving | PetState::Jumping => 40,
            PetState::LookRow9 | PetState::LookRow10 => 20,
            PetState::RunningLeft | PetState::RunningRight => 10,
            PetState::Idle => 0,
        }
    }

    /// One-shot states return to their fallback after playing once.
    pub fn is_one_shot(self) -> bool {
        matches!(self, PetState::Waving | PetState::Jumping)
    }

    /// Locomotion states are driven by the auto-walk engine as the base state.
    pub fn is_locomotion(self) -> bool {
        matches!(self, PetState::RunningLeft | PetState::RunningRight)
    }

    pub fn requires_row(self) -> u32 {
        self.row()
    }
}

/// A resolved animation: global sprite indices with per-frame durations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Animation {
    pub state: PetState,
    pub row: u32,
    pub sprites: Vec<u32>,
    pub durations_ms: Vec<f32>,
    pub loop_anim: bool,
    pub fallback: PetState,
    pub total_ms: f32,
}

impl Animation {
    fn new(
        state: PetState,
        row: u32,
        columns: u32,
        durations_ms: Vec<f32>,
        loop_anim: bool,
        fallback: PetState,
    ) -> Self {
        let count = durations_ms.len() as u32;
        let sprites = (0..count).map(|i| row * columns + i).collect::<Vec<_>>();
        let total_ms = durations_ms.iter().sum();
        Self {
            state,
            row,
            sprites,
            durations_ms,
            loop_anim,
            fallback,
            total_ms,
        }
    }

    /// Global sprite index for the frame at `elapsed_ms`.
    pub fn sprite_at(&self, elapsed_ms: f32) -> Option<u32> {
        if self.sprites.is_empty() {
            return None;
        }
        if self.total_ms <= 0.0 {
            return self.sprites.first().copied();
        }
        let mut t = elapsed_ms.max(0.0);
        if self.loop_anim {
            t %= self.total_ms;
        } else if t >= self.total_ms {
            return self.sprites.last().copied();
        }
        let mut acc = 0.0;
        for (i, dur) in self.durations_ms.iter().enumerate() {
            acc += dur.max(0.0);
            if t < acc {
                return self.sprites.get(i).copied();
            }
        }
        self.sprites.last().copied()
    }
}

/// Official Codex per-row frame durations (milliseconds).
pub fn official_durations(state: PetState) -> Vec<f32> {
    match state {
        PetState::Idle => vec![280.0, 110.0, 110.0, 140.0, 140.0, 320.0],
        PetState::RunningRight | PetState::RunningLeft => {
            let mut v = vec![120.0; 7];
            v.push(220.0);
            v
        }
        PetState::Waving => vec![140.0, 140.0, 140.0, 280.0],
        PetState::Jumping => vec![140.0, 140.0, 140.0, 140.0, 280.0],
        PetState::Failed => {
            let mut v = vec![140.0; 7];
            v.push(240.0);
            v
        }
        PetState::Waiting => {
            let mut v = vec![150.0; 5];
            v.push(260.0);
            v
        }
        PetState::Running => {
            let mut v = vec![120.0; 5];
            v.push(220.0);
            v
        }
        PetState::Review => {
            let mut v = vec![150.0; 5];
            v.push(280.0);
            v
        }
        PetState::LookRow9 | PetState::LookRow10 => vec![140.0; 8],
    }
}

/// Build every animation available for the given geometry and manifest.
pub fn resolve_animations(
    frame: FrameSpec,
    manifest: &PetManifest,
) -> BTreeMap<PetState, Animation> {
    let mut out = BTreeMap::new();
    for state in PetState::ALL {
        if !frame.has_row(state.requires_row()) {
            continue;
        }
        if let Some(anim) = resolve_one(state, frame, manifest) {
            out.insert(state, anim);
        }
    }
    out
}

fn track_key(
    manifest: &PetManifest,
    state: PetState,
) -> Option<&crate::pet::manifest::AnimationTrack> {
    let name = state.name();
    let snake = name.replace('-', "_");
    manifest
        .animations
        .get(name)
        .or_else(|| manifest.animations.get(&snake))
        .or_else(|| manifest.animations.get(&name.replace('-', "")))
        .or_else(|| manifest.animations.get(&snake.replace('_', "")))
}

fn resolve_one(state: PetState, frame: FrameSpec, manifest: &PetManifest) -> Option<Animation> {
    let fallback = track_key(manifest, state)
        .and_then(|t| t.fallback.as_deref())
        .and_then(PetState::from_name)
        .unwrap_or(PetState::Idle);

    if let Some(track) = track_key(manifest, state) {
        if let Some(frames) = track.frames.as_ref().filter(|f| !f.is_empty()) {
            let default_dur = track
                .fps
                .filter(|f| *f > 0.0)
                .map(|f| 1000.0 / f)
                .unwrap_or(140.0);
            let sprites = frames.iter().map(|f| f.sprite_index()).collect::<Vec<_>>();
            let durations = frames
                .iter()
                .map(|f| f.duration_ms().unwrap_or(default_dur).max(1.0))
                .collect::<Vec<_>>();
            let loop_anim = track.loop_anim.unwrap_or(!state.is_one_shot());
            let total_ms = durations.iter().sum();
            return Some(Animation {
                state,
                row: state.row(),
                sprites,
                durations_ms: durations,
                loop_anim,
                fallback,
                total_ms,
            });
        }
    }

    // Fall back to the official Codex timing table, clipped to the real grid.
    let durations = official_durations(state);
    let available = frame.columns as usize;
    let durations = durations.into_iter().take(available).collect::<Vec<_>>();
    if durations.is_empty() {
        return None;
    }
    let loop_anim = !state.is_one_shot();
    Some(Animation::new(
        state,
        state.row(),
        frame.columns,
        durations,
        loop_anim,
        fallback,
    ))
}

/// A state change emitted by the engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    pub state: PetState,
    pub source: String,
    pub message: Option<String>,
    pub one_shot: bool,
}

struct ActiveOverride {
    state: PetState,
    source: String,
    message: Option<String>,
    expires_at: Option<Instant>,
    one_shot: bool,
}

/// Priority state machine.
///
/// `base` is the passive state (idle or auto-walk locomotion); `override_state`
/// is the highest-priority transient event (chat / agent status / greeting).
pub struct PetEngine {
    pub frame: FrameSpec,
    pub animations: BTreeMap<PetState, Animation>,
    base: PetState,
    active: Option<ActiveOverride>,
}

impl PetEngine {
    pub fn new(frame: FrameSpec, manifest: &PetManifest) -> Self {
        Self {
            frame,
            animations: resolve_animations(frame, manifest),
            base: PetState::Idle,
            active: None,
        }
    }

    pub fn current(&self) -> PetState {
        self.active.as_ref().map(|a| a.state).unwrap_or(self.base)
    }

    pub fn base(&self) -> PetState {
        self.base
    }

    pub fn bubble(&self) -> Option<&str> {
        self.active.as_ref().and_then(|a| a.message.as_deref())
    }

    pub fn source(&self) -> Option<&str> {
        self.active.as_ref().map(|a| a.source.as_str())
    }

    pub fn animation(&self, state: PetState) -> Option<&Animation> {
        self.animations.get(&state)
    }

    pub fn current_animation(&self) -> Option<&Animation> {
        self.animations
            .get(&self.current())
            .or_else(|| self.animations.get(&PetState::Idle))
    }

    /// Passive locomotion / idle state, only applied when no override is active.
    pub fn set_base(&mut self, state: PetState) -> bool {
        if !state.is_locomotion() && state != PetState::Idle {
            return false;
        }
        if !self.animations.contains_key(&state) {
            return false;
        }
        if self.base == state {
            return false;
        }
        self.base = state;
        self.active.is_none()
    }

    /// Raise a transient state. Returns `true` if the visible state changed.
    pub fn raise(
        &mut self,
        state: PetState,
        source: &str,
        message: Option<String>,
        ttl: Option<Duration>,
        now: Instant,
    ) -> Option<Transition> {
        if !self.animations.contains_key(&state) {
            return None;
        }
        let incoming = state.priority();
        if let Some(active) = &self.active {
            let expired = active.expires_at.is_some_and(|t| now >= t);
            // A source owns its own lifecycle: `running -> review` from the same
            // agent must be allowed. Priority only arbitrates *competing* sources.
            let same_source = active.source == source;
            if !expired && !same_source && incoming <= active.state.priority() {
                return None;
            }
        }
        let one_shot = state.is_one_shot();
        self.active = Some(ActiveOverride {
            state,
            source: source.to_string(),
            message,
            expires_at: ttl.map(|d| now + d),
            one_shot,
        });
        Some(Transition {
            state,
            source: source.to_string(),
            message: self.active.as_ref().and_then(|a| a.message.clone()),
            one_shot,
        })
    }

    /// Clear overrides from a source (e.g. an agent session ended).
    pub fn clear_source(&mut self, source: &str, now: Instant) -> Option<Transition> {
        match &self.active {
            Some(active) if active.source == source => {
                self.active = None;
                Some(Transition {
                    state: self.base,
                    source: source.to_string(),
                    message: None,
                    one_shot: false,
                })
            }
            _ => {
                let _ = now;
                None
            }
        }
    }

    pub fn clear_all(&mut self) -> Option<Transition> {
        if self.active.take().is_some() {
            Some(Transition {
                state: self.base,
                source: "system".to_string(),
                message: None,
                one_shot: false,
            })
        } else {
            None
        }
    }

    /// Called when a one-shot animation finished playing.
    pub fn on_one_shot_finished(&mut self) -> Option<Transition> {
        let active = self.active.as_ref()?;
        if !active.one_shot {
            return None;
        }
        let source = active.source.clone();
        let fallback = self
            .animations
            .get(&active.state)
            .map(|a| a.fallback)
            .unwrap_or(self.base);
        self.active = None;
        // Prefer the state's declared fallback if it is renderable, else base.
        let next = if self.animations.contains_key(&fallback) {
            fallback
        } else {
            self.base
        };
        Some(Transition {
            state: next,
            source,
            message: None,
            one_shot: false,
        })
    }

    /// Expire TTLs. Returns a transition when the visible state changed.
    pub fn tick(&mut self, now: Instant) -> Option<Transition> {
        let expired = self
            .active
            .as_ref()
            .and_then(|a| a.expires_at)
            .is_some_and(|t| now >= t);
        if !expired {
            return None;
        }
        let source = self
            .active
            .as_ref()
            .map(|a| a.source.clone())
            .unwrap_or_else(|| "system".to_string());
        self.active = None;
        Some(Transition {
            state: self.base,
            source,
            message: None,
            one_shot: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(rows: u32) -> PetEngine {
        let manifest = PetManifest::from_json_str(
            r#"{"id":"t","displayName":"T","spritesheetPath":"s.webp"}"#,
        )
        .unwrap();
        PetEngine::new(FrameSpec::new(8, rows), &manifest)
    }

    #[test]
    fn official_table_matches_codex_spec() {
        assert_eq!(official_durations(PetState::Idle).len(), 6);
        assert_eq!(official_durations(PetState::RunningRight).len(), 8);
        assert_eq!(official_durations(PetState::Waving).len(), 4);
        assert_eq!(official_durations(PetState::Running)[5], 220.0);
        assert_eq!(official_durations(PetState::Review)[5], 280.0);
    }

    #[test]
    fn nine_row_pet_has_no_look_rows() {
        let e = engine(9);
        assert!(e.animation(PetState::Idle).is_some());
        assert!(e.animation(PetState::LookRow9).is_none());
        let e = engine(11);
        assert!(e.animation(PetState::LookRow9).is_some());
        assert!(e.animation(PetState::LookRow10).is_some());
    }

    #[test]
    fn sprite_indices_are_global() {
        let e = engine(9);
        let anim = e.animation(PetState::Waving).unwrap();
        assert_eq!(anim.sprites, vec![24, 25, 26, 27]);
    }

    #[test]
    fn sprite_at_respects_durations_and_loop() {
        let e = engine(9);
        let idle = e.animation(PetState::Idle).unwrap();
        assert_eq!(idle.sprite_at(0.0), Some(0));
        assert_eq!(idle.sprite_at(300.0), Some(1));
        // 280 + 110 = 390ms, so 450ms is the third frame
        assert_eq!(idle.sprite_at(450.0), Some(2));
        assert_eq!(idle.sprite_at(1099.0), Some(5));
        // loops back to the start
        assert_eq!(idle.sprite_at(idle.total_ms), Some(0));
        assert_eq!(idle.sprite_at(idle.total_ms + 1.0), Some(0));
        let wave = e.animation(PetState::Waving).unwrap();
        assert!(!wave.loop_anim);
        assert_eq!(wave.sprite_at(10_000.0), Some(27));
    }

    #[test]
    fn priority_arbitration() {
        let mut e = engine(9);
        let now = Instant::now();
        assert!(e
            .raise(PetState::Running, "agent:codex", None, None, now)
            .is_some());
        // waiting outranks running
        assert!(e
            .raise(
                PetState::Waiting,
                "agent:codex",
                Some("approve".into()),
                None,
                now
            )
            .is_some());
        assert_eq!(e.current(), PetState::Waiting);
        // idle cannot override waiting
        assert!(e.raise(PetState::Idle, "system", None, None, now).is_none());
        assert_eq!(e.current(), PetState::Waiting);
        // failed outranks waiting
        assert!(e.raise(PetState::Failed, "chat", None, None, now).is_some());
        assert_eq!(e.current(), PetState::Failed);
    }

    #[test]
    fn same_source_can_step_down_but_others_cannot() {
        let mut e = engine(9);
        let now = Instant::now();
        assert!(e
            .raise(PetState::Running, "agent:claude", None, None, now)
            .is_some());
        // Same agent session finishing: running -> review is a downgrade but valid.
        assert!(e
            .raise(PetState::Review, "agent:claude", None, None, now)
            .is_some());
        assert_eq!(e.current(), PetState::Review);
        // A different source may not downgrade it.
        assert!(e.raise(PetState::Idle, "system", None, None, now).is_none());
        assert_eq!(e.current(), PetState::Review);
        // ...but may still upgrade it.
        assert!(e
            .raise(PetState::Waiting, "agent:codex", None, None, now)
            .is_some());
        assert_eq!(e.current(), PetState::Waiting);
    }

    #[test]
    fn ttl_expiry_falls_back_to_base() {
        let mut e = engine(9);
        let now = Instant::now();
        e.set_base(PetState::RunningLeft);
        e.raise(
            PetState::Running,
            "agent:codex",
            None,
            Some(Duration::from_millis(50)),
            now,
        );
        assert_eq!(e.current(), PetState::Running);
        assert!(e.tick(now + Duration::from_millis(10)).is_none());
        let t = e.tick(now + Duration::from_millis(60)).unwrap();
        assert_eq!(t.state, PetState::RunningLeft);
    }

    #[test]
    fn one_shot_returns_to_fallback() {
        let mut e = engine(9);
        let now = Instant::now();
        e.raise(PetState::Waving, "system", None, None, now);
        assert_eq!(e.current(), PetState::Waving);
        let t = e.on_one_shot_finished().unwrap();
        assert_eq!(t.state, PetState::Idle);
    }

    #[test]
    fn manifest_track_overrides_defaults() {
        let manifest = PetManifest::from_json_str(
            r#"{"id":"t","spritesheetPath":"s.webp",
                "animations":{"waving":{"frames":[24,25],"fps":10,"loop":false,"fallback":"idle"}}}"#,
        )
        .unwrap();
        let e = PetEngine::new(FrameSpec::new(8, 9), &manifest);
        let wave = e.animation(PetState::Waving).unwrap();
        assert_eq!(wave.sprites, vec![24, 25]);
        assert_eq!(wave.durations_ms, vec![100.0, 100.0]);
        assert!(!wave.loop_anim);
    }

    #[test]
    fn parses_state_aliases() {
        assert_eq!(
            PetState::from_name("running-right"),
            Some(PetState::RunningRight)
        );
        assert_eq!(
            PetState::from_name("running_right"),
            Some(PetState::RunningRight)
        );
        assert_eq!(PetState::from_name("working"), Some(PetState::Running));
        assert_eq!(PetState::from_name("RUNNING"), Some(PetState::Running));
        assert_eq!(PetState::from_name("look-row-9"), Some(PetState::LookRow9));
        assert_eq!(PetState::from_name("nope"), None);
    }
}
