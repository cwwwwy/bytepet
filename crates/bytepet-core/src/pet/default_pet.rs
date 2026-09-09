//! The pet that ships with the app.
//!
//! A fresh install must show something even when the user has never installed a
//! Codex pet, so a self-authored atlas is embedded in the binary and written to
//! the writable library exactly once (see `AppConfig::default_pet_seeded`).

use std::path::Path;

use crate::error::Result;
use crate::pet::library::{PetEntry, PetLibrary, RootKind};

/// Pet id of the bundled character.
pub const DEFAULT_PET_ID: &str = "bytepet-default";
/// Manifest of the bundled character.
pub const DEFAULT_PET_MANIFEST: &str = include_str!("../../assets/default-pet/pet.json");
/// Spritesheet of the bundled character (8x9 Codex atlas, 1536x1872).
pub const DEFAULT_PET_SPRITESHEET: &[u8] =
    include_bytes!("../../assets/default-pet/spritesheet.png");

/// File name of the bundled spritesheet, derived from the manifest.
fn spritesheet_name() -> String {
    crate::pet::manifest::PetManifest::from_json_str(DEFAULT_PET_MANIFEST)
        .map(|manifest| manifest.spritesheet_rel().to_string_lossy().to_string())
        .unwrap_or_else(|_| "spritesheet.png".to_string())
}

/// Write the bundled pet into the writable library (idempotent).
pub fn install(library: &PetLibrary) -> Result<PetEntry> {
    let root = library.app_root()?;
    let dir = root.path.join(DEFAULT_PET_ID);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("pet.json"), DEFAULT_PET_MANIFEST)?;
    std::fs::write(dir.join(spritesheet_name()), DEFAULT_PET_SPRITESHEET)?;
    PetLibrary::load_entry(&dir, RootKind::AppData)
}

/// Seed the bundled pet when the user has no pets at all.
///
/// Returns the installed entry only when seeding actually happened. An empty
/// result means the user already has a pet (Codex, UniPet or local) and nothing
/// was written.
pub fn seed_if_empty(library: &PetLibrary) -> Result<Option<PetEntry>> {
    if !library.list().is_empty() {
        return Ok(None);
    }
    Ok(Some(install(library)?))
}

/// True when the given directory is the bundled pet.
pub fn is_default_pet(dir: &Path) -> bool {
    dir.file_name()
        .map(|name| name.to_string_lossy() == DEFAULT_PET_ID)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_library() -> (tempfile::TempDir, PetLibrary) {
        let tmp = tempfile::tempdir().unwrap();
        let library = PetLibrary::single_root(tmp.path().join("pets"));
        (tmp, library)
    }

    #[test]
    fn bundled_manifest_is_valid_and_matches_the_asset() {
        let manifest = crate::pet::manifest::PetManifest::from_json_str(DEFAULT_PET_MANIFEST)
            .expect("bundled manifest parses");
        assert_eq!(manifest.id, DEFAULT_PET_ID);
        assert_eq!(
            manifest.spritesheet_rel().to_string_lossy(),
            spritesheet_name()
        );

        // The embedded atlas must decode to the exact Codex V1 geometry.
        let image = image::load_from_memory(DEFAULT_PET_SPRITESHEET)
            .expect("bundled spritesheet decodes")
            .to_rgba8();
        let (frame, warnings) = manifest
            .resolve_frame(image.width(), image.height())
            .expect("geometry resolves");
        assert!(warnings.is_empty(), "no geometry warnings: {warnings:?}");
        assert_eq!((frame.columns, frame.rows), (8, 9));
        assert_eq!((image.width(), image.height()), (1536, 1872));

        let atlas = crate::pet::PetAtlas::from_image(image, frame, DEFAULT_PET_ID.into())
            .expect("atlas builds");
        // idle frame 0 must contain visible pixels, or the pet would be invisible.
        assert!(!atlas.cell_is_empty(0), "idle frame 0 is not transparent");
    }

    #[test]
    fn seeds_only_when_the_library_is_empty() {
        let (_tmp, library) = temp_library();
        let seeded = seed_if_empty(&library).unwrap().expect("seeds when empty");
        assert_eq!(seeded.id, DEFAULT_PET_ID);
        assert!(seeded.dir.join("pet.json").is_file());

        // Second call: the library is no longer empty.
        assert!(seed_if_empty(&library).unwrap().is_none());
        assert_eq!(library.list().len(), 1);
    }

    #[test]
    fn does_not_seed_when_the_user_already_has_a_pet() {
        let (tmp, library) = temp_library();
        let other = tmp.path().join("pets").join("user-pet");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(
            other.join("pet.json"),
            r#"{"id":"user-pet","displayName":"User","spritesheetPath":"spritesheet.png"}"#,
        )
        .unwrap();
        let frame = crate::pet::manifest::FrameSpec::new(8, 9);
        image::RgbaImage::new(frame.atlas_width(), frame.atlas_height())
            .save(other.join("spritesheet.png"))
            .unwrap();

        assert!(seed_if_empty(&library).unwrap().is_none());
        assert!(!tmp.path().join("pets").join(DEFAULT_PET_ID).exists());
        assert_eq!(library.list().len(), 1);
    }

    #[test]
    fn install_is_idempotent_and_repairs_missing_files() {
        let (_tmp, library) = temp_library();
        let entry = install(&library).unwrap();
        // Simulate a user or an update wiping the spritesheet.
        std::fs::remove_file(entry.dir.join(spritesheet_name())).unwrap();
        let repaired = install(&library).unwrap();
        assert!(repaired.spritesheet.is_file());
        assert_eq!(library.list().len(), 1);
        assert!(is_default_pet(&repaired.dir));
    }
}
