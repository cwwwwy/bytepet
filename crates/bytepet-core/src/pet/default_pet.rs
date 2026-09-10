//! The pet that ships with the app.
//!
//! A fresh install must show something even when the user has never installed a
//! Codex pet, so a self-authored atlas is embedded in the binary and written to
//! the writable library. It stays available next to the user's own pets; only an
//! explicit delete opts out (see `AppConfig::bundled_pet_removed`).

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

/// Make sure the bundled pet exists in the writable library.
///
/// `user_removed` is the opt-out recorded when the pet is deleted from the
/// local library; it is the only case where nothing is written. The bundled
/// pet is otherwise installed even when the user already owns Codex, UniPet or
/// imported pets, so it is always there to fall back on.
pub fn ensure_installed(library: &PetLibrary, user_removed: bool) -> Result<Option<PetEntry>> {
    if user_removed {
        return Ok(None);
    }
    let root = library.app_root()?;
    if root.path.join(DEFAULT_PET_ID).join("pet.json").is_file() {
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
    fn installs_when_the_library_is_empty() {
        let (_tmp, library) = temp_library();
        let seeded = ensure_installed(&library, false)
            .unwrap()
            .expect("installs when empty");
        assert_eq!(seeded.id, DEFAULT_PET_ID);
        assert!(seeded.dir.join("pet.json").is_file());

        // Second call: it is already there, so nothing is rewritten.
        assert!(ensure_installed(&library, false).unwrap().is_none());
        assert_eq!(library.list().len(), 1);
    }

    #[test]
    fn installs_next_to_the_users_own_pets() {
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

        // The user already has a pet, but the bundled one is still installed so
        // it can be switched back to.
        let installed = ensure_installed(&library, false).unwrap();
        assert_eq!(
            installed.map(|pet| pet.id),
            Some(DEFAULT_PET_ID.to_string())
        );
        assert!(tmp.path().join("pets").join(DEFAULT_PET_ID).exists());
        assert_eq!(library.list().len(), 2);
    }

    #[test]
    fn respects_an_explicit_removal() {
        let (_tmp, library) = temp_library();
        assert!(ensure_installed(&library, true).unwrap().is_none());
        assert!(library.list().is_empty());

        // ...and the flag only suppresses the bundled pet.
        let installed = ensure_installed(&library, false).unwrap();
        assert!(installed.is_some());
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
