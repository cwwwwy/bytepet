//! Integration tests for the Codex pet format: geometry, V2 look rows,
//! UniPet animation overrides, path safety and (when available) the real pet
//! installed on this machine.

use std::path::Path;

use bytepet_core::pet::library::{PetLibrary, RootKind};
use bytepet_core::pet::manifest::FrameSpec;
use bytepet_core::pet::state::PetState;
use bytepet_core::pet::PetAtlas;
use image::{Rgba, RgbaImage};

fn write_pet(dir: &Path, manifest_json: &str, frame: FrameSpec, paint: bool) {
    std::fs::create_dir_all(dir).unwrap();
    let mut img = RgbaImage::new(frame.atlas_width(), frame.atlas_height());
    if paint {
        // Paint the first cell so the alpha mask has something to hit.
        for y in 0..40 {
            for x in 0..40 {
                img.put_pixel(x, y, Rgba([255, 0, 0, 255]));
            }
        }
    }
    img.save(dir.join("spritesheet.webp")).unwrap();
    std::fs::write(dir.join("pet.json"), manifest_json).unwrap();
}

#[test]
fn v1_pet_uses_official_timing_and_has_no_look_rows() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("v1");
    write_pet(
        &dir,
        r#"{"id":"v1","displayName":"V1","spritesheetPath":"spritesheet.webp"}"#,
        FrameSpec::new(8, 9),
        true,
    );

    let report = PetLibrary::validate_dir(&dir);
    assert!(report.ok, "{:?}", report.errors);
    assert_eq!(report.image_size, Some((1536, 1872)));
    assert_eq!(report.frame, FrameSpec::new(8, 9));

    let entry = PetLibrary::load_entry(&dir, RootKind::AppData).unwrap();
    let idle = entry.animations.get(&PetState::Idle).unwrap();
    assert_eq!(idle.sprites, vec![0, 1, 2, 3, 4, 5]);
    assert_eq!(
        idle.durations_ms,
        vec![280.0, 110.0, 110.0, 140.0, 140.0, 320.0]
    );
    assert!(idle.loop_anim);
    assert!(!entry.animations.contains_key(&PetState::LookRow9));

    let engine = entry.engine();
    assert_eq!(engine.current(), PetState::Idle);
}

#[test]
fn v2_pet_infers_look_rows_and_hits_alpha_mask() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("v2");
    // Manifest deliberately omits spriteVersionNumber: rows must be inferred
    // from the image size.
    write_pet(
        &dir,
        r#"{"id":"v2","displayName":"V2","spritesheetPath":"spritesheet.webp"}"#,
        FrameSpec::new(8, 11),
        true,
    );

    let report = PetLibrary::validate_dir(&dir);
    assert!(report.ok, "{:?}", report.errors);
    assert_eq!(report.image_size, Some((1536, 2288)));
    assert_eq!(report.frame.rows, 11);
    assert!(
        report.warnings.iter().any(|w| w.contains("inferred")),
        "expected an inference warning, got {:?}",
        report.warnings
    );

    let entry = PetLibrary::load_entry(&dir, RootKind::AppData).unwrap();
    assert!(entry.animations.contains_key(&PetState::LookRow9));
    assert!(entry.animations.contains_key(&PetState::LookRow10));

    let (atlas, _) = PetAtlas::open(&dir, &entry.manifest).unwrap();
    assert!(atlas.opaque_at(0, 10.0, 10.0));
    assert!(!atlas.opaque_at(0, 150.0, 150.0));
    assert!(!atlas.opaque_at(8, 10.0, 10.0));
}

#[test]
fn unipet_animation_override_wins() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("unipet");
    write_pet(
        &dir,
        r#"{
          "id":"unipet","displayName":"Uni","spritesheetPath":"spritesheet.webp",
          "frame":{"width":192,"height":208,"columns":8,"rows":9},
          "animations":{
            "idle":{"frames":[{"spriteIndex":0,"durationMs":500},{"spriteIndex":8,"durationMs":250}],"loop":true},
            "waving":{"frames":[24,25,26,27],"fps":10,"loop":false,"fallback":"idle"}
          }
        }"#,
        FrameSpec::new(8, 9),
        false,
    );

    let entry = PetLibrary::load_entry(&dir, RootKind::AppData).unwrap();
    let idle = entry.animations.get(&PetState::Idle).unwrap();
    assert_eq!(idle.sprites, vec![0, 8]);
    assert_eq!(idle.durations_ms, vec![500.0, 250.0]);
    let waving = entry.animations.get(&PetState::Waving).unwrap();
    assert_eq!(waving.durations_ms, vec![100.0, 100.0, 100.0, 100.0]);
    assert!(!waving.loop_anim);
}

#[test]
fn rejects_traversal_and_bad_geometry() {
    let tmp = tempfile::tempdir().unwrap();

    let evil = tmp.path().join("evil");
    std::fs::create_dir_all(&evil).unwrap();
    std::fs::write(
        evil.join("pet.json"),
        r#"{"id":"evil","spritesheetPath":"../../../etc/hosts"}"#,
    )
    .unwrap();
    let report = PetLibrary::validate_dir(&evil);
    assert!(!report.ok);
    assert!(report.errors.iter().any(|e| e.contains("..")));

    let bad = tmp.path().join("bad");
    std::fs::create_dir_all(&bad).unwrap();
    RgbaImage::new(100, 100)
        .save(bad.join("spritesheet.webp"))
        .unwrap();
    std::fs::write(
        bad.join("pet.json"),
        r#"{"id":"bad","spritesheetPath":"spritesheet.webp"}"#,
    )
    .unwrap();
    let report = PetLibrary::validate_dir(&bad);
    assert!(!report.ok);
    assert!(report.errors.iter().any(|e| e.contains("cells")));
}

#[test]
fn loads_the_real_codex_pet_when_installed() {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let dir = home.join(".codex").join("pets").join("zip");
    if !dir.join("pet.json").is_file() {
        eprintln!("skipping: ~/.codex/pets/zip is not installed on this machine");
        return;
    }

    let report = PetLibrary::validate_dir(&dir);
    assert!(report.ok, "{:?}", report.errors);
    assert_eq!(report.image_size, Some((1536, 1872)));

    let entry = PetLibrary::load_entry(&dir, RootKind::Codex).unwrap();
    assert_eq!(entry.id, "zhenzhu-xiaozi");
    assert!(entry.linked, "Codex pets must stay linked, not copied");
    let idle = entry.animations.get(&PetState::Idle).unwrap();
    assert_eq!(idle.sprites.len(), 6);

    let (atlas, _) = PetAtlas::open(&dir, &entry.manifest).unwrap();
    // The real atlas must decode to the exact Codex geometry.
    assert_eq!(atlas.frame.atlas_width(), 1536);
    assert_eq!(atlas.frame.atlas_height(), 1872);
}
