//! Load the library from disk.
//!
//! A broken item or demo file is skipped with a warning (shown via `library_info`).

use std::fs;
use std::path::{Path, PathBuf};

use super::{Library, Origin};
use crate::error::{AppError, AppResult};
use crate::model::{DemoPack, FloorplanLayout, ItemLibraryFile, LibraryItem};

/// `type` of `library/items/*.json`.
pub const ITEMS_TYPE: &str = "lw-demo-items";
/// `type` of `library/demos/*.json`.
pub const DEMO_TYPE: &str = "lw-demo";
/// `type` of `library/floorplans/<demoId>/*.json`.
pub const FLOORPLAN_TYPE: &str = "lw-demo-floorplan";
/// The only supported `schemaVersion` of every library file.
pub const SCHEMA_VERSION: u32 = 1;

/// Load items and demos of `root` (the `library/` folder). Unreadable, unparsable or wrongly
/// typed files are skipped and reported in `Library::warnings`; semantic findings are added by
/// [`super::validate`].
pub fn load(root: &Path, version: &str) -> AppResult<Library> {
    if !root.is_dir() {
        return Err(AppError::Library(format!(
            "{}: not a folder",
            root.display()
        )));
    }
    let mut lib = Library {
        root: root.to_path_buf(),
        version: version.to_owned(),
        ..Default::default()
    };
    let mut warnings = Vec::new();

    for path in json_files(&root.join("items")) {
        match read_json::<ItemLibraryFile>(&path).and_then(|f| {
            check_header(&path, &f.file_type, ITEMS_TYPE, f.schema_version)?;
            Ok(f)
        }) {
            Ok(file) => {
                let rel = relative(root, &path);
                for index in 0..file.slaves.len() {
                    lib.origins.slaves.push(Origin {
                        file: rel.clone(),
                        index,
                    });
                }
                for index in 0..file.items.len() {
                    lib.origins.items.push(Origin {
                        file: rel.clone(),
                        index,
                    });
                }
                lib.slaves.extend(file.slaves);
                lib.items.extend(file.items);
            }
            Err(e) => warnings.push(format!("skipped {}: {e}", display_name(&path))),
        }
    }
    for path in json_files(&root.join("demos")) {
        match read_json::<DemoPack>(&path).and_then(|d| {
            check_header(&path, &d.file_type, DEMO_TYPE, d.schema_version)?;
            Ok(d)
        }) {
            Ok(demo) => {
                lib.origins.demos.push(Origin {
                    file: relative(root, &path),
                    index: 0,
                });
                lib.demos.push(demo);
            }
            Err(e) => warnings.push(format!("skipped {}: {e}", display_name(&path))),
        }
    }
    // Floorplan layouts referenced by the demos (a missing one is reported by `validate`).
    let referenced: Vec<String> = lib
        .demos
        .iter()
        .flat_map(|d| d.floorplans.iter().cloned())
        .collect();
    for rel in referenced {
        if lib.floorplans.contains_key(&rel) {
            continue;
        }
        let path = root.join(&rel);
        if !path.is_file() {
            continue;
        }
        match read_json::<FloorplanLayout>(&path).and_then(|f| {
            check_header(&path, &f.file_type, FLOORPLAN_TYPE, f.schema_version)?;
            Ok(f)
        }) {
            Ok(layout) => {
                lib.floorplans.insert(rel, layout);
            }
            Err(e) => warnings.push(format!("skipped {rel}: {e}")),
        }
    }

    resolve_defaults(&mut lib);
    warnings.extend(orphan_floorplans(root, &lib));

    warnings.extend(
        super::validate::validate(&lib)
            .into_iter()
            .map(|f| f.to_string()),
    );
    lib.warnings = warnings;
    Ok(lib)
}

/// Fill in what a demo may omit: a digital input's `index` (taken from its item) and
/// `daliLampCount` (highest converter number in `daliGroups` or selected lamp index, at least 1). After
/// this the demos carry explicit values, so everything downstream (validation, apply, export)
/// sees the same numbers. Values that cannot be derived stay 0 and are reported by validation.
fn resolve_defaults(lib: &mut Library) {
    // (demo, derived lamp count, (DI entry, derived index) pairs)
    type Fix = (usize, u32, Vec<(usize, u32)>);
    let mut fixes: Vec<Fix> = Vec::new();
    for (i, demo) in lib.demos.iter().enumerate() {
        let selected = super::select::select_items(&lib.items, demo);
        let lamps = if demo.dali_lamp_count == 0 {
            let grouped = demo
                .dali_groups
                .iter()
                .flat_map(|g| g.converters.iter().copied());
            let items = selected.iter().filter_map(|it| match it {
                LibraryItem::Dali { index, .. } => Some(*index),
                _ => None,
            });
            grouped.chain(items).max().unwrap_or(1)
        } else {
            demo.dali_lamp_count
        };
        let indexes: Vec<(usize, u32)> = demo
            .digital_inputs
            .iter()
            .enumerate()
            .filter(|(_, di)| di.index == 0)
            .filter_map(|(j, di)| {
                let item = selected
                    .iter()
                    .copied()
                    .find(|it| it.id() == di.item_id)
                    .or_else(|| lib.item(&di.item_id))?;
                Some((j, item.index()?))
            })
            .collect();
        fixes.push((i, lamps, indexes));
    }
    for (i, lamps, indexes) in fixes {
        let demo = &mut lib.demos[i];
        demo.dali_lamp_count = lamps;
        for (j, index) in indexes {
            demo.digital_inputs[j].index = index;
        }
    }
}

/// Warnings for floorplan files no demo references and `floorplans/<dir>/` folders without a
/// demo of that id (their semantic references would never be checked).
fn orphan_floorplans(root: &Path, lib: &Library) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(dirs) = fs::read_dir(root.join("floorplans")) else {
        return out;
    };
    let mut dirs: Vec<PathBuf> = dirs
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for dir in dirs {
        let rel_dir = relative(root, &dir);
        let demo_id = display_name(&dir);
        if lib.demo(&demo_id).is_none() {
            out.push(format!(
                "{rel_dir}/: no demo with id \"{demo_id}\" uses this folder"
            ));
            continue;
        }
        for file in json_files(&dir) {
            let rel = relative(root, &file);
            if !lib.demos.iter().any(|d| d.floorplans.contains(&rel)) {
                out.push(format!(
                    "{rel}: not referenced by any demo (add it to `floorplans`)"
                ));
            }
        }
    }
    out
}

fn check_header(
    path: &Path,
    file_type: &str,
    expected: &str,
    schema_version: u32,
) -> AppResult<()> {
    if file_type != expected {
        return Err(AppError::Library(format!(
            "{}: type is \"{file_type}\", expected \"{expected}\"",
            display_name(path)
        )));
    }
    if schema_version != SCHEMA_VERSION {
        return Err(AppError::Library(format!(
            "{}: schemaVersion {schema_version} not supported (expected {SCHEMA_VERSION})",
            display_name(path)
        )));
    }
    Ok(())
}

/// `path` relative to `root` with `/` separators (as used in `DemoPack::floorplans`).
fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> AppResult<T> {
    let text = fs::read_to_string(path)
        .map_err(|e| AppError::Library(format!("{}: {e}", path.display())))?;
    serde_json::from_str(&text).map_err(|e| AppError::Library(format!("{}: {e}", path.display())))
}

/// Sorted `*.json` files of a directory; missing or unreadable directory = empty.
fn json_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = read
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_library() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../library")
    }

    /// The repo's own `library/` must always load.
    #[test]
    fn loads_repo_library() {
        let lib = load(&repo_library(), "0.0.0").expect("library loads");
        assert!(lib.warnings.is_empty(), "{:?}", lib.warnings);
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lw-sim-lib-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("items")).unwrap();
        fs::create_dir_all(d.join("demos")).unwrap();
        d
    }

    const GOOD_ITEMS: &str = r#"{"type":"lw-demo-items","schemaVersion":1,"items":[
        {"kind":"dali","id":"light-1","index":1,"title":{"de":"Licht 1","en":"Light 1"}}]}"#;

    #[test]
    fn bad_files_are_skipped_with_a_warning() {
        let root = scratch("bad");
        fs::write(root.join("items/good.json"), GOOD_ITEMS).unwrap();
        fs::write(root.join("items/broken.json"), "{ nope").unwrap();
        fs::write(
            root.join("items/wrong-type.json"),
            r#"{"type":"lw-demo","schemaVersion":1,"items":[]}"#,
        )
        .unwrap();
        fs::write(
            root.join("items/future.json"),
            r#"{"type":"lw-demo-items","schemaVersion":2,"items":[]}"#,
        )
        .unwrap();
        // A missing kind-specific field (Modbus item without `slave`) is a parse error too.
        fs::write(
            root.join("items/no-slave.json"),
            r#"{"type":"lw-demo-items","schemaVersion":1,"items":[
                {"kind":"modbus","id":"x","title":{"de":"x","en":"x"}}]}"#,
        )
        .unwrap();
        fs::write(root.join("demos/broken.json"), "[]").unwrap();

        let lib = load(&root, "0.0.0").unwrap();
        assert_eq!(lib.items.len(), 1);
        assert_eq!(lib.items[0].id(), "light-1");
        assert_eq!(lib.warnings.len(), 5, "{:?}", lib.warnings);
        assert!(lib.warnings.iter().any(|w| w.contains("broken.json")));
        assert!(lib.warnings.iter().any(|w| w.contains("schemaVersion 2")));
        let _ = fs::remove_dir_all(&root);
    }

    const MINI_DEMO: &str = r#"{"type":"lw-demo","schemaVersion":1,"id":"mini",
        "name":{"de":"M","en":"M"},"description":{"de":"M","en":"M"},"tags":["mini"],
        "daliGroups":[{"id":1,"name":"G","converters":[1],"minLevel":0,"maxLevel":100}],
        "digitalInputs":[{"itemId":"di-a","type":"measurement"}],
        "sendIntervalSec":30}"#;

    const MINI_ITEMS: &str = r#"{"type":"lw-demo-items","schemaVersion":1,"items":[
        {"kind":"dali","id":"light-1","index":1,"title":{"de":"L","en":"L"},"tags":["mini"]},
        {"kind":"dali","id":"light-3","index":3,"title":{"de":"L","en":"L"},"tags":["mini"]},
        {"kind":"digitalInput","id":"di-a","index":4,"title":{"de":"D","en":"D"},"tags":["mini"]}]}"#;

    /// `index` of a DI entry and `daliLampCount` may be omitted: they are derived on load.
    #[test]
    fn omitted_index_and_lamp_count_are_derived() {
        let root = scratch("derive");
        fs::write(root.join("items/mini.json"), MINI_ITEMS).unwrap();
        fs::write(root.join("demos/mini.json"), MINI_DEMO).unwrap();
        let lib = load(&root, "0.0.0").unwrap();
        let demo = &lib.demos[0];
        assert_eq!(demo.dali_lamp_count, 3);
        assert_eq!(demo.digital_inputs[0].index, 4);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn orphan_floorplans_are_reported() {
        let root = scratch("orphans");
        fs::write(root.join("items/mini.json"), MINI_ITEMS).unwrap();
        fs::write(root.join("demos/mini.json"), MINI_DEMO).unwrap();
        fs::create_dir_all(root.join("floorplans/mini")).unwrap();
        fs::create_dir_all(root.join("floorplans/ghost")).unwrap();
        fs::write(root.join("floorplans/mini/unused.json"), "{}").unwrap();
        fs::write(root.join("floorplans/ghost/a.json"), "{}").unwrap();
        let lib = load(&root, "0.0.0").unwrap();
        assert!(
            lib.warnings
                .iter()
                .any(|w| w.contains("floorplans/mini/unused.json: not referenced")),
            "{:?}",
            lib.warnings
        );
        assert!(
            lib.warnings
                .iter()
                .any(|w| w.contains("floorplans/ghost/: no demo")),
            "{:?}",
            lib.warnings
        );
        let _ = fs::remove_dir_all(&root);
    }
}
