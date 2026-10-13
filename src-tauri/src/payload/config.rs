//! Device fragments: demo device skeleton (PRD §6.1) and the device-side digital input entry.
//!
//! The simulator writes no `lw_config*` fragment (DECISIONS D10): DALI groups, digital inputs
//! and slaves live in `lw_demo` and the library.

use serde_json::{Map, Value, json};

use super::json_number;
use crate::error::{AppError, AppResult};
use crate::model::{
    DigitalInputConfig, DigitalInputEntry, DigitalInputType, LibraryItem, LwDemoState, Position,
};

/// Max length of a digital input `unit` in UTF-8 bytes (the firmware truncates longer values),
/// §5.4.
pub const DI_UNIT_MAX_LEN: usize = 19;

/// Supported operations of a demo device (PRD §6.1).
pub const SUPPORTED_OPERATIONS: &[&str] = &[
    "c8y_Restart",
    "c8y_Firmware",
    "c8y_MeasurementRequestOperation",
    "lw_setGroupColor",
    "lw_fetchCore",
    "lw_fetchGroups",
    "lw_fetchMotion",
    "lw_fetchBrightness",
    "lw_fetchDigitalInputs",
    "lw_fetchModbusSlaves",
    "lw_fetchAlarmBlacklist",
    "lw_fetchTemperatures",
    "lw_fetchRs485",
    "lw_switchGroupMode",
    "lw_switchConverterState",
    "lw_daliReset",
    "lw_modbusWrite",
    "lw_daliVendorScan",
    "lw_daliScan",
];

/// Marks the device as its own agent. Without it Cumulocity rejects every operation for the
/// device (`422 Agent for device '<id>' does not exist in inventory!`): an operation is always
/// addressed to the agent that serves the device, and a device without an agent ancestor must
/// be one itself. A real Core v2 registers with it.
pub const AGENT_FRAGMENT: &str = "com_cumulocity_model_Agent";

/// Simulated firmware version (≥ 01.06.00 enables Modbus controls in the floorplan plugin).
pub const FIRMWARE_VERSION: &str = "V 03.00.02";

/// Managed-object `type` of a Core v2.
pub const DEVICE_TYPE: &str = "LW_Core_v2";
/// `c8y_Hardware.model`; makes the floorplan tab appear.
pub const HARDWARE_MODEL: &str = "Lichtwart Core v2";
/// `c8y_Firmware.name` of the simulator.
pub const FIRMWARE_NAME: &str = "SIMULATOR";
/// Marker in `c8y_DeviceTypes` (the device list shows only these).
pub const DEMO_DEVICE_TYPE_MARKER: &str = "demo";

/// Identity of a new demo device (PRD §6.1).
#[derive(Debug, Clone, PartialEq)]
pub struct NewDemoDevice<'a> {
    pub name: &'a str,
    /// Plain top-level `description` (PRD §19 open point: maybe mirror to `c8y_Notes`).
    pub description: Option<&'a str>,
    pub position: Option<Position>,
    /// `DEMO-<random>`, see [`demo_serial_number`].
    pub serial_number: &'a str,
}

/// `DEMO-<8 hex digits>` from a random number supplied by the caller.
pub fn demo_serial_number(random: u32) -> String {
    format!("DEMO-{random:08X}")
}

/// Managed-object body for a new demo device (PRD §6.1): no `c8y_RequiredAvailability` (never
/// "unavailable"), firmware `V 03.00.02`, no `lw_config*` fragment (D10). `lw_demo` is only
/// written when given (a device without it is a raw-only demo device).
pub fn demo_device_body(
    device: &NewDemoDevice<'_>,
    lw_demo: Option<&LwDemoState>,
) -> AppResult<Value> {
    let mut body = Map::new();
    body.insert("name".into(), json!(device.name));
    if let Some(d) = device.description {
        body.insert("description".into(), json!(d));
    }
    body.insert("type".into(), json!(DEVICE_TYPE));
    body.insert("c8y_IsDevice".into(), json!({}));
    body.insert(AGENT_FRAGMENT.into(), json!({}));
    body.insert("c8y_DeviceTypes".into(), json!([DEMO_DEVICE_TYPE_MARKER]));
    if let Some(p) = device.position {
        body.insert(
            "c8y_Position".into(),
            json!({ "lat": json_number(p.lat), "lng": json_number(p.lng), "alt": json_number(p.alt) }),
        );
    }
    body.insert(
        "c8y_Hardware".into(),
        json!({ "model": HARDWARE_MODEL, "serialNumber": device.serial_number, "revision": "demo" }),
    );
    body.insert(
        "c8y_Firmware".into(),
        json!({ "name": FIRMWARE_NAME, "version": FIRMWARE_VERSION }),
    );
    body.insert(
        "c8y_SupportedOperations".into(),
        json!(SUPPORTED_OPERATIONS),
    );
    if let Some(demo) = lw_demo {
        let v = serde_json::to_value(demo).map_err(|e| AppError::Internal(e.to_string()))?;
        body.insert("lw_demo".into(), v);
    }
    Ok(Value::Object(body))
}

/// The fragments a demo device needs for the simulator to work (operations reach it only as
/// its own agent, the operation menus read `c8y_SupportedOperations`), missing on devices
/// created by older app versions. Returns the PUT body that adds them, `None` when nothing is
/// missing (the caller writes nothing then). Only the fragments are touched, never `lw_*`.
pub fn missing_required_fragments(mo: &Value) -> Option<Value> {
    let mut patch = Map::new();
    if !mo.get("c8y_IsDevice").is_some_and(Value::is_object) {
        patch.insert("c8y_IsDevice".into(), json!({}));
    }
    if !mo.get(AGENT_FRAGMENT).is_some_and(Value::is_object) {
        patch.insert(AGENT_FRAGMENT.into(), json!({}));
    }
    let supported = mo.get("c8y_SupportedOperations").and_then(Value::as_array);
    let complete = supported.is_some_and(|ops| {
        SUPPORTED_OPERATIONS
            .iter()
            .all(|want| ops.iter().any(|o| o == want))
    });
    if !complete {
        // Keep whatever else someone added, append the missing ones in catalog order.
        let mut ops: Vec<Value> = supported.cloned().unwrap_or_default();
        for want in SUPPORTED_OPERATIONS {
            if !ops.iter().any(|o| o == want) {
                ops.push(json!(want));
            }
        }
        patch.insert("c8y_SupportedOperations".into(), Value::Array(ops));
    }
    (!patch.is_empty()).then_some(Value::Object(patch))
}

/// Device-side entry for one demo digital input: the item's German title becomes the
/// `title` (D1: the item is the single source of truth, the demo entry only references it).
pub fn digital_input_entry(
    cfg: &DigitalInputConfig,
    items: &[&LibraryItem],
) -> AppResult<DigitalInputEntry> {
    let item = items
        .iter()
        .find(|i| i.id() == cfg.item_id)
        .ok_or_else(|| AppError::InvalidInput(format!("digital input item {}", cfg.item_id)))?;
    let item_type = match item {
        LibraryItem::DigitalInput { .. } => None,
        LibraryItem::ImpulseCounter { .. } => Some(DigitalInputType::ImpulseCounter),
        _ => {
            return Err(AppError::InvalidInput(format!(
                "{} is not a digital input item",
                cfg.item_id
            )));
        }
    };
    if item.index() != Some(cfg.index) {
        return Err(AppError::InvalidInput(format!(
            "digital input {}: item {} has index {:?}",
            cfg.index,
            cfg.item_id,
            item.index()
        )));
    }
    if item_type.is_some_and(|t| t != cfg.settings.input_type) {
        return Err(AppError::InvalidInput(format!(
            "digital input {}: item {} is an impulse counter",
            cfg.index, cfg.item_id
        )));
    }
    Ok(DigitalInputEntry {
        index: cfg.index,
        title: item.title().de.clone(),
        settings: cfg.settings.clone(),
    })
}

/// `s` cut to at most `max` UTF-8 bytes, on a char boundary (what the firmware buffer holds).
pub fn truncate_bytes(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DigitalInputSettings, LocalizedText};

    fn di_item(id: &str, index: u32, de: &str, counter: bool) -> LibraryItem {
        let (id, title, tags, unit) = (
            id.to_owned(),
            LocalizedText {
                de: de.into(),
                en: de.into(),
            },
            vec![],
            None,
        );
        let (control, generator, effects) = (None, None, vec![]);
        if counter {
            LibraryItem::ImpulseCounter {
                id,
                index,
                title,
                tags,
                unit,
                control,
                generator,
                effects,
            }
        } else {
            LibraryItem::DigitalInput {
                id,
                index,
                title,
                tags,
                unit,
                control,
                generator,
                effects,
            }
        }
    }

    fn settings(t: DigitalInputType) -> DigitalInputSettings {
        DigitalInputSettings {
            input_type: t,
            is_active: true,
            init_count: None,
            increment: None,
            unit: None,
            high_level_action: None,
            low_level_action: None,
            groups: vec![],
        }
    }

    /// DI config / title mapping (D1): the item title lands in `title`, the settings are kept.
    #[test]
    fn di_config_and_title_mapping() {
        let door = di_item("door", 2, "Tür offen", false);
        let gas = di_item("gas", 3, "Gaszähler", true);
        let pump = di_item("pump", 1, "Zapfsäule 1", false);
        let items = [&pump, &door, &gas];

        let mut pump_settings = settings(DigitalInputType::DaliAction);
        pump_settings.high_level_action = Some("on".into());
        pump_settings.groups = vec![10, 2, 3];
        let mut gas_settings = settings(DigitalInputType::ImpulseCounter);
        gas_settings.init_count = Some(1000);

        let pump_entry = digital_input_entry(
            &DigitalInputConfig {
                index: 1,
                item_id: "pump".into(),
                settings: pump_settings.clone(),
            },
            &items,
        )
        .unwrap();
        assert_eq!(pump_entry.title, "Zapfsäule 1");
        assert_eq!(pump_entry.settings, pump_settings);
        let gas_entry = digital_input_entry(
            &DigitalInputConfig {
                index: 3,
                item_id: "gas".into(),
                settings: gas_settings,
            },
            &items,
        )
        .unwrap();
        assert_eq!(gas_entry.title, "Gaszähler");
        assert_eq!(gas_entry.settings.init_count, Some(1000));
        // A counter item cannot be a plain input.
        assert!(
            digital_input_entry(
                &DigitalInputConfig {
                    index: 3,
                    item_id: "gas".into(),
                    settings: settings(DigitalInputType::Measurement),
                },
                &items,
            )
            .is_err()
        );
    }

    #[test]
    fn di_entry_rejects_wrong_item() {
        let door = di_item("door", 2, "Tür", false);
        let cfg = DigitalInputConfig {
            index: 5,
            item_id: "door".into(),
            settings: settings(DigitalInputType::Measurement),
        };
        assert!(digital_input_entry(&cfg, &[&door]).is_err());
        let cfg = DigitalInputConfig {
            index: 2,
            item_id: "missing".into(),
            settings: settings(DigitalInputType::Measurement),
        };
        assert!(digital_input_entry(&cfg, &[&door]).is_err());
    }

    #[test]
    fn device_body_matches_prd() {
        let dev = NewDemoDevice {
            name: "Tankstelle Musterstadt",
            description: Some("Free text"),
            position: Some(Position {
                lat: 48.137,
                lng: 11.575,
                alt: 0.0,
            }),
            serial_number: &demo_serial_number(0xBEEF),
        };
        let b = demo_device_body(&dev, None).unwrap();
        assert_eq!(b["type"], "LW_Core_v2");
        assert_eq!(b["description"], "Free text");
        assert_eq!(b["c8y_DeviceTypes"], json!(["demo"]));
        assert_eq!(b["c8y_IsDevice"], json!({}));
        assert_eq!(b["com_cumulocity_model_Agent"], json!({}));
        assert_eq!(missing_required_fragments(&b), None);
        assert_eq!(
            b["c8y_Position"],
            json!({"lat": 48.137, "lng": 11.575, "alt": 0})
        );
        assert_eq!(
            b["c8y_Hardware"],
            json!({"model": "Lichtwart Core v2", "serialNumber": "DEMO-0000BEEF", "revision": "demo"})
        );
        assert_eq!(
            b["c8y_Firmware"],
            json!({"name": "SIMULATOR", "version": "V 03.00.02"})
        );
        assert_eq!(b["c8y_SupportedOperations"].as_array().unwrap().len(), 19);
        assert!(b.get("c8y_RequiredAvailability").is_none());
        assert!(b.get("lw_demo").is_none());
        // D10: no `lw_config*` fragment at all.
        assert!(
            b.as_object()
                .unwrap()
                .keys()
                .all(|k| !k.starts_with("lw_config")),
            "{b}"
        );
    }

    #[test]
    fn legacy_device_gets_the_missing_fragments_only() {
        let legacy = json!({
            "c8y_IsDevice": {},
            "c8y_SupportedOperations": ["c8y_Restart", "custom_Op"],
            "lw_demo": { "x": 1 }
        });
        let patch = missing_required_fragments(&legacy).unwrap();
        let p = patch.as_object().unwrap();
        assert_eq!(p.len(), 2, "{patch}");
        assert_eq!(patch["com_cumulocity_model_Agent"], json!({}));
        let ops = patch["c8y_SupportedOperations"].as_array().unwrap();
        assert_eq!(ops.len(), SUPPORTED_OPERATIONS.len() + 1);
        assert_eq!(ops[1], "custom_Op");
        assert!(ops.contains(&json!("lw_daliScan")));

        let bare = missing_required_fragments(&json!({})).unwrap();
        assert_eq!(bare["c8y_IsDevice"], json!({}));
        assert_eq!(
            bare["c8y_SupportedOperations"].as_array().unwrap().len(),
            SUPPORTED_OPERATIONS.len()
        );
    }

    #[test]
    fn truncation_respects_utf8_bytes() {
        assert_eq!(truncate_bytes(&"x".repeat(25), DI_UNIT_MAX_LEN).len(), 19);
        // 24 bytes → 9 whole chars = 18 bytes
        assert_eq!(
            truncate_bytes(&"é".repeat(12), DI_UNIT_MAX_LEN),
            "é".repeat(9)
        );
    }
}
