//! Device feature tests against `c8y::fake::FakeC8y`: CRUD, apply (+ floorplans), setup edits,
//! export ↔ apply round trip, upgrade.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{apply, crud, demo_state, export, setup, upgrade};
use crate::c8y::C8yClient;
use crate::c8y::fake::FakeC8y;
use crate::library::{Library, loader};
use crate::model::{
    ApplyDemoRequest, Control, CreateDeviceRequest, DemoSetupPatch, ExportRequest, LibraryItem,
    LocalizedText, ResetDeviceRequest, SlaveDefinition, UpdateDeviceRequest,
};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library")
}

fn fixture(version: &str) -> Library {
    loader::load(&fixture_root(), version).unwrap()
}

fn temp_dir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let d = std::env::temp_dir().join(format!("lw-sim-{name}-{nanos}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), target).unwrap();
        }
    }
}

async fn new_device(client: &C8yClient, lib: &Library, name: &str) -> String {
    crud::create(
        client,
        lib,
        &CreateDeviceRequest {
            name: name.into(),
            description: Some("Beschreibung".into()),
            position: None,
            parent_group_id: None,
            apply_demo: None,
        },
    )
    .await
    .unwrap()
    .id
}

fn req(demo: &str, floorplans: bool, replace: bool) -> ApplyDemoRequest {
    ApplyDemoRequest {
        demo_id: demo.into(),
        apply_floorplans: floorplans,
        replace_existing_floorplans: replace,
        force: false,
    }
}

fn mo(fake: &FakeC8y, id: &str) -> Value {
    fake.state()
        .managed_objects
        .iter()
        .find(|m| m["id"] == id)
        .cloned()
        .unwrap()
}

/// Cumulocity rejects operations for a device that is not an agent (field bug: "Agent for
/// device '…' does not exist in inventory!"). New devices carry the fragment; older ones get it
/// when they are opened or a demo is applied — real devices are never touched.
#[tokio::test]
async fn demo_devices_are_their_own_agent() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let lib = fixture("1.0.0");
    let id = new_device(&client, &lib, "Neu").await;
    let created = fake
        .requests()
        .into_iter()
        .find(|r| r.method == "POST" && r.path == "/inventory/managedObjects")
        .unwrap();
    assert_eq!(created.json()["com_cumulocity_model_Agent"], json!({}));
    assert!(
        fake.try_create_operation(&id, json!({ "c8y_Restart": {} }))
            .is_ok()
    );

    // A device from an older app version: no agent fragment, an old operation list.
    let legacy = fake.insert_managed_object(json!({
        "name": "Alt", "type": "LW_Core_v2", "c8y_IsDevice": {},
        "c8y_DeviceTypes": ["demo"], "c8y_SupportedOperations": ["c8y_Restart"]
    }));
    let err = fake
        .try_create_operation(&legacy, json!({ "c8y_Restart": {} }))
        .unwrap_err();
    assert_eq!(err["error"], "devicecontrol/Unprocessable Entity");
    crud::get(&client, &lib, &legacy).await.unwrap();
    let m = mo(&fake, &legacy);
    assert_eq!(m["com_cumulocity_model_Agent"], json!({}));
    assert!(
        m["c8y_SupportedOperations"]
            .as_array()
            .unwrap()
            .contains(&json!("lw_switchGroupMode"))
    );
    assert!(
        fake.try_create_operation(&legacy, json!({ "c8y_Restart": {} }))
            .is_ok()
    );

    // Opening it again writes nothing (idempotent).
    let puts = |fake: &FakeC8y, id: &str| {
        fake.requests()
            .iter()
            .filter(|r| r.method == "PUT" && r.path == format!("/inventory/managedObjects/{id}"))
            .count()
    };
    let before = puts(&fake, &legacy);
    crud::get(&client, &lib, &legacy).await.unwrap();
    assert_eq!(puts(&fake, &legacy), before);

    // Applying a demo to a legacy device adds it in the same PUT.
    let legacy2 = fake.insert_managed_object(json!({
        "name": "Alt 2", "type": "LW_Core_v2", "c8y_IsDevice": {}, "c8y_DeviceTypes": ["demo"]
    }));
    apply::apply(&client, &lib, &legacy2, &req("fixture", false, false))
        .await
        .unwrap();
    assert_eq!(mo(&fake, &legacy2)["com_cumulocity_model_Agent"], json!({}));

    // A real (non-demo) device is refused and left alone.
    let real = fake.insert_managed_object(json!({ "name": "Real", "c8y_IsDevice": {} }));
    assert!(crud::get(&client, &lib, &real).await.is_err());
    assert!(mo(&fake, &real).get("com_cumulocity_model_Agent").is_none());
}

/// Opening is read-only for the user: a failing migration PUT is logged, the device opens.
#[tokio::test]
async fn opening_a_device_survives_a_failing_migration() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let lib = fixture("1.0.0");
    let legacy = fake.insert_managed_object(json!({
        "name": "Alt", "type": "LW_Core_v2", "c8y_IsDevice": {},
        "c8y_DeviceTypes": ["demo"], "c8y_SupportedOperations": ["c8y_Restart"]
    }));
    fake.fail_next("PUT", "/inventory/managedObjects", 403);
    let d = crud::get(&client, &lib, &legacy).await.unwrap();
    assert_eq!(d.id, legacy);
    assert!(
        mo(&fake, &legacy)
            .get("com_cumulocity_model_Agent")
            .is_none()
    );
    // The mandatory path still reports the failure.
    fake.fail_next("PUT", "/inventory/managedObjects", 403);
    let mut m = mo(&fake, &legacy);
    assert!(
        crud::ensure_required_fragments(&client, &mut m)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn create_list_update_and_raw_device_body() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let lib = fixture("1.0.0");
    let id = new_device(&client, &lib, "Tankstelle Nord").await;
    fake.insert_managed_object(
        json!({ "name": "Real Core", "c8y_IsDevice": {}, "type": "LW_Core_v2" }),
    );
    let m = mo(&fake, &id);
    assert_eq!(m["c8y_DeviceTypes"], json!(["demo"]));
    assert_eq!(m["c8y_Firmware"]["version"], "V 03.00.02");
    assert!(m.get("c8y_RequiredAvailability").is_none());
    assert!(
        m["c8y_Hardware"]["serialNumber"]
            .as_str()
            .unwrap()
            .starts_with("DEMO-")
    );

    let list = crud::list(&client, &lib, None).await.unwrap();
    assert_eq!(list.len(), 1, "only demo devices");
    assert!(
        crud::list(&client, &lib, Some("nord beschreibung"))
            .await
            .unwrap()
            .len()
            == 1
    );
    assert!(
        crud::list(&client, &lib, Some("süd"))
            .await
            .unwrap()
            .is_empty()
    );

    let dev = crud::update(
        &client,
        &lib,
        &id,
        &UpdateDeviceRequest {
            name: Some("Tankstelle Süd".into()),
            description: None,
            position: Some(crate::model::Position {
                lat: 48.1,
                lng: 11.5,
                alt: 0.0,
            }),
        },
    )
    .await
    .unwrap();
    assert_eq!(dev.name, "Tankstelle Süd");
    assert_eq!(dev.position.unwrap().lat, 48.1);
    assert!(dev.demo.is_none());

    // The app refuses real devices.
    let real = fake
        .state()
        .managed_objects
        .iter()
        .find(|m| m["name"] == "Real Core")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(crud::get(&client, &lib, &real).await.is_err());
    assert!(crud::delete(&client, &real).await.is_err());
}

#[tokio::test]
async fn apply_writes_config_and_floorplan_with_child_addition() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let lib = fixture("1.0.0");
    let id = new_device(&client, &lib, "D").await;

    let preview = apply::preview(&client, &lib, &id, &req("fixture", true, false))
        .await
        .unwrap();
    assert_eq!(preview.slaves.len(), 2);
    assert_eq!((preview.slaves[0].id, preview.slaves[0].address), (20, 240));
    assert!(preview.floorplan_conflicts.is_empty());

    let res = apply::apply(&client, &lib, &id, &req("fixture", true, false))
        .await
        .unwrap();
    assert_eq!(res.floorplans_created.len(), 1);
    let fp_id = res.floorplans_created[0].id.clone();
    let m = mo(&fake, &id);
    // D10: groups and inputs live in `lw_demo`, no `lw_config*` fragment is written.
    assert!(
        m.as_object()
            .unwrap()
            .keys()
            .all(|k| !k.starts_with("lw_config")),
        "{m}"
    );
    let demo = demo_state::read(&m).unwrap().unwrap();
    assert_eq!(demo.dali_groups.as_ref().unwrap().len(), 2);
    assert_eq!(
        demo.digital_inputs.as_ref().unwrap()[0].title,
        "Zapfsäule 1"
    );
    assert_eq!(demo.slaves[0].key, "kuehlung");
    assert_eq!(demo.demo_id.as_deref(), Some("fixture"));
    assert_eq!(demo.library_version.as_deref(), Some("1.0.0"));
    assert_eq!(demo.floorplan_ids, vec![fp_id.clone()]);
    assert!(!demo.applied_items.is_empty());

    // Floorplan: binary, lw_floorplan MO, child addition, resolved widgets.
    let fp = mo(&fake, &fp_id);
    assert_eq!(fp["type"], "lw_floorplan");
    let bin = fp["lw_floorplan"]["imageBinaryId"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(fake.state().binaries.contains_key(&bin));
    assert_eq!(fake.state().child_additions[&id], vec![fp_id.clone()]);
    let w = &fp["lw_floorplan"]["widgets"][0];
    assert_eq!(
        w["measurement"],
        json!({"fragment": "Tiefkühler Temperatur", "series": "freezer_temp"})
    );
    assert_eq!(res.device.floorplans.len(), 1);

    // Re-apply without confirmation: kept, warning.
    let res2 = apply::apply(&client, &lib, &id, &req("fixture", true, false))
        .await
        .unwrap();
    assert!(res2.floorplans_created.is_empty());
    assert_eq!(res2.warnings.len(), 1);
    let preview = apply::preview(&client, &lib, &id, &req("fixture", true, false))
        .await
        .unwrap();
    assert_eq!(preview.floorplan_conflicts.len(), 1);
    // With confirmation: replaced, old MO + binary gone.
    let res3 = apply::apply(&client, &lib, &id, &req("fixture", true, true))
        .await
        .unwrap();
    assert_eq!(res3.floorplans_replaced[0].id, fp_id);
    assert!(
        fake.state()
            .managed_objects
            .iter()
            .all(|m| m["id"] != fp_id.as_str())
    );
    assert!(!fake.state().binaries.contains_key(&bin));
    let demo = demo_state::read(&mo(&fake, &id)).unwrap().unwrap();
    assert_eq!(
        demo.floorplan_ids,
        vec![res3.floorplans_created[0].id.clone()]
    );

    // Reset with floorplans: alarms cleared, floorplans deleted.
    fake.state()
        .alarms
        .push(json!({"id": "a1", "source": {"id": id}, "type": "x", "status": "ACTIVE"}));
    crud::reset(
        &client,
        &id,
        &ResetDeviceRequest {
            delete_floorplans: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(fake.state().alarms[0]["status"], "CLEARED");
    assert!(
        fake.state()
            .managed_objects
            .iter()
            .all(|m| m["type"] != "lw_floorplan")
    );
    assert!(
        demo_state::read(&mo(&fake, &id))
            .unwrap()
            .unwrap()
            .floorplan_ids
            .is_empty()
    );

    // Delete: device and its floorplans + binaries.
    apply::apply(&client, &lib, &id, &req("fixture", true, false))
        .await
        .unwrap();
    crud::delete(&client, &id).await.unwrap();
    assert!(fake.state().managed_objects.is_empty());
    assert!(fake.state().binaries.is_empty());
}

/// D10: an apply removes the `lw_config*` fragments an older app version wrote; groups and
/// inputs are read and written in `lw_demo` (a device without them gets the library demo's).
#[tokio::test]
async fn apply_removes_old_config_fragments_and_config_lives_in_lw_demo() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let lib = fixture("1.0.0");
    let id = new_device(&client, &lib, "Alt").await;
    apply::apply(&client, &lib, &id, &req("fixture", false, false))
        .await
        .unwrap();
    // As an older app version left it: `lw_config*` fragments, no groups / inputs in `lw_demo`.
    {
        let mut s = fake.state();
        let m = s
            .managed_objects
            .iter_mut()
            .find(|m| m["id"] == id.as_str())
            .unwrap();
        for f in ["lw_configCore", "lw_configRs485", "lw_configModbusSlaves"] {
            m[f] = json!({});
        }
        m["lw_configGroups"] = json!([{ "id": 9, "name": "Alt", "converters": [1] }]);
        m["lw_configDigitalInputs"] = json!([{ "id": 1, "title": "Alt", "type": "measurement" }]);
        let demo = m["lw_demo"].as_object_mut().unwrap();
        demo.remove("daliGroups");
        demo.remove("digitalInputs");
    }
    // Not read any more: the library demo's groups / inputs apply.
    let cfg = demo_state::get_config(&client, &lib, &id).await.unwrap();
    assert_eq!(cfg.groups, lib.demo("fixture").unwrap().dali_groups);
    assert_eq!(cfg.digital_inputs[0].title, "Zapfsäule 1");

    apply::apply(&client, &lib, &id, &req("fixture", false, false))
        .await
        .unwrap();
    let no_config = |m: &Value| {
        m.as_object()
            .unwrap()
            .keys()
            .all(|k| !k.starts_with("lw_config"))
    };
    assert!(no_config(&mo(&fake, &id)), "{}", mo(&fake, &id));

    // Edits land in `lw_demo` and are read back.
    let mut groups = cfg.groups.clone();
    groups[0].name = "Laden".into();
    demo_state::set_groups(&client, &id, &groups).await.unwrap();
    let mut inputs = cfg.digital_inputs.clone();
    inputs[0].title = "Säule A".into();
    demo_state::set_digital_inputs(&client, &id, &inputs)
        .await
        .unwrap();
    assert_eq!(demo_of(&fake, &id).dali_groups.unwrap()[0].name, "Laden");
    let cfg = demo_state::get_config(&client, &lib, &id).await.unwrap();
    assert_eq!(cfg.groups[0].name, "Laden");
    assert_eq!(cfg.digital_inputs[0].title, "Säule A");
    assert!(no_config(&mo(&fake, &id)));
}

fn custom_slave_patch() -> DemoSetupPatch {
    DemoSetupPatch {
        custom_slaves: Some(vec![SlaveDefinition {
            key: "extra".into(),
            name: "Extra Kuehlung".into(),
            request_interval: 30,
            alarms: vec![],
        }]),
        custom_items: Some(vec![LibraryItem::Modbus {
            id: "extra_temp".into(),
            slave: "extra".into(),
            title: LocalizedText {
                de: "Extra Temperatur".into(),
                en: "Extra temperature".into(),
            },
            tags: vec![],
            register_type: None,
            unit: Some("°C".into()),
            datatype: None,
            writable: true,
            control: Some(Control::Slider {
                min: 0.0,
                max: 50.0,
                step: 1.0,
                default: 20.0,
            }),
            generator: None,
            effects: vec![],
        }]),
        generators: Some(
            [(
                "freezer_temp".to_owned(),
                crate::model::GeneratorConfig::Sine {
                    period: 600.0,
                    amplitude: 1.0,
                    center: Some(-18.0),
                },
            )]
            .into_iter()
            .collect(),
        ),
        ..Default::default()
    }
}

#[tokio::test]
async fn patch_setup_assigns_custom_slaves_without_renumbering() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let lib = fixture("1.0.0");
    let id = new_device(&client, &lib, "D").await;
    apply::apply(&client, &lib, &id, &req("fixture", false, false))
        .await
        .unwrap();
    let st = demo_state::patch_setup(&client, &lib, &id, &custom_slave_patch())
        .await
        .unwrap();
    let ids: Vec<(String, u32, u32)> = st
        .slaves
        .iter()
        .map(|s| (s.key.clone(), s.id, s.address))
        .collect();
    assert_eq!(
        ids,
        vec![
            ("kuehlung".into(), 20, 240),
            ("klima".into(), 19, 239),
            ("extra".into(), 18, 238)
        ]
    );
    let m = mo(&fake, &id);
    assert_eq!(demo_state::read(&m).unwrap().unwrap().slaves.len(), 3);
    assert!(m.get("lw_configModbusSlaves").is_none());
    // A colliding custom id is rejected.
    let mut bad = custom_slave_patch();
    if let Some(items) = &mut bad.custom_items
        && let LibraryItem::Modbus { id, .. } = &mut items[0]
    {
        *id = "freezer_temp".into();
    }
    assert!(
        demo_state::patch_setup(&client, &lib, &id, &bad)
            .await
            .is_err()
    );
}

/// Scenarios edited on the device: validated on save, exported into the pack, and a pack that
/// changes something the start scenario does not reset gets an export warning.
#[tokio::test]
async fn device_scenarios_are_validated_and_exported() {
    use crate::model::{Scenario, ScenarioStep};
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let lib = fixture("1.0.0");
    let dev = new_device(&client, &lib, "Szenarien").await;
    apply::apply(&client, &lib, &dev, &req("fixture", false, false))
        .await
        .unwrap();
    let sc = |j: serde_json::Value| serde_json::from_value::<Scenario>(j).unwrap();
    let door = sc(json!({
        "id": "door", "name": { "de": "Tür offen", "en": "Door open" },
        "description": { "de": "Die Kühlraumtür bleibt offen.", "en": "The cold room door stays open." },
        "set": [{ "kind": "fault", "target": { "kind": "customAlarm", "itemId": "doorOpen" }, "active": true }]
    }));
    let patch = |custom: Vec<Scenario>, hidden: Vec<String>| DemoSetupPatch {
        custom_scenarios: Some(custom),
        hidden_scenarios: Some(hidden),
        ..Default::default()
    };

    // A step outside the control range is rejected, nothing is written.
    let bad = sc(json!({
        "id": "bad", "name": { "de": "Falsch", "en": "Bad" },
        "set": [{ "kind": "value", "itemId": "ac_setpoint", "value": 99 }]
    }));
    let err = demo_state::patch_setup(&client, &lib, &dev, &patch(vec![bad], vec![]))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("value 99 outside 16…28"), "{err}");
    assert!(demo_of(&fake, &dev).custom_scenarios.is_empty());

    let st = demo_state::patch_setup(
        &client,
        &lib,
        &dev,
        &patch(vec![door.clone()], vec!["trouble".into()]),
    )
    .await
    .unwrap();
    assert_eq!(st.custom_scenarios, vec![door.clone()]);
    let ids: Vec<String> = setup::effective_scenarios(&lib, &demo_of(&fake, &dev))
        .into_iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(ids, vec!["normal", "door"]);

    let out = temp_dir("export-scenarios");
    let export_req = ExportRequest {
        device_id: dev.clone(),
        demo_id: "zz-scenarios".into(),
        target_dir: out.display().to_string(),
    };
    let res = export::export(&client, &lib, &export_req).await.unwrap();
    assert!(res.warnings.is_empty(), "{:?}", res.warnings);
    let merged = temp_dir("merged-scenarios");
    copy_dir(&fixture_root(), &merged);
    copy_dir(&out, &merged);
    let lib2 = loader::load(&merged, "1.0.0").unwrap();
    assert!(
        !lib2.warnings.iter().any(|w| w.contains("zz-scenarios")),
        "{:?}",
        lib2.warnings
    );
    let exported = lib2.demo("zz-scenarios").unwrap();
    let ids: Vec<&str> = exported.scenarios.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, vec!["normal", "door"]);
    assert_eq!(exported.scenarios[1].description, door.description);

    // A value the start scenario does not reset: allowed on the device, a warning on export.
    let warm = sc(json!({
        "id": "warm", "name": { "de": "Warm", "en": "Warm" },
        "set": [{ "kind": "value", "itemId": "ac_setpoint", "value": 26 }]
    }));
    demo_state::patch_setup(&client, &lib, &dev, &patch(vec![warm], vec![]))
        .await
        .unwrap();
    let res = export::export(&client, &lib, &export_req).await.unwrap();
    assert!(
        res.warnings
            .iter()
            .any(|w| w.contains("the start scenario does not reset it")),
        "{:?}",
        res.warnings
    );
    assert!(matches!(
        &setup::effective_scenarios(&lib, &demo_of(&fake, &dev))[2].set[0],
        ScenarioStep::Value { item_id, .. } if item_id == "ac_setpoint"
    ));
}

#[tokio::test]
async fn export_round_trips_through_apply() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let lib = fixture("1.0.0");

    // Device A: applied, plus custom slave / item and a generator override.
    let a = new_device(&client, &lib, "Original").await;
    apply::apply(&client, &lib, &a, &req("fixture", true, false))
        .await
        .unwrap();
    demo_state::patch_setup(&client, &lib, &a, &custom_slave_patch())
        .await
        .unwrap();

    let out = temp_dir("export");
    let res = export::export(
        &client,
        &lib,
        &ExportRequest {
            device_id: a.clone(),
            demo_id: "zz-roundtrip".into(),
            target_dir: out.display().to_string(),
        },
    )
    .await
    .unwrap();
    assert!(
        res.files_written
            .contains(&"demos/zz-roundtrip.json".to_owned())
    );
    assert!(
        res.files_written
            .contains(&"items/zz-roundtrip.json".to_owned())
    );
    assert!(res.files_written.iter().any(|f| f.ends_with(".svg")));
    assert!(res.warnings.is_empty(), "{:?}", res.warnings);

    // Merge into a copy of the library and load it: the export must be valid.
    let merged = temp_dir("merged");
    copy_dir(&fixture_root(), &merged);
    copy_dir(&out, &merged);
    let lib2 = loader::load(&merged, "1.0.0").unwrap();
    let problems: Vec<&String> = lib2
        .warnings
        .iter()
        .filter(|w| w.contains("zz-roundtrip"))
        .collect();
    assert!(problems.is_empty(), "{problems:?}");
    let exported = lib2.demo("zz-roundtrip").unwrap();
    assert!(exported.includes.contains(&"extra_temp".to_owned()));
    assert!(exported.scenarios.iter().any(|s| s.start
        && s.set.iter().any(|st| matches!(st, crate::model::ScenarioStep::Generator { item_id, .. } if item_id == "freezer_temp"))));

    // Device B from the exported pack: same configuration and floorplan.
    let b = new_device(&client, &lib2, "Kopie").await;
    apply::apply(&client, &lib2, &b, &req("zz-roundtrip", true, false))
        .await
        .unwrap();
    let (ma, mb) = (mo(&fake, &a), mo(&fake, &b));
    let (da, db) = (
        demo_state::read(&ma).unwrap().unwrap(),
        demo_state::read(&mb).unwrap().unwrap(),
    );
    assert_eq!(da.slaves, db.slaves);
    assert_eq!(da.dali_groups, db.dali_groups);
    assert_eq!(da.digital_inputs, db.digital_inputs);
    let fp = |dev: &str| {
        let id = fake.state().child_additions[dev][0].clone();
        let mut f = mo(&fake, &id)["lw_floorplan"].clone();
        let bin = f["imageBinaryId"].take();
        let bytes = fake.state().binaries[bin.as_str().unwrap()].1.clone();
        (f, bytes)
    };
    let (fa, ba) = fp(&a);
    let (fb, bb) = fp(&b);
    assert_eq!(fa, fb);
    assert_eq!(ba, bb);
    let _ = std::fs::remove_dir_all(out);
    let _ = std::fs::remove_dir_all(merged);
}

#[tokio::test]
async fn upgrade_preview_and_apply_keep_local_setup() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let old = fixture("1.0.0");
    let id = new_device(&client, &old, "D").await;
    apply::apply(&client, &old, &id, &req("fixture", true, false))
        .await
        .unwrap();
    demo_state::patch_setup(&client, &old, &id, &custom_slave_patch())
        .await
        .unwrap();
    assert!(upgrade::check(&client, &old, &id).await.unwrap().is_none());

    // The new library changes one item, removes one, adds one and changes the floorplan.
    let mut new = fixture("1.1.0");
    if let Some(LibraryItem::Modbus { unit, .. }) =
        new.items.iter_mut().find(|i| i.id() == "ac_setpoint")
    {
        *unit = Some("K".into());
    }
    new.items.retain(|i| i.id() != "di-door");
    for d in &mut new.demos {
        d.digital_inputs.retain(|c| c.item_id != "di-door");
    }
    let mut added = new
        .items
        .iter()
        .find(|i| i.id() == "light-64")
        .unwrap()
        .clone();
    if let LibraryItem::Dali {
        id, index, tags, ..
    } = &mut added
    {
        *id = "light-5".into();
        *index = 5;
        *tags = vec!["fixture".into()];
    }
    new.items.push(added);
    for l in new.floorplans.values_mut() {
        l.description = "neu".into();
    }
    let p = upgrade::check(&client, &new, &id).await.unwrap().unwrap();
    assert_eq!(
        (p.from_version.as_str(), p.to_version.as_str()),
        ("1.0.0", "1.1.0")
    );
    assert_eq!(p.items_changed, vec!["ac_setpoint"]);
    assert_eq!(p.items_removed, vec!["di-door"]);
    assert_eq!(p.items_added, vec!["light-5"]);
    assert_eq!(p.floorplans_changed, vec!["Erdgeschoss"]);

    // The floorplan of the new library references di-door → drop that widget for this test.
    for l in new.floorplans.values_mut() {
        l.widgets.retain(|w| w.id() != "w-door");
    }
    let dev = upgrade::apply(&client, &new, &id, false).await.unwrap();
    let st = dev.demo.unwrap();
    assert_eq!(st.library_version.as_deref(), Some("1.1.0"));
    assert_eq!(st.custom_items.len(), 1, "custom items kept");
    assert!(
        st.generators.contains_key("freezer_temp"),
        "generator overrides kept"
    );
    assert_eq!(
        st.slaves.iter().find(|s| s.key == "extra").unwrap().id,
        18,
        "never renumbered"
    );
    assert_eq!(dev.floorplans.len(), 1);
    assert_eq!(st.floorplan_ids.len(), 1);
    assert!(!dev.upgrade_available);
    assert!(upgrade::check(&client, &new, &id).await.unwrap().is_none());
    let fp = mo(&fake, &st.floorplan_ids[0]);
    assert_eq!(fp["lw_floorplan"]["description"], "neu");
}

// ───────────────────────────── phase 3 review fixes (docs/reviews/phase3-engine.md) ─────────────────────────────

use crate::error::AppError;
use crate::model::{DemoSnapshot, GroupRef, StartSessionRequest};

/// The fixture library with a second floorplan layout ("Obergeschoss") on its demo.
fn fixture_two_floorplans(version: &str) -> Library {
    let mut lib = fixture(version);
    let (path, layout) = lib
        .floorplans
        .iter()
        .next()
        .map(|(p, l)| (p.clone(), l.clone()))
        .unwrap();
    let mut second = layout;
    second.name = "Obergeschoss".into();
    let path2 = path.replace("ground.json", "upper.json");
    lib.floorplans.insert(path2.clone(), second);
    lib.demos[0].floorplans.push(path2);
    lib
}

fn set_descriptions(lib: &mut Library, text: &str) {
    for l in lib.floorplans.values_mut() {
        l.description = text.into();
    }
}

/// A floorplan somebody built in the plugin: linked to the device, not created by the app.
fn add_foreign_floorplan(fake: &FakeC8y, device: &str, name: &str) -> String {
    fake.state().binaries.insert(
        "bin-foreign".into(),
        (
            json!({ "id": "bin-foreign", "type": "image/png" }),
            vec![1, 2, 3],
        ),
    );
    let id = fake.insert_managed_object(json!({
        "type": "lw_floorplan", "name": name,
        "lw_floorplan": { "imageBinaryId": "bin-foreign", "description": "mine" }
    }));
    fake.state()
        .child_additions
        .entry(device.to_owned())
        .or_default()
        .push(id.clone());
    id
}

fn exists(fake: &FakeC8y, id: &str) -> bool {
    fake.state().managed_objects.iter().any(|m| m["id"] == id)
}

fn demo_of(fake: &FakeC8y, id: &str) -> crate::model::LwDemoState {
    demo_state::read(&mo(fake, id)).unwrap().unwrap()
}

/// Finding 3: nothing that writes a device config touches a device without the `demo` marker.
#[tokio::test]
async fn every_writer_refuses_a_real_device() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let lib = fixture("1.0.0");
    let real = fake.insert_managed_object(json!({
        "name": "Real Core", "c8y_IsDevice": {}, "type": "LW_Core_v2",
        "lw_configGroups": [{ "id": 1, "name": "G", "converters": [1] }]
    }));
    let before = mo(&fake, &real);
    let invalid = |r: Result<(), AppError>, what: &str| {
        assert!(matches!(r, Err(AppError::InvalidInput(_))), "{what}: {r:?}");
    };
    invalid(
        demo_state::patch_setup(
            &client,
            &lib,
            &real,
            &DemoSetupPatch {
                send_interval_sec: Some(10),
                ..Default::default()
            },
        )
        .await
        .map(|_| ()),
        "update_demo_setup",
    );
    invalid(
        demo_state::set_groups(&client, &real, &[])
            .await
            .map(|_| ()),
        "set_dali_groups",
    );
    invalid(
        demo_state::set_digital_inputs(&client, &real, &[])
            .await
            .map(|_| ()),
        "set_digital_inputs",
    );
    invalid(
        demo_state::get_config(&client, &lib, &real)
            .await
            .map(|_| ()),
        "get_config",
    );
    invalid(
        apply::apply(&client, &lib, &real, &req("fixture", false, false))
            .await
            .map(|_| ()),
        "apply",
    );
    invalid(
        crud::reset(
            &client,
            &real,
            &ResetDeviceRequest {
                delete_floorplans: true,
            },
        )
        .await,
        "reset",
    );
    invalid(
        upgrade::apply(&client, &lib, &real, false)
            .await
            .map(|_| ()),
        "upgrade",
    );
    invalid(crud::delete(&client, &real).await, "delete");
    assert!(
        fake.requests().iter().all(|r| r.method == "GET"),
        "not a single write reached the real device"
    );
    assert_eq!(mo(&fake, &real), before);
}

/// Findings 19 / 21: Reset and Delete remove the app's floorplans and nothing else, even when
/// `lw_demo.floorplanIds` (plain inventory data) points at something else.
#[tokio::test]
async fn reset_and_delete_touch_only_the_apps_floorplans() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let lib = fixture("1.0.0");
    let id = new_device(&client, &lib, "D").await;
    let res = apply::apply(&client, &lib, &id, &req("fixture", true, false))
        .await
        .unwrap();
    let owned = res.floorplans_created[0].id.clone();
    let owned_bin = mo(&fake, &owned)["lw_floorplan"]["imageBinaryId"]
        .as_str()
        .unwrap()
        .to_owned();
    let foreign = add_foreign_floorplan(&fake, &id, "Mein Plan");
    // Another device and a floorplan of another device.
    let other_device = fake.insert_managed_object(json!({ "name": "Other", "c8y_IsDevice": {} }));
    let other_plan = fake.insert_managed_object(json!({ "type": "lw_floorplan", "name": "X",
                                                        "lw_floorplan": { "imageBinaryId": "none" } }));
    fake.state()
        .child_additions
        .entry(other_device.clone())
        .or_default()
        .push(other_plan.clone());
    // Somebody edited lw_demo.floorplanIds by hand.
    let mut st = demo_of(&fake, &id);
    st.floorplan_ids.push(other_device.clone());
    st.floorplan_ids.push(other_plan.clone());
    demo_state::write(&client, &id, &st).await.unwrap();
    fake.state()
        .alarms
        .push(json!({"id": "a1", "source": {"id": id}, "type": "x", "status": "ACTIVE"}));

    crud::reset(
        &client,
        &id,
        &ResetDeviceRequest {
            delete_floorplans: true,
        },
    )
    .await
    .unwrap();
    assert!(!exists(&fake, &owned), "the app's floorplan is gone");
    assert!(
        !fake.state().binaries.contains_key(&owned_bin),
        "with its binary"
    );
    assert!(
        exists(&fake, &foreign),
        "a floorplan the app did not create stays"
    );
    assert!(fake.state().binaries.contains_key("bin-foreign"));
    assert!(
        exists(&fake, &other_device),
        "a forged id does not delete a device"
    );
    assert!(
        exists(&fake, &other_plan),
        "nor a floorplan of another device"
    );
    assert_eq!(fake.state().alarms[0]["status"], "CLEARED");
    let st = demo_of(&fake, &id);
    assert!(st.floorplan_ids.is_empty() && st.applied_floorplans.is_empty());

    // Delete: the device goes, foreign data stays.
    apply::apply(&client, &lib, &id, &req("fixture", true, false))
        .await
        .unwrap();
    let mut st = demo_of(&fake, &id);
    st.floorplan_ids.push(other_plan.clone());
    demo_state::write(&client, &id, &st).await.unwrap();
    crud::delete(&client, &id).await.unwrap();
    assert!(!exists(&fake, &id));
    assert!(exists(&fake, &other_plan));
    assert!(exists(&fake, &other_device));
    assert!(
        fake.state()
            .managed_objects
            .iter()
            .filter(|m| m["type"] == "lw_floorplan")
            .all(|m| m["id"] == foreign.as_str() || m["id"] == other_plan.as_str())
    );
}

/// Finding 11: an apply that fails part-way records what it created, so Reset finds it.
#[tokio::test]
async fn failed_apply_leaves_no_orphaned_floorplans() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let lib = fixture_two_floorplans("1.0.0");
    let id = new_device(&client, &lib, "D").await;
    fake.fail_after("POST", "/inventory/binaries", 1, 500);
    let err = apply::apply(&client, &lib, &id, &req("fixture", true, false))
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Http { status: 500, .. }), "{err:?}");
    let st = demo_of(&fake, &id);
    assert_eq!(st.floorplan_ids.len(), 1, "the first floorplan is recorded");
    assert!(exists(&fake, &st.floorplan_ids[0]));
    // Retrying creates only the missing one.
    let res = apply::apply(&client, &lib, &id, &req("fixture", true, false))
        .await
        .unwrap();
    assert_eq!(res.floorplans_created.len(), 1);
    assert_eq!(demo_of(&fake, &id).floorplan_ids.len(), 2);
    crud::reset(
        &client,
        &id,
        &ResetDeviceRequest {
            delete_floorplans: true,
        },
    )
    .await
    .unwrap();
    assert!(
        fake.state()
            .managed_objects
            .iter()
            .all(|m| m["type"] != "lw_floorplan")
    );
    assert!(fake.state().binaries.is_empty());
}

/// Finding 21: applying the same demo twice is idempotent, with and without confirmation.
#[tokio::test]
async fn reapplying_is_idempotent() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let lib = fixture("1.0.0");
    let id = new_device(&client, &lib, "D").await;
    for replace in [false, false, true, true] {
        apply::apply(&client, &lib, &id, &req("fixture", true, replace))
            .await
            .unwrap();
        let st = demo_of(&fake, &id);
        assert_eq!(st.floorplan_ids.len(), 1);
        assert_eq!(
            fake.state()
                .managed_objects
                .iter()
                .filter(|m| m["type"] == "lw_floorplan")
                .count(),
            1
        );
        assert_eq!(fake.state().binaries.len(), 1, "no leaked binaries");
        assert_eq!(st.slaves.len(), 2);
    }
}

/// Finding 5: a failing upgrade does not mark the device as up to date; a retry finishes it.
#[tokio::test]
async fn failed_upgrade_keeps_the_old_version_and_can_be_retried() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let old = fixture_two_floorplans("1.0.0");
    let id = new_device(&client, &old, "D").await;
    apply::apply(&client, &old, &id, &req("fixture", true, false))
        .await
        .unwrap();
    let mut new = fixture_two_floorplans("1.1.0");
    set_descriptions(&mut new, "neu");
    fake.fail_after("POST", "/inventory/binaries", 1, 500);
    assert!(upgrade::apply(&client, &new, &id, false).await.is_err());
    let st = demo_of(&fake, &id);
    assert_eq!(
        st.library_version.as_deref(),
        Some("1.0.0"),
        "still the old version"
    );
    assert!(
        upgrade::check(&client, &new, &id).await.unwrap().is_some(),
        "the upgrade is still offered"
    );
    assert_eq!(st.floorplan_ids.len(), 2, "all app floorplans are recorded");
    let dev = upgrade::apply(&client, &new, &id, false).await.unwrap();
    assert_eq!(dev.demo.unwrap().library_version.as_deref(), Some("1.1.0"));
    assert!(!dev.upgrade_available);
    let st = demo_of(&fake, &id);
    assert_eq!(st.floorplan_ids.len(), 2);
    for fp in &st.floorplan_ids {
        assert_eq!(mo(&fake, fp)["lw_floorplan"]["description"], "neu");
    }
    assert_eq!(
        fake.state()
            .managed_objects
            .iter()
            .filter(|m| m["type"] == "lw_floorplan")
            .count(),
        2,
        "nothing left over"
    );
}

/// Finding 4: an upgrade never deletes or replaces a floorplan the app did not create.
#[tokio::test]
async fn upgrade_never_touches_a_same_name_floorplan_of_the_user() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let old = fixture("1.0.0");
    let id = new_device(&client, &old, "D").await;
    apply::apply(&client, &old, &id, &req("fixture", false, false))
        .await
        .unwrap();
    let mine = add_foreign_floorplan(&fake, &id, "Erdgeschoss");
    let new = fixture("1.1.0");
    let p = upgrade::check(&client, &new, &id).await.unwrap().unwrap();
    assert!(p.floorplans_added.contains(&"Erdgeschoss".to_owned()));
    assert_eq!(
        p.floorplan_conflicts,
        vec![crate::model::FloorplanRef {
            id: mine.clone(),
            name: "Erdgeschoss".into()
        }]
    );
    let dev = upgrade::apply(&client, &new, &id, false).await.unwrap();
    assert!(exists(&fake, &mine), "the user's floorplan is still there");
    assert_eq!(mo(&fake, &mine)["lw_floorplan"]["description"], "mine");
    assert!(fake.state().binaries.contains_key("bin-foreign"));
    assert_eq!(
        dev.floorplans.len(),
        1,
        "the layout was skipped, nothing was created"
    );
    assert!(dev.demo.unwrap().floorplan_ids.is_empty());
}

/// Finding 13: a floorplan edited in the plugin is kept; the new layout comes next to it.
#[tokio::test]
async fn upgrade_keeps_an_edited_floorplan() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let old = fixture("1.0.0");
    let id = new_device(&client, &old, "D").await;
    apply::apply(&client, &old, &id, &req("fixture", true, false))
        .await
        .unwrap();
    let edited = demo_of(&fake, &id).floorplan_ids[0].clone();
    // Untouched: it is replaced.
    let mut new = fixture("1.1.0");
    set_descriptions(&mut new, "neu");
    let p = upgrade::check(&client, &new, &id).await.unwrap().unwrap();
    assert!(p.floorplans_edited.is_empty());
    // Somebody moves a widget in the plugin.
    {
        let mut s = fake.state();
        let m = s
            .managed_objects
            .iter_mut()
            .find(|m| m["id"] == edited.as_str())
            .unwrap();
        m["lw_floorplan"]["description"] = json!("von Hand");
    }
    let p = upgrade::check(&client, &new, &id).await.unwrap().unwrap();
    assert_eq!(p.floorplans_edited, vec!["Erdgeschoss"]);
    let dev = upgrade::apply(&client, &new, &id, false).await.unwrap();
    assert_eq!(
        mo(&fake, &edited)["lw_floorplan"]["description"],
        "von Hand"
    );
    assert_eq!(dev.floorplans.len(), 2);
    let st = dev.demo.unwrap();
    assert_eq!(st.floorplan_ids.len(), 1);
    let fresh = mo(&fake, &st.floorplan_ids[0]);
    assert_eq!(fresh["name"], "Erdgeschoss (v1.1.0)");
    assert_eq!(fresh["lw_floorplan"]["description"], "neu");
}

/// Finding 12: demo-pack level fields follow a three-way merge.
#[tokio::test]
async fn upgrade_merges_demo_level_fields_per_field() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let old = fixture("1.0.0");
    let a = new_device(&client, &old, "A").await;
    let b = new_device(&client, &old, "B").await;
    for d in [&a, &b] {
        apply::apply(&client, &old, d, &req("fixture", false, false))
            .await
            .unwrap();
    }
    // B was changed on the device.
    demo_state::patch_setup(
        &client,
        &old,
        &b,
        &DemoSetupPatch {
            send_interval_sec: Some(20),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mut new = fixture("1.1.0");
    new.demos[0].send_interval_sec = 60;
    new.demos[0].dali_groups[0].name = "Laden".into();

    let pa = upgrade::check(&client, &new, &a).await.unwrap().unwrap();
    assert_eq!(pa.demo_changes, vec!["daliGroups", "sendIntervalSec"]);
    assert!(pa.kept_overrides.is_empty());
    let pb = upgrade::check(&client, &new, &b).await.unwrap().unwrap();
    assert_eq!(pb.demo_changes, vec!["daliGroups"]);
    assert_eq!(pb.kept_overrides, vec!["sendIntervalSec"]);

    upgrade::apply(&client, &new, &a, false).await.unwrap();
    upgrade::apply(&client, &new, &b, false).await.unwrap();
    assert_eq!(demo_of(&fake, &a).send_interval_sec, 60);
    assert_eq!(
        demo_of(&fake, &b).send_interval_sec,
        20,
        "local override kept"
    );
    for d in [&a, &b] {
        assert_eq!(demo_of(&fake, d).dali_groups.unwrap()[0].name, "Laden");
        assert_eq!(demo_of(&fake, d).library_version.as_deref(), Some("1.1.0"));
    }
    // The next upgrade still sees B's override.
    let mut newer = fixture("1.2.0");
    newer.demos[0].send_interval_sec = 90;
    newer.demos[0].dali_groups[0].name = "Laden".into();
    let p = upgrade::check(&client, &newer, &b).await.unwrap().unwrap();
    assert_eq!(p.kept_overrides, vec!["sendIntervalSec"]);
    let p = upgrade::check(&client, &newer, &a).await.unwrap().unwrap();
    assert_eq!(p.demo_changes, vec!["sendIntervalSec"]);
}

/// Finding 14: after an exported pack was released, the device it came from keeps working.
#[tokio::test]
async fn released_export_does_not_break_the_source_device() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let lib = fixture("1.0.0");
    let id = new_device(&client, &lib, "Original").await;
    apply::apply(&client, &lib, &id, &req("fixture", false, false))
        .await
        .unwrap();
    demo_state::patch_setup(&client, &lib, &id, &custom_slave_patch())
        .await
        .unwrap();

    // The pack is released: the custom entries are library entries now.
    let mut released = fixture("1.1.0");
    let patch = custom_slave_patch();
    released.items.extend(patch.custom_items.clone().unwrap());
    released.slaves.extend(patch.custom_slaves.clone().unwrap());

    let p = upgrade::check(&client, &released, &id)
        .await
        .unwrap()
        .unwrap();
    let mut now_in_library = p.custom_now_in_library.clone();
    now_in_library.sort();
    assert_eq!(now_in_library, vec!["extra", "extra_temp"]);

    // `update_demo_setup` used to fail forever ("item id … is already used").
    let st = demo_state::patch_setup(
        &client,
        &released,
        &id,
        &DemoSetupPatch {
            send_interval_sec: Some(45),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(st.custom_items.is_empty() && st.custom_slaves.is_empty());
    assert!(
        st.includes.contains(&"extra_temp".to_owned()),
        "the item stays selected"
    );
    assert_eq!(st.slaves.iter().find(|s| s.key == "extra").unwrap().id, 18);
    assert_eq!(demo_of(&fake, &id).slaves.len(), 3);
    // A NEW custom item that collides with the library is still an error.
    let mut bad = custom_slave_patch();
    if let Some(items) = &mut bad.custom_items
        && let LibraryItem::Modbus { id, .. } = &mut items[0]
    {
        *id = "freezer_temp".into();
    }
    assert!(
        demo_state::patch_setup(&client, &released, &id, &bad)
            .await
            .is_err()
    );
    // Applying the pack again carries no warning either.
    let plan = apply::plan(&released, released.demo("fixture").unwrap(), Some(&st)).unwrap();
    assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
}

/// Finding 15: a deselected slave / register keeps its numbers (D6: never renumbered).
#[tokio::test]
async fn deselected_slaves_keep_their_numbers() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let lib = fixture("1.0.0");
    let id = new_device(&client, &lib, "D").await;
    apply::apply(&client, &lib, &id, &req("fixture", false, false))
        .await
        .unwrap();
    let before = demo_of(&fake, &id);
    let klima = before
        .slaves
        .iter()
        .find(|s| s.key == "klima")
        .unwrap()
        .clone();
    assert_eq!((klima.id, klima.address), (19, 239));

    let excluded = demo_state::patch_setup(
        &client,
        &lib,
        &id,
        &DemoSetupPatch {
            excludes: Some(vec!["ac_mode".into(), "ac_setpoint".into()]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(excluded.slaves.iter().all(|s| s.key != "klima"));
    let retired = excluded
        .retired_slaves
        .iter()
        .find(|s| s.key == "klima")
        .unwrap();
    assert_eq!((retired.id, retired.address), (19, 239));
    assert_eq!(retired.registers, klima.registers);
    assert_eq!(demo_of(&fake, &id).slaves.len(), 1);

    // A new slave must not take the retired numbers.
    let with_extra = demo_state::patch_setup(&client, &lib, &id, &custom_slave_patch())
        .await
        .unwrap();
    let extra = with_extra.slaves.iter().find(|s| s.key == "extra").unwrap();
    assert_eq!((extra.id, extra.address), (18, 238));

    // Selecting the items again brings the slave back under its old numbers and registers.
    let back = demo_state::patch_setup(
        &client,
        &lib,
        &id,
        &DemoSetupPatch {
            excludes: Some(vec![]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        back.slaves.iter().find(|s| s.key == "klima").unwrap(),
        &klima
    );
    assert!(back.retired_slaves.iter().all(|s| s.key != "klima"));
}

/// Finding 25: an apply honours the advisory lock of another presenter and keeps it.
#[tokio::test]
async fn apply_respects_a_foreign_lock() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let lib = fixture("1.0.0");
    let id = new_device(&client, &lib, "D").await;
    apply::apply(&client, &lib, &id, &req("fixture", false, false))
        .await
        .unwrap();
    client
        .update_managed_object(
            &id,
            &demo_state::lock_body(Some(&demo_state::new_lock("someone-else"))).unwrap(),
        )
        .await
        .unwrap();
    let err = apply::apply(&client, &lib, &id, &req("fixture", false, false))
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::DeviceLocked { .. }));
    let mut forced = req("fixture", false, false);
    forced.force = true;
    apply::apply(&client, &lib, &id, &forced).await.unwrap();
    assert_eq!(
        mo(&fake, &id)["lw_demoLock"]["user"],
        "someone-else",
        "an apply does not clear their marker"
    );
}

/// Finding 10: the snapshot and the lock have their own fragments; `lw_demo` never carries them.
#[tokio::test]
async fn lock_and_snapshot_live_outside_lw_demo() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let lib = fixture("1.0.0");
    let id = new_device(&client, &lib, "D").await;
    apply::apply(&client, &lib, &id, &req("fixture", false, false))
        .await
        .unwrap();
    let snap = DemoSnapshot {
        saved_at: "2026-10-12T12:00:00.000Z".into(),
        values: [("freezer_temp".to_owned(), -12.0)].into_iter().collect(),
    };
    client
        .update_managed_object(&id, &demo_state::snapshot_body(Some(&snap)).unwrap())
        .await
        .unwrap();
    demo_state::write_lock(&client, &id, "demo").await.unwrap();
    // A setup edit from another laptop does not revert either.
    let st = demo_state::patch_setup(
        &client,
        &lib,
        &id,
        &DemoSetupPatch {
            realistic_mode: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(st.snapshot.as_ref(), Some(&snap));
    assert_eq!(st.lock.as_ref().unwrap().user, "demo");
    let m = mo(&fake, &id);
    assert!(m["lw_demo"].get("lock").is_none() && m["lw_demo"].get("snapshot").is_none());
    assert_eq!(m["lw_demoLock"]["user"], "demo");
    // A lock / snapshot written by an older app inside lw_demo is still read.
    let legacy = json!({ "lw_demo": { "schemaVersion": 1, "sendIntervalSec": 30,
        "lock": { "user": "old", "since": "2026-10-12T12:00:00Z" } } });
    assert_eq!(
        demo_state::read(&legacy)
            .unwrap()
            .unwrap()
            .lock
            .unwrap()
            .user,
        "old"
    );
}

/// Finding 20: parent groups (incl. the rollback) and the grouping in the device list.
#[tokio::test]
async fn create_with_parent_group_lists_grouped_and_rolls_back() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let lib = fixture("1.0.0");
    let group = fake.insert_managed_object(json!({ "name": "Nord", "c8y_IsDeviceGroup": {} }));
    let create = |name: &str| CreateDeviceRequest {
        name: name.into(),
        description: None,
        position: None,
        parent_group_id: Some(group.clone()),
        apply_demo: None,
    };
    let dev = crud::create(&client, &lib, &create("In Gruppe"))
        .await
        .unwrap();
    assert_eq!(
        dev.parent_group,
        Some(GroupRef {
            id: group.clone(),
            name: "Nord".into()
        })
    );
    assert_eq!(fake.state().child_assets[&group], vec![dev.id.clone()]);
    let list = crud::list(&client, &lib, None).await.unwrap();
    assert_eq!(list[0].parent_group.as_ref().unwrap().name, "Nord");
    let loose = new_device(&client, &lib, "Ohne").await;
    let list = crud::list(&client, &lib, None).await.unwrap();
    assert_eq!(
        list.iter().find(|d| d.id == loose).unwrap().parent_group,
        None
    );

    // Linking fails: the device is removed again.
    fake.fail_next(
        "POST",
        &format!("/inventory/managedObjects/{group}/childAssets"),
        500,
    );
    let before = fake.state().managed_objects.len();
    let err = crud::create(&client, &lib, &create("Scheitert")).await;
    assert!(err.is_err());
    assert_eq!(
        fake.state().managed_objects.len(),
        before,
        "no half-created device"
    );
}

/// Finding 20: a counter starts from the NEWEST stored value (by time), whatever the order.
#[tokio::test]
async fn counter_is_seeded_from_the_newest_stored_value() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let lib = fixture("1.0.0");
    let id = new_device(&client, &lib, "D").await;
    apply::apply(&client, &lib, &id, &req("fixture", false, false))
        .await
        .unwrap();
    {
        let mut s = fake.state();
        for (t, v) in [
            ("2026-10-12T10:00:00.000Z", 1500),
            ("2026-10-12T12:00:00.000Z", 1100),
            ("2026-10-12T11:00:00.000Z", 1300),
        ] {
            s.measurements.push(json!({
                "source": { "id": id }, "type": "impulseCounter", "time": t,
                "Gaszähler": { "gas-meter": { "value": v } }
            }));
        }
        // A different series of the same fragment must not be picked up, nor the old
        // firmware naming (D10).
        s.measurements.push(json!({
            "source": { "id": id }, "type": "impulseCounter", "time": "2026-10-12T13:00:00.000Z",
            "Gaszähler": { "other": { "value": 9999 } }
        }));
        s.measurements.push(json!({
            "source": { "id": id }, "type": "digitalInput", "time": "2026-10-12T13:00:00.000Z",
            "LW Digital Input": { "ImpulseCounter03": { "value": 8888 } }
        }));
    }
    let prepared = crate::sim::session::prepare(
        &client,
        &lib,
        &StartSessionRequest {
            device_id: id.clone(),
            force: false,
            auto_accept: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(prepared.params.counter_seeds["gas-meter"], 1100.0);
    // `revert=true` and the series were part of the query.
    assert!(
        fake.requests()
            .iter()
            .any(|r| r.path == "/measurement/measurements"
                && r.query.contains("revert=true")
                && r.query.contains("valueFragmentSeries=gas-meter"))
    );
    // prepare takes the lock last; a failed prepare (unknown demo item) leaves none behind.
    assert_eq!(mo(&fake, &id)["lw_demoLock"]["user"], "demo");
}

/// Finding 24: `prepare` fails on a broken setup before it writes the lock.
#[tokio::test]
async fn prepare_takes_the_lock_only_after_the_setup_resolves() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let lib = fixture("1.0.0");
    let id = new_device(&client, &lib, "D").await;
    apply::apply(&client, &lib, &id, &req("fixture", false, false))
        .await
        .unwrap();
    // A slave the library does not know.
    let mut st = demo_of(&fake, &id);
    st.includes.push("does-not-exist".into());
    st.custom_items.push(LibraryItem::Modbus {
        id: "broken".into(),
        slave: "ghost".into(),
        title: LocalizedText {
            de: "x".into(),
            en: "x".into(),
        },
        tags: vec![],
        register_type: None,
        unit: None,
        datatype: None,
        writable: false,
        control: None,
        generator: None,
        effects: vec![],
    });
    demo_state::write(&client, &id, &st).await.unwrap();
    let res = crate::sim::session::prepare(
        &client,
        &lib,
        &StartSessionRequest {
            device_id: id.clone(),
            force: false,
            auto_accept: false,
        },
    )
    .await;
    assert!(res.is_err());
    assert!(
        mo(&fake, &id).get("lw_demoLock").is_none(),
        "no lock left behind"
    );
}

/// F6: an upgrade honours the advisory lock of another presenter, like an apply.
#[tokio::test]
async fn upgrade_respects_a_foreign_lock() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let old = fixture("1.0.0");
    let id = new_device(&client, &old, "D").await;
    apply::apply(&client, &old, &id, &req("fixture", false, false))
        .await
        .unwrap();
    client
        .update_managed_object(
            &id,
            &demo_state::lock_body(Some(&demo_state::new_lock("someone-else"))).unwrap(),
        )
        .await
        .unwrap();
    let mut new = fixture("1.1.0");
    for l in new.floorplans.values_mut() {
        l.widgets.retain(|w| w.id() != "w-door");
    }
    let err = upgrade::apply(&client, &new, &id, false).await.unwrap_err();
    assert!(matches!(err, AppError::DeviceLocked { .. }));
    assert_eq!(
        demo_of(&fake, &id).library_version.as_deref(),
        Some("1.0.0"),
        "nothing was written"
    );
    let dev = upgrade::apply(&client, &new, &id, true).await.unwrap();
    assert_eq!(dev.demo.unwrap().library_version.as_deref(), Some("1.1.0"));
}

/// F16: creating with a pack returns the pack's warnings; a failed pack removes the device.
#[tokio::test]
async fn create_with_pack_returns_the_result_and_rolls_back_a_failed_apply() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let lib = fixture("1.0.0");
    let create = |demo: &str| CreateDeviceRequest {
        name: "Neu".into(),
        description: None,
        position: None,
        parent_group_id: None,
        apply_demo: Some(req(demo, false, false)),
    };
    let out = apply::create_device(&client, &lib, &create("fixture"))
        .await
        .unwrap();
    assert_eq!(out.device.demo.unwrap().demo_id.as_deref(), Some("fixture"));
    assert!(out.warnings.is_empty());

    let before = fake.state().managed_objects.len();
    assert!(
        apply::create_device(&client, &lib, &create("no-such-demo"))
            .await
            .is_err()
    );
    assert_eq!(
        fake.state().managed_objects.len(),
        before,
        "the half-created device was deleted"
    );
}
