//! The bundled demo library (PRD §11): loading, in-memory index, validation,
//! item selection.
//!
//! The `library/` folder of the repo is bundled as a Tauri resource and loaded
//! once at startup into an immutable [`Library`] (held in `AppState` behind an
//! `Arc`). Library version == app version.
//!
//! Ownership:
//! - [`loader`]: read `tenants.json` (separately, [`loader::load_tenants`]), `items/*.json`,
//!   `demos/*.json` and the floorplan layouts the demos reference. Bad files are skipped with a
//!   warning.
//! - [`validate`]: semantic checks beyond serde / JSON schema (id rules, lengths, limits,
//!   references), reported with file + JSON path. JSON-schema validation of every file runs in
//!   `cargo test` (`schema_tests`) and `npm run validate:library`.
//! - [`series`]: the demo editor's "generate series" helper.
//! - [`select`]: demo item selection `tags ∪ includes − excludes`, full-text search, filter chips.
//!
//! The serde file model lives in `crate::model::library` (it crosses IPC); the normative
//! file spec is `library/schema/*.schema.json`.

pub mod loader;
pub mod select;
pub mod series;
pub mod validate;

#[cfg(test)]
mod schema_tests;

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::model::{DemoPack, FloorplanLayout, LibraryItem, SlaveDefinition};

/// Where an item / slave / demo was loaded from: file relative to the library root and the
/// index in that file's array (for validation messages).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Origin {
    pub file: String,
    pub index: usize,
}

/// Origins, index-aligned with `Library::items` / `slaves` / `demos`.
#[derive(Debug, Clone, Default)]
pub struct Origins {
    pub items: Vec<Origin>,
    pub slaves: Vec<Origin>,
    pub demos: Vec<Origin>,
}

/// Loaded, immutable library.
#[derive(Debug, Clone, Default)]
pub struct Library {
    /// Absolute path of the bundled `library/` folder.
    pub root: PathBuf,
    /// == app version.
    pub version: String,
    pub items: Vec<LibraryItem>,
    pub slaves: Vec<SlaveDefinition>,
    pub demos: Vec<DemoPack>,
    /// Floorplan layouts referenced by demos, keyed by their path relative to `root`
    /// (as written in `DemoPack::floorplans`).
    pub floorplans: BTreeMap<String, FloorplanLayout>,
    /// Source file of every item / slave / demo.
    pub origins: Origins,
    /// Non-fatal load / validation findings.
    pub warnings: Vec<String>,
}

impl Library {
    pub fn demo(&self, id: &str) -> Option<&DemoPack> {
        self.demos.iter().find(|d| d.id == id)
    }

    pub fn item(&self, id: &str) -> Option<&LibraryItem> {
        self.items.iter().find(|i| i.id() == id)
    }

    pub fn slave(&self, key: &str) -> Option<&SlaveDefinition> {
        self.slaves.iter().find(|s| s.key == key)
    }

    pub fn floorplan(&self, path: &str) -> Option<&FloorplanLayout> {
        self.floorplans.get(path)
    }

    /// Library items of a demo (`tags ∪ includes − excludes`, library order).
    pub fn demo_items(&self, demo: &DemoPack) -> Vec<&LibraryItem> {
        select::select_items(&self.items, demo)
    }
}
