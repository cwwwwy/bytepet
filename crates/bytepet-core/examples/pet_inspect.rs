//! Developer tool: inventory a Codex pet spritesheet.
//!
//! Prints the resolved geometry, the cells that contain pixels in every
//! animation row and the animation table the engine derives from it. With an
//! output directory it also writes one PNG per row so the artwork can be
//! compared against the documented Codex row semantics.
//!
//! ```text
//! cargo run -p bytepet-core --example pet_inspect -- <pet-dir> [out-dir]
//! ```

use std::path::{Path, PathBuf};

use bytepet_core::pet::state::{PetEngine, PetState};
use bytepet_core::pet::{PetAtlas, PetManifest};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let pet_dir = PathBuf::from(args.next().unwrap_or_else(|| {
        eprintln!("usage: pet_inspect <pet-dir> [out-dir]");
        std::process::exit(2);
    }));
    let out_dir = args.next().map(PathBuf::from);

    let manifest_text = std::fs::read_to_string(pet_dir.join("pet.json"))?;
    let manifest = PetManifest::from_json_str(&manifest_text)?;
    let (atlas, warnings) = PetAtlas::open(&pet_dir, &manifest)?;

    println!("pet        : {}", manifest.id);
    println!(
        "display    : {}",
        manifest.display_name.as_deref().unwrap_or("(none)")
    );
    println!(
        "declared   : spriteVersionNumber={:?}",
        manifest.sprite_version_number
    );
    println!("image      : {}", atlas.path.display());
    println!(
        "geometry   : {}x{} px, {} columns x {} rows, cell {}x{}",
        atlas.image.width(),
        atlas.image.height(),
        atlas.frame.columns,
        atlas.frame.rows,
        atlas.frame.width,
        atlas.frame.height
    );
    for warning in &warnings {
        println!("warning    : {warning}");
    }

    println!("\nrow occupancy (cells that contain any pixel):");
    for row in 0..atlas.frame.rows {
        let mut occupied = Vec::new();
        for col in 0..atlas.frame.columns {
            let index = row * atlas.frame.columns + col;
            if !atlas.cell_is_empty(index) {
                occupied.push(col);
            }
        }
        println!("  row {row:>2}: {occupied:?}");
    }

    let engine = PetEngine::from_atlas(&atlas, &manifest);
    println!("\nresolved animations:");
    for state in PetState::ALL {
        if let Some(animation) = engine.animation(state) {
            println!(
                "  {:<14} row {:>2}  frames {}  loop={}  fallback={}",
                animation.state.name(),
                animation.row,
                animation.durations_ms.len(),
                animation.loop_anim,
                animation.fallback.name()
            );
        }
    }

    if let Some(out_dir) = out_dir {
        std::fs::create_dir_all(&out_dir)?;
        for row in 0..atlas.frame.rows {
            write_row(&atlas, row, &out_dir)?;
        }
        println!("\nwrote row PNGs to {}", out_dir.display());
    }
    Ok(())
}

fn write_row(atlas: &PetAtlas, row: u32, out_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let frame = atlas.frame;
    let mut sheet = image::RgbaImage::from_pixel(
        frame.width * frame.columns,
        frame.height,
        image::Rgba([0, 0, 0, 0]),
    );
    for col in 0..frame.columns {
        for y in 0..frame.height {
            for x in 0..frame.width {
                let pixel = atlas
                    .image
                    .get_pixel(col * frame.width + x, row * frame.height + y);
                sheet.put_pixel(col * frame.width + x, y, *pixel);
            }
        }
    }
    let path = out_dir.join(format!("row-{row:02}.png"));
    sheet.save(&path)?;
    Ok(())
}
