//! Native naming rules (PRD §7.2–§7.4, DECISIONS D1/D2). Every name here is checked
//! against `docs/research/firmware-core-v2.md` (section numbers in the doc comments).

use crate::error::{AppError, AppResult};
use crate::model::{
    DigitalInputEntry, DigitalInputType, LibraryItem, MeasurementNaming, ModbusRegisterType,
    RegisterAssignment, SeriesNaming, SlaveAssignment, SystemKind,
};

/// Max Modbus slaves per device.
pub const MAX_SLAVES: usize = 20;
/// Max registers per slave.
pub const MAX_REGISTERS_PER_SLAVE: usize = 40;
/// First (highest) slave id.
pub const FIRST_SLAVE_ID: u32 = 20;
/// First (highest) slave address.
pub const FIRST_SLAVE_ADDRESS: u32 = 240;
/// Max length of slave names, register names (item ids). The firmware buffer is 20 **bytes**
/// including the NUL, so this counts UTF-8 bytes (`"Kühlraum Tür links"` is 18 chars but 20
/// bytes and would be truncated); validation uses `str::len`.
pub const MAX_NAME_LEN: usize = 19;
/// Max length of Modbus units, in UTF-8 bytes (8-byte buffer including the NUL).
pub const MAX_UNIT_LEN: usize = 7;
/// Highest raw register address.
pub const MAX_REGISTER_ADDRESS: u32 = 65535;

/// DALI measurement type (§1.2).
pub const DALI_MEASUREMENT_TYPE: &str = "daliControlGear";
/// DALI lamp series with their firmware units (`None` = empty unit, omitted in JSON), §1.2.
pub const DALI_SERIES: [(&str, Option<&str>); 4] = [
    ("Converter Status", None),
    ("Lamp Status", None),
    ("Converter Runtime", Some("hr")),
    ("Brightness", Some("%")),
];
/// The DALI series the item value drives.
pub const DALI_VALUE_SERIES: &str = "Brightness";

/// DALI brightness sensor (§1.3): type / fragment / series prefix / unit (`Lux`, not `lx`).
pub const DALI_BRIGHTNESS_TYPE: &str = "daliBrightness";
pub const DALI_BRIGHTNESS_FRAGMENT: &str = "LW Dali Brightness";
pub const DALI_BRIGHTNESS_SERIES_PREFIX: &str = "Lux";
pub const DALI_BRIGHTNESS_UNIT: &str = "Lux";
/// DALI motion sensor (§1.3): type / fragment / series prefix; the firmware sends no unit.
pub const DALI_MOTION_TYPE: &str = "daliMotion";
pub const DALI_MOTION_FRAGMENT: &str = "LW Motion";
pub const DALI_MOTION_SERIES_PREFIX: &str = "State";

/// Digital input measurement type / fragment (§1.4, D1).
pub const DI_MEASUREMENT_TYPE: &str = "digitalInput";
pub const DI_FRAGMENT: &str = "LW Digital Input";

/// System: signal strength (`210`) and device temperature (`211`), §1.1 and the REST section.
pub const RSSI_TYPE: &str = "c8y_SignalStrength";
pub const RSSI_SERIES: &str = "rssi";
pub const RSSI_UNIT: &str = "dBm";
pub const TEMPERATURE_TYPE: &str = "c8y_TemperatureMeasurement";
pub const TEMPERATURE_SERIES: &str = "T";
pub const TEMPERATURE_UNIT: &str = "C";

/// Modbus measurement type (§1.5).
pub const MODBUS_MEASUREMENT_TYPE: &str = "modbus";

/// `LW Dali Control Gear 1-NN` (bus 1, lamp 1–64).
pub fn dali_fragment(lamp: u32) -> String {
    format!("LW Dali Control Gear 1-{lamp:02}")
}

/// `Lux<N>` (N = 1–10, not zero-padded).
pub fn dali_brightness_series(n: u32) -> String {
    format!("{DALI_BRIGHTNESS_SERIES_PREFIX}{n}")
}

/// `State<N>` (N = 1–10, not zero-padded).
pub fn dali_motion_series(n: u32) -> String {
    format!("{DALI_MOTION_SERIES_PREFIX}{n}")
}

/// `LW_Modbus_Slave_<id:02>_<address:03>`.
pub fn modbus_fragment(id: u32, address: u32) -> String {
    format!("LW_Modbus_Slave_{id:02}_{address:03}")
}

/// Digital input value series `Value<NN>` (NN = 01–10). The title is never part of it (D1).
pub fn di_value_series(index: u32) -> String {
    format!("Value{index:02}")
}

/// Digital input counter series `ImpulseCounter<NN>` (NN = 01–10).
pub fn di_counter_series(index: u32) -> String {
    format!("ImpulseCounter{index:02}")
}

/// Naming of an item's measurement; `None` for items without one (`customAlarm`) and for
/// a `modbus` item whose `slave` assignment is missing. Digital-input items use the library
/// item's kind and unit; use [`item_naming_with_di`] once the device's DI entry is known.
pub fn item_naming(
    item: &LibraryItem,
    slave: Option<&SlaveAssignment>,
) -> Option<MeasurementNaming> {
    item_naming_with_di(item, slave, None)
}

/// [`item_naming`] where a digital-input item follows the device's `lw_configDigitalInputs`
/// entry `di` (the firmware's source of truth, D6): the series is `ImpulseCounter<NN>` when the
/// entry's `type` is `impulseCounter`, else `Value<NN>`, and the counter unit is the entry's
/// `unit`. Without an entry the item's own kind and unit apply. `di` is ignored for other kinds.
pub fn item_naming_with_di(
    item: &LibraryItem,
    slave: Option<&SlaveAssignment>,
    di: Option<&DigitalInputEntry>,
) -> Option<MeasurementNaming> {
    let series = |name: String, unit: Option<&str>| SeriesNaming {
        name,
        unit: unit.map(str::to_owned),
    };
    match item {
        LibraryItem::Dali { index, .. } => Some(MeasurementNaming {
            measurement_type: DALI_MEASUREMENT_TYPE.to_owned(),
            fragment: dali_fragment(*index),
            series: DALI_SERIES
                .iter()
                .map(|(n, u)| series((*n).to_owned(), *u))
                .collect(),
        }),
        LibraryItem::DaliBrightness { index, .. } => Some(MeasurementNaming {
            measurement_type: DALI_BRIGHTNESS_TYPE.to_owned(),
            fragment: DALI_BRIGHTNESS_FRAGMENT.to_owned(),
            series: vec![series(
                dali_brightness_series(*index),
                Some(DALI_BRIGHTNESS_UNIT),
            )],
        }),
        LibraryItem::DaliMotion { index, .. } => Some(MeasurementNaming {
            measurement_type: DALI_MOTION_TYPE.to_owned(),
            fragment: DALI_MOTION_FRAGMENT.to_owned(),
            series: vec![series(dali_motion_series(*index), None)],
        }),
        LibraryItem::DigitalInput { index, .. } | LibraryItem::ImpulseCounter { index, .. } => {
            let item_unit = item.unit();
            let (counter, unit) = match di.filter(|e| e.index == *index) {
                Some(e) => (
                    e.settings.input_type == DigitalInputType::ImpulseCounter,
                    e.settings.unit.as_deref(),
                ),
                None => (
                    matches!(item, LibraryItem::ImpulseCounter { .. }),
                    item_unit,
                ),
            };
            let name = if counter {
                di_counter_series(*index)
            } else {
                di_value_series(*index)
            };
            Some(MeasurementNaming {
                measurement_type: DI_MEASUREMENT_TYPE.to_owned(),
                fragment: DI_FRAGMENT.to_owned(),
                series: vec![series(name, counter.then_some(unit).flatten())],
            })
        }
        LibraryItem::System { system, .. } => Some(match system {
            SystemKind::Rssi => MeasurementNaming {
                measurement_type: RSSI_TYPE.to_owned(),
                fragment: RSSI_TYPE.to_owned(),
                series: vec![series(RSSI_SERIES.to_owned(), Some(RSSI_UNIT))],
            },
            SystemKind::Temperature => MeasurementNaming {
                measurement_type: TEMPERATURE_TYPE.to_owned(),
                fragment: TEMPERATURE_TYPE.to_owned(),
                series: vec![series(
                    TEMPERATURE_SERIES.to_owned(),
                    Some(TEMPERATURE_UNIT),
                )],
            },
        }),
        LibraryItem::Modbus {
            id,
            slave: slave_key,
            unit,
            ..
        } => {
            let slave = slave.filter(|s| s.key == *slave_key)?;
            Some(MeasurementNaming {
                measurement_type: MODBUS_MEASUREMENT_TYPE.to_owned(),
                fragment: modbus_fragment(slave.id, slave.address),
                series: vec![series(id.clone(), unit.as_deref())],
            })
        }
        LibraryItem::CustomAlarm { .. } => None,
    }
}

/// The first value that occurs twice.
fn first_duplicate<'a>(values: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut seen = std::collections::BTreeSet::new();
    values.into_iter().find(|v| !seen.insert(*v))
}

/// Assign slave ids / addresses for `keys` (order = priority for new slaves).
///
/// Slaves already in `existing` keep their id and address (upgrades never renumber, and their
/// `registers` are kept); new keys get the next free id (counting down from 20) and the next
/// free address (counting down from 240). Existing entries not in `keys` are **not** returned,
/// but their id / address stay taken (a deselected slave that comes back gets its numbers again,
/// and no other slave can reuse them; the caller keeps such entries as retired, D6). The result
/// follows the order of `keys`.
pub fn assign_slaves(
    existing: &[SlaveAssignment],
    keys: &[&str],
) -> AppResult<Vec<SlaveAssignment>> {
    if keys.len() > MAX_SLAVES {
        return Err(AppError::InvalidInput(format!(
            "{} slaves, max {MAX_SLAVES}",
            keys.len()
        )));
    }
    if let Some(dup) = first_duplicate(keys.iter().copied()) {
        return Err(AppError::InvalidInput(format!("duplicate slave key {dup}")));
    }
    let kept: Vec<&SlaveAssignment> = existing
        .iter()
        .filter(|e| keys.contains(&e.key.as_str()))
        .collect();
    let mut used_ids: Vec<u32> = existing.iter().map(|e| e.id).collect();
    let mut used_addresses: Vec<u32> = existing.iter().map(|e| e.address).collect();
    let mut out = Vec::with_capacity(keys.len());
    for key in keys {
        if let Some(e) = kept.iter().find(|e| e.key == *key) {
            out.push((*e).clone());
            continue;
        }
        let id = (1..=FIRST_SLAVE_ID)
            .rev()
            .find(|id| !used_ids.contains(id))
            .ok_or_else(|| AppError::InvalidInput("no free slave id".into()))?;
        let address = (1..=FIRST_SLAVE_ADDRESS)
            .rev()
            .find(|a| !used_addresses.contains(a))
            .ok_or_else(|| AppError::InvalidInput("no free slave address".into()))?;
        used_ids.push(id);
        used_addresses.push(address);
        out.push(SlaveAssignment {
            key: (*key).to_owned(),
            id,
            address,
            registers: Vec::new(),
        });
    }
    Ok(out)
}

/// First register address handed out for a register type (4xxxx / 3xxxx reference style, as in
/// the floorplan plugin's examples; the firmware takes any raw address 0–65535).
///
/// Simulator convention (DECISIONS D6): these are **raw, 0-based PDU addresses** and consecutive
/// 32-bit registers are not spaced by their word count, so they overlap on a real bus. The
/// simulator has no bus, and the numbers are consistent across `registers`, `writeRegisters`,
/// the plugin `ref` and `lw_modbusWrite`; a real Core given this config would read wrong words.
pub fn register_base_address(register_type: ModbusRegisterType) -> u32 {
    match register_type {
        ModbusRegisterType::Coil => 1,
        ModbusRegisterType::Input => 30001,
        ModbusRegisterType::Holding => 40001,
    }
}

/// Deterministic register addresses for one slave. `items` are `(item id, register type)` in
/// library order.
///
/// An existing assignment with the same item id and type is kept; otherwise the item gets the
/// lowest unused address of its type, starting at [`register_base_address`]. Entries of items no
/// longer listed are not returned, but their addresses stay taken (the caller keeps them as
/// retired, D6). The result follows the order of `items`.
pub fn assign_registers(
    existing: &[RegisterAssignment],
    items: &[(&str, ModbusRegisterType)],
) -> AppResult<Vec<RegisterAssignment>> {
    if items.len() > MAX_REGISTERS_PER_SLAVE {
        return Err(AppError::InvalidInput(format!(
            "{} registers, max {MAX_REGISTERS_PER_SLAVE}",
            items.len()
        )));
    }
    if let Some(dup) = first_duplicate(items.iter().map(|(id, _)| *id)) {
        return Err(AppError::InvalidInput(format!("duplicate item id {dup}")));
    }
    let kept = |item_id: &str, t: ModbusRegisterType| {
        existing
            .iter()
            .find(|r| r.item_id == item_id && r.register_type == t)
    };
    let mut taken: Vec<(ModbusRegisterType, u32)> = existing
        .iter()
        .map(|r| (r.register_type, r.address))
        .collect();
    let mut out = Vec::with_capacity(items.len());
    for (id, t) in items {
        if let Some(r) = kept(id, *t) {
            out.push(r.clone());
            continue;
        }
        let address = (register_base_address(*t)..=MAX_REGISTER_ADDRESS)
            .find(|a| !taken.contains(&(*t, *a)))
            .ok_or_else(|| AppError::InvalidInput("no free register address".into()))?;
        taken.push((*t, address));
        out.push(RegisterAssignment {
            item_id: (*id).to_owned(),
            register_type: *t,
            address,
        });
    }
    Ok(out)
}

/// `[A-Za-z0-9_-]+` — allowed in alarm names (they end up in the alarm type).
pub fn is_valid_alarm_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A device-level alarm name must not start with two digits plus `_`, otherwise
/// `lw_demo_<name>` could collide with a slave alarm `lw_demo_<id:02>_<name>` (D2).
pub fn is_valid_device_alarm_name(name: &str) -> bool {
    let b = name.as_bytes();
    let collides = b.len() >= 3 && b[0].is_ascii_digit() && b[1].is_ascii_digit() && b[2] == b'_';
    is_valid_alarm_name(name) && !collides
}

/// Device-level custom alarm type `lw_demo_<name>` (D2).
pub fn device_alarm_type(name: &str) -> String {
    format!("lw_demo_{name}")
}

/// Per-slave custom alarm type `lw_demo_<id:02>_<name>`.
pub fn slave_alarm_type(slave_id: u32, alarm_name: &str) -> String {
    format!("lw_demo_{slave_id:02}_{alarm_name}")
}

/// Slave timeout alarm type `lw_modbus_<id:02>_timeoutError`.
pub fn slave_timeout_alarm_type(slave_id: u32) -> String {
    format!("lw_modbus_{slave_id:02}_timeoutError")
}

/// Slave timeout alarm text `Modbus %02u-%03u Timeout Error <name>` (§2).
pub fn slave_timeout_text(slave_id: u32, address: u32, slave_name: &str) -> String {
    format!("Modbus {slave_id:02}-{address:03} Timeout Error {slave_name}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LocalizedText, Severity};

    fn title() -> LocalizedText {
        LocalizedText {
            de: "x".into(),
            en: "x".into(),
        }
    }

    /// A native item of the kind built by `make` (`LibraryItem::Dali`, …).
    fn native(
        make: fn(String, u32, LocalizedText, Vec<String>, Option<String>) -> LibraryItem,
        index: u32,
        unit: Option<&str>,
    ) -> LibraryItem {
        make(
            "item".into(),
            index,
            title(),
            vec![],
            unit.map(str::to_owned),
        )
    }

    fn dali(
        id: String,
        index: u32,
        title: LocalizedText,
        tags: Vec<String>,
        unit: Option<String>,
    ) -> LibraryItem {
        LibraryItem::Dali {
            id,
            index,
            title,
            tags,
            unit,
            control: None,
            generator: None,
            effects: vec![],
        }
    }
    fn brightness(
        id: String,
        index: u32,
        title: LocalizedText,
        tags: Vec<String>,
        unit: Option<String>,
    ) -> LibraryItem {
        LibraryItem::DaliBrightness {
            id,
            index,
            title,
            tags,
            unit,
            control: None,
            generator: None,
            effects: vec![],
        }
    }
    fn motion(
        id: String,
        index: u32,
        title: LocalizedText,
        tags: Vec<String>,
        unit: Option<String>,
    ) -> LibraryItem {
        LibraryItem::DaliMotion {
            id,
            index,
            title,
            tags,
            unit,
            control: None,
            generator: None,
            effects: vec![],
        }
    }
    fn input(
        id: String,
        index: u32,
        title: LocalizedText,
        tags: Vec<String>,
        unit: Option<String>,
    ) -> LibraryItem {
        LibraryItem::DigitalInput {
            id,
            index,
            title,
            tags,
            unit,
            control: None,
            generator: None,
            effects: vec![],
        }
    }
    fn counter(
        id: String,
        index: u32,
        title: LocalizedText,
        tags: Vec<String>,
        unit: Option<String>,
    ) -> LibraryItem {
        LibraryItem::ImpulseCounter {
            id,
            index,
            title,
            tags,
            unit,
            control: None,
            generator: None,
            effects: vec![],
        }
    }

    fn slave(key: &str, id: u32, address: u32) -> SlaveAssignment {
        SlaveAssignment {
            key: key.into(),
            id,
            address,
            registers: vec![],
        }
    }

    #[test]
    fn fragments() {
        assert_eq!(dali_fragment(3), "LW Dali Control Gear 1-03");
        assert_eq!(dali_fragment(64), "LW Dali Control Gear 1-64");
        assert_eq!(modbus_fragment(20, 240), "LW_Modbus_Slave_20_240");
        assert_eq!(modbus_fragment(1, 5), "LW_Modbus_Slave_01_005");
    }

    #[test]
    fn di_series_have_no_title() {
        assert_eq!(di_value_series(3), "Value03");
        assert_eq!(di_value_series(10), "Value10");
        assert_eq!(di_counter_series(4), "ImpulseCounter04");
    }

    #[test]
    fn native_names() {
        let n = item_naming(&native(dali, 3, None), None).unwrap();
        assert_eq!(n.measurement_type, "daliControlGear");
        assert_eq!(n.fragment, "LW Dali Control Gear 1-03");
        let series: Vec<(&str, Option<&str>)> = n
            .series
            .iter()
            .map(|s| (s.name.as_str(), s.unit.as_deref()))
            .collect();
        assert_eq!(
            series,
            vec![
                ("Converter Status", None),
                ("Lamp Status", None),
                ("Converter Runtime", Some("hr")),
                ("Brightness", Some("%")),
            ]
        );

        let n = item_naming(&native(brightness, 2, None), None).unwrap();
        assert_eq!(
            (n.measurement_type.as_str(), n.fragment.as_str()),
            ("daliBrightness", "LW Dali Brightness")
        );
        assert_eq!(n.series[0].name, "Lux2");
        assert_eq!(n.series[0].unit.as_deref(), Some("Lux"));

        let n = item_naming(&native(motion, 10, None), None).unwrap();
        assert_eq!(n.fragment, "LW Motion");
        assert_eq!(n.series[0].name, "State10");
        assert_eq!(n.series[0].unit, None);

        let n = item_naming(&native(input, 3, None), None).unwrap();
        assert_eq!(n.fragment, "LW Digital Input");
        assert_eq!(n.series[0].name, "Value03");
        let n = item_naming(&native(counter, 4, Some("m3")), None).unwrap();
        assert_eq!(n.series[0].name, "ImpulseCounter04");
        assert_eq!(n.series[0].unit.as_deref(), Some("m3"));
    }

    #[test]
    fn system_names() {
        let sys = |system| LibraryItem::System {
            id: "s".into(),
            system,
            title: title(),
            tags: vec![],
            control: None,
            generator: None,
        };
        let n = item_naming(&sys(SystemKind::Rssi), None).unwrap();
        assert_eq!(n.fragment, "c8y_SignalStrength");
        assert_eq!(n.series[0].name, "rssi");
        assert_eq!(n.series[0].unit.as_deref(), Some("dBm"));
        let n = item_naming(&sys(SystemKind::Temperature), None).unwrap();
        assert_eq!(n.measurement_type, "c8y_TemperatureMeasurement");
        assert_eq!(n.series[0].name, "T");
        assert_eq!(n.series[0].unit.as_deref(), Some("C"));
    }

    #[test]
    fn modbus_and_alarm_items() {
        let item = LibraryItem::Modbus {
            id: "freezer_temp".into(),
            slave: "kuehlung".into(),
            title: title(),
            tags: vec![],
            register_type: None,
            unit: Some("°C".into()),
            datatype: None,
            writable: false,
            control: None,
            generator: None,
            effects: vec![],
        };
        assert!(item_naming(&item, None).is_none());
        assert!(item_naming(&item, Some(&slave("other", 19, 239))).is_none());
        let n = item_naming(&item, Some(&slave("kuehlung", 20, 240))).unwrap();
        assert_eq!(n.fragment, "LW_Modbus_Slave_20_240");
        assert_eq!(n.series[0].name, "freezer_temp");

        let alarm = LibraryItem::CustomAlarm {
            id: "co2".into(),
            title: title(),
            tags: vec![],
            severity: Severity::Major,
            text: "CO2".into(),
        };
        assert!(item_naming(&alarm, None).is_none());
    }

    #[test]
    fn slave_assignment() {
        let a = assign_slaves(&[], &["kuehlung", "klima"]).unwrap();
        assert_eq!((a[0].id, a[0].address), (20, 240));
        assert_eq!((a[1].id, a[1].address), (19, 239));
        let too_many: Vec<&str> = (0..21).map(|_| "x").collect();
        assert!(assign_slaves(&[], &too_many).is_err());
    }

    #[test]
    fn slave_assignment_keeps_existing() {
        // "klima" was assigned 19/239 before; a new slave "ev" is listed first and must not
        // take its place; "kuehlung" was dropped, so 20/240 is free again.
        let existing = [slave("klima", 19, 239)];
        let a = assign_slaves(&existing, &["ev", "klima"]).unwrap();
        assert_eq!((a[0].key.as_str(), a[0].id, a[0].address), ("ev", 20, 240));
        assert_eq!(
            (a[1].key.as_str(), a[1].id, a[1].address),
            ("klima", 19, 239)
        );
    }

    #[test]
    fn deselected_slave_numbers_are_not_reused() {
        // "kuehlung" (20/240) is deselected: a new slave must not take 20/240, and when
        // "kuehlung" comes back it gets its numbers again.
        let existing = [slave("kuehlung", 20, 240), slave("klima", 19, 239)];
        let a = assign_slaves(&existing, &["klima", "ev"]).unwrap();
        assert_eq!((a[1].key.as_str(), a[1].id, a[1].address), ("ev", 18, 238));
        let b = assign_slaves(&existing, &["kuehlung", "klima"]).unwrap();
        assert_eq!((b[0].id, b[0].address), (20, 240));
    }

    #[test]
    fn register_assignment_is_stable() {
        use ModbusRegisterType::*;
        let a = assign_registers(
            &[],
            &[("t", Input), ("sp", Holding), ("on", Coil), ("t2", Input)],
        )
        .unwrap();
        let addr = |id: &str| a.iter().find(|r| r.item_id == id).unwrap().address;
        assert_eq!(addr("t"), 30001);
        assert_eq!(addr("t2"), 30002);
        assert_eq!(addr("sp"), 40001);
        assert_eq!(addr("on"), 1);
        // A new item in front of the old ones must not renumber them.
        let b = assign_registers(
            &a,
            &[
                ("new", Input),
                ("t", Input),
                ("sp", Holding),
                ("on", Coil),
                ("t2", Input),
            ],
        )
        .unwrap();
        let addr_b = |id: &str| b.iter().find(|r| r.item_id == id).unwrap().address;
        assert_eq!(addr_b("t"), 30001);
        assert_eq!(addr_b("t2"), 30002);
        assert_eq!(addr_b("new"), 30003);
        // A changed type gets a fresh address; the address of the unlisted `sp` stays taken.
        let c = assign_registers(&a, &[("t", Holding)]).unwrap();
        assert_eq!((c[0].register_type, c[0].address), (Holding, 40002));
        // A deselected item that comes back gets its address again, nobody else took it.
        let d = assign_registers(&a, &[("new", Holding)]).unwrap();
        assert_eq!(d[0].address, 40002);
        let e = assign_registers(&a, &[("new", Holding), ("sp", Holding)]).unwrap();
        assert_eq!(e[1].address, 40001);
    }

    #[test]
    fn alarm_names() {
        assert_eq!(slave_alarm_type(19, "co2Leakage"), "lw_demo_19_co2Leakage");
        assert_eq!(device_alarm_type("doorOpen"), "lw_demo_doorOpen");
        assert_eq!(slave_timeout_alarm_type(20), "lw_modbus_20_timeoutError");
        assert!(is_valid_device_alarm_name("doorOpen"));
        assert!(is_valid_device_alarm_name("1door"));
        assert!(is_valid_device_alarm_name("19x"));
        assert!(!is_valid_device_alarm_name("19_door"));
        assert!(!is_valid_device_alarm_name("00_x"));
        assert!(!is_valid_device_alarm_name(""));
        assert!(!is_valid_device_alarm_name("a b"));
    }

    #[test]
    fn timeout_text_is_zero_padded() {
        assert_eq!(
            slave_timeout_text(20, 240, "Kuehlung"),
            "Modbus 20-240 Timeout Error Kuehlung"
        );
        assert_eq!(
            slave_timeout_text(1, 5, "Klima"),
            "Modbus 01-005 Timeout Error Klima"
        );
    }

    #[test]
    fn duplicate_keys_and_item_ids_are_rejected() {
        use ModbusRegisterType::*;
        assert!(assign_slaves(&[], &["a", "a"]).is_err());
        assert!(assign_registers(&[], &[("t", Input), ("t", Holding)]).is_err());
        assert!(assign_registers(&[], &[("t", Input), ("u", Input)]).is_ok());
    }

    /// The device's DI entry decides series and unit (D6), not the library item.
    #[test]
    fn di_naming_follows_the_device_entry() {
        use crate::model::DigitalInputSettings;
        let settings = |t, unit: Option<&str>| DigitalInputSettings {
            input_type: t,
            is_active: true,
            init_count: None,
            increment: None,
            unit: unit.map(str::to_owned),
            high_level_action: None,
            low_level_action: None,
            groups: vec![],
        };
        let entry = |t, unit| DigitalInputEntry {
            index: 4,
            title: "x".into(),
            settings: settings(t, unit),
        };
        let item = native(counter, 4, Some("m3"));
        // Unit edited on the device: naming follows it.
        let n = item_naming_with_di(
            &item,
            None,
            Some(&entry(DigitalInputType::ImpulseCounter, Some("kWh"))),
        )
        .unwrap();
        assert_eq!(n.series[0].name, "ImpulseCounter04");
        assert_eq!(n.series[0].unit.as_deref(), Some("kWh"));
        // Mode edited on the device: a counter item that is now a plain input sends Value04.
        let n = item_naming_with_di(
            &item,
            None,
            Some(&entry(DigitalInputType::Measurement, None)),
        )
        .unwrap();
        assert_eq!(n.series[0].name, "Value04");
        assert_eq!(n.series[0].unit, None);
        // An entry of another input is ignored.
        let mut other = entry(DigitalInputType::Measurement, None);
        other.index = 5;
        let n = item_naming_with_di(&item, None, Some(&other)).unwrap();
        assert_eq!(n.series[0].name, "ImpulseCounter04");
        assert_eq!(n.series[0].unit.as_deref(), Some("m3"));
    }
}
