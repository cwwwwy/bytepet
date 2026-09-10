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

    /// Like [`Self::opaque_at_cell`], but a point also counts as solid when a
    /// sample within `radius` mask cells of it is solid.
    ///
    /// Pixel-perfect click-through is great until the cursor is one pixel off
    /// an anti-aliased edge, so the window uses a one-cell halo (4 logical
    /// pixels) when deciding whether to keep receiving mouse input.
    pub fn opaque_at_cell_dilated(&self, sprite_index: u32, x: f32, y: f32, radius: u32) -> bool {
        if self.opaque_at_cell(sprite_index, x, y) {
            return true;
        }
        if radius == 0 || self.columns == 0 || self.rows == 0 {
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
        let step = self.scale.max(1) as f32;
        let offsets = radius as i32;
        for oy in -offsets..=offsets {
            for ox in -offsets..=offsets {
                if ox == 0 && oy == 0 {
                    continue;
                }
                let nx = x + ox as f32 * step;
                let ny = y + oy as f32 * step;
                if nx < 0.0
                    || ny < 0.0
                    || nx >= self.cell_width as f32
                    || ny >= self.cell_height as f32
                {
                    continue;
                }
                if self.opaque_at_cell(sprite_index, nx, ny) {
                    return true;
                }
            }
        }
        false
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
        let meta = std::fs::metadata(&path)
            .map_err(|e| Error::atlas(format!("spritesheet {} is missing: {e}", path.display())))?;
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

    /// `columns * rows` bitmap: `true` when the cell holds at least one opaque
    /// pixel. The animation engine uses it to follow the frames a pet actually
    /// drew instead of assuming the art fills the whole grid.
    pub fn occupancy(&self) -> Vec<bool> {
        (0..self.frame.cell_count())
            .map(|index| !self.cell_is_empty(index))
            .collect()
    }

    /// Crop one sprite to the drawn content and scale it into a square RGBA
    /// icon, so windows and the tray can show the actual pet.
    pub fn icon_rgba(&self, sprite_index: u32, size: u32) -> Option<Vec<u8>> {
        let size = size.max(4);
        let frame = self.frame;
        let columns = frame.columns.max(1);
        let row = sprite_index / columns;
        let col = sprite_index % columns;
        if row >= frame.rows || col >= columns {
            return None;
        }

        let (mut min_x, mut min_y) = (frame.width, frame.height);
        let (mut max_x, mut max_y) = (0_u32, 0_u32);
        for y in 0..frame.height {
            for x in 0..frame.width {
                let pixel = self
                    .image
                    .get_pixel(col * frame.width + x, row * frame.height + y);
                if pixel.0[3] >= ALPHA_THRESHOLD {
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }
            }
        }
        if min_x > max_x || min_y > max_y {
            return None;
        }

        // Center the drawn content in a square so the icon keeps its aspect.
        let content_w = max_x - min_x + 1;
        let content_h = max_y - min_y + 1;
        let radius = (content_w.max(content_h) / 2 + 2) as i64;
        let center_x = (min_x + content_w / 2) as i64;
        let center_y = (min_y + content_h / 2) as i64;
        let side = (radius * 2) as u32;
        let mut square = RgbaImage::new(side, side);
        for y in 0..side {
            for x in 0..side {
                let src_x = center_x - radius + x as i64;
                let src_y = center_y - radius + y as i64;
                if src_x < 0
                    || src_y < 0
                    || src_x >= frame.width as i64
                    || src_y >= frame.height as i64
                {
                    continue;
                }
                let pixel = self.image.get_pixel(
                    col * frame.width + src_x as u32,
                    row * frame.height + src_y as u32,
                );
                square.put_pixel(x, y, *pixel);
            }
        }
        let resized =
            image::imageops::resize(&square, size, size, image::imageops::FilterType::Triangle);
        Some(resized.into_raw())
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
    fn dilated_hits_cover_the_antialiased_edge() {
        let frame = FrameSpec::new(2, 1);
        let atlas = PetAtlas::from_image(synthetic(frame), frame, PathBuf::from("x")).unwrap();
        // The 20x20 px square ends at x=19; a point just outside it is only a
        // hit when the halo is applied.
        assert!(!atlas.mask.opaque_at_cell(0, 22.0, 10.0));
        assert!(atlas.mask.opaque_at_cell_dilated(0, 22.0, 10.0, 1));
        // The halo never leaks into a fully transparent sprite.
        assert!(!atlas.mask.opaque_at_cell_dilated(1, 22.0, 10.0, 1));
        // A radius of zero keeps the exact mask.
        assert!(!atlas.mask.opaque_at_cell_dilated(0, 22.0, 10.0, 0));
    }

    #[test]
    fn icon_crops_the_drawn_content() {
        let frame = FrameSpec::new(2, 1);
        let atlas = PetAtlas::from_image(synthetic(frame), frame, PathBuf::from("x")).unwrap();
        let icon = atlas.icon_rgba(0, 32).expect("cell 0 has content");
        assert_eq!(icon.len(), 32 * 32 * 4);
        // The 20x20 px square sits in the top-left corner, so the square crop is
        // mostly transparent on the opposite side.
        let bottom_right = &icon[(31 * 32 + 31) * 4..][..4];
        assert_eq!(bottom_right[3], 0);
        let center = &icon[(16 * 32 + 16) * 4..][..4];
        assert_eq!(center[3], 255);
        // A fully transparent sprite yields no icon at all.
        assert!(atlas.icon_rgba(1, 32).is_none());
    }

    #[test]
    fn sprite_index_matches_codex_layout() {
        let frame = FrameSpec::default();
        let atlas = PetAtlas::from_image(
            RgbaImage::new(frame.atlas_width(), frame.atlas_height()),
            frame,
            PathBuf::from("x"),
        )
        .unwrap();
        assert_eq!(atlas.sprite_index(0, 0), 0);
        assert_eq!(atlas.sprite_index(1, 0), 8);
        assert_eq!(atlas.sprite_index(4, 3), 35);
    }
}
