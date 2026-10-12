//! Device fragments: demo device skeleton (PRD §6.1) and `lw_config*` fragments.

use serde_json::{Map, Value, json};

use super::json_number;
use crate::error::{AppError, AppResult};
use crate::model::{
    Control, DaliGroup, DemoPack, DigitalInputConfig, DigitalInputEntry, DigitalInputType,
    LibraryItem, LwDemoState, ModbusDatatype, ModbusRegisterType, Position, SlaveAssignment,
    SlaveDefinition,
};

/// Max length of a digital input `unit` in UTF-8 bytes (the firmware truncates longer values),
/// §5.4.
pub const DI_UNIT_MAX_LEN: usize = 19;
/// Firmware default (and minimum written) `lw_configCore.maxControlGears`.
pub const DEFAULT_MAX_CONTROL_GEARS: u32 = 55;

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
/// Marker in `c8y_DeviceTypes` (the device list shows only these) and slave `description`.
pub const DEMO_DEVICE_TYPE_MARKER: &str = "demo";
/// `lw_configModbusSlaves[].description` of slaves written by the app.
pub const SLAVE_DESCRIPTION: &str = "lw-demo";
/// Default `lw_configGroups[].luxThreshold` (firmware default).
pub const DEFAULT_LUX_THRESHOLD: u32 = 4500;

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

/// `lw_configCore`, always the full 13-field object (D5): the firmware fails `lw_fetchCore` if
/// any field except `timeZone` / `ntpServer` is missing (firmware-core-v2.md §5.1).
///
/// `maxControlGears` is the firmware default 55; use [`config_core_fragment_for`] when the demo
/// has more lamps (a real Core raises `lw_converterExceed` above the limit).
pub fn config_core_fragment() -> Value {
    config_core_fragment_for(0)
}

/// [`config_core_fragment`] for a demo with `lamp_count` DALI lamps:
/// `maxControlGears = max(55, lamp_count)` (limit 1–64).
pub fn config_core_fragment_for(lamp_count: u32) -> Value {
    json!({
        "maxControlGears": DEFAULT_MAX_CONTROL_GEARS.max(lamp_count.min(64)),
        "measurementIntervalMinutes": 10,
        "digitalInputsMsgPerMinute": 10,
        "errorManagerSetThresholdCounts": 3,
        "errorManagerClearThresholdCounts": 2,
        "cloudWatchdogRestartTimeoutMinutes": 120,
        "isDaliScanActive": false,
        "isDaliScanForNewDevicesActive": true,
        "daliSchedulerIntervalSeconds": 60,
        "daliSchedulerDelayMilliseconds": 500,
        "daliSchedulerErrorIntervalSeconds": 60,
        "timeZone": "Europe/Berlin",
        "ntpServer": ""
    })
}

/// `lw_configRs485` (all five fields required by the firmware, §5.2).
pub fn config_rs485_fragment() -> Value {
    json!({ "isActive": true, "baudrate": 9600, "parity": "none", "stopBit": 1, "dataBits": 8 })
}

/// Managed-object body for a new demo device (PRD §6.1): no `c8y_RequiredAvailability` (never
/// "unavailable"), firmware `V 03.00.02`, full `lw_configCore`, empty `lw_configGroups` /
/// `lw_configDigitalInputs` / `lw_configModbusSlaves`. `lw_demo` is only written when given
/// (a device without it is a raw-only demo device).
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
    let lamps = lw_demo.map_or(0, |d| d.dali_lamp_count);
    body.insert("lw_configCore".into(), config_core_fragment_for(lamps));
    body.insert("lw_configRs485".into(), config_rs485_fragment());
    body.insert("lw_configGroups".into(), json!([]));
    body.insert("lw_configDigitalInputs".into(), json!([]));
    body.insert("lw_configModbusSlaves".into(), json!([]));
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

/// `lw_configGroups` (§5.3): one full firmware record per group; `stdLevel` = `maxLevel`,
/// empty weekly scheduler (7 × `{off:"", on:""}`, Sunday first = not configured), channel 1.
pub fn groups_fragment(groups: &[DaliGroup]) -> Value {
    let list: Vec<Value> = groups
        .iter()
        .map(|g| {
            json!({
                "id": g.id,
                "name": g.name,
                "minLevel": g.min_level,
                "maxLevel": g.max_level,
                "stdLevel": g.max_level,
                "dayState": false,
                "dynamicControl": g.dynamic_control,
                "luxThreshold": g.lux_threshold.unwrap_or(DEFAULT_LUX_THRESHOLD),
                "slaveOf": g.slave_of,
                "automaticSwitch": g.automatic_switch,
                "channel": 1,
                "scheduler": vec![json!({"off": "", "on": ""}); 7],
                "converters": g.converters,
            })
        })
        .collect();
    Value::Array(list)
}

/// Firmware `datatype` of a register: `bool` for coils, else the item's `datatype`, else
/// `uint16` for switch / select controls and for **writable holding** registers (`lw_modbusWrite`
/// `register` is FC6: one 16-bit word, 0-65535), `float32` otherwise.
fn register_datatype(item: &LibraryItem, register_type: ModbusRegisterType) -> &'static str {
    if register_type == ModbusRegisterType::Coil {
        return "bool";
    }
    let explicit = match item {
        LibraryItem::Modbus { datatype, .. } => *datatype,
        _ => None,
    };
    match explicit {
        Some(ModbusDatatype::Int16) => "int16",
        Some(ModbusDatatype::Uint16) => "uint16",
        Some(ModbusDatatype::Int32) => "int32",
        Some(ModbusDatatype::Uint32) => "uint32",
        Some(ModbusDatatype::Float32) => "float32",
        None => match item.control() {
            Some(Control::Switch { .. } | Control::Select { .. }) => "uint16",
            _ if writable_holding(item, register_type) => "uint16",
            _ => "float32",
        },
    }
}

fn writable_holding(item: &LibraryItem, register_type: ModbusRegisterType) -> bool {
    register_type == ModbusRegisterType::Holding
        && matches!(item, LibraryItem::Modbus { writable: true, .. })
}

/// One `lw_configModbusSlaves[]` entry (§5.5): `isSavingOnStorage: false`, `description:
/// "lw-demo"`, `registers[]` from the persisted assignment (name = item id, firmware register
/// fields), writable items additionally in `writeRegisters[] {name, address, type: coil|holding}`
/// (D5). Fails if an assigned register has no Modbus item of this slave in `items`.
pub fn modbus_slave_entry(
    slave: &SlaveDefinition,
    assignment: &SlaveAssignment,
    items: &[&LibraryItem],
) -> AppResult<Value> {
    let mut registers = Vec::new();
    let mut write_registers = Vec::new();
    for reg in &assignment.registers {
        let item = items
            .iter()
            .find(|i| i.id() == reg.item_id && i.slave_key() == Some(slave.key.as_str()))
            .ok_or_else(|| {
                AppError::InvalidInput(format!(
                    "register {} of slave {}: no such Modbus item",
                    reg.item_id, slave.key
                ))
            })?;
        let register_type = match reg.register_type {
            ModbusRegisterType::Coil => "coil",
            ModbusRegisterType::Holding => "holding",
            ModbusRegisterType::Input => "input",
        };
        registers.push(json!({
            "name": reg.item_id,
            "unit": item.unit().unwrap_or(""),
            "address": reg.address,
            "type": register_type,
            "datatype": register_datatype(item, reg.register_type),
            "scalingFactor": 0,
            "swap": "none",
            "changeOfValue": 0,
            "conversion": "",
            "custom": ""
        }));
        let writable = matches!(item, LibraryItem::Modbus { writable: true, .. });
        if let (true, Some(write_type)) = (writable, reg.register_type.write_register_type()) {
            write_registers.push(json!({
                "name": reg.item_id,
                "address": reg.address,
                "type": write_type
            }));
        }
    }
    Ok(json!({
        "id": assignment.id,
        "address": assignment.address,
        "name": slave.name,
        "requestInterval": slave.request_interval,
        "isSavingOnStorage": false,
        "isActive": true,
        "description": SLAVE_DESCRIPTION,
        "registers": registers,
        "writeRegisters": write_registers
    }))
}

/// `lw_configModbusSlaves` for all slaves (array position is irrelevant to the firmware).
pub fn modbus_slaves_fragment(
    slaves: &[(&SlaveDefinition, &SlaveAssignment)],
    items: &[&LibraryItem],
) -> AppResult<Value> {
    slaves
        .iter()
        .map(|(s, a)| modbus_slave_entry(s, a, items))
        .collect::<AppResult<Vec<_>>>()
        .map(Value::Array)
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

/// `lw_configDigitalInputs` (firmware-core-v2.md §5.4): `id`, `isActive`, `title` (D1 extra
/// field, ignored by the firmware), `type`, plus the type-specific fields only
/// (`daliAction`: `highLevelAction` / `lowLevelAction` / `groups`;
/// `impulseCounter`: `initCount` / `increment` / `unit`).
pub fn digital_inputs_fragment(entries: &[DigitalInputEntry]) -> Value {
    let list: Vec<Value> = entries
        .iter()
        .map(|e| {
            let s = &e.settings;
            let mut o = Map::new();
            o.insert("id".into(), json!(e.index));
            o.insert("isActive".into(), json!(s.is_active));
            o.insert("title".into(), json!(e.title));
            match s.input_type {
                DigitalInputType::Measurement => {
                    o.insert("type".into(), json!("measurement"));
                }
                DigitalInputType::DaliAction => {
                    o.insert("type".into(), json!("daliAction"));
                    if let Some(a) = &s.high_level_action {
                        o.insert("highLevelAction".into(), json!(a));
                    }
                    if let Some(a) = &s.low_level_action {
                        o.insert("lowLevelAction".into(), json!(a));
                    }
                    o.insert("groups".into(), json!(s.groups));
                }
                DigitalInputType::ImpulseCounter => {
                    o.insert("type".into(), json!("impulseCounter"));
                    o.insert("initCount".into(), json!(s.init_count.unwrap_or(0)));
                    o.insert("increment".into(), json!(s.increment.unwrap_or(1)));
                    if let Some(u) = &s.unit {
                        o.insert("unit".into(), json!(truncate_bytes(u, DI_UNIT_MAX_LEN)));
                    }
                }
            }
            Value::Object(o)
        })
        .collect();
    Value::Array(list)
}

/// `s` cut to at most `max` UTF-8 bytes, on a char boundary (what the firmware buffer holds).
pub fn truncate_bytes(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// `lw_configGroups`, `lw_configDigitalInputs`, `lw_configModbusSlaves` for applying a demo,
/// as one inventory patch object. `items` are the demo's resolved items (library selection plus
/// custom items); `slaves` carry their persisted assignments incl. register addresses.
pub fn config_fragments(
    demo: &DemoPack,
    items: &[&LibraryItem],
    slaves: &[(&SlaveDefinition, &SlaveAssignment)],
) -> AppResult<Value> {
    let inputs = demo
        .digital_inputs
        .iter()
        .map(|c| digital_input_entry(c, items))
        .collect::<AppResult<Vec<_>>>()?;
    Ok(json!({
        "lw_configGroups": groups_fragment(&demo.dali_groups),
        "lw_configDigitalInputs": digital_inputs_fragment(&inputs),
        "lw_configModbusSlaves": modbus_slaves_fragment(slaves, items)?,
    }))
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

    /// DI config / title mapping (D1): the item title lands in `title`, the series stays native.
    #[test]
    fn di_config_and_title_mapping() {
        let door = di_item("door", 2, "Tür offen", false);
        let gas = di_item("gas", 3, "Gaszähler", true);
        let pump = di_item("pump", 1, "Zapfsäule 1", false);
        let items = [&pump, &door, &gas];

        let mut pump_settings = settings(DigitalInputType::DaliAction);
        pump_settings.high_level_action = Some("on".into());
        pump_settings.low_level_action = Some("off".into());
        pump_settings.groups = vec![10, 2, 3];
        let mut gas_settings = settings(DigitalInputType::ImpulseCounter);
        gas_settings.init_count = Some(1000);
        gas_settings.increment = Some(10);
        gas_settings.unit = Some("pulse".into());

        let cfgs = [
            DigitalInputConfig {
                index: 1,
                item_id: "pump".into(),
                settings: pump_settings,
            },
            DigitalInputConfig {
                index: 2,
                item_id: "door".into(),
                settings: settings(DigitalInputType::Measurement),
            },
            DigitalInputConfig {
                index: 3,
                item_id: "gas".into(),
                settings: gas_settings,
            },
        ];
        let entries: Vec<DigitalInputEntry> = cfgs
            .iter()
            .map(|c| digital_input_entry(c, &items).unwrap())
            .collect();
        assert_eq!(entries[0].title, "Zapfsäule 1");

        // Same shape as the firmware doc example (firmware-core-v2.md §5.4).
        assert_eq!(
            digital_inputs_fragment(&entries),
            json!([
                {"id":1,"isActive":true,"title":"Zapfsäule 1","type":"daliAction",
                 "highLevelAction":"on","lowLevelAction":"off","groups":[10,2,3]},
                {"id":2,"isActive":true,"title":"Tür offen","type":"measurement"},
                {"id":3,"isActive":true,"title":"Gaszähler","type":"impulseCounter",
                 "initCount":1000,"increment":10,"unit":"pulse"}
            ])
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
    fn di_unit_is_truncated_to_19() {
        let mut s = settings(DigitalInputType::ImpulseCounter);
        s.unit = Some("x".repeat(25));
        let v = digital_inputs_fragment(&[DigitalInputEntry {
            index: 1,
            title: String::new(),
            settings: s,
        }]);
        assert_eq!(v[0]["unit"].as_str().unwrap().len(), 19);
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
        let core = b["lw_configCore"].as_object().unwrap();
        assert_eq!(core.len(), 13);
        assert_eq!(core["measurementIntervalMinutes"], 10);
        assert_eq!(core["timeZone"], "Europe/Berlin");
        assert_eq!(b["lw_configRs485"]["baudrate"], 9600);
        for f in [
            "lw_configGroups",
            "lw_configDigitalInputs",
            "lw_configModbusSlaves",
        ] {
            assert_eq!(b[f], json!([]), "{f}");
        }
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
    fn groups_are_full_firmware_records() {
        let g = groups_fragment(&[DaliGroup {
            id: 1,
            name: "Shop".into(),
            converters: vec![1, 2, 3],
            min_level: 0,
            max_level: 100,
            lux_threshold: None,
            slave_of: None,
            dynamic_control: false,
            automatic_switch: false,
        }]);
        let e = &g[0];
        assert_eq!(e["id"], 1);
        assert_eq!(e["maxLevel"], 100);
        assert_eq!(e["stdLevel"], 100);
        assert_eq!(e["luxThreshold"], 4500);
        assert_eq!(e["slaveOf"], Value::Null);
        assert_eq!(e["converters"], json!([1, 2, 3]));
        assert_eq!(e["scheduler"].as_array().unwrap().len(), 7);
        assert_eq!(e["scheduler"][0], json!({"off": "", "on": ""}));
    }

    fn modbus(
        id: &str,
        writable: bool,
        control: Option<Control>,
        unit: Option<&str>,
    ) -> LibraryItem {
        LibraryItem::Modbus {
            id: id.into(),
            slave: "klima".into(),
            title: LocalizedText {
                de: id.into(),
                en: id.into(),
            },
            tags: vec![],
            register_type: None,
            unit: unit.map(str::to_owned),
            datatype: None,
            writable,
            control,
            generator: None,
            effects: vec![],
        }
    }

    /// Writable items land in `registers[]` and `writeRegisters[]` (D5); input registers only in
    /// `registers[]`; `isSavingOnStorage` false, `description` "lw-demo".
    #[test]
    fn modbus_slave_entry_has_registers_and_write_registers() {
        use crate::model::RegisterAssignment;
        let on = modbus("ac_on", true, Some(Control::Switch { default: 0.0 }), None);
        let sp = modbus("ac_setpoint", true, None, Some("°C"));
        let temp = modbus("room_temp", false, None, Some("°C"));
        assert_eq!(
            register_datatype(&temp, ModbusRegisterType::Holding),
            "float32"
        );
        let slave = SlaveDefinition {
            key: "klima".into(),
            name: "Klima".into(),
            request_interval: 60,
            alarms: vec![],
        };
        let reg = |id: &str, t, address| RegisterAssignment {
            item_id: id.into(),
            register_type: t,
            address,
        };
        let assignment = SlaveAssignment {
            key: "klima".into(),
            id: 19,
            address: 239,
            registers: vec![
                reg("ac_on", ModbusRegisterType::Coil, 1),
                reg("ac_setpoint", ModbusRegisterType::Holding, 40001),
                reg("room_temp", ModbusRegisterType::Input, 30001),
            ],
        };
        let e = modbus_slave_entry(&slave, &assignment, &[&on, &sp, &temp]).unwrap();
        assert_eq!(e["id"], 19);
        assert_eq!(e["address"], 239);
        assert_eq!(e["name"], "Klima");
        assert_eq!(e["isSavingOnStorage"], false);
        assert_eq!(e["description"], "lw-demo");
        assert_eq!(e["registers"].as_array().unwrap().len(), 3);
        assert_eq!(e["registers"][0]["type"], "coil");
        assert_eq!(e["registers"][0]["datatype"], "bool");
        assert_eq!(e["registers"][1]["unit"], "°C");
        // Writable holding = FC6 single word: default `uint16`, not `float32`.
        assert_eq!(e["registers"][1]["datatype"], "uint16");
        assert_eq!(e["registers"][2]["type"], "input");
        assert_eq!(
            e["writeRegisters"],
            json!([
                {"name": "ac_on", "address": 1, "type": "coil"},
                {"name": "ac_setpoint", "address": 40001, "type": "holding"}
            ])
        );
        // A register without an item is an error.
        let mut broken = assignment.clone();
        broken
            .registers
            .push(reg("ghost", ModbusRegisterType::Holding, 40002));
        assert!(modbus_slave_entry(&slave, &broken, &[&on, &sp, &temp]).is_err());
    }

    /// `lw_configCore` equals the firmware-core-v2.md §5.1 JSON literally (all 13 fields).
    #[test]
    fn config_core_matches_the_firmware_doc() {
        assert_eq!(
            config_core_fragment(),
            json!({ "maxControlGears": 55, "measurementIntervalMinutes": 10,
                "digitalInputsMsgPerMinute": 10, "errorManagerSetThresholdCounts": 3,
                "errorManagerClearThresholdCounts": 2, "cloudWatchdogRestartTimeoutMinutes": 120,
                "isDaliScanActive": false, "isDaliScanForNewDevicesActive": true,
                "daliSchedulerIntervalSeconds": 60, "daliSchedulerDelayMilliseconds": 500,
                "daliSchedulerErrorIntervalSeconds": 60, "timeZone": "Europe/Berlin",
                "ntpServer": "" })
        );
    }

    #[test]
    fn max_control_gears_grows_with_the_lamp_count() {
        assert_eq!(config_core_fragment_for(10)["maxControlGears"], 55);
        assert_eq!(config_core_fragment_for(55)["maxControlGears"], 55);
        assert_eq!(config_core_fragment_for(60)["maxControlGears"], 60);
        assert_eq!(config_core_fragment_for(64)["maxControlGears"], 64);
        assert_eq!(config_core_fragment_for(99)["maxControlGears"], 64);
        let dev = NewDemoDevice {
            name: "x",
            description: None,
            position: None,
            serial_number: "DEMO-1",
        };
        let state: LwDemoState = serde_json::from_value(json!({
            "schemaVersion": 1, "daliLampCount": 60, "sendIntervalSec": 30
        }))
        .unwrap();
        let b = demo_device_body(&dev, Some(&state)).unwrap();
        assert_eq!(b["lw_configCore"]["maxControlGears"], 60);
    }

    #[test]
    fn di_unit_truncation_respects_utf8_bytes() {
        let mut s = settings(DigitalInputType::ImpulseCounter);
        s.unit = Some("é".repeat(12)); // 24 bytes → 9 whole chars = 18 bytes
        let v = digital_inputs_fragment(&[DigitalInputEntry {
            index: 1,
            title: String::new(),
            settings: s,
        }]);
        assert_eq!(v[0]["unit"].as_str().unwrap(), "é".repeat(9));
    }
}
