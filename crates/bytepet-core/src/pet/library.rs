//! Pet library: discovery across Codex / UniPet / app-local roots, validation,
//! import and export.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::pet::atlas::PetAtlas;
use crate::pet::manifest::{FrameSpec, PetManifest};
use crate::pet::state::{resolve_animations, Animation, PetEngine, PetState};

/// Where a pet was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RootKind {
    /// The app's own writable library.
    AppData,
    /// `~/.codex/pets` (Codex compatibility).
    Codex,
    /// `~/.unipet/pets` (UniPet compatibility).
    UniPet,
    /// A user-provided extra directory.
    Custom,
}

impl RootKind {
    pub fn label(self) -> &'static str {
        match self {
            RootKind::AppData => "local",
            RootKind::Codex => "codex",
            RootKind::UniPet => "unipet",
            RootKind::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryRoot {
    pub kind: RootKind,
    pub path: PathBuf,
    pub writable: bool,
}

/// A pet discovered in the library, ready to be rendered by the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetEntry {
    pub id: String,
    pub display_name: String,
    pub description: Option<String>,
    pub root: RootKind,
    pub dir: PathBuf,
    pub spritesheet: PathBuf,
    pub frame: FrameSpec,
    pub sprite_version_number: Option<u32>,
    /// True when the pet is used in place (Codex/UniPet roots) instead of copied.
    pub linked: bool,
    pub animations: BTreeMap<PetState, Animation>,
    #[serde(skip)]
    pub manifest: PetManifest,
}

impl PetEntry {
    pub fn engine(&self) -> PetEngine {
        PetEngine::new(self.frame, &self.manifest)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateReport {
    pub state: PetState,
    pub frames: usize,
    pub loop_anim: bool,
    pub fallback: PetState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationReport {
    pub ok: bool,
    pub id: String,
    pub display_name: String,
    pub dir: PathBuf,
    pub spritesheet: Option<PathBuf>,
    pub frame: FrameSpec,
    pub image_size: Option<(u32, u32)>,
    pub animations: Vec<StateReport>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// The pet library.
pub struct PetLibrary {
    roots: Vec<LibraryRoot>,
}

impl PetLibrary {
    /// Discover all standard roots. `app_pets_dir` is created on demand.
    pub fn discover(app_pets_dir: PathBuf) -> Self {
        let mut roots = vec![LibraryRoot {
            kind: RootKind::AppData,
            path: app_pets_dir,
            writable: true,
        }];
        if let Some(home) = dirs::home_dir() {
            roots.push(LibraryRoot {
                kind: RootKind::Codex,
                path: home.join(".codex").join("pets"),
                writable: false,
            });
            roots.push(LibraryRoot {
                kind: RootKind::UniPet,
                path: home.join(".unipet").join("pets"),
                writable: false,
            });
        }
        Self { roots }
    }

    pub fn with_extra_root(mut self, path: PathBuf) -> Self {
        self.roots.push(LibraryRoot {
            kind: RootKind::Custom,
            path,
            writable: false,
        });
        self
    }

    pub fn roots(&self) -> &[LibraryRoot] {
        &self.roots
    }

    /// App-local (writable) root, created if missing.
    pub fn app_root(&self) -> Result<&LibraryRoot> {
        let root = self
            .roots
            .iter()
            .find(|r| r.kind == RootKind::AppData)
            .ok_or_else(|| Error::config("app pet root is not configured"))?;
        if !root.path.exists() {
            std::fs::create_dir_all(&root.path)?;
        }
        Ok(root)
    }

    /// Scan every root. Invalid packages are skipped with a warning rather than
    /// failing the whole library. App-local pets win over linked ones.
    pub fn list(&self) -> Vec<PetEntry> {
        let mut found: Vec<PetEntry> = Vec::new();
        for root in &self.roots {
            let Ok(entries) = std::fs::read_dir(&root.path) else {
                continue;
            };
            for entry in entries.flatten() {
                let dir = entry.path();
                if !dir.is_dir() {
                    continue;
                }
                match Self::load_entry(&dir, root.kind) {
                    Ok(pet) => {
                        if let Some(existing) = found.iter_mut().find(|p| p.id == pet.id) {
                            if root.kind == RootKind::AppData {
                                *existing = pet;
                            }
                        } else {
                            found.push(pet);
                        }
                    }
                    Err(err) => {
                        tracing::debug!(dir = %dir.display(), error = %err, "skipping invalid pet");
                    }
                }
            }
        }
        found.sort_by_key(|p| p.display_name.to_lowercase());
        found
    }

    pub fn get(&self, id: &str) -> Option<PetEntry> {
        self.list().into_iter().find(|p| p.id == id)
    }

    /// Load a single pet directory (reads the image header for geometry, but
    /// does not decode pixels).
    pub fn load_entry(dir: &Path, kind: RootKind) -> Result<PetEntry> {
        let manifest_path = dir.join("pet.json");
        if !manifest_path.is_file() {
            return Err(Error::manifest(format!(
                "{} does not contain pet.json",
                dir.display()
            )));
        }
        let manifest = PetManifest::load(&manifest_path)?;
        manifest.validate_id()?;
        let spritesheet = dir.join(crate::pet::manifest::safe_relative_path(
            &manifest.spritesheet_rel(),
        )?);
        if !spritesheet.is_file() {
            return Err(Error::manifest(format!(
                "spritesheet {} is missing",
                spritesheet.display()
            )));
        }
        // Prefer the real image geometry: V2 packages may omit
        // `spriteVersionNumber`, and `declared_frame` would then hide rows 9/10.
        // `into_dimensions` only parses the header, so this stays cheap.
        let declared = manifest.declared_frame();
        let frame = match image::ImageReader::open(&spritesheet) {
            Ok(reader) => match reader.into_dimensions() {
                Ok((width, height)) => match manifest.resolve_frame(width, height) {
                    Ok((resolved, warnings)) => {
                        for warning in &warnings {
                            tracing::debug!(%warning, path = %spritesheet.display(), "inferred pet frame from image");
                        }
                        resolved
                    }
                    Err(err) => {
                        tracing::debug!(%err, path = %spritesheet.display(), "keeping declared frame");
                        declared
                    }
                },
                Err(err) => {
                    tracing::debug!(%err, path = %spritesheet.display(), "cannot read image header");
                    declared
                }
            },
            Err(err) => {
                tracing::debug!(%err, path = %spritesheet.display(), "cannot open spritesheet");
                declared
            }
        };
        Ok(PetEntry {
            id: manifest.id.clone(),
            display_name: manifest.display_name(),
            description: manifest.description.clone(),
            root: kind,
            dir: dir.to_path_buf(),
            spritesheet,
            frame,
            sprite_version_number: manifest.sprite_version_number,
            linked: kind != RootKind::AppData,
            animations: resolve_animations(frame, &manifest),
            manifest,
        })
    }

    /// Decode and fully validate a pet directory.
    pub fn validate_dir(dir: &Path) -> ValidationReport {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        let mut report = ValidationReport {
            ok: false,
            id: dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            display_name: String::new(),
            dir: dir.to_path_buf(),
            spritesheet: None,
            frame: FrameSpec::default(),
            image_size: None,
            animations: Vec::new(),
            errors: Vec::new(),
            warnings: Vec::new(),
        };

        let manifest = match PetManifest::load(&dir.join("pet.json")) {
            Ok(m) => m,
            Err(e) => {
                errors.push(e.to_string());
                report.errors = errors;
                return report;
            }
        };
        report.id = manifest.id.clone();
        report.display_name = manifest.display_name();
        if let Err(e) = manifest.validate_id() {
            errors.push(e.to_string());
        }

        match PetAtlas::open(dir, &manifest) {
            Ok((atlas, mut open_warnings)) => {
                report.spritesheet = Some(atlas.path.clone());
                report.frame = atlas.frame;
                report.image_size = Some((atlas.image.width(), atlas.image.height()));
                let engine = PetEngine::new(atlas.frame, &manifest);
                let mut used = Vec::new();
                for (state, anim) in &engine.animations {
                    used.extend(anim.sprites.iter().copied());
                    report.animations.push(StateReport {
                        state: *state,
                        frames: anim.sprites.len(),
                        loop_anim: anim.loop_anim,
                        fallback: anim.fallback,
                    });
                }
                if !engine.animations.contains_key(&PetState::Idle) {
                    errors.push("no idle animation could be resolved".to_string());
                }
                warnings.append(&mut open_warnings);
                warnings.extend(atlas.unused_cell_warnings(&used));
            }
            Err(e) => errors.push(e.to_string()),
        }

        report.ok = errors.is_empty();
        report.errors = errors;
        report.warnings = warnings;
        report
    }

    /// Import a pet directory into the writable app library.
    pub fn import_dir(&self, src: &Path, overwrite: bool) -> Result<PetEntry> {
        let report = Self::validate_dir(src);
        if !report.ok {
            return Err(Error::manifest(format!(
                "refusing to import invalid pet: {}",
                report.errors.join("; ")
            )));
        }
        let root = self.app_root()?;
        let dest = root.path.join(&report.id);
        if dest.exists() && !overwrite {
            return Err(Error::manifest(format!(
                "pet '{}' already exists in the local library",
                report.id
            )));
        }
        if dest.exists() {
            std::fs::remove_dir_all(&dest)?;
        }
        copy_pet_package(src, &dest)?;
        Self::load_entry(&dest, RootKind::AppData)
    }

    /// Remove a pet from the app-local library.
    pub fn remove_local(&self, id: &str) -> Result<()> {
        crate::pet::manifest::validate_pet_id(id)?;
        let root = self.app_root()?;
        let dir = root.path.join(id);
        if !dir.exists() {
            return Err(Error::PetNotFound(id.to_string()));
        }
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    /// Write the Codex upload format: a zip containing exactly `pet.json`
    /// (re-serialised strict UTF-8, no BOM, 2-space pretty) and the original
    /// spritesheet bytes under the filename declared by the manifest.
    pub fn export_zip(&self, id: &str, out: &Path) -> Result<()> {
        use std::io::Write;

        let entry = self
            .get(id)
            .ok_or_else(|| Error::PetNotFound(id.to_string()))?;
        let spritesheet_rel =
            crate::pet::manifest::safe_relative_path(&entry.manifest.spritesheet_rel())?;
        let spritesheet_name = spritesheet_rel
            .components()
            .map(|component| component.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("/");
        let manifest_json = entry.manifest.to_json_string()?;
        let spritesheet_bytes = std::fs::read(&entry.spritesheet).map_err(|err| {
            Error::manifest(format!(
                "cannot read spritesheet {}: {err}",
                entry.spritesheet.display()
            ))
        })?;

        if let Some(parent) = out.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let file = std::fs::File::create(out)
            .map_err(|err| Error::Zip(format!("cannot create {}: {err}", out.display())))?;
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o644);

        writer
            .start_file("pet.json", options)
            .map_err(|err| Error::Zip(format!("cannot add pet.json: {err}")))?;
        writer
            .write_all(manifest_json.as_bytes())
            .map_err(|err| Error::Zip(format!("cannot write pet.json: {err}")))?;
        writer
            .start_file(&spritesheet_name, options)
            .map_err(|err| Error::Zip(format!("cannot add {spritesheet_name}: {err}")))?;
        writer
            .write_all(&spritesheet_bytes)
            .map_err(|err| Error::Zip(format!("cannot write {spritesheet_name}: {err}")))?;
        writer
            .finish()
            .map_err(|err| Error::Zip(format!("cannot finalize {}: {err}", out.display())))?;
        Ok(())
    }

    /// Import a pet from a zip produced by [`PetLibrary::export_zip`] (or any
    /// zip holding a flat pet package). Entries are extracted into a private
    /// temporary directory after rejecting absolute paths and `..` traversal,
    /// then validated with the normal directory import.
    pub fn import_zip(&self, zip_path: &Path, overwrite: bool) -> Result<PetEntry> {
        use std::io::Read;

        const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
        const MAX_ENTRIES: usize = 64;

        let file = std::fs::File::open(zip_path)
            .map_err(|err| Error::Zip(format!("cannot open {}: {err}", zip_path.display())))?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|err| Error::Zip(format!("cannot read {}: {err}", zip_path.display())))?;
        if archive.len() > MAX_ENTRIES {
            return Err(Error::Zip(format!(
                "{} contains {} entries; the limit is {MAX_ENTRIES}",
                zip_path.display(),
                archive.len()
            )));
        }

        let temp = TempDir::new("pet-import")?;
        let mut total: u64 = 0;
        for index in 0..archive.len() {
            let entry = archive
                .by_index(index)
                .map_err(|err| Error::Zip(format!("cannot read zip entry {index}: {err}")))?;
            let name = entry.name().to_string();
            let Some(relative) = entry.enclosed_name() else {
                return Err(Error::Zip(format!(
                    "zip entry '{name}' has an unsafe path (absolute or contains '..')"
                )));
            };
            for component in relative.components() {
                match component {
                    std::path::Component::Normal(_) | std::path::Component::CurDir => {}
                    _ => {
                        return Err(Error::Zip(format!(
                            "zip entry '{name}' has an unsafe path (absolute or contains '..')"
                        )))
                    }
                }
            }
            if entry.is_dir() {
                std::fs::create_dir_all(temp.path().join(&relative))?;
                continue;
            }
            if entry.size() > MAX_TOTAL_BYTES.saturating_sub(total) {
                return Err(Error::Zip(format!(
                    "{} expands to more than 64 MiB; refusing to import",
                    zip_path.display()
                )));
            }
            let dest = temp.path().join(&relative);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = std::fs::File::create(&dest)?;
            let declared = entry.size();
            let mut limited = entry.take(declared.saturating_add(1));
            let copied = std::io::copy(&mut limited, &mut out)?;
            if copied > declared || copied > MAX_TOTAL_BYTES.saturating_sub(total) {
                return Err(Error::Zip(format!(
                    "zip entry '{name}' is larger than declared or exceeds the 64 MiB import limit"
                )));
            }
            total += copied;
        }

        self.import_dir(temp.path(), overwrite)
    }
}

/// Minimal self-cleaning temporary directory (the `tempfile` crate is only a
/// dev-dependency of this crate).
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Result<Self> {
        let path = std::env::temp_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Copy only the files that make up a pet package (manifest + spritesheet +
/// optional public assets), skipping development artefacts and hidden files.
fn copy_pet_package(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "node_modules" || name == "target" {
            continue;
        }
        let from = entry.path();
        let to = dest.join(&name);
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let from = entry.path();
        let to = dest.join(&name);
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    fn write_pet(dir: &Path, id: &str, rows: u32, manifest_extra: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let frame = FrameSpec::new(8, rows);
        let img = RgbaImage::new(frame.atlas_width(), frame.atlas_height());
        img.save(dir.join("spritesheet.webp")).unwrap();
        let json = format!(
            r#"{{"id":"{id}","displayName":"Test {id}","spritesheetPath":"spritesheet.webp"{manifest_extra}}}"#
        );
        std::fs::write(dir.join("pet.json"), json).unwrap();
    }

    #[test]
    fn discovers_and_validates_pets() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("app-pets");
        write_pet(&app.join("alpha"), "alpha", 9, "");
        write_pet(&app.join("beta"), "beta", 11, r#","spriteVersionNumber":2"#);
        // Invalid package: manifest only, no spritesheet.
        std::fs::create_dir_all(app.join("broken")).unwrap();
        std::fs::write(app.join("broken/pet.json"), r#"{"id":"broken"}"#).unwrap();

        let lib = PetLibrary::discover(app.clone());
        let list = lib.list();
        // `discover` also adds the real ~/.codex and ~/.unipet roots, so only
        // the app-local entries are asserted here.
        let local: Vec<_> = list
            .iter()
            .filter(|pet| pet.root == RootKind::AppData)
            .collect();
        assert_eq!(local.len(), 2);
        assert!(local.iter().any(|p| p.id == "alpha"));
        assert_eq!(lib.get("beta").unwrap().frame.rows, 11);

        let report = PetLibrary::validate_dir(&app.join("alpha"));
        assert!(report.ok, "{:?}", report.errors);
        assert_eq!(report.image_size, Some((1536, 1872)));
        assert!(report.animations.iter().any(|a| a.state == PetState::Idle));

        let broken = PetLibrary::validate_dir(&app.join("broken"));
        assert!(!broken.ok);
    }

    #[test]
    fn imports_into_writable_root_and_dedupes() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("app-pets");
        let src = tmp.path().join("source/alpha");
        write_pet(&src, "alpha", 9, "");
        let lib = PetLibrary::discover(app.clone());
        let imported = lib.import_dir(&src, false).unwrap();
        assert_eq!(imported.root, RootKind::AppData);
        assert!(app.join("alpha/pet.json").is_file());
        assert!(lib.import_dir(&src, false).is_err());
        assert!(lib.import_dir(&src, true).is_ok());
        lib.remove_local("alpha").unwrap();
        assert!(lib.get("alpha").is_none());
    }

    #[test]
    fn skips_path_traversal_spritesheet() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("evil");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("pet.json"),
            r#"{"id":"evil","spritesheetPath":"../../secret.webp"}"#,
        )
        .unwrap();
        let report = PetLibrary::validate_dir(&dir);
        assert!(!report.ok);
        assert!(report.errors.iter().any(|e| e.contains("..")));
    }

    #[test]
    fn exports_and_imports_zip_round_trip() {
        use std::io::Read;

        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("app-pets");
        let src = tmp.path().join("source/alpha");
        write_pet(
            &src,
            "alpha",
            9,
            r#","description":"你好，世界","spriteVersionNumber":2"#,
        );

        let lib = PetLibrary::discover(app.clone());
        let imported = lib.import_dir(&src, false).unwrap();
        let out = tmp.path().join("out/alpha.zip");
        lib.export_zip("alpha", &out).unwrap();
        assert!(out.is_file());

        let mut archive = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
        let names: Vec<String> = (0..archive.len())
            .map(|index| archive.by_index(index).unwrap().name().to_string())
            .collect();
        assert_eq!(names, vec!["pet.json", "spritesheet.webp"]);

        let mut manifest_text = String::new();
        archive
            .by_index(0)
            .unwrap()
            .read_to_string(&mut manifest_text)
            .unwrap();
        assert!(!manifest_text.starts_with('\u{feff}'));
        assert!(manifest_text.contains("\n  \"id\": \"alpha\""));
        let manifest = PetManifest::from_json_str(&manifest_text).unwrap();
        assert_eq!(manifest.id, "alpha");
        assert_eq!(manifest.description.as_deref(), Some("你好，世界"));
        drop(archive);

        let other = PetLibrary::discover(tmp.path().join("other-app-pets"));
        let reimported = other.import_zip(&out, false).unwrap();
        assert_eq!(reimported.id, "alpha");
        assert_eq!(reimported.display_name, imported.display_name);
        assert_eq!(
            std::fs::read(&reimported.spritesheet).unwrap(),
            std::fs::read(&imported.spritesheet).unwrap()
        );
    }

    #[test]
    fn rejects_zip_path_traversal() {
        use std::io::Write;

        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("evil.zip");
        {
            let mut writer = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("../evil.txt", options).unwrap();
            writer.write_all(b"pwned").unwrap();
            writer.start_file("pet.json", options).unwrap();
            writer
                .write_all(br#"{"id":"evil","spritesheetPath":"spritesheet.webp"}"#)
                .unwrap();
            writer.finish().unwrap();
        }
        let lib = PetLibrary::discover(tmp.path().join("app-pets"));
        let err = lib.import_zip(&zip_path, false).unwrap_err();
        assert!(
            err.to_string().contains("unsafe"),
            "unexpected error: {err}"
        );
        assert!(!tmp.path().join("evil.txt").exists());
    }

    #[test]
    fn rejects_zip_with_too_many_entries() {
        use std::io::Write;

        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("many.zip");
        {
            let mut writer = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            for index in 0..65 {
                writer.start_file(format!("f{index}.txt"), options).unwrap();
                writer.write_all(b"x").unwrap();
            }
            writer.finish().unwrap();
        }
        let lib = PetLibrary::discover(tmp.path().join("app-pets"));
        let err = lib.import_zip(&zip_path, false).unwrap_err();
        assert!(err.to_string().contains("limit"), "unexpected error: {err}");
    }
}
