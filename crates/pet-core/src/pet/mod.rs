//! Codex-compatible desktop pet format.
//!
//! A pet is a directory containing a manifest (`pet.json`) and one spritesheet
//! image. The canonical atlas is 8 columns x 9 rows of 192x208 px cells
//! (1536x1872). Codex "V2" pets add two look-direction rows (8x11, 1536x2288).
//! UniPet-style manifests with an explicit `frame`/`animations` block are also
//! accepted, so a single pet package works across compatible tools.

pub mod atlas;
pub mod library;
pub mod manifest;
pub mod state;

pub use atlas::{AlphaMask, PetAtlas};
pub use library::{LibraryRoot, PetEntry, PetLibrary, RootKind, ValidationReport};
pub use manifest::{AnimationTrack, FrameRef, FrameSpec, PetManifest};
pub use state::{Animation, PetEngine, PetState, Transition};

/// Maximum accepted spritesheet size (bytes). Matches UniPet's safety rule.
pub const MAX_SPRITESHEET_BYTES: u64 = 16 * 1024 * 1024;

/// Default cell size used by the Codex pet atlas.
pub const DEFAULT_CELL_WIDTH: u32 = 192;
pub const DEFAULT_CELL_HEIGHT: u32 = 208;
