//! Alarm bodies and the fault catalog (PRD §7.2, §7.4; firmware-core-v2.md §2).
//!
//! Native alarm types / texts / severities are copied from the firmware table, including the
//! real typo `referenceMeasuerment`. Custom alarms (`lw_demo_*`, D2) are a simulator convention.

use serde_json::{Value, json};

use super::naming::{self, MAX_SLAVES};
use crate::error::{AppError, AppResult};
use crate::model::{
    FaultDefinition, FaultTarget, LibraryItem, Severity, SlaveAssignment, SlaveDefinition,
};

/// A resolved alarm: Cumulocity `type`, `text`, `severity`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlarmSpec {
    pub alarm_type: String,
    pub text: String,
    pub severity: Severity,
}

/// One native per-device-number alarm: type = `<prefix><NN><suffix>`, text =
/// `<text prefix><NN><text suffix>` (NN = `%02u`, channel always `1`).
struct NumberedAlarm {
    prefix: &'static str,
    suffix: &'static str,
    text_prefix: &'static str,
    text_suffix: &'static str,
}

const fn numbered(
    prefix: &'static str,
    suffix: &'static str,
    text_prefix: &'static str,
    text_suffix: &'static str,
) -> NumberedAlarm {
    NumberedAlarm {
        prefix,
        suffix,
        text_prefix,
        text_suffix,
    }
}

impl NumberedAlarm {
    fn spec(&self, n: u32, severity: Severity) -> AlarmSpec {
        AlarmSpec {
            alarm_type: format!("{}{n:02}{}", self.prefix, self.suffix),
            text: format!("{}{n:02}{}", self.text_prefix, self.text_suffix),
            severity,
        }
    }
}

/// Index of `lw_controlGearFailure_1_NN` / `lw_converter_1_NN_lost` in [`LAMP_ALARMS`].
const LAMP_CONTROL_GEAR_FAILURE: usize = 0;
const LAMP_LOST: usize = 11;

/// Per-lamp DALI alarms (firmware §2), all MAJOR. Same set the floorplan plugin suggests for a
/// DALI lamp widget (firmware naming `LW Dali Control Gear 1-NN`).
const LAMP_ALARMS: [NumberedAlarm; 14] = [
    numbered(
        "lw_controlGearFailure_1_",
        "",
        "Control Gear Failure 1-",
        "",
    ),
    numbered("lw_converterFailure_1_", "", "Converter Failure 1-", ""),
    numbered(
        "lw_lampFailure_1_",
        "_shortCircuit",
        "Lamp Failure 1-",
        " Short Circuit",
    ),
    numbered(
        "lw_lampFailure_1_",
        "_openCircuit",
        "Lamp Failure 1-",
        " Open Circuit",
    ),
    numbered(
        "lw_lampFailure_1_",
        "_loadDecrease",
        "Lamp Failure 1-",
        " Load Decrease",
    ),
    numbered(
        "lw_lampFailure_1_",
        "_generic",
        "Lamp Failure 1-",
        " Generic",
    ),
    numbered(
        "lw_lampFailure_1_",
        "_loadIncrease",
        "Lamp Failure 1-",
        " Load Increase",
    ),
    numbered(
        "lw_lampFailure_1_",
        "_currentProtectorActive",
        "Lamp Failure 1-",
        " Current Protector Active",
    ),
    numbered(
        "lw_lampFailure_1_",
        "_thermalShutdown",
        "Lamp Failure 1-",
        " Thermal Shutdown",
    ),
    numbered(
        "lw_lampFailure_1_",
        "_thermalOverload",
        "Lamp Failure 1-",
        " Thermal Overload",
    ),
    // Typo is real (firmware + plugin).
    numbered(
        "lw_lampFailure_1_",
        "_referenceMeasuerment",
        "Lamp Failure 1-",
        " Reference Measuerment",
    ),
    numbered("lw_converter_1_", "_lost", "Converter 1-", " Lost"),
    numbered(
        "lw_converter_1_",
        "_notExisting",
        "Converter 1-",
        " Not Existing",
    ),
    numbered(
        "lw_converter_1_",
        "_doubleAddressing",
        "Converter 1-",
        " Double Addressing",
    ),
];

/// DALI brightness sensor alarms (raised ones only; `conversion` / `noConfig` are unused), MAJOR.
const BRIGHTNESS_ALARMS: [NumberedAlarm; 4] = [
    numbered("lw_brightness_1_", "_lost", "Brightness 1-", " Lost"),
    numbered(
        "lw_brightness_1_",
        "_notExisting",
        "Brightness 1-",
        " Not Existing",
    ),
    numbered(
        "lw_brightness_1_",
        "_wrongBitType",
        "Brightness 1-",
        " Wrong Bit Type",
    ),
    numbered(
        "lw_brightness_1_",
        "_wrongOpType",
        "Brightness 1-",
        " Wrong Op Type",
    ),
];

/// DALI motion sensor alarms (`noConfig` is never raised), MAJOR.
const MOTION_ALARMS: [NumberedAlarm; 2] = [
    numbered("lw_motion_1_", "_lost", "Motion 1-", " Lost"),
    numbered("lw_motion_1_", "_notExisting", "Motion 1-", " Not Existing"),
];

/// Bus / system alarms selectable as `FaultTarget::DaliBus` (firmware §2): DALI bus power,
/// light system and `lw_ntpSyncFailed` (the floorplan plugin offers it as a system alarm).
/// `lw_daliBusPowerUnknown` is documented as unused and therefore not offered.
const BUS_ALARMS: [(&str, &str, Severity); 6] = [
    (
        "lw_daliBusPowerOff",
        "DALI Bus Power Off",
        Severity::Critical,
    ),
    (
        "lw_daliBusPowerFail",
        "DALI Bus Power Fail",
        Severity::Critical,
    ),
    (
        "lw_daliBusPowerWrong",
        "DALI Bus Power Wrong",
        Severity::Critical,
    ),
    ("lw_powerFailure", "Power Failure", Severity::Critical),
    ("lw_converterExceed", "Converter Exceed", Severity::Warning),
    ("lw_ntpSyncFailed", "NTP Sync Failed", Severity::Major),
];

/// All native alarms of DALI lamp `lamp` (1–64), in firmware order.
pub fn dali_lamp_alarms(lamp: u32) -> Vec<AlarmSpec> {
    LAMP_ALARMS
        .iter()
        .map(|a| a.spec(lamp, Severity::Major))
        .collect()
}

/// Native alarms of DALI brightness sensor `n` (1–10).
pub fn brightness_alarms(n: u32) -> Vec<AlarmSpec> {
    BRIGHTNESS_ALARMS
        .iter()
        .map(|a| a.spec(n, Severity::Major))
        .collect()
}

/// Native alarms of DALI motion sensor `n` (1–10).
pub fn motion_alarms(n: u32) -> Vec<AlarmSpec> {
    MOTION_ALARMS
        .iter()
        .map(|a| a.spec(n, Severity::Major))
        .collect()
}

/// DALI bus / system alarms (`lw_daliBusPower*`, `lw_powerFailure`, `lw_converterExceed`,
/// `lw_ntpSyncFailed`).
pub fn dali_bus_alarms() -> Vec<AlarmSpec> {
    BUS_ALARMS
        .iter()
        .map(|(t, text, severity)| AlarmSpec {
            alarm_type: (*t).to_owned(),
            text: (*text).to_owned(),
            severity: *severity,
        })
        .collect()
}

/// The DALI bus power alarms during which no gear or sensor answers (firmware §2.1: libdali
/// refuses every action while bus power is not ON). `lw_powerFailure` (mains) is not one of
/// them: the firmware suspends DALI polling instead.
pub const BUS_POWER_ALARMS: [&str; 3] = [
    "lw_daliBusPowerOff",
    "lw_daliBusPowerFail",
    "lw_daliBusPowerWrong",
];

/// Whether `alarm_type` is a DALI bus power alarm (see [`BUS_POWER_ALARMS`]).
pub fn is_bus_power_alarm(alarm_type: &str) -> bool {
    BUS_POWER_ALARMS.contains(&alarm_type)
}

/// What a real Core raises while the DALI bus has no power (firmware §2.1): per grouped lamp
/// `lw_converter_1_NN_lost` and the forced `lw_controlGearFailure_1_NN` (the generic failure
/// follows the lost alarm), per brightness / motion sensor its `_lost` alarm. `sensors` are
/// `(kind, index)`; unknown kinds are skipped.
/// `lw_controlGearFailure_1_NN` of lamp `lamp`: the firmware raises it with every
/// `lw_converter_1_NN_lost` (`LSY_IsGearGenericFailureActive` counts `ERR_CONTROL_GEAR_LOST`).
pub fn control_gear_failure_spec(lamp: u32) -> AlarmSpec {
    LAMP_ALARMS[LAMP_CONTROL_GEAR_FAILURE].spec(lamp, Severity::Major)
}

pub fn bus_power_cascade(lamps: &[u32], sensors: &[(&str, u32)]) -> Vec<AlarmSpec> {
    let mut out = Vec::new();
    for &lamp in lamps {
        out.push(LAMP_ALARMS[LAMP_LOST].spec(lamp, Severity::Major));
        out.push(LAMP_ALARMS[LAMP_CONTROL_GEAR_FAILURE].spec(lamp, Severity::Major));
    }
    for &(kind, index) in sensors {
        let lost = match kind {
            SENSOR_BRIGHTNESS => &BRIGHTNESS_ALARMS[0],
            SENSOR_MOTION => &MOTION_ALARMS[0],
            _ => continue,
        };
        out.push(lost.spec(index, Severity::Major));
    }
    out
}

/// Sensor kind of a brightness / motion sensor fault (`FaultTarget` sensor variant).
pub const SENSOR_BRIGHTNESS: &str = "brightness";
pub const SENSOR_MOTION: &str = "motion";

/// Native alarms of sensor `index` (1–10) of `kind` (`"brightness"` | `"motion"`); `None` for an
/// unknown kind.
pub fn sensor_alarms(kind: &str, index: u32) -> Option<Vec<AlarmSpec>> {
    match kind {
        SENSOR_BRIGHTNESS => Some(brightness_alarms(index)),
        SENSOR_MOTION => Some(motion_alarms(index)),
        _ => None,
    }
}

/// The alarm `alarm_type` of sensor `index` of `kind`. Fails for an unknown kind, index outside
/// 1–10 or an alarm type the firmware does not raise for that sensor.
pub fn resolve_sensor_alarm(kind: &str, index: u32, alarm_type: &str) -> AppResult<AlarmSpec> {
    if !(1..=10).contains(&index) {
        return Err(AppError::NotFound(format!("{kind} sensor {index}")));
    }
    sensor_alarms(kind, index)
        .ok_or_else(|| AppError::NotFound(format!("sensor kind {kind}")))?
        .into_iter()
        .find(|s| s.alarm_type == alarm_type)
        .ok_or_else(|| AppError::NotFound(format!("alarm type {alarm_type} for {kind} {index}")))
}

/// Every raisable sensor alarm of the brightness / motion sensor items in `items`, as
/// `(kind, index, alarm)` in item order. Other item kinds are skipped.
pub fn sensor_fault_entries(items: &[&LibraryItem]) -> Vec<(&'static str, u32, AlarmSpec)> {
    let mut out = Vec::new();
    for item in items {
        let (kind, index) = match item {
            LibraryItem::DaliBrightness { index, .. } => (SENSOR_BRIGHTNESS, *index),
            LibraryItem::DaliMotion { index, .. } => (SENSOR_MOTION, *index),
            _ => continue,
        };
        if (1..=10).contains(&index) {
            for spec in sensor_alarms(kind, index).unwrap_or_default() {
                out.push((kind, index, spec));
            }
        }
    }
    out
}

/// Whether `alarm_type` is a type the real firmware (or the floorplan plugin's suggestion
/// list) knows: lamp, brightness / motion / temperature sensor, bus / system and Modbus
/// timeout / config alarms. `lw_demo_*` simulator types are **not** native.
pub fn is_native_alarm_type(alarm_type: &str) -> bool {
    static KNOWN: std::sync::OnceLock<std::collections::BTreeSet<String>> =
        std::sync::OnceLock::new();
    KNOWN
        .get_or_init(|| {
            let mut set = std::collections::BTreeSet::new();
            for lamp in 1..=64 {
                set.extend(dali_lamp_alarms(lamp).into_iter().map(|a| a.alarm_type));
            }
            for n in 1..=10 {
                set.extend(brightness_alarms(n).into_iter().map(|a| a.alarm_type));
                set.extend(motion_alarms(n).into_iter().map(|a| a.alarm_type));
                set.insert(format!("lw_brightness_1_{n:02}_conversion"));
                set.insert(format!("lw_brightness_1_{n:02}_noConfig"));
                set.insert(format!("lw_motion_1_{n:02}_noConfig"));
                for suffix in ["lost", "externalNotConnected", "notExisting"] {
                    set.insert(format!("lw_temperatureSensor_1_{n:02}_{suffix}"));
                }
            }
            set.extend(dali_bus_alarms().into_iter().map(|a| a.alarm_type));
            set.insert("lw_daliBusPowerUnknown".to_owned());
            for id in 0..=MAX_SLAVES {
                set.insert(format!("lw_modbus_{id:02}_configError"));
                if id > 0 {
                    set.insert(naming::slave_timeout_alarm_type(id as u32));
                }
            }
            set
        })
        .contains(alarm_type)
}

/// `lw_modbus_<id:02>_timeoutError`, text `Modbus <id:02>-<addr:03> Timeout Error <name>`, MAJOR.
pub fn slave_timeout_alarm(slave: &SlaveDefinition, assignment: &SlaveAssignment) -> AlarmSpec {
    AlarmSpec {
        alarm_type: naming::slave_timeout_alarm_type(assignment.id),
        text: naming::slave_timeout_text(assignment.id, assignment.address, &slave.name),
        severity: Severity::Major,
    }
}

/// Every custom alarm of a slave: `lw_demo_<id:02>_<name>` with the pack's text / severity (D2).
pub fn slave_custom_alarms(
    slave: &SlaveDefinition,
    assignment: &SlaveAssignment,
) -> Vec<AlarmSpec> {
    slave
        .alarms
        .iter()
        .map(|a| AlarmSpec {
            alarm_type: naming::slave_alarm_type(assignment.id, &a.name),
            text: a.text.clone(),
            severity: a.severity,
        })
        .collect()
}

/// Device-level custom alarm `lw_demo_<id>` of a `customAlarm` item (D2); `None` for other kinds.
pub fn device_custom_alarm(item: &LibraryItem) -> Option<AlarmSpec> {
    match item {
        LibraryItem::CustomAlarm {
            id, severity, text, ..
        } => Some(AlarmSpec {
            alarm_type: naming::device_alarm_type(id),
            text: text.clone(),
            severity: *severity,
        }),
        _ => None,
    }
}

/// What a device's faults are built from: lamp count, slaves with their assignments, and the
/// device-level `customAlarm` items (other kinds are ignored).
#[derive(Debug, Clone, Copy)]
pub struct FaultSetup<'a> {
    /// `lw_demo.daliLampCount`; lamps 1..=count get the per-lamp faults.
    pub lamp_count: u32,
    pub slaves: &'a [(&'a SlaveDefinition, &'a SlaveAssignment)],
    pub items: &'a [&'a LibraryItem],
}

impl FaultSetup<'_> {
    fn slave(&self, key: &str) -> AppResult<(&SlaveDefinition, &SlaveAssignment)> {
        self.slaves
            .iter()
            .find(|(d, _)| d.key == key)
            .map(|(d, a)| (*d, *a))
            .ok_or_else(|| AppError::NotFound(format!("slave {key}")))
    }
}

fn definition(target: FaultTarget, spec: AlarmSpec) -> FaultDefinition {
    FaultDefinition {
        target,
        alarm_type: spec.alarm_type,
        text: spec.text,
        severity: spec.severity,
    }
}

/// All selectable faults: per lamp every native lamp alarm, the bus alarms, per slave the
/// timeout and **every** custom alarm, and each device-level `customAlarm` item.
pub fn fault_catalog(setup: &FaultSetup<'_>) -> Vec<FaultDefinition> {
    let mut out = Vec::new();
    for lamp in 1..=setup.lamp_count.min(64) {
        for spec in dali_lamp_alarms(lamp) {
            let target = FaultTarget::DaliLamp {
                lamp,
                alarm_type: spec.alarm_type.clone(),
            };
            out.push(definition(target, spec));
        }
    }
    for (kind, index, spec) in sensor_fault_entries(setup.items) {
        let target = FaultTarget::Sensor {
            sensor_kind: kind.to_owned(),
            index: u8::try_from(index).unwrap_or(u8::MAX),
            alarm: spec.alarm_type.clone(),
        };
        out.push(definition(target, spec));
    }
    for spec in dali_bus_alarms() {
        let target = FaultTarget::DaliBus {
            alarm_type: spec.alarm_type.clone(),
        };
        out.push(definition(target, spec));
    }
    for (slave, assignment) in setup.slaves.iter().take(MAX_SLAVES) {
        let target = FaultTarget::SlaveTimeout {
            slave_key: slave.key.clone(),
        };
        out.push(definition(target, slave_timeout_alarm(slave, assignment)));
        for (def, spec) in slave
            .alarms
            .iter()
            .zip(slave_custom_alarms(slave, assignment))
        {
            let target = FaultTarget::SlaveAlarm {
                slave_key: slave.key.clone(),
                alarm_name: def.name.clone(),
            };
            out.push(definition(target, spec));
        }
    }
    for item in setup.items {
        if let Some(spec) = device_custom_alarm(item) {
            let target = FaultTarget::CustomAlarm {
                item_id: item.id().to_owned(),
            };
            out.push(definition(target, spec));
        }
    }
    out
}

/// The alarm a fault target raises. Fails for unknown lamps / types / slaves / items.
pub fn resolve_fault(target: &FaultTarget, setup: &FaultSetup<'_>) -> AppResult<AlarmSpec> {
    let unknown = |what: String| AppError::NotFound(what);
    match target {
        FaultTarget::DaliLamp { lamp, alarm_type } => {
            if *lamp == 0 || *lamp > setup.lamp_count.min(64) {
                return Err(unknown(format!("DALI lamp {lamp}")));
            }
            dali_lamp_alarms(*lamp)
                .into_iter()
                .find(|s| s.alarm_type == *alarm_type)
                .ok_or_else(|| unknown(format!("alarm type {alarm_type} for lamp {lamp}")))
        }
        FaultTarget::DaliBus { alarm_type } => dali_bus_alarms()
            .into_iter()
            .find(|s| s.alarm_type == *alarm_type)
            .ok_or_else(|| unknown(format!("bus alarm type {alarm_type}"))),
        FaultTarget::SlaveTimeout { slave_key } => {
            let (def, a) = setup.slave(slave_key)?;
            Ok(slave_timeout_alarm(def, a))
        }
        FaultTarget::SlaveAlarm {
            slave_key,
            alarm_name,
        } => {
            let (def, a) = setup.slave(slave_key)?;
            let alarm = def
                .alarms
                .iter()
                .find(|x| x.name == *alarm_name)
                .ok_or_else(|| unknown(format!("alarm {alarm_name} of slave {slave_key}")))?;
            Ok(AlarmSpec {
                alarm_type: naming::slave_alarm_type(a.id, &alarm.name),
                text: alarm.text.clone(),
                severity: alarm.severity,
            })
        }
        FaultTarget::CustomAlarm { item_id } => setup
            .items
            .iter()
            .find(|i| i.id() == item_id)
            .and_then(|i| device_custom_alarm(i))
            .ok_or_else(|| unknown(format!("custom alarm item {item_id}"))),
        FaultTarget::Sensor {
            sensor_kind,
            index,
            alarm,
        } => {
            let index = u32::from(*index);
            let present = setup.items.iter().any(|i| match i {
                LibraryItem::DaliBrightness { index: n, .. } => {
                    sensor_kind == SENSOR_BRIGHTNESS && *n == index
                }
                LibraryItem::DaliMotion { index: n, .. } => {
                    sensor_kind == SENSOR_MOTION && *n == index
                }
                _ => false,
            });
            if !present {
                return Err(unknown(format!("{sensor_kind} sensor {index}")));
            }
            resolve_sensor_alarm(sensor_kind, index, alarm)
        }
    }
}

/// `POST /alarm/alarms` body (status ACTIVE). Cumulocity de-duplicates ACTIVE alarms by
/// source + type.
pub fn build_alarm(device_id: &str, time: &str, alarm: &AlarmSpec) -> Value {
    json!({
        "source": super::source(device_id),
        "type": alarm.alarm_type,
        "text": alarm.text,
        "severity": alarm.severity,
        "status": "ACTIVE",
        "time": time,
    })
}

/// `PUT /alarm/alarms/{id}` body that clears an alarm (skipped in realistic mode, PRD §7.5).
pub fn build_alarm_clear() -> Value {
    json!({ "status": "CLEARED" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LocalizedText, SlaveAlarmDefinition};

    fn kuehlung() -> (SlaveDefinition, SlaveAssignment) {
        (
            SlaveDefinition {
                key: "kuehlung".into(),
                name: "Kuehlung".into(),
                request_interval: 30,
                alarms: vec![
                    SlaveAlarmDefinition {
                        name: "co2Leakage".into(),
                        text: "CO2 Leckage".into(),
                        severity: Severity::Critical,
                    },
                    SlaveAlarmDefinition {
                        name: "freezerDoor".into(),
                        text: "Tür offen".into(),
                        severity: Severity::Minor,
                    },
                ],
            },
            SlaveAssignment {
                key: "kuehlung".into(),
                id: 20,
                address: 240,
                registers: vec![],
            },
        )
    }

    fn door() -> LibraryItem {
        LibraryItem::CustomAlarm {
            id: "doorOpen".into(),
            title: LocalizedText {
                de: "Tür".into(),
                en: "Door".into(),
            },
            tags: vec![],
            severity: Severity::Major,
            text: "Hintertür offen".into(),
        }
    }

    /// Firmware table, §2 (types, texts, severities incl. the `Measuerment` typo).
    #[test]
    fn lamp_alarms_match_firmware() {
        let a = dali_lamp_alarms(5);
        assert_eq!(a.len(), 14);
        let find = |t: &str| a.iter().find(|s| s.alarm_type == t).unwrap();
        assert_eq!(
            find("lw_lampFailure_1_05_openCircuit").text,
            "Lamp Failure 1-05 Open Circuit"
        );
        assert_eq!(find("lw_converter_1_05_lost").text, "Converter 1-05 Lost");
        assert_eq!(
            find("lw_controlGearFailure_1_05").text,
            "Control Gear Failure 1-05"
        );
        assert_eq!(
            find("lw_lampFailure_1_05_referenceMeasuerment").text,
            "Lamp Failure 1-05 Reference Measuerment"
        );
        assert_eq!(
            find("lw_converter_1_05_doubleAddressing").text,
            "Converter 1-05 Double Addressing"
        );
        assert!(a.iter().all(|s| s.severity == Severity::Major));
        assert_eq!(
            dali_lamp_alarms(64)[0].alarm_type,
            "lw_controlGearFailure_1_64"
        );
    }

    #[test]
    fn bus_power_cascade_matches_firmware() {
        assert!(is_bus_power_alarm("lw_daliBusPowerFail"));
        assert!(is_bus_power_alarm("lw_daliBusPowerOff"));
        assert!(is_bus_power_alarm("lw_daliBusPowerWrong"));
        assert!(!is_bus_power_alarm("lw_powerFailure"));
        assert!(!is_bus_power_alarm("lw_converterExceed"));
        let c = bus_power_cascade(&[3], &[("brightness", 1), ("motion", 2), ("x", 1)]);
        let rows: Vec<_> = c
            .iter()
            .map(|a| (a.alarm_type.as_str(), a.text.as_str(), a.severity))
            .collect();
        assert_eq!(
            rows,
            vec![
                (
                    "lw_converter_1_03_lost",
                    "Converter 1-03 Lost",
                    Severity::Major
                ),
                (
                    "lw_controlGearFailure_1_03",
                    "Control Gear Failure 1-03",
                    Severity::Major
                ),
                (
                    "lw_brightness_1_01_lost",
                    "Brightness 1-01 Lost",
                    Severity::Major
                ),
                ("lw_motion_1_02_lost", "Motion 1-02 Lost", Severity::Major),
            ]
        );
        // The floorplan watches every lamp alarm of a lamp widget: both are among them.
        let lamp: Vec<_> = dali_lamp_alarms(3)
            .into_iter()
            .map(|a| a.alarm_type)
            .collect();
        assert!(lamp.contains(&c[0].alarm_type) && lamp.contains(&c[1].alarm_type));
    }

    #[test]
    fn bus_and_sensor_alarms_match_firmware() {
        let bus = dali_bus_alarms();
        let find = |t: &str| bus.iter().find(|s| s.alarm_type == t).unwrap().clone();
        let off = find("lw_daliBusPowerOff");
        assert_eq!(
            (off.text.as_str(), off.severity),
            ("DALI Bus Power Off", Severity::Critical)
        );
        assert_eq!(find("lw_daliBusPowerFail").text, "DALI Bus Power Fail");
        assert_eq!(find("lw_daliBusPowerWrong").text, "DALI Bus Power Wrong");
        assert_eq!(find("lw_powerFailure").severity, Severity::Critical);
        assert_eq!(find("lw_converterExceed").severity, Severity::Warning);

        let b = brightness_alarms(2);
        assert_eq!(b[0].alarm_type, "lw_brightness_1_02_lost");
        assert_eq!(b[2].text, "Brightness 1-02 Wrong Bit Type");
        let m = motion_alarms(10);
        assert_eq!(m[1].alarm_type, "lw_motion_1_10_notExisting");
        assert_eq!(m[1].text, "Motion 1-10 Not Existing");
    }

    #[test]
    fn modbus_and_custom_alarms() {
        let (s, a) = kuehlung();
        let t = slave_timeout_alarm(&s, &a);
        assert_eq!(t.alarm_type, "lw_modbus_20_timeoutError");
        assert_eq!(t.text, "Modbus 20-240 Timeout Error Kuehlung");
        assert_eq!(t.severity, Severity::Major);
        let c = slave_custom_alarms(&s, &a);
        assert_eq!(c[0].alarm_type, "lw_demo_20_co2Leakage");
        assert_eq!(c[1].severity, Severity::Minor);
        let d = device_custom_alarm(&door()).unwrap();
        assert_eq!(d.alarm_type, "lw_demo_doorOpen");
        assert_eq!(d.text, "Hintertür offen");
    }

    #[test]
    fn catalog_covers_lamps_bus_slaves_and_custom_alarms() {
        let (s, a) = kuehlung();
        let door = door();
        let slaves = [(&s, &a)];
        let items = [&door];
        let setup = FaultSetup {
            lamp_count: 2,
            slaves: &slaves,
            items: &items,
        };
        let cat = fault_catalog(&setup);
        assert_eq!(cat.len(), 2 * 14 + 6 + 1 + 2 + 1);
        for f in &cat {
            assert_eq!(
                resolve_fault(&f.target, &setup).unwrap().alarm_type,
                f.alarm_type
            );
        }
        assert!(
            resolve_fault(
                &FaultTarget::DaliLamp {
                    lamp: 3,
                    alarm_type: "lw_converter_1_03_lost".into()
                },
                &setup
            )
            .is_err()
        );
        assert!(
            resolve_fault(
                &FaultTarget::SlaveAlarm {
                    slave_key: "kuehlung".into(),
                    alarm_name: "nope".into()
                },
                &setup
            )
            .is_err()
        );
    }

    #[test]
    fn alarm_body_has_time_and_status() {
        let spec = AlarmSpec {
            alarm_type: "lw_converter_1_03_lost".into(),
            text: "Converter 1-03 Lost".into(),
            severity: Severity::Major,
        };
        assert_eq!(
            build_alarm("42", "2026-10-12T12:00:00.000Z", &spec),
            json!({
                "source": {"id": "42"}, "type": "lw_converter_1_03_lost",
                "text": "Converter 1-03 Lost", "severity": "MAJOR", "status": "ACTIVE",
                "time": "2026-10-12T12:00:00.000Z"
            })
        );
        assert_eq!(build_alarm_clear(), json!({"status": "CLEARED"}));
    }

    fn table(specs: Vec<AlarmSpec>) -> Vec<(String, String, Severity)> {
        specs
            .into_iter()
            .map(|s| (s.alarm_type, s.text, s.severity))
            .collect()
    }

    fn row(t: &str, text: &str, severity: Severity) -> (String, String, Severity) {
        (t.to_owned(), text.to_owned(), severity)
    }

    /// The whole lamp table, copied from firmware-core-v2.md §2 (lamp 5), in firmware order.
    #[test]
    fn lamp_table_is_pinned_completely() {
        use Severity::Major;
        let expected = vec![
            row(
                "lw_controlGearFailure_1_05",
                "Control Gear Failure 1-05",
                Major,
            ),
            row("lw_converterFailure_1_05", "Converter Failure 1-05", Major),
            row(
                "lw_lampFailure_1_05_shortCircuit",
                "Lamp Failure 1-05 Short Circuit",
                Major,
            ),
            row(
                "lw_lampFailure_1_05_openCircuit",
                "Lamp Failure 1-05 Open Circuit",
                Major,
            ),
            row(
                "lw_lampFailure_1_05_loadDecrease",
                "Lamp Failure 1-05 Load Decrease",
                Major,
            ),
            row(
                "lw_lampFailure_1_05_generic",
                "Lamp Failure 1-05 Generic",
                Major,
            ),
            row(
                "lw_lampFailure_1_05_loadIncrease",
                "Lamp Failure 1-05 Load Increase",
                Major,
            ),
            row(
                "lw_lampFailure_1_05_currentProtectorActive",
                "Lamp Failure 1-05 Current Protector Active",
                Major,
            ),
            row(
                "lw_lampFailure_1_05_thermalShutdown",
                "Lamp Failure 1-05 Thermal Shutdown",
                Major,
            ),
            row(
                "lw_lampFailure_1_05_thermalOverload",
                "Lamp Failure 1-05 Thermal Overload",
                Major,
            ),
            row(
                "lw_lampFailure_1_05_referenceMeasuerment",
                "Lamp Failure 1-05 Reference Measuerment",
                Major,
            ),
            row("lw_converter_1_05_lost", "Converter 1-05 Lost", Major),
            row(
                "lw_converter_1_05_notExisting",
                "Converter 1-05 Not Existing",
                Major,
            ),
            row(
                "lw_converter_1_05_doubleAddressing",
                "Converter 1-05 Double Addressing",
                Major,
            ),
        ];
        assert_eq!(table(dali_lamp_alarms(5)), expected);
    }

    #[test]
    fn sensor_and_bus_tables_are_pinned_completely() {
        use Severity::{Critical, Major, Warning};
        assert_eq!(
            table(brightness_alarms(2)),
            vec![
                row("lw_brightness_1_02_lost", "Brightness 1-02 Lost", Major),
                row(
                    "lw_brightness_1_02_notExisting",
                    "Brightness 1-02 Not Existing",
                    Major
                ),
                row(
                    "lw_brightness_1_02_wrongBitType",
                    "Brightness 1-02 Wrong Bit Type",
                    Major
                ),
                row(
                    "lw_brightness_1_02_wrongOpType",
                    "Brightness 1-02 Wrong Op Type",
                    Major
                ),
            ]
        );
        assert_eq!(
            table(motion_alarms(10)),
            vec![
                row("lw_motion_1_10_lost", "Motion 1-10 Lost", Major),
                row(
                    "lw_motion_1_10_notExisting",
                    "Motion 1-10 Not Existing",
                    Major
                ),
            ]
        );
        assert_eq!(
            table(dali_bus_alarms()),
            vec![
                row("lw_daliBusPowerOff", "DALI Bus Power Off", Critical),
                row("lw_daliBusPowerFail", "DALI Bus Power Fail", Critical),
                row("lw_daliBusPowerWrong", "DALI Bus Power Wrong", Critical),
                row("lw_powerFailure", "Power Failure", Critical),
                row("lw_converterExceed", "Converter Exceed", Warning),
                row("lw_ntpSyncFailed", "NTP Sync Failed", Major),
            ]
        );
    }

    #[test]
    fn sensor_alarms_resolve_and_list_per_item() {
        let a = resolve_sensor_alarm("brightness", 3, "lw_brightness_1_03_lost").unwrap();
        assert_eq!(a.text, "Brightness 1-03 Lost");
        let a = resolve_sensor_alarm("motion", 1, "lw_motion_1_01_notExisting").unwrap();
        assert_eq!(a.severity, Severity::Major);
        // Wrong sensor number, kind or an alarm of another sensor.
        assert!(resolve_sensor_alarm("motion", 1, "lw_motion_1_02_lost").is_err());
        assert!(resolve_sensor_alarm("motion", 11, "lw_motion_1_11_lost").is_err());
        assert!(resolve_sensor_alarm("humidity", 1, "x").is_err());
        // Not raised by the firmware.
        assert!(resolve_sensor_alarm("motion", 1, "lw_motion_1_01_noConfig").is_err());

        let lux = LibraryItem::DaliBrightness {
            id: "lux".into(),
            index: 2,
            title: LocalizedText {
                de: "x".into(),
                en: "x".into(),
            },
            tags: vec![],
            unit: None,
            control: None,
            generator: None,
            effects: vec![],
        };
        let motion = LibraryItem::DaliMotion {
            id: "mo".into(),
            index: 1,
            title: LocalizedText {
                de: "x".into(),
                en: "x".into(),
            },
            tags: vec![],
            unit: None,
            control: None,
            generator: None,
            effects: vec![],
        };
        let door = door();
        let entries = sensor_fault_entries(&[&lux, &motion, &door]);
        assert_eq!(entries.len(), 4 + 2);
        assert_eq!(entries[0].0, "brightness");
        assert_eq!(entries[4].1, 1);
    }

    #[test]
    fn native_alarm_type_catalog() {
        for t in [
            "lw_daliBusPowerFail",
            "lw_ntpSyncFailed",
            "lw_lampFailure_1_64_openCircuit",
            "lw_controlGearFailure_1_01",
            "lw_brightness_1_10_wrongBitType",
            "lw_motion_1_01_noConfig",
            "lw_temperatureSensor_1_01_lost",
            "lw_modbus_20_timeoutError",
            "lw_modbus_00_configError",
        ] {
            assert!(is_native_alarm_type(t), "{t}");
        }
        for t in [
            "lw_daliBusPowerFailure",
            "lw_lampFailure_1_65_openCircuit",
            "lw_lampFailure_1_5_openCircuit",
            "lw_modbus_21_timeoutError",
            "lw_modbus_00_timeoutError",
            "lw_demo_doorOpen",
            "doorOpen",
        ] {
            assert!(!is_native_alarm_type(t), "{t}");
        }
    }
}
