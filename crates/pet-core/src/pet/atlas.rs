//! Spritesheet decoding, geometry validation and the alpha hit mask used for
//! per-pixel click-through.

use std::path::{Path, PathBuf};

use image::RgbaImage;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::pet::manifest::{safe_relative_path, FrameSpec, PetManifest};
use crate::pet::MAX_SPRITESHEET_BYTES;

/// Downscale factor of the alpha hit mask (1 mask sample per 4x4 px).
pub const MASK_SCALE: u32 = 4;
/// Alpha value at or above which a pixel counts as "solid" for hit testing.
pub const ALPHA_THRESHOLD: u8 = 8;

/// Compact per-cell alpha mask used for pixel-accurate click-through.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlphaMask {
    pub scale: u32,
    pub cell_width: u32,
    pub cell_height: u32,
    pub columns: u32,
    pub rows: u32,
    pub mask_width: u32,
    pub mask_height: u32,
    /// `columns * rows * mask_width * mask_height` bytes, 1 = solid.
    pub data: Vec<u8>,
}

impl AlphaMask {
    fn build(image: &RgbaImage, frame: FrameSpec) -> Self {
        let scale = MASK_SCALE;
        let mask_width = frame.width.div_ceil(scale).max(1);
        let mask_height = frame.height.div_ceil(scale).max(1);
        let cells = (frame.columns * frame.rows) as usize;
        let per_cell = (mask_width * mask_height) as usize;
        let mut data = vec![0u8; cells * per_cell];

        for row in 0..frame.rows {
            for col in 0..frame.columns {
                let cell_index = (row * frame.columns + col) as usize;
                let base_x = col * frame.width;
                let base_y = row * frame.height;
                let cell = &mut data[cell_index * per_cell..(cell_index + 1) * per_cell];
                for my in 0..mask_height {
                    for mx in 0..mask_width {
                        let mut solid = false;
                        let x0 = base_x + mx * scale;
                        let y0 = base_y + my * scale;
                        'block: for dy in 0..scale {
                            for dx in 0..scale {
                                let x = x0 + dx;
                                let y = y0 + dy;
                                if x >= image.width() || y >= image.height() {
                                    continue;
                                }
                                if image.get_pixel(x, y).0[3] >= ALPHA_THRESHOLD {
                                    solid = true;
                                    break 'block;
                                }
                            }
                        }
                        if solid {
                            cell[(my * mask_width + mx) as usize] = 1;
                        }
                    }
                }
            }
        }

        Self {
            scale,
            cell_width: frame.width,
            cell_height: frame.height,
            columns: frame.columns,
            rows: frame.rows,
            mask_width,
            mask_height,
            data,
        }
    }

    /// Is the given point inside the given cell opaque?
    ///
    /// `x`/`y` are logical pixels relative to the top-left of the cell.
    pub fn opaque_at_cell(&self, sprite_index: u32, x: f32, y: f32) -> bool {
        if self.columns == 0 || self.rows == 0 {
            return false;
        }
        let row = sprite_index / self.columns;
        let col = sprite_index % self.columns;
        if row >= self.rows || col >= self.columns {
            return false;
        }
        if x < 0.0 || y < 0.0 || x >= self.cell_width as f32 || y >= self.cell_height as f32 {
            return false;
        }
        let mx = (x / self.scale as f32).floor() as u32;
        let my = (y / self.scale as f32).floor() as u32;
        if mx >= self.mask_width || my >= self.mask_height {
            return false;
        }
        let cell_index = (row * self.columns + col) as usize;
        let per_cell = (self.mask_width * self.mask_height) as usize;
        let idx = cell_index * per_cell + (my * self.mask_width + mx) as usize;
        self.data.get(idx).copied().unwrap_or(0) != 0
    }

    /// True when every sampled pixel of the cell is transparent.
    pub fn cell_is_empty(&self, sprite_index: u32) -> bool {
        if self.columns == 0 {
            return true;
        }
        let row = sprite_index / self.columns;
        let col = sprite_index % self.columns;
        if row >= self.rows || col >= self.columns {
            return true;
        }
        let cell_index = (row * self.columns + col) as usize;
        let per_cell = (self.mask_width * self.mask_height) as usize;
        let slice = &self.data[cell_index * per_cell..(cell_index + 1) * per_cell];
        slice.iter().all(|b| *b == 0)
    }
}

/// A decoded pet spritesheet plus its hit mask.
pub struct PetAtlas {
    pub frame: FrameSpec,
    pub path: PathBuf,
    pub image: RgbaImage,
    pub mask: AlphaMask,
}

impl PetAtlas {
    /// Decode a spritesheet and validate its geometry.
    pub fn from_image(image: RgbaImage, frame: FrameSpec, path: PathBuf) -> Result<Self> {
        if image.width() != frame.atlas_width() || image.height() != frame.atlas_height() {
            return Err(Error::atlas(format!(
                "image is {}x{} px but the {}x{} grid needs {}x{} px",
                image.width(),
                image.height(),
                frame.columns,
                frame.rows,
                frame.atlas_width(),
                frame.atlas_height()
            )));
        }
        let mask = AlphaMask::build(&image, frame);
        Ok(Self {
            frame,
            path,
            image,
            mask,
        })
    }

    /// Resolve the manifest against a pet directory, decode the spritesheet and
    /// return the atlas plus any non-fatal warnings.
    pub fn open(pet_dir: &Path, manifest: &PetManifest) -> Result<(Self, Vec<String>)> {
        let rel = safe_relative_path(&manifest.spritesheet_rel())?;
        let path = pet_dir.join(&rel);
        let meta = std::fs::metadata(&path).map_err(|e| {
            Error::atlas(format!("spritesheet {} is missing: {e}", path.display()))
        })?;
        if !meta.is_file() {
            return Err(Error::atlas(format!(
                "spritesheet {} is not a file",
                path.display()
            )));
        }
        if meta.len() > MAX_SPRITESHEET_BYTES {
            return Err(Error::atlas(format!(
                "spritesheet is {:.1} MB which exceeds the {} MB limit",
                meta.len() as f64 / (1024.0 * 1024.0),
                MAX_SPRITESHEET_BYTES / (1024 * 1024)
            )));
        }

        let bytes = std::fs::read(&path)?;
        let decoded = image::load_from_memory(&bytes).map_err(|e| {
            Error::atlas(format!(
                "cannot decode {} (expected webp/png/jpeg): {e}",
                path.display()
            ))
        })?;
        let rgba = decoded.to_rgba8();
        let (frame, warnings) = manifest.resolve_frame(rgba.width(), rgba.height())?;
        let atlas = Self::from_image(rgba, frame, path)?;
        Ok((atlas, warnings))
    }

    pub fn sprite_index(&self, row: u32, col: u32) -> u32 {
        row * self.frame.columns + col
    }

    /// Logical pixel hit test for a sprite (global index) and cell-local point.
    pub fn opaque_at(&self, sprite_index: u32, x: f32, y: f32) -> bool {
        self.mask.opaque_at_cell(sprite_index, x, y)
    }

    /// Convenience: is the top-left pixel region of a cell non-empty?
    pub fn cell_is_empty(&self, sprite_index: u32) -> bool {
        self.mask.cell_is_empty(sprite_index)
    }

    /// Unused cells (beyond the state's frame list) should be transparent.
    pub fn unused_cell_warnings(&self, used: &[u32]) -> Vec<String> {
        let mut warnings = Vec::new();
        for index in 0..self.frame.cell_count() {
            if !used.contains(&index) && !self.cell_is_empty(index) {
                warnings.push(format!(
                    "cell {index} (row {}, col {}) is not part of any animation but is not transparent",
                    index / self.frame.columns,
                    index % self.frame.columns
                ));
            }
        }
        warnings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn synthetic(frame: FrameSpec) -> RgbaImage {
        let mut img = RgbaImage::new(frame.atlas_width(), frame.atlas_height());
        // Cell 0: opaque square in the top-left 20x20 px.
        for y in 0..20 {
            for x in 0..20 {
                img.put_pixel(x, y, Rgba([255, 0, 0, 255]));
            }
        }
        // Cell 1: fully transparent.
        img
    }

    #[test]
    fn mask_hits_only_opaque_pixels() {
        let frame = FrameSpec::new(2, 1);
        let atlas = PetAtlas::from_image(synthetic(frame), frame, PathBuf::from("x")).unwrap();
        assert!(atlas.opaque_at(0, 4.0, 4.0));
        assert!(!atlas.opaque_at(0, 100.0, 100.0));
        assert!(!atlas.opaque_at(1, 4.0, 4.0));
        assert!(atlas.cell_is_empty(1));
        assert!(!atlas.cell_is_empty(0));
    }

    #[test]
    fn rejects_wrong_geometry() {
        let frame = FrameSpec::new(2, 2);
        let img = RgbaImage::new(10, 10);
        assert!(PetAtlas::from_image(img, frame, PathBuf::from("x")).is_err());
    }

    #[test]
    fn sprite_index_matches_codex_layout() {
        let frame = FrameSpec::default();
        let atlas =
            PetAtlas::from_image(RgbaImage::new(frame.atlas_width(), frame.atlas_height()), frame, PathBuf::from("x"))
                .unwrap();
        assert_eq!(atlas.sprite_index(0, 0), 0);
        assert_eq!(atlas.sprite_index(1, 0), 8);
        assert_eq!(atlas.sprite_index(4, 3), 35);
    }
}
