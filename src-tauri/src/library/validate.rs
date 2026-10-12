//! Semantic library validation (beyond serde / JSON schema). Every finding names the file and a
//! JSON path inside it, e.g. `items/cooling.json: /items/3/unit: "Grad C" is longer than 7 chars`.
//!
//! Rules (PRD §7, §11, DECISIONS D1–D6, firmware limits in firmware-core-v2.md):
//! - items: ids `[A-Za-z0-9_-]` ≤ 19 and globally unique; tags `[client]`, `category:*`,
//!   `zone:*`; native index ranges (DALI 1–64, sensors / DI 1–10) and one item per native
//!   target; `unit` only on `impulseCounter` (≤ 19 bytes) and `modbus` (≤ 7 bytes); Modbus slave
//!   exists, only `coil` / `holding` writable, coils use a switch control, writable holding
//!   registers are one 16-bit word (no `float32` / `*32`, integer control 0-65535);
//!   `customAlarm` id must not start with two digits + `_`; controls / generators / effects
//!   well-formed; generators fit the control (`counter` only on counters, no continuous
//!   generator on switch / select / DI value / motion) and stay inside a slider range; effects
//!   cannot write reserved inventory fragments or fake native events.
//! - slaves: unique keys, name 1–19 bytes, requestInterval 1–99999, alarm names valid and unique.
//! - lengths that mirror firmware buffers count UTF-8 **bytes** (`str::len`); the JSON schema's
//!   `maxLength` counts code points, so this is the stricter gate.
//! - demos: unique id, `daliLampCount` 1–64, selected DALI lamps ≤ `daliLampCount` and grouped,
//!   ≤ 16 groups (ids 1–16, unique, converters ≤ `daliLampCount`, `slaveOf` resolves),
//!   digital inputs (index 1–10 unique, item resolves to a selected DI item with that index and
//!   a matching type, type-specific fields, actions `on|off|auto`, counter unit / `initCount`
//!   equal to the item's; every selected DI item needs an active entry),
//!   `sendIntervalSec` 5–899, includes / excludes
//!   resolve, selected Modbus items ≤ 20 slaves and ≤ 40 registers per slave, scenarios (unique
//!   ids, ≤ 1 start, steps reference selected items / valid fault targets, the start scenario
//!   resets everything the other scenarios change, no scenario hides a value behind a slave
//!   timeout), floorplans exist.
//! - floorplans: name, image exists, size > 0, unique widget ids inside the image, references
//!   resolve against the demo (selected items, slaves, groups); literal `lw_*` alarm types must be
//!   real firmware types or `lw_demo_*`; ranges do not overlap; Modbus controls target writable
//!   coil / holding items; `number` / `multi` controls use integers 0-65535 and `multi` needs
//!   holding registers.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use super::Library;
use super::select::{parse_tag, select_items};
use crate::model::{
    AlarmRef, Control, DemoPack, DigitalInputType, FaultTarget, FloorplanLayout, FloorplanWidget,
    GeneratorConfig, ItemEffect, ItemKind, LibraryItem, ModbusDatatype, ModbusRegisterType,
    ModbusValueType, ScenarioStep, SensorDisplay, SlaveDefinition, WidgetControl, WidgetRange,
};
use crate::payload::alarm::{dali_bus_alarms, dali_lamp_alarms, is_native_alarm_type};
use crate::payload::naming::{
    DALI_SERIES, MAX_NAME_LEN, MAX_REGISTERS_PER_SLAVE, MAX_SLAVES, MAX_UNIT_LEN,
    is_valid_alarm_name, is_valid_device_alarm_name,
};

/// Max DALI lamps / groups / digital inputs / sensors (firmware).
pub const MAX_DALI_LAMPS: u32 = 64;
pub const MAX_DALI_GROUPS: usize = 16;
pub const MAX_DIGITAL_INPUTS: u32 = 10;
pub const MAX_SENSORS: u32 = 10;
/// `sendIntervalSec` must stay below the plugin's stale threshold of 900 s (D5).
pub const MAX_SEND_INTERVAL_SEC: u32 = 899;
/// Lower bound of `sendIntervalSec` (shared with the session scheduler).
pub const MIN_SEND_INTERVAL_SEC: u32 = crate::sim::scheduler::MIN_SEND_INTERVAL_SEC;
/// Max length of a digital input / impulse counter unit, in UTF-8 bytes.
pub const MAX_DI_UNIT_LEN: usize = 19;
/// Event types the firmware / plugin own: an `event` effect must not fake them (the plugin
/// derives DALI state from the newest `lw_daliGroupChange`).
pub const RESERVED_EVENT_TYPES: [&str; 3] = ["lw_daliGroupChange", "lw_alarm", "lw_modbus"];
/// Allowed DI `highLevelAction` / `lowLevelAction` values (the firmware matches lowercase).
pub const DI_ACTIONS: [&str; 3] = ["on", "off", "auto"];

/// Inventory properties an `inventory` effect must not write: firmware config fragments
/// (`lw_config*`), device identity (`c8y_*`, `lw_demo`, `lw_floorplan`) and managed-object core
/// fields. Use a `lw_demo*` prefix for effect properties and event types.
fn reserved_inventory_property(property: &str) -> bool {
    property.starts_with("lw_config")
        || property.starts_with("c8y_")
        || matches!(
            property,
            "lw_demo"
                | "lw_demoLock"
                | "lw_demoSnapshot"
                | "lw_floorplan"
                | "name"
                | "type"
                | "id"
                | "owner"
                | "self"
                | "description"
                | "lastUpdated"
                | "creationTime"
        )
}

/// One validation problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// File relative to the library root (or `lw_demo` for device-local setup).
    pub file: String,
    /// JSON pointer inside the file, e.g. `/items/3/id`.
    pub path: String,
    pub message: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            write!(f, "{}: {}", self.file, self.message)
        } else {
            write!(f, "{}: {}: {}", self.file, self.path, self.message)
        }
    }
}

#[derive(Default)]
struct Report(Vec<Finding>);

impl Report {
    fn add(&mut self, file: &str, path: impl Into<String>, message: impl Into<String>) {
        self.0.push(Finding {
            file: file.to_owned(),
            path: path.into(),
            message: message.into(),
        });
    }
}

/// `[A-Za-z0-9_-]{1,19}`.
pub fn is_valid_item_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().count() <= MAX_NAME_LEN
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// `((category|zone):)?[a-z0-9][a-z0-9-]*`.
pub fn is_valid_tag(tag: &str) -> bool {
    let Some((_, value)) = parse_tag(tag) else {
        return false;
    };
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 40
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn finite(values: &[f64]) -> bool {
    values.iter().all(|v| v.is_finite())
}

// ---------------------------------------------------------------------------------------------
// Items and slaves
// ---------------------------------------------------------------------------------------------

fn check_control(r: &mut Report, file: &str, path: &str, control: &Control) {
    match control {
        Control::Slider {
            min,
            max,
            step,
            default,
        } => {
            if !finite(&[*min, *max, *step, *default]) {
                r.add(file, path, "slider values must be finite numbers");
            } else if min >= max {
                r.add(
                    file,
                    format!("{path}/min"),
                    format!("min {min} ≥ max {max}"),
                );
            } else if *step <= 0.0 {
                r.add(file, format!("{path}/step"), "step must be > 0");
            } else if default < min || default > max {
                r.add(
                    file,
                    format!("{path}/default"),
                    format!("default {default} outside {min}…{max}"),
                );
            }
        }
        Control::Switch { default } => {
            if *default != 0.0 && *default != 1.0 {
                r.add(
                    file,
                    format!("{path}/default"),
                    "switch default must be 0 or 1",
                );
            }
        }
        Control::Select { options, default } => {
            if options.is_empty() {
                r.add(file, format!("{path}/options"), "select needs options");
            }
            let mut seen = Vec::new();
            for (i, o) in options.iter().enumerate() {
                if !o.value.is_finite() || seen.contains(&o.value.to_bits()) {
                    r.add(
                        file,
                        format!("{path}/options/{i}/value"),
                        format!("option value {} is not unique / finite", o.value),
                    );
                }
                seen.push(o.value.to_bits());
            }
            if !options.iter().any(|o| o.value == *default) {
                r.add(
                    file,
                    format!("{path}/default"),
                    format!("default {default} is not an option value"),
                );
            }
        }
        Control::Counter { default } => {
            if !default.is_finite() || *default < 0.0 {
                r.add(
                    file,
                    format!("{path}/default"),
                    "counter default must be ≥ 0",
                );
            }
        }
    }
}

/// Generator rules (also used for scenario steps and `lw_demo.generators`).
pub fn generator_problem(g: &GeneratorConfig) -> Option<String> {
    match g {
        GeneratorConfig::RandomWalk {
            center,
            spread,
            step,
        } => {
            if !finite(&[*center, *spread, step.unwrap_or(1.0)]) {
                Some("randomWalk values must be finite".into())
            } else if *spread < 0.0 {
                Some("randomWalk spread must be ≥ 0".into())
            } else if step.is_some_and(|s| s <= 0.0) {
                Some("randomWalk step must be > 0".into())
            } else {
                None
            }
        }
        GeneratorConfig::Sine {
            period,
            amplitude,
            center,
        } => {
            if !finite(&[*period, *amplitude, center.unwrap_or(0.0)]) {
                Some("sine values must be finite".into())
            } else if *period <= 0.0 {
                Some("sine period must be > 0 s".into())
            } else {
                None
            }
        }
        GeneratorConfig::DailyProfile { points } => {
            if points.len() != 24 {
                Some(format!(
                    "dailyProfile needs 24 points, has {}",
                    points.len()
                ))
            } else if !finite(points) {
                Some("dailyProfile points must be finite".into())
            } else {
                None
            }
        }
        GeneratorConfig::Counter { rate_per_hour } => {
            if !rate_per_hour.is_finite() || *rate_per_hour < 0.0 {
                Some("counter ratePerHour must be ≥ 0".into())
            } else {
                None
            }
        }
    }
}

/// Writable holding registers are written with FC6: **one 16-bit word**, value 0-65535, so the
/// datatype must not be 32-bit / float and the control must produce integers in that range.
fn check_writable_holding(
    r: &mut Report,
    file: &str,
    path: &str,
    datatype: Option<ModbusDatatype>,
    control: Option<&Control>,
) {
    if matches!(
        datatype,
        Some(ModbusDatatype::Float32 | ModbusDatatype::Int32 | ModbusDatatype::Uint32)
    ) {
        r.add(
            file,
            format!("{path}/datatype"),
            "a writable holding register is written as one 16-bit word (FC6); use int16 / uint16",
        );
    }
    let int_word = |v: f64| v.is_finite() && v.fract() == 0.0 && (0.0..=65_535.0).contains(&v);
    match control {
        Some(Control::Slider { min, max, step, .. }) => {
            if !(int_word(*min) && int_word(*max) && step.fract() == 0.0) {
                r.add(
                    file,
                    format!("{path}/control"),
                    "a writable holding register needs a slider with integer min / max in \
                     0-65535 and an integer step (the plugin writes whole words)",
                );
            }
        }
        Some(Control::Select { options, .. }) => {
            if let Some(i) = options.iter().position(|o| !int_word(o.value)) {
                r.add(
                    file,
                    format!("{path}/control/options/{i}/value"),
                    "option values of a writable holding register must be integers 0-65535",
                );
            }
        }
        _ => {}
    }
}

/// Whether `generator` makes sense for `item`: `counter` only for counters, continuous
/// generators not for on/off style values (switch / select controls, DI values, motion).
fn generator_fit_problem(item: &LibraryItem, generator: &GeneratorConfig) -> Option<String> {
    let counter_item = item.kind() == ItemKind::ImpulseCounter
        || matches!(item.control(), Some(Control::Counter { .. }));
    let discrete_item = matches!(
        item.kind(),
        ItemKind::DigitalInput | ItemKind::DaliMotion | ItemKind::ImpulseCounter
    ) || matches!(
        item.control(),
        Some(Control::Switch { .. } | Control::Select { .. } | Control::Counter { .. })
    );
    match generator {
        GeneratorConfig::Counter { .. } if !counter_item => {
            Some("a counter generator needs a counter control or an impulseCounter item".into())
        }
        GeneratorConfig::Counter { .. } => None,
        _ if discrete_item => Some(
            "randomWalk / sine / dailyProfile would send fractions to a switch, select, counter, \
             digital input or motion value; use a scenario step or no generator"
                .into(),
        ),
        _ => None,
    }
}

/// A generator whose band leaves the item's slider range sends values the slider cannot show
/// (a lamp with `center 90, spread 20` would send 110 %).
fn generator_band_problem(item: &LibraryItem, generator: &GeneratorConfig) -> Option<String> {
    let Some(Control::Slider {
        min, max, default, ..
    }) = item.control()
    else {
        return None;
    };
    let (lo, hi, what) = match generator {
        GeneratorConfig::RandomWalk { center, spread, .. } => (
            center - spread.abs(),
            center + spread.abs(),
            "randomWalk band",
        ),
        GeneratorConfig::Sine {
            amplitude, center, ..
        } => {
            let c = center.unwrap_or(*default);
            (c - amplitude.abs(), c + amplitude.abs(), "sine range")
        }
        GeneratorConfig::DailyProfile { points } => (
            points.iter().copied().fold(f64::INFINITY, f64::min),
            points.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            "dailyProfile range",
        ),
        GeneratorConfig::Counter { .. } => return None,
    };
    (lo < *min || hi > *max)
        .then(|| format!("{what} {lo}…{hi} leaves the control range {min}…{max}"))
}

fn effects_of(item: &LibraryItem) -> &[ItemEffect] {
    item.effects()
}

fn index_range(kind: ItemKind) -> Option<u32> {
    match kind {
        ItemKind::Dali => Some(MAX_DALI_LAMPS),
        ItemKind::DaliBrightness | ItemKind::DaliMotion => Some(MAX_SENSORS),
        ItemKind::DigitalInput | ItemKind::ImpulseCounter => Some(MAX_DIGITAL_INPUTS),
        _ => None,
    }
}

/// Rules for one item on its own (plus its slave reference).
fn check_item<'a>(
    r: &mut Report,
    file: &str,
    path: &str,
    item: &LibraryItem,
    slave_of: &dyn Fn(&str) -> Option<&'a SlaveDefinition>,
) {
    let id = item.id();
    if !is_valid_item_id(id) {
        r.add(
            file,
            format!("{path}/id"),
            format!("id \"{id}\" must be 1–{MAX_NAME_LEN} chars of [A-Za-z0-9_-]"),
        );
    }
    let title = item.title();
    if title.de.trim().is_empty() || title.en.trim().is_empty() {
        r.add(file, format!("{path}/title"), "title needs de and en");
    }
    for (i, tag) in item.tags().iter().enumerate() {
        if !is_valid_tag(tag) {
            r.add(
                file,
                format!("{path}/tags/{i}"),
                format!("tag \"{tag}\" must be a client tag, category:* or zone:* ([a-z0-9-])"),
            );
        }
    }
    if let (Some(index), Some(max)) = (item.index(), index_range(item.kind()))
        && (index == 0 || index > max)
    {
        r.add(
            file,
            format!("{path}/index"),
            format!("index {index} outside 1–{max}"),
        );
    }
    match item {
        LibraryItem::Dali { unit, .. }
        | LibraryItem::DaliBrightness { unit, .. }
        | LibraryItem::DaliMotion { unit, .. }
        | LibraryItem::DigitalInput { unit, .. }
            if unit.is_some() =>
        {
            r.add(
                file,
                format!("{path}/unit"),
                "unit is fixed by the firmware for this kind",
            );
        }
        LibraryItem::ImpulseCounter {
            unit: Some(unit), ..
        } if unit.len() > MAX_DI_UNIT_LEN => {
            r.add(
                file,
                format!("{path}/unit"),
                format!("unit \"{unit}\" is longer than {MAX_DI_UNIT_LEN} bytes"),
            );
        }
        LibraryItem::Modbus {
            slave,
            unit,
            writable,
            control,
            datatype,
            ..
        } => {
            if slave_of(slave).is_none() {
                r.add(
                    file,
                    format!("{path}/slave"),
                    format!("slave \"{slave}\" is not defined"),
                );
            }
            if let Some(u) = unit
                && u.len() > MAX_UNIT_LEN
            {
                r.add(
                    file,
                    format!("{path}/unit"),
                    format!("unit \"{u}\" is longer than {MAX_UNIT_LEN} bytes"),
                );
            }
            let register_type = item.register_type();
            if *writable && register_type == Some(ModbusRegisterType::Input) {
                r.add(
                    file,
                    format!("{path}/writable"),
                    "input registers cannot be writable (use coil or holding)",
                );
            }
            if register_type == Some(ModbusRegisterType::Coil)
                && control
                    .as_ref()
                    .is_some_and(|c| !matches!(c, Control::Switch { .. }))
            {
                r.add(
                    file,
                    format!("{path}/control"),
                    "a coil (0/1) needs a switch control",
                );
            }
            if *writable && register_type == Some(ModbusRegisterType::Holding) {
                check_writable_holding(r, file, path, *datatype, control.as_ref());
            }
        }
        LibraryItem::CustomAlarm { id, text, .. } => {
            if !is_valid_device_alarm_name(id) {
                r.add(
                    file,
                    format!("{path}/id"),
                    format!("custom alarm id \"{id}\" must not start with two digits plus _"),
                );
            }
            if text.trim().is_empty() {
                r.add(file, format!("{path}/text"), "alarm text is empty");
            }
        }
        _ => {}
    }
    if let Some(c) = item.control() {
        check_control(r, file, &format!("{path}/control"), c);
    }
    if let Some(g) = item.generator() {
        if let Some(problem) = generator_problem(g) {
            r.add(file, format!("{path}/generator"), problem);
        } else if let Some(problem) =
            generator_fit_problem(item, g).or_else(|| generator_band_problem(item, g))
        {
            r.add(file, format!("{path}/generator"), problem);
        }
    }
    for (i, e) in effects_of(item).iter().enumerate() {
        let p = format!("{path}/effects/{i}");
        match e {
            ItemEffect::Event { event_type, text } => {
                if event_type.trim().is_empty() || text.trim().is_empty() {
                    r.add(file, &p, "event effect needs type and text");
                }
                if RESERVED_EVENT_TYPES.contains(&event_type.as_str()) {
                    r.add(
                        file,
                        format!("{p}/type"),
                        format!(
                            "event type \"{event_type}\" is a native firmware event; use a \
                             lw_demo* type"
                        ),
                    );
                }
            }
            ItemEffect::Inventory { property } => {
                let ok = property
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                    && property
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_');
                if !ok {
                    r.add(
                        file,
                        &p,
                        format!("inventory property \"{property}\" is invalid"),
                    );
                }
                if reserved_inventory_property(property) {
                    r.add(
                        file,
                        format!("{p}/property"),
                        format!(
                            "inventory property \"{property}\" is reserved (firmware config, \
                             device identity or managed-object field); use a lw_demo* property"
                        ),
                    );
                }
            }
            ItemEffect::Alarm {
                alarm_type,
                text,
                threshold,
                ..
            } => {
                if !is_valid_alarm_name(alarm_type)
                    || text.trim().is_empty()
                    || !threshold.is_finite()
                {
                    r.add(
                        file,
                        &p,
                        "alarm effect needs a valid type, text and threshold",
                    );
                }
                if is_native_alarm_type(alarm_type) {
                    r.add(
                        file,
                        format!("{p}/type"),
                        format!(
                            "alarm type \"{alarm_type}\" is a native firmware alarm; use a \
                             lw_demo* type"
                        ),
                    );
                }
            }
        }
    }
}

fn check_slave(r: &mut Report, file: &str, path: &str, slave: &SlaveDefinition) {
    if !is_valid_key(&slave.key) {
        r.add(
            file,
            format!("{path}/key"),
            format!("slave key \"{}\" must be [A-Za-z0-9_-]", slave.key),
        );
    }
    let len = slave.name.len(); // UTF-8 bytes: the firmware buffer is 20 bytes incl. NUL
    if len == 0 || len > MAX_NAME_LEN {
        r.add(
            file,
            format!("{path}/name"),
            format!(
                "slave name \"{}\" must be 1–{MAX_NAME_LEN} bytes ({len} in UTF-8)",
                slave.name
            ),
        );
    }
    if slave.request_interval == 0 || slave.request_interval > 99_999 {
        r.add(
            file,
            format!("{path}/requestInterval"),
            "requestInterval must be 1–99999 s",
        );
    }
    let mut names = BTreeSet::new();
    for (i, a) in slave.alarms.iter().enumerate() {
        let p = format!("{path}/alarms/{i}");
        if !is_valid_alarm_name(&a.name) {
            r.add(
                file,
                format!("{p}/name"),
                format!("alarm name \"{}\" must be [A-Za-z0-9_-]", a.name),
            );
        }
        if !names.insert(a.name.as_str()) {
            r.add(
                file,
                format!("{p}/name"),
                format!("duplicate alarm name \"{}\"", a.name),
            );
        }
        if a.text.trim().is_empty() {
            r.add(file, format!("{p}/text"), "alarm text is empty");
        }
    }
}

/// Native target of an item for the "one item = one thing" rule.
fn native_target(item: &LibraryItem) -> Option<String> {
    match item {
        LibraryItem::System { system, .. } => Some(format!("system {system:?}")),
        _ => item.index().map(|i| format!("{:?} {i}", item.kind())),
    }
}

// ---------------------------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------------------------

/// Validate the whole loaded library. Empty = valid.
pub fn validate(lib: &Library) -> Vec<Finding> {
    let mut r = Report::default();
    let origin = |list: &[super::Origin], i: usize| {
        list.get(i).cloned().unwrap_or_else(|| super::Origin {
            file: "?".into(),
            index: i,
        })
    };

    let mut slave_keys: BTreeMap<&str, String> = BTreeMap::new();
    for (i, s) in lib.slaves.iter().enumerate() {
        let o = origin(&lib.origins.slaves, i);
        let path = format!("/slaves/{}", o.index);
        check_slave(&mut r, &o.file, &path, s);
        if let Some(first) = slave_keys.get(s.key.as_str()) {
            r.add(
                &o.file,
                format!("{path}/key"),
                format!("duplicate slave key \"{}\" (first in {first})", s.key),
            );
        } else {
            slave_keys.insert(&s.key, format!("{}{path}", o.file));
        }
    }

    let slave_of = |k: &str| lib.slave(k);
    let mut ids: BTreeMap<&str, String> = BTreeMap::new();
    let mut targets: BTreeMap<String, String> = BTreeMap::new();
    for (i, item) in lib.items.iter().enumerate() {
        let o = origin(&lib.origins.items, i);
        let path = format!("/items/{}", o.index);
        check_item(&mut r, &o.file, &path, item, &slave_of);
        let here = format!("{}{path}", o.file);
        if let Some(first) = ids.get(item.id()) {
            r.add(
                &o.file,
                format!("{path}/id"),
                format!("duplicate item id \"{}\" (first in {first})", item.id()),
            );
        } else {
            ids.insert(item.id(), here.clone());
        }
        if let Some(t) = native_target(item) {
            if let Some(first) = targets.get(&t) {
                r.add(
                    &o.file,
                    &path,
                    format!("{t} already has an item ({first}); one item = one thing"),
                );
            } else {
                targets.insert(t, here);
            }
        }
    }

    let mut demo_ids: BTreeMap<&str, String> = BTreeMap::new();
    for (i, demo) in lib.demos.iter().enumerate() {
        let file = origin(&lib.origins.demos, i).file;
        if let Some(first) = demo_ids.get(demo.id.as_str()) {
            r.add(
                &file,
                "/id",
                format!("duplicate demo id \"{}\" (first in {first})", demo.id),
            );
        } else {
            demo_ids.insert(&demo.id, file.clone());
        }
        check_demo(&mut r, lib, &file, demo);
    }
    r.0
}

/// Validate device-local slaves / items (`lw_demo.customSlaves` / `customItems`, D6) against
/// the library: same item / slave rules, ids must not collide with library ids, Modbus items may
/// reference library or custom slaves. Paths are `/customSlaves/<i>`, `/customItems/<i>`.
pub fn validate_custom(
    lib: &Library,
    slaves: &[SlaveDefinition],
    items: &[LibraryItem],
) -> Vec<Finding> {
    const FILE: &str = "lw_demo";
    let mut r = Report::default();
    let mut keys = BTreeSet::new();
    for (i, s) in slaves.iter().enumerate() {
        let path = format!("/customSlaves/{i}");
        check_slave(&mut r, FILE, &path, s);
        if lib.slave(&s.key).is_some() || !keys.insert(s.key.as_str()) {
            r.add(
                FILE,
                format!("{path}/key"),
                format!("slave key \"{}\" is already used", s.key),
            );
        }
    }
    let slave_of = |k: &str| lib.slave(k).or_else(|| slaves.iter().find(|s| s.key == k));
    let mut ids = BTreeSet::new();
    for (i, item) in items.iter().enumerate() {
        let path = format!("/customItems/{i}");
        check_item(&mut r, FILE, &path, item, &slave_of);
        if lib.item(item.id()).is_some() || !ids.insert(item.id()) {
            r.add(
                FILE,
                format!("{path}/id"),
                format!("item id \"{}\" is already used", item.id()),
            );
        }
    }
    r.0
}

// ---------------------------------------------------------------------------------------------
// Demos
// ---------------------------------------------------------------------------------------------

/// Whether `value` fits the item's control (slider range, select option, switch 0/1).
fn value_problem(item: &LibraryItem, value: f64) -> Option<String> {
    if !value.is_finite() {
        return Some("value must be a finite number".into());
    }
    match item.control() {
        Some(Control::Slider { min, max, .. }) if value < *min || value > *max => {
            Some(format!("value {value} outside {min}…{max}"))
        }
        Some(Control::Switch { .. }) if value != 0.0 && value != 1.0 => {
            Some(format!("value {value} must be 0 or 1"))
        }
        Some(Control::Select { options, .. }) if !options.iter().any(|o| o.value == value) => {
            Some(format!("value {value} is not an option value"))
        }
        // Counters are monotonic (PRD §8): a value below the start would send them backwards.
        Some(Control::Counter { default }) if value < *default => Some(format!(
            "counter value {value} is below its start {default} (counters never decrease)"
        )),
        _ => None,
    }
}

fn check_demo(r: &mut Report, lib: &Library, file: &str, demo: &DemoPack) {
    let id_ok = !demo.id.is_empty()
        && demo.id.split('-').all(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        });
    if !id_ok {
        r.add(
            file,
            "/id",
            format!("demo id \"{}\" must be [a-z0-9-]", demo.id),
        );
    }
    for (field, text) in [("name", &demo.name), ("description", &demo.description)] {
        if text.de.trim().is_empty() || text.en.trim().is_empty() {
            r.add(
                file,
                format!("/{field}"),
                format!("{field} needs de and en"),
            );
        }
    }
    for (i, tag) in demo.tags.iter().enumerate() {
        if !is_valid_tag(tag) {
            r.add(
                file,
                format!("/tags/{i}"),
                format!("tag \"{tag}\" is invalid"),
            );
        }
    }
    for (field, list) in [("includes", &demo.includes), ("excludes", &demo.excludes)] {
        for (i, id) in list.iter().enumerate() {
            if lib.item(id).is_none() {
                r.add(
                    file,
                    format!("/{field}/{i}"),
                    format!("item \"{id}\" does not exist"),
                );
            }
        }
    }
    let lamps = demo.dali_lamp_count;
    if lamps == 0 || lamps > MAX_DALI_LAMPS {
        r.add(
            file,
            "/daliLampCount",
            format!("daliLampCount {lamps} outside 1–{MAX_DALI_LAMPS}"),
        );
    }
    if demo.send_interval_sec < MIN_SEND_INTERVAL_SEC
        || demo.send_interval_sec > MAX_SEND_INTERVAL_SEC
    {
        r.add(
            file,
            "/sendIntervalSec",
            format!(
                "sendIntervalSec {} outside 1–{MAX_SEND_INTERVAL_SEC} (stale after 900 s)",
                demo.send_interval_sec
            ),
        );
    }

    let selected = select_items(&lib.items, demo);
    if selected.is_empty() {
        r.add(file, "/tags", "the demo selects no items");
    }
    let selected_item = |id: &str| selected.iter().copied().find(|i| i.id() == id);

    // DALI groups.
    if demo.dali_groups.len() > MAX_DALI_GROUPS {
        r.add(
            file,
            "/daliGroups",
            format!("{} groups, max {MAX_DALI_GROUPS}", demo.dali_groups.len()),
        );
    }
    let group_ids: Vec<u32> = demo.dali_groups.iter().map(|g| g.id).collect();
    let mut seen_groups = BTreeSet::new();
    for (i, g) in demo.dali_groups.iter().enumerate() {
        let p = format!("/daliGroups/{i}");
        if g.id == 0 || g.id as usize > MAX_DALI_GROUPS {
            r.add(
                file,
                format!("{p}/id"),
                format!("group id {} outside 1–16", g.id),
            );
        }
        if !seen_groups.insert(g.id) {
            r.add(
                file,
                format!("{p}/id"),
                format!("duplicate group id {}", g.id),
            );
        }
        if g.name.trim().is_empty() {
            r.add(file, format!("{p}/name"), "group name is empty");
        }
        if g.min_level > 100 || g.max_level > 100 || g.min_level > g.max_level {
            r.add(
                file,
                format!("{p}/maxLevel"),
                format!(
                    "levels must be 0 ≤ min ({}) ≤ max ({}) ≤ 100",
                    g.min_level, g.max_level
                ),
            );
        }
        if g.lux_threshold.is_some_and(|l| l > 200_000) {
            r.add(
                file,
                format!("{p}/luxThreshold"),
                "luxThreshold must be 0–200000",
            );
        }
        if let Some(master) = g.slave_of
            && (master == g.id || !group_ids.contains(&master))
        {
            r.add(
                file,
                format!("{p}/slaveOf"),
                format!("slaveOf {master} is not another group of this demo"),
            );
        }
        let mut seen = BTreeSet::new();
        for (j, c) in g.converters.iter().enumerate() {
            if *c == 0 || *c > lamps {
                r.add(
                    file,
                    format!("{p}/converters/{j}"),
                    format!("lamp {c} outside 1–{lamps} (daliLampCount)"),
                );
            }
            if !seen.insert(*c) {
                r.add(
                    file,
                    format!("{p}/converters/{j}"),
                    format!("duplicate lamp {c}"),
                );
            }
        }
    }

    // Selected items vs. the device setup.
    let mut per_slave: BTreeMap<&str, usize> = BTreeMap::new();
    for item in &selected {
        match item {
            LibraryItem::Dali { index, id, .. } => {
                if *index > lamps {
                    r.add(
                        file,
                        "/daliLampCount",
                        format!("item {id} uses lamp {index} > daliLampCount {lamps}"),
                    );
                } else if !demo
                    .dali_groups
                    .iter()
                    .any(|g| g.converters.contains(index))
                {
                    r.add(
                        file,
                        "/daliGroups",
                        format!(
                            "lamp {index} (item {id}) is in no group; the firmware only sends \
                             grouped lamps: add lamp {index} to a group's `converters` or add \
                             `{id}` to `excludes`"
                        ),
                    );
                }
            }
            LibraryItem::Modbus { slave, .. } => *per_slave.entry(slave).or_default() += 1,
            LibraryItem::DigitalInput { index, id, .. }
            | LibraryItem::ImpulseCounter { index, id, .. } => {
                // Same reasoning as for lamps: the firmware only sends configured, active inputs.
                match demo.digital_inputs.iter().find(|d| d.item_id == *id) {
                    None => r.add(
                        file,
                        "/digitalInputs",
                        format!(
                            "input {index} (item {id}) has no digitalInputs[] entry; the firmware \
                             only sends configured, active inputs: add \
                             {{\"index\": {index}, \"itemId\": \"{id}\", \"type\": ...}} or add \
                             `{id}` to `excludes`"
                        ),
                    ),
                    Some(di) if !di.settings.is_active => r.add(
                        file,
                        "/digitalInputs",
                        format!(
                            "input {index} (item {id}) has isActive: false, so it never sends; \
                             activate it or add `{id}` to `excludes`"
                        ),
                    ),
                    Some(_) => {}
                }
            }
            _ => {}
        }
    }
    if per_slave.len() > MAX_SLAVES {
        r.add(
            file,
            "/tags",
            format!(
                "selects {} Modbus slaves, max {MAX_SLAVES}",
                per_slave.len()
            ),
        );
    }
    for (slave, n) in &per_slave {
        if *n > MAX_REGISTERS_PER_SLAVE {
            r.add(
                file,
                "/tags",
                format!("slave {slave} gets {n} registers, max {MAX_REGISTERS_PER_SLAVE}"),
            );
        }
    }

    // Digital inputs.
    let mut seen_inputs = BTreeSet::new();
    for (i, di) in demo.digital_inputs.iter().enumerate() {
        let p = format!("/digitalInputs/{i}");
        let s = &di.settings;
        if di.index == 0 || di.index > MAX_DIGITAL_INPUTS {
            r.add(
                file,
                format!("{p}/index"),
                format!("index {} outside 1–10", di.index),
            );
        }
        if !seen_inputs.insert(di.index) {
            r.add(
                file,
                format!("{p}/index"),
                format!("duplicate input {}", di.index),
            );
        }
        match selected_item(&di.item_id) {
            None => r.add(
                file,
                format!("{p}/itemId"),
                format!("item \"{}\" is not selected by this demo", di.item_id),
            ),
            Some(item) => {
                let expected = match item.kind() {
                    ItemKind::DigitalInput => Some(false),
                    ItemKind::ImpulseCounter => Some(true),
                    _ => None,
                };
                match expected {
                    None => r.add(
                        file,
                        format!("{p}/itemId"),
                        format!(
                            "item \"{}\" is not a digitalInput / impulseCounter",
                            di.item_id
                        ),
                    ),
                    Some(counter) => {
                        if item.index() != Some(di.index) {
                            r.add(
                                file,
                                format!("{p}/index"),
                                format!(
                                    "item \"{}\" has index {:?}, not {}",
                                    di.item_id,
                                    item.index(),
                                    di.index
                                ),
                            );
                        }
                        if counter != (s.input_type == DigitalInputType::ImpulseCounter) {
                            r.add(
                                file,
                                format!("{p}/type"),
                                "impulseCounter items need type impulseCounter, digitalInput \
                                 items daliAction or measurement",
                            );
                        } else if counter {
                            // Two sources of truth (item vs DI entry): they must agree.
                            if item.unit() != s.unit.as_deref() {
                                r.add(
                                    file,
                                    format!("{p}/unit"),
                                    format!(
                                        "unit {:?} differs from the unit {:?} of item \"{}\" \
                                         (the measurement unit comes from the item)",
                                        s.unit,
                                        item.unit(),
                                        di.item_id
                                    ),
                                );
                            }
                            if let Some(Control::Counter { default }) = item.control()
                                && *default != f64::from(s.init_count.unwrap_or(0))
                            {
                                r.add(
                                    file,
                                    format!("{p}/initCount"),
                                    format!(
                                        "initCount {} differs from the counter start {default} \
                                         (control.default) of item \"{}\"",
                                        s.init_count.unwrap_or(0),
                                        di.item_id
                                    ),
                                );
                            }
                        }
                    }
                }
            }
        }
        let counter_fields = s.init_count.is_some() || s.increment.is_some() || s.unit.is_some();
        let action_fields =
            s.high_level_action.is_some() || s.low_level_action.is_some() || !s.groups.is_empty();
        match s.input_type {
            DigitalInputType::DaliAction => {
                if counter_fields {
                    r.add(
                        file,
                        &p,
                        "initCount / increment / unit only for impulseCounter",
                    );
                }
                if s.high_level_action.is_none() && s.low_level_action.is_none() {
                    r.add(
                        file,
                        &p,
                        "daliAction needs highLevelAction or lowLevelAction",
                    );
                }
                for (field, action) in [
                    ("highLevelAction", &s.high_level_action),
                    ("lowLevelAction", &s.low_level_action),
                ] {
                    if let Some(a) = action
                        && !DI_ACTIONS.contains(&a.as_str())
                    {
                        r.add(
                            file,
                            format!("{p}/{field}"),
                            format!(
                                "action \"{a}\" must be on, off or auto (lowercase; the \
                                 firmware matches with a lowercase strstr)"
                            ),
                        );
                    }
                }
                for (j, g) in s.groups.iter().enumerate() {
                    if !group_ids.contains(g) {
                        r.add(
                            file,
                            format!("{p}/groups/{j}"),
                            format!("group {g} is not a group of this demo"),
                        );
                    }
                }
            }
            DigitalInputType::ImpulseCounter => {
                if action_fields {
                    r.add(file, &p, "actions / groups only for daliAction");
                }
                if s.increment.is_some_and(|x| x == 0 || x > 65_535) {
                    r.add(file, format!("{p}/increment"), "increment must be 1–65535");
                }
                if s.unit.as_ref().is_some_and(|u| u.len() > MAX_DI_UNIT_LEN) {
                    r.add(file, format!("{p}/unit"), "unit is longer than 19 bytes");
                }
            }
            DigitalInputType::Measurement => {
                if counter_fields || action_fields {
                    r.add(file, &p, "measurement inputs take no extra settings");
                }
            }
        }
    }

    // Scenarios.
    let used_slaves: BTreeSet<&str> = per_slave.keys().copied().collect();
    let mut scenario_ids = BTreeSet::new();
    let mut starts = 0;
    for (i, sc) in demo.scenarios.iter().enumerate() {
        let p = format!("/scenarios/{i}");
        if sc.id.is_empty()
            || !sc
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            r.add(
                file,
                format!("{p}/id"),
                format!("scenario id \"{}\" must be [A-Za-z0-9_-]", sc.id),
            );
        }
        if !scenario_ids.insert(sc.id.as_str()) {
            r.add(
                file,
                format!("{p}/id"),
                format!("duplicate scenario id \"{}\"", sc.id),
            );
        }
        if sc.start {
            starts += 1;
            if starts > 1 {
                r.add(
                    file,
                    format!("{p}/start"),
                    "only one scenario can be the start scenario",
                );
            }
        }
        for (j, step) in sc.set.iter().enumerate() {
            let sp = format!("{p}/set/{j}");
            match step {
                ScenarioStep::Value { item_id, value } => match selected_item(item_id) {
                    None => r.add(
                        file,
                        format!("{sp}/itemId"),
                        format!("item \"{item_id}\" is not selected by this demo"),
                    ),
                    Some(item) if item.kind() == ItemKind::CustomAlarm => r.add(
                        file,
                        format!("{sp}/itemId"),
                        "custom alarms have no value; use a fault step",
                    ),
                    Some(item) => {
                        if let Some(problem) = value_problem(item, *value) {
                            r.add(file, format!("{sp}/value"), problem);
                        }
                    }
                },
                ScenarioStep::Generator { item_id, generator } => {
                    match selected_item(item_id) {
                        None => r.add(
                            file,
                            format!("{sp}/itemId"),
                            format!("item \"{item_id}\" is not selected by this demo"),
                        ),
                        Some(item) if item.kind() == ItemKind::CustomAlarm => r.add(
                            file,
                            format!("{sp}/itemId"),
                            "custom alarms have no generator",
                        ),
                        Some(item) => {
                            if let Some(problem) = generator_fit_problem(item, generator)
                                .or_else(|| generator_band_problem(item, generator))
                            {
                                r.add(file, format!("{sp}/generator"), problem);
                            }
                        }
                    }
                    if let Some(problem) = generator_problem(generator) {
                        r.add(file, format!("{sp}/generator"), problem);
                    }
                }
                ScenarioStep::Fault { target, .. } => {
                    if let Some(problem) =
                        fault_problem(lib, target, lamps, &used_slaves, &selected_item)
                    {
                        r.add(file, format!("{sp}/target"), problem);
                    }
                }
            }
        }
    }

    check_scenario_resets(r, file, demo, &selected_item);
    check_scenario_hidden_values(r, file, demo, &selected_item);

    // Floorplans.
    for (i, rel) in demo.floorplans.iter().enumerate() {
        let p = format!("/floorplans/{i}");
        let prefix = format!("floorplans/{}/", demo.id);
        if !rel.starts_with(&prefix) || rel[prefix.len()..].contains('/') {
            r.add(file, &p, format!("floorplan must live in {prefix}"));
        }
        match lib.floorplan(rel) {
            Some(layout) => check_floorplan(r, lib, rel, layout, demo, &selected),
            None => r.add(
                file,
                &p,
                format!("floorplan {rel} is missing or could not be loaded"),
            ),
        }
    }
}

/// The start scenario is the full reset ("Normalbetrieb" restores the demo): every item and
/// fault another scenario touches must be reset by it, else a presenter who ran "Störung" and
/// then restarts the session (or applies the start scenario) still sees the old trouble.
/// An item with a library generator is reset by a `generator` step, any other item by a `value`
/// step, a fault by `fault … active: false`. (Applied by hand the start scenario also clears every
/// open alarm, D9; at session start only its steps run, so these clears still matter.)
fn check_scenario_resets<'a>(
    r: &mut Report,
    file: &str,
    demo: &DemoPack,
    selected_item: &dyn Fn(&str) -> Option<&'a LibraryItem>,
) {
    let start = demo.scenarios.iter().find(|s| s.start);
    let mut no_start_reported = false;
    for (i, sc) in demo.scenarios.iter().enumerate().filter(|(_, s)| !s.start) {
        for (j, step) in sc.set.iter().enumerate() {
            let sp = format!("/scenarios/{i}/set/{j}");
            let problem = match step {
                ScenarioStep::Value { item_id, .. } | ScenarioStep::Generator { item_id, .. } => {
                    // Unknown / unselected items are reported by the step checks.
                    let Some(item) = selected_item(item_id) else {
                        continue;
                    };
                    let needs_generator = item.generator().is_some();
                    let reset = start.is_some_and(|s| {
                        s.set.iter().any(|x| match x {
                            ScenarioStep::Generator { item_id: o, .. } => {
                                needs_generator && o == item_id
                            }
                            ScenarioStep::Value { item_id: o, .. } => {
                                !needs_generator && o == item_id
                            }
                            ScenarioStep::Fault { .. } => false,
                        })
                    });
                    (!reset).then(|| {
                        format!(
                            "scenario \"{}\" changes item \"{item_id}\", but the start scenario does not reset it (add a {} step)",
                            sc.id,
                            if needs_generator { "generator" } else { "value" }
                        )
                    })
                }
                ScenarioStep::Fault { target, .. } => {
                    let reset = start.is_some_and(|s| {
                        s.set.iter().any(|x| {
                            matches!(x, ScenarioStep::Fault { target: t, active: false } if t == target)
                        })
                    });
                    (!reset).then(|| {
                        format!(
                            "scenario \"{}\" touches a fault, but the start scenario does not clear it (add `fault … active: false`)",
                            sc.id
                        )
                    })
                }
            };
            if let Some(message) = problem {
                if start.is_none() {
                    if !no_start_reported {
                        no_start_reported = true;
                        r.add(
                            file,
                            "/scenarios",
                            "the demo has scenarios but no start scenario that resets them",
                        );
                    }
                    continue;
                }
                r.add(file, sp, message);
            }
        }
    }
}

/// A scenario that sets a value on a Modbus item and puts that item's slave into timeout hides
/// its own effect: the timeout stops the slave's measurements, so the value is never sent.
fn check_scenario_hidden_values<'a>(
    r: &mut Report,
    file: &str,
    demo: &DemoPack,
    selected_item: &dyn Fn(&str) -> Option<&'a LibraryItem>,
) {
    for (i, sc) in demo.scenarios.iter().enumerate() {
        let timed_out: BTreeSet<&str> = sc
            .set
            .iter()
            .filter_map(|s| match s {
                ScenarioStep::Fault {
                    target: FaultTarget::SlaveTimeout { slave_key },
                    active: true,
                } => Some(slave_key.as_str()),
                _ => None,
            })
            .collect();
        if timed_out.is_empty() {
            continue;
        }
        for (j, step) in sc.set.iter().enumerate() {
            if let ScenarioStep::Value { item_id, .. } = step
                && let Some(slave) = selected_item(item_id).and_then(LibraryItem::slave_key)
                && timed_out.contains(slave)
            {
                r.add(
                    file,
                    format!("/scenarios/{i}/set/{j}"),
                    format!(
                        "value of \"{item_id}\" is never sent: the same scenario puts its slave \"{slave}\" into timeout"
                    ),
                );
            }
        }
    }
}

fn fault_problem<'a>(
    lib: &Library,
    target: &FaultTarget,
    lamps: u32,
    used_slaves: &BTreeSet<&str>,
    selected_item: &dyn Fn(&str) -> Option<&'a LibraryItem>,
) -> Option<String> {
    match target {
        FaultTarget::DaliLamp { lamp, alarm_type } => {
            if *lamp == 0 || *lamp > lamps {
                Some(format!("lamp {lamp} outside 1–{lamps} (daliLampCount)"))
            } else if !dali_lamp_alarms(*lamp)
                .iter()
                .any(|a| a.alarm_type == *alarm_type)
            {
                Some(format!(
                    "\"{alarm_type}\" is not a native alarm of lamp {lamp}"
                ))
            } else {
                None
            }
        }
        FaultTarget::DaliBus { alarm_type } => (!dali_bus_alarms()
            .iter()
            .any(|a| a.alarm_type == *alarm_type))
        .then(|| format!("\"{alarm_type}\" is not a bus alarm")),
        FaultTarget::SlaveTimeout { slave_key } => (!used_slaves.contains(slave_key.as_str()))
            .then(|| format!("slave \"{slave_key}\" has no selected items in this demo")),
        FaultTarget::SlaveAlarm {
            slave_key,
            alarm_name,
        } => {
            if !used_slaves.contains(slave_key.as_str()) {
                Some(format!(
                    "slave \"{slave_key}\" has no selected items in this demo"
                ))
            } else if !lib
                .slave(slave_key)
                .is_some_and(|s| s.alarms.iter().any(|a| a.name == *alarm_name))
            {
                Some(format!(
                    "slave \"{slave_key}\" has no alarm \"{alarm_name}\""
                ))
            } else {
                None
            }
        }
        FaultTarget::CustomAlarm { item_id } => match selected_item(item_id) {
            Some(item) if item.kind() == ItemKind::CustomAlarm => None,
            Some(_) => Some(format!("item \"{item_id}\" is not a customAlarm")),
            None => Some(format!("item \"{item_id}\" is not selected by this demo")),
        },
        FaultTarget::Sensor {
            sensor_kind,
            index,
            alarm,
        } => {
            let index = u32::from(*index);
            let wanted = match sensor_kind.as_str() {
                "brightness" => ItemKind::DaliBrightness,
                "motion" => ItemKind::DaliMotion,
                other => return Some(format!("unknown sensor kind \"{other}\"")),
            };
            let selected = lib
                .items
                .iter()
                .filter(|i| i.kind() == wanted && i.index() == Some(index))
                .any(|i| selected_item(i.id()).is_some());
            if !selected {
                Some(format!(
                    "no selected {sensor_kind} sensor {index} in this demo"
                ))
            } else if !crate::payload::alarm::sensor_alarms(sensor_kind, index)
                .is_some_and(|v| v.iter().any(|a| a.alarm_type == *alarm))
            {
                Some(format!(
                    "\"{alarm}\" is not a native alarm of {sensor_kind} {index}"
                ))
            } else {
                None
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Floorplans
// ---------------------------------------------------------------------------------------------

fn check_alarm_refs(
    r: &mut Report,
    file: &str,
    path: &str,
    refs: &[AlarmRef],
    lib: &Library,
    selected: &[&LibraryItem],
    used_slaves: &BTreeSet<&str>,
) {
    for (i, a) in refs.iter().enumerate() {
        let p = format!("{path}/{i}");
        match a {
            AlarmRef::Item { item_id } => {
                if !selected.iter().any(|s| s.id() == item_id) {
                    r.add(
                        file,
                        format!("{p}/itemId"),
                        format!("item \"{item_id}\" is not selected by the demo"),
                    );
                }
            }
            AlarmRef::SlaveTimeout { slave_key } => {
                if !used_slaves.contains(slave_key.as_str()) {
                    r.add(
                        file,
                        format!("{p}/slaveKey"),
                        format!("slave \"{slave_key}\" is not used by the demo"),
                    );
                }
            }
            AlarmRef::SlaveAlarm {
                slave_key,
                alarm_name,
            } => {
                if !used_slaves.contains(slave_key.as_str()) {
                    r.add(
                        file,
                        format!("{p}/slaveKey"),
                        format!("slave \"{slave_key}\" is not used by the demo"),
                    );
                } else if !lib
                    .slave(slave_key)
                    .is_some_and(|s| s.alarms.iter().any(|x| x.name == *alarm_name))
                {
                    r.add(
                        file,
                        format!("{p}/alarmName"),
                        format!("slave \"{slave_key}\" has no alarm \"{alarm_name}\""),
                    );
                }
            }
            AlarmRef::Type { alarm_type } => {
                if !is_valid_alarm_name(alarm_type) {
                    r.add(
                        file,
                        format!("{p}/alarmType"),
                        format!("alarm type \"{alarm_type}\" is invalid"),
                    );
                } else if alarm_type.starts_with("lw_")
                    && !alarm_type.starts_with("lw_demo_")
                    && !is_native_alarm_type(alarm_type)
                {
                    // The plugin matches alarm types exactly: a made-up lw_* type never alarms.
                    r.add(
                        file,
                        format!("{p}/alarmType"),
                        format!(
                            "alarm type \"{alarm_type}\" is neither a firmware alarm type \
                             (e.g. lw_daliBusPowerFail) nor a lw_demo_* type"
                        ),
                    );
                }
            }
        }
    }
}

fn check_floorplan(
    r: &mut Report,
    lib: &Library,
    file: &str,
    fp: &FloorplanLayout,
    demo: &DemoPack,
    selected: &[&LibraryItem],
) {
    if fp.name.trim().is_empty() {
        r.add(file, "/name", "floorplan name is empty");
    }
    let lower = fp.image.to_ascii_lowercase();
    let ext_ok = [".png", ".jpg", ".jpeg", ".svg"]
        .iter()
        .any(|e| lower.ends_with(e));
    let dir = std::path::Path::new(file)
        .parent()
        .unwrap_or(std::path::Path::new(""));
    if !ext_ok || fp.image.contains('/') || fp.image.contains('\\') {
        r.add(
            file,
            "/image",
            "image must be a .png/.jpg/.svg file next to the layout",
        );
    } else if !lib.root.join(dir).join(&fp.image).is_file() {
        r.add(file, "/image", format!("image {} does not exist", fp.image));
    }
    if fp.icon.as_deref().is_some_and(|i| i.trim().is_empty()) {
        r.add(file, "/icon", "icon must not be empty");
    }
    if fp.image_width == 0 || fp.image_height == 0 {
        r.add(file, "/imageWidth", "image size must be > 0");
    }
    if fp.image_opacity.is_some_and(|o| !(0.1..=1.0).contains(&o)) {
        r.add(file, "/imageOpacity", "imageOpacity must be 0.1–1");
    }
    let used_slaves: BTreeSet<&str> = selected.iter().filter_map(|i| i.slave_key()).collect();
    let selected_item = |id: &str| selected.iter().copied().find(|i| i.id() == id);
    let mut ids = BTreeSet::new();
    for (i, w) in fp.widgets.iter().enumerate() {
        let p = format!("/widgets/{i}");
        if !ids.insert(w.id()) {
            r.add(
                file,
                format!("{p}/id"),
                format!("duplicate widget id \"{}\"", w.id()),
            );
        }
        if w.icon().trim().is_empty() {
            r.add(file, format!("{p}/icon"), "icon must not be empty");
        }
        let (x, y) = w.position();
        if !(0.0..=f64::from(fp.image_width)).contains(&x)
            || !(0.0..=f64::from(fp.image_height)).contains(&y)
        {
            r.add(
                file,
                format!("{p}/x"),
                format!("position {x},{y} is outside the image"),
            );
        }
        match w {
            FloorplanWidget::Sensor {
                measurement,
                ranges,
                alarms,
                display,
                decimals,
                stale_after_min,
                ..
            } => {
                if decimals.is_some_and(|d| d > 6) {
                    r.add(file, format!("{p}/decimals"), "decimals must be 0–6");
                }
                if *stale_after_min == Some(0) {
                    r.add(
                        file,
                        format!("{p}/staleAfterMin"),
                        "staleAfterMin must be ≥ 1",
                    );
                }
                match selected_item(&measurement.item_id) {
                    None => r.add(
                        file,
                        format!("{p}/measurement/itemId"),
                        format!(
                            "item \"{}\" is not selected by the demo",
                            measurement.item_id
                        ),
                    ),
                    Some(item) if item.kind() == ItemKind::CustomAlarm => r.add(
                        file,
                        format!("{p}/measurement/itemId"),
                        "custom alarms have no measurement; use an alarm widget",
                    ),
                    Some(item) => {
                        if let Some(series) = &measurement.series {
                            let ok = item.kind() == ItemKind::Dali
                                && DALI_SERIES.iter().any(|(n, _)| n == series);
                            if !ok {
                                r.add(
                                    file,
                                    format!("{p}/measurement/series"),
                                    "series is only selectable for DALI lamps (Brightness, \
                                     Lamp Status, Converter Status, Converter Runtime)",
                                );
                            }
                        }
                    }
                }
                for (j, range) in ranges.iter().enumerate() {
                    if let (Some(min), Some(max)) = (range.min, range.max)
                        && min >= max
                    {
                        r.add(file, format!("{p}/ranges/{j}"), "range min must be < max");
                    }
                }
                for j in overlapping_ranges(ranges) {
                    r.add(
                        file,
                        format!("{p}/ranges/{j}"),
                        "range overlaps another range (the plugin needs non-overlapping ranges)",
                    );
                }
                if let Some(SensorDisplay::Boolean {
                    on_value,
                    off_value,
                    ..
                }) = display
                    && on_value == off_value
                {
                    r.add(
                        file,
                        format!("{p}/display"),
                        "onValue and offValue must differ",
                    );
                }
                check_alarm_refs(
                    r,
                    file,
                    &format!("{p}/alarms"),
                    alarms,
                    lib,
                    selected,
                    &used_slaves,
                );
            }
            FloorplanWidget::Alarm { alarms, .. } => {
                if alarms.is_empty() {
                    r.add(file, format!("{p}/alarms"), "alarm widget needs alarms");
                }
                check_alarm_refs(
                    r,
                    file,
                    &format!("{p}/alarms"),
                    alarms,
                    lib,
                    selected,
                    &used_slaves,
                );
            }
            FloorplanWidget::Control { control, .. } => {
                check_widget_control(
                    r,
                    file,
                    &format!("{p}/control"),
                    control,
                    demo,
                    &selected_item,
                );
            }
        }
    }
}

/// Indexes (into `ranges`) of ranges that start before the previous one (sorted by `min`,
/// `None` = −∞) ends. `min` is inclusive, `max` exclusive, `None` = open.
fn overlapping_ranges(ranges: &[WidgetRange]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..ranges.len()).collect();
    order.sort_by(|&a, &b| {
        let key = |i: usize| ranges[i].min.unwrap_or(f64::NEG_INFINITY);
        key(a).total_cmp(&key(b))
    });
    let mut out = Vec::new();
    let mut furthest = f64::NEG_INFINITY;
    for (n, &i) in order.iter().enumerate() {
        let r = &ranges[i];
        if n > 0 && r.min.unwrap_or(f64::NEG_INFINITY) < furthest {
            out.push(i);
        }
        furthest = furthest.max(r.max.unwrap_or(f64::INFINITY));
    }
    out.sort_unstable();
    out
}

fn check_widget_control<'a>(
    r: &mut Report,
    file: &str,
    p: &str,
    control: &WidgetControl,
    demo: &DemoPack,
    selected_item: &dyn Fn(&str) -> Option<&'a LibraryItem>,
) {
    match control {
        WidgetControl::Dali { group, modes } => {
            if !demo.dali_groups.iter().any(|g| g.id == *group) {
                r.add(
                    file,
                    format!("{p}/group"),
                    format!("group {group} is not a group of the demo"),
                );
            }
            let unique: BTreeSet<_> = modes.iter().map(|m| format!("{m:?}")).collect();
            if modes.len() < 2 || unique.len() != modes.len() {
                r.add(file, format!("{p}/modes"), "at least two different modes");
            }
        }
        WidgetControl::Modbus {
            item_id,
            value_type,
            on_label,
            off_label,
            min,
            max,
            step,
            options,
            ..
        } => {
            let Some(item) = selected_item(item_id) else {
                r.add(
                    file,
                    format!("{p}/itemId"),
                    format!("item \"{item_id}\" is not selected by the demo"),
                );
                return;
            };
            let register_type = item.register_type();
            let writable = matches!(item, LibraryItem::Modbus { writable: true, .. });
            if !writable || register_type == Some(ModbusRegisterType::Input) {
                r.add(
                    file,
                    format!("{p}/itemId"),
                    format!("item \"{item_id}\" is not a writable coil / holding Modbus item"),
                );
                return;
            }
            let coil = register_type == Some(ModbusRegisterType::Coil);
            match value_type {
                ModbusValueType::Boolean => {
                    if on_label.is_none() || off_label.is_none() {
                        r.add(file, p, "boolean control needs onLabel and offLabel");
                    }
                }
                ModbusValueType::Number | ModbusValueType::Multi if coil => {
                    r.add(
                        file,
                        format!("{p}/valueType"),
                        "a coil only takes a boolean control",
                    );
                }
                ModbusValueType::Number => match (min, max, step) {
                    (Some(lo), Some(hi), Some(st))
                        if lo < hi
                            && *st > 0.0
                            && *lo >= 0.0
                            && *hi <= 65_535.0
                            && lo.fract() == 0.0
                            && hi.fract() == 0.0
                            && st.fract() == 0.0 => {}
                    _ => r.add(
                        file,
                        p,
                        "number control needs integers 0 ≤ min < max ≤ 65535 and step ≥ 1 \
                         (the plugin writes whole words)",
                    ),
                },
                ModbusValueType::Multi => {
                    let mut seen = BTreeSet::new();
                    if options.len() < 2 {
                        r.add(
                            file,
                            format!("{p}/options"),
                            "multi control needs ≥ 2 options",
                        );
                    }
                    for (j, o) in options.iter().enumerate() {
                        if o.value > 65_535 || !seen.insert(o.value) {
                            r.add(
                                file,
                                format!("{p}/options/{j}/value"),
                                "option values must be unique 0–65535",
                            );
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::loader;
    use std::path::{Path, PathBuf};

    fn fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library")
    }

    fn fixture() -> Library {
        loader::load(&fixture_root(), "0.0.0").expect("fixture loads")
    }

    fn messages(findings: &[Finding]) -> Vec<String> {
        findings.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn fixture_library_is_valid() {
        let lib = fixture();
        assert!(lib.warnings.is_empty(), "{:#?}", lib.warnings);
        assert_eq!(lib.demos.len(), 1);
        assert_eq!(lib.floorplans.len(), 1);
    }

    /// Mutate the fixture, validate, and expect a finding containing `needle` (with file + path).
    fn expect(mutate: impl FnOnce(&mut Library), needle: &str) {
        let mut lib = fixture();
        mutate(&mut lib);
        let found = messages(&validate(&lib));
        assert!(
            found.iter().any(|m| m.contains(needle)),
            "expected {needle:?} in {found:#?}"
        );
    }

    fn item_mut<'a>(lib: &'a mut Library, id: &str) -> &'a mut LibraryItem {
        lib.items.iter_mut().find(|i| i.id() == id).unwrap()
    }

    fn set_json<T: serde::Serialize + serde::de::DeserializeOwned>(
        v: &mut T,
        f: impl FnOnce(&mut serde_json::Value),
    ) {
        let mut j = serde_json::to_value(&*v).unwrap();
        f(&mut j);
        *v = serde_json::from_value(j).unwrap();
    }

    /// The start scenario is a full reset; a scenario must not hide its own value.
    #[test]
    fn scenario_reset_rules() {
        let step = |j: serde_json::Value| serde_json::from_value::<ScenarioStep>(j).unwrap();
        // "trouble" sets a value the start scenario does not reset.
        expect(
            |l| {
                l.demos[0].scenarios[1].set.push(step(serde_json::json!(
                    { "kind": "value", "itemId": "ac_setpoint", "value": 25 }
                )));
            },
            "/scenarios/1/set/5: scenario \"trouble\" changes item \"ac_setpoint\", but the start scenario does not reset it (add a value step)",
        );
        // An item with a library generator needs a generator step in the start scenario.
        expect(
            |l| {
                l.demos[0].scenarios[1].set.push(step(serde_json::json!(
                    { "kind": "value", "itemId": "freezer_temp", "value": -5 }
                )));
                l.demos[0].scenarios[0].set.retain(|s| {
                    !matches!(s, ScenarioStep::Generator { item_id, .. } if item_id == "freezer_temp")
                });
                // freezer_temp has a generator in the fixture library
                if let LibraryItem::Modbus { generator, .. } = item_mut(l, "freezer_temp") {
                    *generator = Some(GeneratorConfig::RandomWalk {
                        center: -18.0,
                        spread: 1.0,
                        step: None,
                    });
                }
            },
            "does not reset it (add a generator step)",
        );
        // A fault the start scenario does not clear.
        expect(
            |l| {
                l.demos[0].scenarios[0].set.retain(|s| {
                    !matches!(
                        s,
                        ScenarioStep::Fault {
                            target: FaultTarget::DaliBus { .. },
                            ..
                        }
                    )
                });
            },
            "touches a fault, but the start scenario does not clear it",
        );
        // No start scenario at all.
        expect(
            |l| l.demos[0].scenarios[0].start = false,
            "the demo has scenarios but no start scenario that resets them",
        );
        // A value behind a slave timeout.
        expect(
            |l| {
                l.demos[0].scenarios[1].set.push(step(serde_json::json!(
                    { "kind": "value", "itemId": "freezer_temp", "value": -5 }
                )));
            },
            "value of \"freezer_temp\" is never sent: the same scenario puts its slave \"kuehlung\" into timeout",
        );
    }

    #[test]
    fn item_rules() {
        expect(
            |l| set_json(item_mut(l, "rssi"), |j| j["id"] = "x".repeat(20).into()),
            "items/system.json: /items/0/id: id",
        );
        expect(
            |l| set_json(item_mut(l, "rssi"), |j| j["id"] = "board_temp".into()),
            "duplicate item id \"board_temp\"",
        );
        expect(
            |l| {
                set_json(item_mut(l, "freezer_temp"), |j| {
                    j["unit"] = "Grad C!!".into()
                })
            },
            "items/cooling.json: /items/0/unit: unit \"Grad C!!\" is longer than 7",
        );
        expect(
            |l| set_json(item_mut(l, "freezer_temp"), |j| j["slave"] = "nope".into()),
            "/items/0/slave: slave \"nope\" is not defined",
        );
        expect(
            |l| set_json(item_mut(l, "freezer_temp"), |j| j["writable"] = true.into()),
            "input registers cannot be writable",
        );
        expect(
            |l| set_json(item_mut(l, "light-2"), |j| j["index"] = 1.into()),
            "Dali 1 already has an item",
        );
        expect(
            |l| set_json(item_mut(l, "light-2"), |j| j["index"] = 65.into()),
            "/items/1/index: index 65 outside 1–64",
        );
        expect(
            |l| set_json(item_mut(l, "motion-1"), |j| j["index"] = 11.into()),
            "index 11 outside 1–10",
        );
        expect(
            |l| set_json(item_mut(l, "doorOpen"), |j| j["id"] = "19_door".into()),
            "must not start with two digits plus _",
        );
        expect(
            |l| set_json(item_mut(l, "light-1"), |j| j["tags"][0] = "Fixture".into()),
            "/items/0/tags/0",
        );
        expect(
            |l| set_json(item_mut(l, "light-1"), |j| j["unit"] = "%".into()),
            "unit is fixed by the firmware",
        );
        expect(
            |l| {
                set_json(item_mut(l, "lux-1"), |j| {
                    j["generator"]["points"].as_array_mut().unwrap().pop();
                })
            },
            "dailyProfile needs 24 points, has 23",
        );
        expect(
            |l| {
                set_json(item_mut(l, "light-1"), |j| {
                    j["control"]["default"] = 101.into()
                })
            },
            "default 101 outside 0…100",
        );
        expect(
            |l| {
                set_json(item_mut(l, "ac_mode"), |j| {
                    j["control"]["default"] = 7.into()
                })
            },
            "default 7 is not an option value",
        );
        expect(
            |l| {
                set_json(item_mut(l, "hood_fan"), |j| {
                    j["registerType"] = "coil".into();
                    j["control"] = serde_json::json!({"kind": "slider", "min": 0, "max": 1,
                                                      "step": 1, "default": 0});
                })
            },
            "a coil (0/1) needs a switch control",
        );
    }

    #[test]
    fn slave_rules() {
        expect(
            |l| l.slaves[0].name = "x".repeat(20),
            "items/cooling.json: /slaves/0/name: slave name",
        );
        expect(
            |l| l.slaves[1].key = "kuehlung".into(),
            "duplicate slave key",
        );
        expect(
            |l| l.slaves[0].alarms[1].name = "co2Leakage".into(),
            "/slaves/0/alarms/1/name: duplicate alarm name",
        );
        expect(
            |l| l.slaves[0].alarms[0].name = "co2 leak".into(),
            "must be [A-Za-z0-9_-]",
        );
        expect(
            |l| l.slaves[0].request_interval = 0,
            "requestInterval must be 1–99999",
        );
    }

    #[test]
    fn demo_rules() {
        let d = "demos/fixture.json";
        expect(
            |l| l.demos[0].dali_lamp_count = 65,
            &format!("{d}: /daliLampCount"),
        );
        expect(
            |l| l.demos[0].dali_lamp_count = 1,
            "item light-2 uses lamp 2 > daliLampCount 1",
        );
        expect(
            |l| l.demos[0].send_interval_sec = 900,
            &format!("{d}: /sendIntervalSec"),
        );
        expect(
            |l| l.demos[0].includes.push("ghost".into()),
            "/includes/0: item \"ghost\"",
        );
        expect(|l| l.demos[0].excludes[0] = "ghost".into(), "/excludes/0");
        expect(
            |l| l.demos[0].dali_groups[0].converters = vec![1],
            "lamp 2 (item light-2) is in no group",
        );
        expect(
            |l| l.demos[0].dali_groups[1].converters.push(9),
            "/daliGroups/1/converters/2: lamp 9 outside 1–4",
        );
        expect(
            |l| l.demos[0].dali_groups[1].id = 1,
            "/daliGroups/1/id: duplicate group id 1",
        );
        expect(
            |l| l.demos[0].dali_groups[1].slave_of = Some(9),
            "/daliGroups/1/slaveOf",
        );
        expect(
            |l| l.demos[0].dali_groups[0].min_level = 101,
            "/daliGroups/0/maxLevel",
        );
        expect(
            |l| l.demos[0].digital_inputs[1].index = 1,
            "/digitalInputs/1/index: duplicate",
        );
        expect(
            |l| l.demos[0].digital_inputs[1].item_id = "light-1".into(),
            "is not a digitalInput / impulseCounter",
        );
        expect(
            |l| l.demos[0].digital_inputs[0].settings.groups = vec![5],
            "/digitalInputs/0/groups/0: group 5",
        );
        expect(
            |l| l.demos[0].digital_inputs[2].settings.input_type = DigitalInputType::Measurement,
            "/digitalInputs/2/type",
        );
        expect(
            |l| l.demos[0].digital_inputs[1].index = 5,
            "/digitalInputs/1/index: item \"di-door\" has index Some(2), not 5",
        );
        expect(
            |l| l.demos[0].scenarios[1].start = true,
            "/scenarios/1/start",
        );
        expect(
            |l| {
                set_json(&mut l.demos[0].scenarios[0].set[0], |j| {
                    j["value"] = 5.into()
                });
            },
            "/scenarios/0/set/0/value: value 5 outside -30…0",
        );
        expect(
            |l| {
                set_json(&mut l.demos[0].scenarios[0].set[0], |j| {
                    j["itemId"] = "light-64".into()
                });
            },
            "item \"light-64\" is not selected",
        );
        expect(
            |l| {
                set_json(&mut l.demos[0].scenarios[1].set[3], |j| {
                    j["target"]["alarmType"] = "lw_lampFailure_1_03_openCircuit".into();
                });
            },
            "is not a native alarm of lamp 2",
        );
        expect(
            |l| {
                set_json(&mut l.demos[0].scenarios[1].set[1], |j| {
                    j["target"]["alarmName"] = "nope".into();
                });
            },
            "slave \"kuehlung\" has no alarm \"nope\"",
        );
        expect(
            |l| {
                set_json(&mut l.demos[0].scenarios[1].set[4], |j| {
                    j["target"]["alarmType"] = "lw_daliBusPowerUnknown".into();
                });
            },
            "is not a bus alarm",
        );
        expect(
            |l| l.demos[0].floorplans[0] = "floorplans/fixture/missing.json".into(),
            "/floorplans/0: floorplan floorplans/fixture/missing.json is missing",
        );
        expect(
            |l| l.demos[0].tags = vec!["nothing".into()],
            "the demo selects no items",
        );
    }

    #[test]
    fn floorplan_rules() {
        let f = "floorplans/fixture/ground.json";
        fn fp(l: &mut Library) -> &mut FloorplanLayout {
            l.floorplans.values_mut().next().unwrap()
        }
        let widget = |l: &mut Library, i: usize, edit: &dyn Fn(&mut serde_json::Value)| {
            set_json(&mut fp(l).widgets[i], |j| edit(j));
        };
        expect(
            |l| fp(l).image = "nope.png".into(),
            &format!("{f}: /image: image nope.png"),
        );
        expect(
            |l| widget(l, 0, &|j| j["measurement"]["itemId"] = "light-3".into()),
            "/widgets/0/measurement/itemId: item \"light-3\" is not selected",
        );
        expect(
            |l| widget(l, 0, &|j| j["measurement"]["series"] = "Lamp Status".into()),
            "/widgets/0/measurement/series",
        );
        expect(
            |l| widget(l, 1, &|j| j["id"] = "w-freezer".into()),
            "/widgets/1/id: duplicate widget id",
        );
        expect(
            |l| widget(l, 1, &|j| j["x"] = 900.into()),
            "/widgets/1/x: position 900",
        );
        expect(
            |l| widget(l, 3, &|j| j["alarms"][0]["alarmName"] = "nope".into()),
            "/widgets/3/alarms/0/alarmName",
        );
        expect(
            |l| widget(l, 4, &|j| j["control"]["itemId"] = "freezer_temp".into()),
            "is not a writable coil / holding Modbus item",
        );
        expect(
            |l| {
                widget(l, 4, &|j| {
                    j["control"]["valueType"] = "number".into();
                })
            },
            "a coil only takes a boolean control",
        );
        expect(
            |l| widget(l, 7, &|j| j["control"]["group"] = 9.into()),
            "/widgets/7/control/group: group 9",
        );
        expect(
            |l| widget(l, 6, &|j| j["control"]["options"][1]["value"] = 0.into()),
            "/widgets/6/control/options/1/value",
        );
    }

    /// DI items follow the same "must be sent" rule as lamps; the item and the DI entry agree.
    #[test]
    fn digital_input_rules() {
        expect(
            |l| {
                l.demos[0].digital_inputs.remove(1);
            },
            "input 2 (item di-door) has no digitalInputs[] entry",
        );
        expect(
            |l| l.demos[0].digital_inputs[1].settings.is_active = false,
            "has isActive: false, so it never sends",
        );
        expect(
            |l| l.demos[0].digital_inputs[2].settings.unit = Some("kWh".into()),
            "/digitalInputs/2/unit: unit Some(\"kWh\") differs",
        );
        expect(
            |l| l.demos[0].digital_inputs[2].settings.init_count = Some(5),
            "/digitalInputs/2/initCount: initCount 5 differs from the counter start 1000",
        );
        expect(
            |l| l.demos[0].digital_inputs[0].settings.high_level_action = Some("On".into()),
            "/digitalInputs/0/highLevelAction: action \"On\" must be on, off or auto",
        );
        expect(
            |l| l.demos[0].digital_inputs[0].settings.low_level_action = Some("ein".into()),
            "/digitalInputs/0/lowLevelAction",
        );
    }

    #[test]
    fn effect_rules() {
        let effect = |l: &mut Library, i: usize, edit: &dyn Fn(&mut serde_json::Value)| {
            set_json(item_mut(l, "ac_mode"), |j| edit(&mut j["effects"][i]));
        };
        expect(
            |l| effect(l, 1, &|j| j["property"] = "lw_configCore".into()),
            "inventory property \"lw_configCore\" is reserved",
        );
        expect(
            |l| effect(l, 1, &|j| j["property"] = "c8y_Hardware".into()),
            "is reserved",
        );
        expect(
            |l| effect(l, 1, &|j| j["property"] = "lw_demo".into()),
            "is reserved",
        );
        expect(
            |l| effect(l, 1, &|j| j["property"] = "type".into()),
            "is reserved",
        );
        expect(
            |l| effect(l, 0, &|j| j["type"] = "lw_daliGroupChange".into()),
            "is a native firmware event",
        );
        expect(
            |l| effect(l, 0, &|j| j["type"] = "lw_modbus".into()),
            "is a native firmware event",
        );
        expect(
            |l| {
                set_json(item_mut(l, "ac_mode"), |j| {
                    j["effects"]
                        .as_array_mut()
                        .unwrap()
                        .push(serde_json::json!({
                            "kind": "alarm", "type": "lw_powerFailure", "severity": "MAJOR",
                            "text": "x", "threshold": 1
                        }))
                })
            },
            "is a native firmware alarm",
        );
    }

    #[test]
    fn generator_and_control_must_fit() {
        let generator = |id: &'static str, g: serde_json::Value| {
            move |l: &mut Library| set_json(item_mut(l, id), |j| j["generator"] = g.clone())
        };
        let walk =
            |c: f64, s: f64| serde_json::json!({"kind": "randomWalk", "center": c, "spread": s});
        expect(
            generator("motion-1", walk(0.5, 0.2)),
            "would send fractions to a switch, select, counter, digital input or motion value",
        );
        expect(
            generator("di-door", walk(0.5, 0.2)),
            "/generator: randomWalk / sine / dailyProfile would send fractions",
        );
        expect(
            generator("gas-meter", walk(5.0, 1.0)),
            "would send fractions",
        );
        expect(
            generator(
                "freezer_temp",
                serde_json::json!({"kind": "counter", "ratePerHour": 1}),
            ),
            "a counter generator needs a counter control",
        );
        expect(
            generator(
                "light-1",
                serde_json::json!({"kind": "counter", "ratePerHour": 1}),
            ),
            "a counter generator needs a counter control",
        );
        // Band outside the slider range.
        expect(
            generator("light-1", walk(90.0, 20.0)),
            "randomWalk band 70…110 leaves the control range 0…100",
        );
        expect(
            generator(
                "light-1",
                serde_json::json!({"kind": "sine", "period": 60, "amplitude": 30}),
            ),
            "sine range 50…110 leaves the control range 0…100",
        );
        // Scenario generator steps are checked against the item as well.
        expect(
            |l| {
                set_json(&mut l.demos[0].scenarios[0].set[1], |j| {
                    j["generator"] = serde_json::json!({"kind": "randomWalk", "center": -18,
                                                         "spread": 20});
                })
            },
            "/scenarios/0/set/1/generator: randomWalk band -38…2 leaves the control range -30…0",
        );
    }

    #[test]
    fn writable_holding_registers_are_one_word() {
        expect(
            |l| {
                set_json(item_mut(l, "ac_setpoint"), |j| {
                    j["datatype"] = "float32".into()
                })
            },
            "/datatype: a writable holding register is written as one 16-bit word",
        );
        expect(
            |l| {
                set_json(item_mut(l, "ac_setpoint"), |j| {
                    j["datatype"] = "uint32".into()
                })
            },
            "one 16-bit word",
        );
        expect(
            |l| {
                set_json(item_mut(l, "ac_setpoint"), |j| {
                    j["control"]["step"] = 0.5.into()
                })
            },
            "integer min / max in 0-65535 and an integer step",
        );
        expect(
            |l| {
                set_json(item_mut(l, "ac_setpoint"), |j| {
                    j["control"] = serde_json::json!({"kind": "slider", "min": -30, "max": 0,
                                                      "step": 1, "default": -18});
                })
            },
            "integer min / max in 0-65535",
        );
        expect(
            |l| {
                set_json(item_mut(l, "ac_mode"), |j| {
                    j["control"]["options"][1]["value"] = 1.5.into()
                })
            },
            "/control/options/1/value: option values of a writable holding register",
        );
    }

    /// Counters never go backwards: a scenario value below the start is rejected.
    #[test]
    fn counter_scenario_values_must_not_decrease() {
        expect(
            |l| {
                set_json(&mut l.demos[0].scenarios[0].set[0], |j| {
                    j["itemId"] = "gas-meter".into();
                    j["value"] = 5.into();
                })
            },
            "counter value 5 is below its start 1000",
        );
    }

    #[test]
    fn firmware_lengths_count_utf8_bytes() {
        // 18 chars, 20 bytes.
        expect(
            |l| l.slaves[0].name = "Kühlraum Tür links".into(),
            "/slaves/0/name: slave name \"Kühlraum Tür links\" must be 1–19 bytes (20 in UTF-8)",
        );
        expect(
            |l| set_json(item_mut(l, "freezer_temp"), |j| j["unit"] = "°°°°".into()),
            "is longer than 7 bytes",
        );
        expect(
            |l| {
                l.demos[0].digital_inputs[2].settings.unit = Some("°".repeat(10));
            },
            "unit is longer than 19 bytes",
        );
        let mut lib = fixture();
        lib.slaves[0].name = "Kühlraum Tür li".into(); // 17 chars, 19 bytes
        assert!(validate(&lib).is_empty());
    }

    #[test]
    fn floorplan_value_rules() {
        fn fp(l: &mut Library) -> &mut FloorplanLayout {
            l.floorplans.values_mut().next().unwrap()
        }
        let widget = |l: &mut Library, i: usize, edit: &dyn Fn(&mut serde_json::Value)| {
            set_json(&mut fp(l).widgets[i], |j| edit(j));
        };
        expect(
            |l| widget(l, 0, &|j| j["ranges"][1]["min"] = (-16).into()),
            "/widgets/0/ranges/1: range overlaps another range",
        );
        expect(
            |l| {
                widget(l, 0, &|j| {
                    j["ranges"] = serde_json::json!([
                        {"min": 0, "max": 100, "level": "good"},
                        {"min": 10, "max": 20, "level": "warning"},
                        {"min": 30, "max": 40, "level": "alarm"}
                    ])
                })
            },
            "/widgets/0/ranges/2: range overlaps",
        );
        expect(
            |l| {
                widget(l, 3, &|j| {
                    j["alarms"][1]["alarmType"] = "lw_daliBusPowerFailure".into()
                })
            },
            "/widgets/3/alarms/1/alarmType: alarm type \"lw_daliBusPowerFailure\" is neither",
        );
        expect(
            |l| widget(l, 0, &|j| j["decimals"] = 7.into()),
            "decimals must be 0–6",
        );
        expect(
            |l| widget(l, 0, &|j| j["staleAfterMin"] = 0.into()),
            "staleAfterMin must be ≥ 1",
        );
        expect(
            |l| widget(l, 0, &|j| j["icon"] = "".into()),
            "/widgets/0/icon: icon must not be empty",
        );
        expect(
            |l| fp(l).icon = Some(String::new()),
            "/icon: icon must not be empty",
        );
        expect(
            |l| widget(l, 5, &|j| j["control"]["step"] = 0.5.into()),
            "number control needs integers",
        );
        expect(
            |l| l.demos[0].scenarios[1].id = "bad id".into(),
            "/scenarios/1/id: scenario id \"bad id\"",
        );
        // lw_demo_* and real firmware types stay valid.
        let mut lib = fixture();
        set_json(&mut fp(&mut lib).widgets[3], |j| {
            j["alarms"][1]["alarmType"] = "lw_demo_custom".into()
        });
        assert!(validate(&lib).is_empty(), "{:#?}", validate(&lib));
    }

    #[test]
    fn custom_setup_rules() {
        let lib = fixture();
        let items: Vec<LibraryItem> = serde_json::from_value(serde_json::json!([
            {"kind": "modbus", "id": "freezer_temp", "slave": "kuehlung",
             "title": {"de": "x", "en": "x"}},
            {"kind": "modbus", "id": "new_reg", "slave": "custom",
             "title": {"de": "x", "en": "x"}},
            {"kind": "modbus", "id": "orphan", "slave": "ghost",
             "title": {"de": "x", "en": "x"}}
        ]))
        .unwrap();
        let slaves: Vec<SlaveDefinition> = serde_json::from_value(serde_json::json!([
            {"key": "custom", "name": "Eigen", "requestInterval": 30},
            {"key": "klima", "name": "Doppelt", "requestInterval": 30}
        ]))
        .unwrap();
        let found = messages(&validate_custom(&lib, &slaves, &items));
        assert_eq!(found.len(), 3, "{found:#?}");
        assert!(found[0].contains("/customSlaves/1/key"));
        assert!(found[1].contains("/customItems/0/id: item id \"freezer_temp\" is already used"));
        assert!(found[2].contains("/customItems/2/slave"));
    }
}
