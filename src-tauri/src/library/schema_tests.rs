//! `library/schema/*.schema.json` ⇄ files ⇄ serde model.
//!
//! - every JSON file of the repo `library/` and of the test fixture library validates against
//!   its schema (same mapping as `scripts/validate-library.mjs`);
//! - schema and serde agree: every fixture file parses with the serde model, its serde round
//!   trip is still schema-valid, and a set of broken documents is rejected by both.

use std::fs;
use std::path::{Path, PathBuf};

use jsonschema::{Registry, Validator};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::model::{DemoPack, FloorplanLayout, ItemLibraryFile};

const BASE: &str = "https://schemas.lichtwart.io/demo-simulator/";

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn repo_library() -> PathBuf {
    manifest().join("../library")
}

fn fixture_library() -> PathBuf {
    manifest().join("tests/fixtures/library")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Items,
    Demo,
    Floorplan,
}

impl Kind {
    fn schema(self) -> &'static str {
        match self {
            Kind::Items => "items.schema.json",
            Kind::Demo => "demo.schema.json",
            Kind::Floorplan => "floorplan.schema.json",
        }
    }
}

/// Which schema a library file (path relative to the library root) must satisfy.
fn kind_of(rel: &[String]) -> Option<Kind> {
    match rel {
        [d, _] if d == "items" => Some(Kind::Items),
        [d, _] if d == "demos" => Some(Kind::Demo),
        [d, _, _] if d == "floorplans" => Some(Kind::Floorplan),
        _ => None,
    }
}

struct Schemas {
    registry: Registry<'static>,
}

impl Schemas {
    fn load() -> Self {
        let dir = repo_library().join("schema");
        let mut builder = Registry::new();
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if !name.ends_with(".schema.json") {
                continue;
            }
            let schema: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(schema["$id"], format!("{BASE}{name}"), "$id of {name}");
            jsonschema::meta::validate(&schema).unwrap_or_else(|e| panic!("{name}: {e}"));
            builder = builder.add(format!("{BASE}{name}"), schema).unwrap();
        }
        Self {
            registry: builder.prepare().unwrap(),
        }
    }

    fn validator(&self, kind: Kind) -> Validator {
        jsonschema::options()
            .with_registry(&self.registry)
            .build(&json!({ "$ref": format!("{BASE}{}", kind.schema()) }))
            .unwrap()
    }

    /// Schema errors as `"/path: message"` lines.
    fn errors(&self, kind: Kind, doc: &Value) -> Vec<String> {
        self.validator(kind)
            .iter_errors(doc)
            .map(|e| format!("{}: {e}", e.instance_path()))
            .collect()
    }
}

fn json_files(root: &Path) -> Vec<(Vec<String>, PathBuf)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(Vec<String>, PathBuf)>) {
        let mut entries: Vec<PathBuf> = fs::read_dir(dir)
            .map(|r| r.filter_map(|e| e.ok().map(|e| e.path())).collect())
            .unwrap_or_default();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n == "schema") && dir == root {
                    continue;
                }
                walk(root, &p, out);
            } else if p.extension().is_some_and(|x| x == "json") {
                let rel = p
                    .strip_prefix(root)
                    .unwrap()
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                out.push((rel, p));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap())
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The CI gate: every file under the repo library and the fixture library is schema-valid.
#[test]
fn every_library_file_matches_its_schema() {
    let schemas = Schemas::load();
    let mut problems = Vec::new();
    let mut checked = 0;
    for root in [repo_library(), fixture_library()] {
        for (rel, path) in json_files(&root) {
            let name = format!("{}/{}", root.display(), rel.join("/"));
            let Some(kind) = kind_of(&rel) else {
                problems.push(format!(
                    "{name}: JSON outside items/, demos/, floorplans/<id>/"
                ));
                continue;
            };
            checked += 1;
            for e in schemas.errors(kind, &read(&path)) {
                problems.push(format!("{name}: {e}"));
            }
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
    assert!(checked >= 9, "only {checked} files checked");
}

fn round_trip<T: Serialize + DeserializeOwned>(
    schemas: &Schemas,
    kind: Kind,
    name: &str,
    doc: &Value,
) {
    let parsed: T = serde_json::from_value(doc.clone())
        .unwrap_or_else(|e| panic!("{name}: schema-valid but serde rejects it: {e}"));
    let back = serde_json::to_value(&parsed).unwrap();
    let errors = schemas.errors(kind, &back);
    assert!(
        errors.is_empty(),
        "{name}: serde output violates the schema: {errors:#?}"
    );
}

/// Schema-valid fixture files parse with serde, and what serde writes (e.g. the export) is
/// schema-valid again.
#[test]
fn schema_and_serde_accept_the_same_fixtures() {
    let schemas = Schemas::load();
    for (rel, path) in json_files(&fixture_library()) {
        let name = rel.join("/");
        let doc = read(&path);
        let kind = kind_of(&rel).unwrap();
        assert!(schemas.errors(kind, &doc).is_empty(), "{name}");
        match kind {
            Kind::Items => round_trip::<ItemLibraryFile>(&schemas, kind, &name, &doc),
            Kind::Demo => round_trip::<DemoPack>(&schemas, kind, &name, &doc),
            Kind::Floorplan => round_trip::<FloorplanLayout>(&schemas, kind, &name, &doc),
        }
    }
}

fn rejected_by_serde(kind: Kind, doc: &Value) -> bool {
    let v = doc.clone();
    match kind {
        Kind::Items => serde_json::from_value::<ItemLibraryFile>(v).is_err(),
        Kind::Demo => serde_json::from_value::<DemoPack>(v).is_err(),
        Kind::Floorplan => serde_json::from_value::<FloorplanLayout>(v).is_err(),
    }
}

/// Structural breakage is rejected by both the schema and serde.
#[test]
fn schema_and_serde_reject_the_same_breakage() {
    let schemas = Schemas::load();
    let fixture = |rel: &str| read(&fixture_library().join(rel));
    let items = fixture("items/cooling.json");
    let lights = fixture("items/lights.json");
    let alarms = fixture("items/alarms.json");
    let demo = fixture("demos/fixture.json");
    let floorplan = fixture("floorplans/fixture/ground.json");

    type Edit = fn(&mut Value);
    let cases: Vec<(&str, Kind, &Value, Edit)> = vec![
        ("unknown top-level field", Kind::Items, &items, |d| {
            d["extra"] = json!(1)
        }),
        ("modbus item without slave", Kind::Items, &items, |d| {
            d["items"][0].as_object_mut().unwrap().remove("slave");
        }),
        ("unknown item kind", Kind::Items, &items, |d| {
            d["items"][0]["kind"] = json!("analog")
        }),
        ("typo in item field", Kind::Items, &lights, |d| {
            d["items"][0]["indx"] = json!(1)
        }),
        ("system field on a dali item", Kind::Items, &lights, |d| {
            d["items"][0]["system"] = json!("rssi");
        }),
        ("custom alarm without severity", Kind::Items, &alarms, |d| {
            d["items"][0].as_object_mut().unwrap().remove("severity");
        }),
        ("bad severity", Kind::Items, &alarms, |d| {
            d["items"][0]["severity"] = json!("HIGH")
        }),
        ("slave alarm field typo", Kind::Items, &items, |d| {
            d["slaves"][0]["alarms"][0]["sev"] = json!("MAJOR");
        }),
        ("slider without step", Kind::Items, &items, |d| {
            d["items"][0]["control"]
                .as_object_mut()
                .unwrap()
                .remove("step");
        }),
        ("generator field typo", Kind::Items, &items, |d| {
            d["items"][0]["generator"]["centre"] = json!(1);
        }),
        ("title without en", Kind::Items, &items, |d| {
            d["items"][0]["title"].as_object_mut().unwrap().remove("en");
        }),
        ("demo without sendIntervalSec", Kind::Demo, &demo, |d| {
            d.as_object_mut().unwrap().remove("sendIntervalSec");
        }),
        ("typo in a digital input entry", Kind::Demo, &demo, |d| {
            d["digitalInputs"][0]["groupz"] = json!([1])
        }),
        ("typo in an impulse counter entry", Kind::Demo, &demo, |d| {
            d["digitalInputs"][2]["initcount"] = json!(5)
        }),
        ("group field typo", Kind::Demo, &demo, |d| {
            d["daliGroups"][0]["convertors"] = json!([])
        }),
        ("value step without value", Kind::Demo, &demo, |d| {
            d["scenarios"][0]["set"][0]
                .as_object_mut()
                .unwrap()
                .remove("value");
        }),
        ("scenario step with unknown kind", Kind::Demo, &demo, |d| {
            d["scenarios"][0]["set"][0]["kind"] = json!("wait");
        }),
        ("negative group level", Kind::Demo, &demo, |d| {
            d["daliGroups"][0]["minLevel"] = json!(-1)
        }),
        ("unknown DI type", Kind::Demo, &demo, |d| {
            d["digitalInputs"][1]["type"] = json!("none")
        }),
        ("floorplan widget typo", Kind::Floorplan, &floorplan, |d| {
            d["widgets"][0]["titel"] = json!("x");
        }),
        (
            "sensor without measurement",
            Kind::Floorplan,
            &floorplan,
            |d| {
                d["widgets"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("measurement");
            },
        ),
        ("unknown widget kind", Kind::Floorplan, &floorplan, |d| {
            d["widgets"][0]["kind"] = json!("chart");
        }),
        ("unknown alarm ref kind", Kind::Floorplan, &floorplan, |d| {
            d["widgets"][3]["alarms"][0]["kind"] = json!("lamp");
        }),
        ("bad group mode", Kind::Floorplan, &floorplan, |d| {
            d["widgets"][7]["control"]["modes"] = json!(["on", "dim"]);
        }),
        (
            "modbus control without valueType",
            Kind::Floorplan,
            &floorplan,
            |d| {
                d["widgets"][4]["control"]
                    .as_object_mut()
                    .unwrap()
                    .remove("valueType");
            },
        ),
    ];
    for (name, kind, base, edit) in cases {
        let mut doc = base.clone();
        edit(&mut doc);
        assert!(
            !schemas.errors(kind, &doc).is_empty(),
            "schema accepts: {name}"
        );
        assert!(rejected_by_serde(kind, &doc), "serde accepts: {name}");
    }
}

/// Rules only the schema enforces (serde has no regex / ranges); `validate` covers them for
/// files that bypass CI. Documented here so a change in either side is noticed.
#[test]
fn schema_enforces_formats_and_ranges() {
    let schemas = Schemas::load();
    let items = read(&fixture_library().join("items/cooling.json"));
    let lights = read(&fixture_library().join("items/lights.json"));
    let demo = read(&fixture_library().join("demos/fixture.json"));
    type Edit = fn(&mut Value);
    let cases: Vec<(Kind, &Value, Edit)> = vec![
        // `unit` only exists on impulseCounter / modbus (validate.rs agrees).
        (Kind::Items, &lights, |d| d["items"][0]["unit"] = json!("%")),
        // Reserved inventory fragments and native events cannot be faked by effects.
        (Kind::Items, &items, |d| {
            d["items"][2]["effects"][1]["property"] = json!("lw_configCore")
        }),
        (Kind::Items, &items, |d| {
            d["items"][2]["effects"][1]["property"] = json!("c8y_Firmware")
        }),
        (Kind::Items, &items, |d| {
            d["items"][2]["effects"][0]["type"] = json!("lw_daliGroupChange")
        }),
        // DI actions are lowercase on / off / auto; index and daliLampCount are optional.
        (Kind::Demo, &demo, |d| {
            d["digitalInputs"][0]["highLevelAction"] = json!("On")
        }),
        (Kind::Demo, &demo, |d| {
            d["digitalInputs"][0]["lowLevelAction"] = json!("ein")
        }),
        (Kind::Items, &items, |d| {
            d["items"][0]["id"] = json!("x".repeat(20))
        }),
        (Kind::Items, &items, |d| {
            d["items"][0]["unit"] = json!("12345678")
        }),
        (Kind::Items, &items, |d| {
            d["slaves"][0]["name"] = json!("x".repeat(20))
        }),
        (Kind::Items, &items, |d| {
            d["items"][0]["tags"][0] = json!("Zone:Shop")
        }),
        (Kind::Demo, &demo, |d| d["sendIntervalSec"] = json!(900)),
        (Kind::Demo, &demo, |d| d["daliLampCount"] = json!(65)),
        (Kind::Demo, &demo, |d| {
            d["digitalInputs"][1]["index"] = json!(11)
        }),
        (Kind::Demo, &demo, |d| {
            d["digitalInputs"][1]["groups"] = json!([1])
        }),
    ];
    for (i, (kind, base, edit)) in cases.into_iter().enumerate() {
        let mut doc = base.clone();
        edit(&mut doc);
        assert!(!schemas.errors(kind, &doc).is_empty(), "case {i} accepted");
    }
}

/// Optional fields: `daliLampCount` and a DI entry `index` may be left out.
#[test]
fn schema_accepts_derived_fields() {
    let schemas = Schemas::load();
    let mut demo = read(&fixture_library().join("demos/fixture.json"));
    demo.as_object_mut().unwrap().remove("daliLampCount");
    demo["digitalInputs"][1]
        .as_object_mut()
        .unwrap()
        .remove("index");
    assert!(schemas.errors(Kind::Demo, &demo).is_empty());
    let parsed: DemoPack = serde_json::from_value(demo).unwrap();
    assert_eq!(parsed.dali_lamp_count, 0); // derived by the loader
    assert_eq!(parsed.digital_inputs[1].index, 0);
}

/// Device-side digital input entries (`lw_demo.digitalInputs`) are strict too (`flatten` would
/// hide typos).
#[test]
fn device_di_entries_reject_unknown_fields() {
    use crate::model::{DigitalInputConfig, DigitalInputEntry};
    let ok = json!({"index": 1, "title": "x", "type": "daliAction", "groups": [1]});
    assert!(serde_json::from_value::<DigitalInputEntry>(ok.clone()).is_ok());
    let mut typo = ok.clone();
    typo["groupz"] = json!([2]);
    assert!(serde_json::from_value::<DigitalInputEntry>(typo).is_err());
    // `itemId` belongs to the demo entry, not the device entry.
    let mut foreign = ok;
    foreign["itemId"] = json!("x");
    assert!(serde_json::from_value::<DigitalInputEntry>(foreign).is_err());
    let cfg = json!({"itemId": "x", "type": "measurement", "isActve": true});
    assert!(serde_json::from_value::<DigitalInputConfig>(cfg).is_err());
}
