//! `pet.json` parsing and validation.
//!
//! The parser is intentionally tolerant: it accepts the official Codex manifest
//! (`id`/`displayName`/`description`/`spritesheetPath`/`spriteVersionNumber`),
//! UniPet-style geometry (`frame` object or legacy `frameWidth`/`columns`...)
//! and unknown extra fields, which are preserved for round-tripping.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::pet::{DEFAULT_CELL_HEIGHT, DEFAULT_CELL_WIDTH};

/// Atlas geometry in pixels and cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameSpec {
    pub width: u32,
    pub height: u32,
    pub columns: u32,
    pub rows: u32,
}

impl Default for FrameSpec {
    fn default() -> Self {
        Self {
            width: DEFAULT_CELL_WIDTH,
            height: DEFAULT_CELL_HEIGHT,
            columns: 8,
            rows: 9,
        }
    }
}

impl FrameSpec {
    pub fn new(columns: u32, rows: u32) -> Self {
        Self {
            columns,
            rows,
            ..Self::default()
        }
    }

    pub fn atlas_width(&self) -> u32 {
        self.width.saturating_mul(self.columns)
    }

    pub fn atlas_height(&self) -> u32 {
        self.height.saturating_mul(self.rows)
    }

    pub fn cell_count(&self) -> u32 {
        self.columns.saturating_mul(self.rows)
    }

    /// Row index for a Codex state, if the geometry actually has that row.
    pub fn has_row(&self, row: u32) -> bool {
        row < self.rows
    }
}

/// One frame reference inside a UniPet animation track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FrameRef {
    /// Global sprite index (`row * columns + column`).
    Index(u32),
    /// Explicit sprite index with an optional per-frame duration.
    Detailed {
        #[serde(alias = "spriteIndex")]
        sprite_index: u32,
        #[serde(default, alias = "durationMs")]
        duration_ms: Option<f32>,
    },
}

impl FrameRef {
    pub fn sprite_index(&self) -> u32 {
        match self {
            FrameRef::Index(i) => *i,
            FrameRef::Detailed { sprite_index, .. } => *sprite_index,
        }
    }

    pub fn duration_ms(&self) -> Option<f32> {
        match self {
            FrameRef::Index(_) => None,
            FrameRef::Detailed { duration_ms, .. } => *duration_ms,
        }
    }
}

/// UniPet-compatible animation track.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnimationTrack {
    #[serde(default)]
    pub frames: Option<Vec<FrameRef>>,
    #[serde(default)]
    pub fps: Option<f32>,
    #[serde(default, rename = "loop")]
    pub loop_anim: Option<bool>,
    #[serde(default, alias = "loopStart")]
    pub loop_start: Option<Option<u32>>,
    #[serde(default)]
    pub fallback: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Parsed `pet.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetManifest {    pub id: String,

    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub version: Option<String>,

    #[serde(default)]
    pub sprite_version_number: Option<u32>,
    #[serde(default)]
    pub spritesheet_path: Option<String>,
    /// Alternate field name seen in some packages.
    #[serde(default)]
    pub spritesheet: Option<String>,

    #[serde(default)]
    pub frame: Option<FrameSpec>,
    #[serde(default)]
    pub frame_width: Option<u32>,
    #[serde(default)]
    pub frame_height: Option<u32>,
    #[serde(default)]
    pub columns: Option<u32>,
    #[serde(default)]
    pub rows: Option<u32>,

    #[serde(default)]
    pub animations: BTreeMap<String, AnimationTrack>,

    /// Unknown fields are preserved so export round-trips.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for PetManifest {
    fn default() -> Self {
        Self {
            id: String::new(),
            display_name: None,
            name: None,
            description: None,
            author: None,
            version: None,
            sprite_version_number: None,
            spritesheet_path: None,
            spritesheet: None,
            frame: None,
            frame_width: None,
            frame_height: None,
            columns: None,
            rows: None,
            animations: BTreeMap::new(),
            extra: serde_json::Map::new(),
        }
    }
}

impl PetManifest {
    pub fn from_json_str(json: &str) -> Result<Self> {
        let manifest: Self = serde_json::from_str(json)
            .map_err(|e| Error::manifest(format!("cannot parse pet.json: {e}")))?;
        Ok(manifest)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|e| {
            Error::manifest(format!("cannot read {}: {e}", path.display()))
        })?;
        // Strip a UTF-8 BOM if present: some exporters write one.
        let bytes = bytes
            .strip_prefix(&[0xEF, 0xBB, 0xBF])
            .unwrap_or(&bytes)
            .to_vec();
        let text = String::from_utf8(bytes)
            .map_err(|e| Error::manifest(format!("pet.json is not UTF-8: {e}")))?;
        Self::from_json_str(&text)
    }

    pub fn to_json_string(&self) -> Result<String> {
        // Codex requires strict UTF-8 JSON without a BOM.
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn display_name(&self) -> String {
        self.display_name
            .clone()
            .or_else(|| self.name.clone())
            .unwrap_or_else(|| self.id.clone())
    }

    /// Relative path of the spritesheet inside the pet directory.
    pub fn spritesheet_rel(&self) -> PathBuf {
        let raw = self
            .spritesheet_path
            .clone()
            .or_else(|| self.spritesheet.clone())
            .unwrap_or_else(|| "spritesheet.webp".to_string());
        PathBuf::from(raw)
    }

    /// Geometry declared by the manifest, before looking at the image.
    pub fn declared_frame(&self) -> FrameSpec {
        if let Some(frame) = self.frame {
            return frame;
        }
        let default = if self.sprite_version_number.unwrap_or(1) >= 2 {
            FrameSpec::new(8, 11)
        } else {
            FrameSpec::default()
        };
        FrameSpec {
            width: self.frame_width.unwrap_or(default.width),
            height: self.frame_height.unwrap_or(default.height),
            columns: self.columns.unwrap_or(default.columns),
            rows: self.rows.unwrap_or(default.rows),
        }
    }

    /// Resolve geometry against the real image size.
    ///
    /// If the declared geometry does not match the image but the image divides
    /// evenly into cells of the declared cell size, the grid is inferred from
    /// the image and a warning is returned (some community packages ship a V2
    /// atlas without `spriteVersionNumber`).
    pub fn resolve_frame(&self, image_w: u32, image_h: u32) -> Result<(FrameSpec, Vec<String>)> {
        let mut warnings = Vec::new();
        let declared = self.declared_frame();
        if declared.atlas_width() == image_w && declared.atlas_height() == image_h {
            return Ok((declared, warnings));
        }
        if declared.width == 0 || declared.height == 0 {
            return Err(Error::manifest("frame width/height must be greater than zero"));
        }
        if image_w % declared.width == 0 && image_h % declared.height == 0 {
            let inferred = FrameSpec {
                width: declared.width,
                height: declared.height,
                columns: image_w / declared.width,
                rows: image_h / declared.height,
            };
            warnings.push(format!(
                "manifest declares {}x{} cells ({}x{} px atlas) but the image is {}x{} px; inferred a {}x{} grid",
                declared.columns,
                declared.rows,
                declared.atlas_width(),
                declared.atlas_height(),
                image_w,
                image_h,
                inferred.columns,
                inferred.rows
            ));
            return Ok((inferred, warnings));
        }
        Err(Error::manifest(format!(
            "spritesheet is {image_w}x{image_h} px which is not a whole number of {}x{} cells",
            declared.width, declared.height
        )))
    }

    /// Validate the manifest's own invariants (no filesystem access).
    pub fn validate_id(&self) -> Result<()> {
        validate_pet_id(&self.id)
    }
}

/// Pet ids must be safe to use as a directory name.
pub fn validate_pet_id(id: &str) -> Result<()> {
    if id.is_empty() {
        return Err(Error::manifest("pet id must not be empty"));
    }
    if id.len() > 64 {
        return Err(Error::manifest("pet id must be at most 64 characters"));
    }
    if id == "." || id == ".." {
        return Err(Error::manifest("pet id must not be '.' or '..'"));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(Error::manifest(format!(
            "pet id '{id}' may only contain ASCII letters, digits, '-', '_' and '.'"
        )));
    }
    if id.eq_ignore_ascii_case("uni") {
        return Err(Error::manifest(
            "pet id 'uni' is reserved by the UniPet format",
        ));
    }
    Ok(())
}

/// Ensure a manifest-relative path cannot escape the pet directory.
pub fn safe_relative_path(rel: &Path) -> Result<PathBuf> {
    if rel.as_os_str().is_empty() {
        return Err(Error::manifest("spritesheet path must not be empty"));
    }
    let mut out = PathBuf::new();
    for component in rel.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(Error::manifest(
                    "spritesheet path must not contain '..' (path traversal is rejected)",
                ))
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(Error::manifest(
                    "spritesheet path must be relative to the pet directory",
                ))
            }
        }
    }
    if out.as_os_str().is_empty() {
        return Err(Error::manifest("spritesheet path must not be empty"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codex_v1_manifest() {
        let json = r#"{
            "id": "zhenzhu-xiaozi",
            "displayName": "珍珠小子",
            "description": "他的狗叫“鼠标”。",
            "spritesheetPath": "spritesheet.webp"
        }"#;
        let m = PetManifest::from_json_str(json).unwrap();
        assert_eq!(m.id, "zhenzhu-xiaozi");
        assert_eq!(m.display_name(), "珍珠小子");
        assert_eq!(m.spritesheet_rel(), PathBuf::from("spritesheet.webp"));
        assert_eq!(m.declared_frame(), FrameSpec::new(8, 9));
        m.validate_id().unwrap();
    }

    #[test]
    fn parses_v2_manifest_and_infers_rows() {
        let json = r#"{"id":"cat","displayName":"Cat","spriteVersionNumber":2,"spritesheetPath":"spritesheet.webp"}"#;
        let m = PetManifest::from_json_str(json).unwrap();
        assert_eq!(m.declared_frame().rows, 11);
        let (frame, warnings) = m.resolve_frame(1536, 2288).unwrap();
        assert_eq!(frame.rows, 11);
        assert!(warnings.is_empty());
    }

    #[test]
    fn infers_grid_when_manifest_lies() {
        let json = r#"{"id":"cat","displayName":"Cat","spritesheetPath":"spritesheet.webp"}"#;
        let m = PetManifest::from_json_str(json).unwrap();
        let (frame, warnings) = m.resolve_frame(1536, 2288).unwrap();
        assert_eq!(frame.rows, 11);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn parses_unipet_manifest() {
        let json = r#"{
          "id":"my-pet","displayName":"My Pet","description":"d",
          "spritesheetPath":"spritesheet.webp",
          "frame":{"width":192,"height":208,"columns":8,"rows":9},
          "animations":{"waving":{"frames":[24,25,26,27],"fps":8,"loop":false,"fallback":"idle"}}
        }"#;
        let m = PetManifest::from_json_str(json).unwrap();
        assert_eq!(m.declared_frame(), FrameSpec::new(8, 9));
        let track = m.animations.get("waving").unwrap();
        assert_eq!(track.frames.as_ref().unwrap().len(), 4);
        assert_eq!(track.loop_anim, Some(false));
    }

    #[test]
    fn accepts_legacy_geometry_fields() {
        let json = r#"{"id":"old","frameWidth":192,"frameHeight":208,"columns":8,"rows":11,"spritesheetPath":"s.webp"}"#;
        let m = PetManifest::from_json_str(json).unwrap();
        assert_eq!(m.declared_frame().rows, 11);
    }

    #[test]
    fn preserves_unknown_fields() {
        let json = r#"{"id":"x","displayName":"X","spritesheetPath":"s.webp","customFlag":true}"#;
        let m = PetManifest::from_json_str(json).unwrap();
        assert!(m.extra.contains_key("customFlag"));
        assert!(m.to_json_string().unwrap().contains("customFlag"));
    }

    #[test]
    fn rejects_path_traversal() {
        assert!(safe_relative_path(Path::new("../../etc/passwd")).is_err());
        assert!(safe_relative_path(Path::new("/etc/passwd")).is_err());
        assert!(safe_relative_path(Path::new("sub/dir/spritesheet.webp")).is_ok());
    }

    #[test]
    fn rejects_unsafe_ids() {
        assert!(validate_pet_id("good-id_1.0").is_ok());
        assert!(validate_pet_id("").is_err());
        assert!(validate_pet_id("../evil").is_err());
        assert!(validate_pet_id("uni").is_err());
        assert!(validate_pet_id("有中文").is_err());
    }
}
