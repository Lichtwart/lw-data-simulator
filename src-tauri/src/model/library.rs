//! Library file format (PRD §11) and library summary types.
//!
//! These structs are the serde mirror of `library/items/*.json` and
//! `library/demos/*.json`. The JSON schemas in `library/schema/` are the
//! normative spec for CI; keep both in sync (schema change ⇒ model change ⇒
//! `npm run gen:types`). Unknown fields are rejected so typos fail loudly.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use specta::Type;

/// German + English text. Both are required in library files.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
pub struct LocalizedText {
    pub de: String,
    pub en: String,
}

/// Cumulocity alarm severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Severity {
    Critical,
    Major,
    Minor,
    Warning,
}

/// What an item simulates; decides the native naming (PRD §7, §11.3).
/// Discriminator of [`LibraryItem`] (use [`LibraryItem::kind`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ItemKind {
    /// DALI lamp (`LW Dali Control Gear 1-NN`), uses `index` 1–64.
    Dali,
    /// DALI brightness sensor (`LW Dali Brightness.Lux<N>`, unit `Lux`), uses `index` 1–10.
    DaliBrightness,
    /// DALI motion sensor (`LW Motion.State<N>`, no unit), uses `index` 1–10.
    DaliMotion,
    /// Digital input value (`LW Digital Input.Value<NN>`, D1), uses `index` 1–10.
    DigitalInput,
    /// Digital input impulse counter (`LW Digital Input.ImpulseCounter<NN>`, D1), uses `index` 1–10.
    ImpulseCounter,
    /// System measurement (`c8y_SignalStrength.rssi` / `c8y_TemperatureMeasurement.T`).
    System,
    /// Modbus register on a slave; uses `slave`.
    Modbus,
    /// Device-level custom alarm `lw_demo_<name>` (D2); no measurement.
    CustomAlarm,
}

/// Which system measurement a `system` item simulates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SystemKind {
    /// `c8y_SignalStrength.rssi`, unit `dBm`.
    Rssi,
    /// `c8y_TemperatureMeasurement.T`, unit `C`.
    Temperature,
}

/// Modbus register type (`lw_configModbusSlaves[].registers[].type`).
/// `discrete` exists in the firmware but is not simulated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ModbusRegisterType {
    Coil,
    Holding,
    Input,
}

impl ModbusRegisterType {
    /// `type` of `writeRegisters[]` entries (floorplan plugin): `coil` | `holding`.
    /// `None` for `input` (read-only).
    pub fn write_register_type(self) -> Option<&'static str> {
        match self {
            Self::Coil => Some("coil"),
            Self::Holding => Some("holding"),
            Self::Input => None,
        }
    }

    /// `type` of a `lw_modbusWrite` operation: `coil` (FC5) | `register` (FC6).
    /// `None` for `input` (not writable).
    pub fn write_operation_type(self) -> Option<&'static str> {
        match self {
            Self::Coil => Some("coil"),
            Self::Holding => Some("register"),
            Self::Input => None,
        }
    }
}

/// Modbus register data type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ModbusDatatype {
    Int16,
    Uint16,
    Int32,
    Uint32,
    Float32,
}

/// One option of a `select` control.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectOption {
    #[specta(type = specta_typescript::Number)]
    pub value: f64,
    pub label: LocalizedText,
}

/// UI control of an item (PRD §8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Control {
    Slider {
        #[specta(type = specta_typescript::Number)]
        min: f64,
        #[specta(type = specta_typescript::Number)]
        max: f64,
        #[specta(type = specta_typescript::Number)]
        step: f64,
        #[specta(type = specta_typescript::Number)]
        default: f64,
    },
    Switch {
        #[specta(type = specta_typescript::Number)]
        default: f64,
    },
    Select {
        options: Vec<SelectOption>,
        #[specta(type = specta_typescript::Number)]
        default: f64,
    },
    Counter {
        #[specta(type = specta_typescript::Number)]
        default: f64,
    },
}

/// Value generator running on top of a control (PRD §8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum GeneratorConfig {
    RandomWalk {
        #[specta(type = specta_typescript::Number)]
        center: f64,
        #[specta(type = specta_typescript::Number)]
        spread: f64,
        #[specta(type = Option<specta_typescript::Number>)]
        step: Option<f64>,
    },
    Sine {
        /// Period in seconds.
        #[specta(type = specta_typescript::Number)]
        period: f64,
        #[specta(type = specta_typescript::Number)]
        amplitude: f64,
        /// Center line; defaults to the control default.
        #[specta(type = Option<specta_typescript::Number>)]
        center: Option<f64>,
    },
    DailyProfile {
        /// Exactly 24 hourly points (local time), linearly interpolated.
        #[specta(type = Vec<specta_typescript::Number>)]
        points: Vec<f64>,
    },
    Counter {
        #[serde(rename = "ratePerHour")]
        #[specta(type = specta_typescript::Number)]
        rate_per_hour: f64,
    },
}

/// Additional output of a multi-effect item (PRD §8). The primary measurement
/// is implied by `kind`; effects add events / inventory properties / alarms.
/// Shape mirrored in `library/schema/common.schema.json#/$defs/effect`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ItemEffect {
    /// Send an event on every value change. `text` may contain `{value}` / `{label}`.
    Event {
        #[serde(rename = "type")]
        event_type: String,
        text: String,
    },
    /// Write the value (or select label) to a device inventory property.
    Inventory { property: String },
    /// Raise an alarm while the value is ≥ `threshold` (cleared below it).
    Alarm {
        #[serde(rename = "type")]
        alarm_type: String,
        severity: Severity,
        text: String,
        #[specta(type = specta_typescript::Number)]
        threshold: f64,
    },
}

/// One reusable item (PRD §11.3). Tagged by `kind`, so every kind carries exactly the fields
/// it needs (a Modbus item without `slave` or a custom alarm without `severity` does not
/// parse). Unknown fields are rejected.
///
/// Common to all kinds: `id` (globally unique, `[A-Za-z0-9_-]`, ≤ 19 chars; for Modbus items
/// it becomes the register name), `title` (de + en), `tags`. Use the accessors below instead of
/// matching all variants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum LibraryItem {
    /// DALI lamp `LW Dali Control Gear 1-NN`; the item value is its `Brightness`.
    Dali {
        /// Globally unique, `[A-Za-z0-9_-]`, ≤ 19 chars.
        id: String,
        /// DALI lamp 1–64.
        index: u32,

        title: LocalizedText,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        control: Option<Control>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        generator: Option<GeneratorConfig>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        effects: Vec<ItemEffect>,
    },
    /// DALI brightness sensor `LW Dali Brightness.Lux<N>` (unit `Lux`).
    DaliBrightness {
        /// Globally unique, `[A-Za-z0-9_-]`, ≤ 19 chars.
        id: String,
        /// Sensor / input number: DI 1–10, brightness / motion sensor 1–10.
        index: u32,

        title: LocalizedText,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        control: Option<Control>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        generator: Option<GeneratorConfig>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        effects: Vec<ItemEffect>,
    },
    /// DALI motion sensor `LW Motion.State<N>` (no unit).
    DaliMotion {
        /// Globally unique, `[A-Za-z0-9_-]`, ≤ 19 chars.
        id: String,
        /// Sensor / input number: DI 1–10, brightness / motion sensor 1–10.
        index: u32,

        title: LocalizedText,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        control: Option<Control>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        generator: Option<GeneratorConfig>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        effects: Vec<ItemEffect>,
    },
    /// Digital input value `LW Digital Input.Value<NN>` (D1).
    DigitalInput {
        /// Globally unique, `[A-Za-z0-9_-]`, ≤ 19 chars.
        id: String,
        /// Sensor / input number: DI 1–10, brightness / motion sensor 1–10.
        index: u32,
        /// For digital inputs the German title is written to `lw_configDigitalInputs[].title` (D1).
        title: LocalizedText,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        control: Option<Control>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        generator: Option<GeneratorConfig>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        effects: Vec<ItemEffect>,
    },
    /// Digital input impulse counter `LW Digital Input.ImpulseCounter<NN>` (D1); `unit` is the counter unit (≤ 19 bytes).
    ImpulseCounter {
        /// Globally unique, `[A-Za-z0-9_-]`, ≤ 19 chars.
        id: String,
        /// Sensor / input number: DI 1–10, brightness / motion sensor 1–10.
        index: u32,
        /// For digital inputs the German title is written to `lw_configDigitalInputs[].title` (D1).
        title: LocalizedText,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        control: Option<Control>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        generator: Option<GeneratorConfig>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        effects: Vec<ItemEffect>,
    },
    /// System measurement: signal strength or board temperature.
    System {
        id: String,
        /// Decides fragment / series / unit (see `payload::naming`).
        system: SystemKind,
        title: LocalizedText,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        control: Option<Control>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        generator: Option<GeneratorConfig>,
    },
    /// Modbus register on a slave. Register name = `id`.
    #[serde(rename_all = "camelCase")]
    Modbus {
        id: String,
        /// Key of the owning [`SlaveDefinition`].
        slave: String,
        title: LocalizedText,
        #[serde(default)]
        tags: Vec<String>,
        /// Register type; default `holding`, `coil` for a `switch` control
        /// (see [`LibraryItem::register_type`]).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        register_type: Option<ModbusRegisterType>,
        /// Unit, ≤ 7 UTF-8 bytes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        datatype: Option<ModbusDatatype>,
        /// Writable items go into `registers[]` **and** `writeRegisters[]` (only `coil` / `holding`).
        #[serde(default)]
        writable: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        control: Option<Control>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        generator: Option<GeneratorConfig>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        effects: Vec<ItemEffect>,
    },
    /// Device-level custom alarm (D2): type `lw_demo_<id>`, not bound to a register or slave.
    /// The id is the alarm name; it must not start with two digits plus `_`
    /// (`payload::naming::is_valid_device_alarm_name`).
    CustomAlarm {
        id: String,
        title: LocalizedText,
        #[serde(default)]
        tags: Vec<String>,
        severity: Severity,
        /// Alarm text sent to Cumulocity.
        text: String,
    },
}

impl LibraryItem {
    /// The item's effects (`system` and `customAlarm` items have none).
    pub fn effects(&self) -> &[ItemEffect] {
        match self {
            Self::Dali { effects, .. }
            | Self::DaliBrightness { effects, .. }
            | Self::DaliMotion { effects, .. }
            | Self::DigitalInput { effects, .. }
            | Self::ImpulseCounter { effects, .. }
            | Self::Modbus { effects, .. } => effects,
            Self::System { .. } | Self::CustomAlarm { .. } => &[],
        }
    }

    pub fn kind(&self) -> ItemKind {
        match self {
            Self::Dali { .. } => ItemKind::Dali,
            Self::DaliBrightness { .. } => ItemKind::DaliBrightness,
            Self::DaliMotion { .. } => ItemKind::DaliMotion,
            Self::DigitalInput { .. } => ItemKind::DigitalInput,
            Self::ImpulseCounter { .. } => ItemKind::ImpulseCounter,
            Self::System { .. } => ItemKind::System,
            Self::Modbus { .. } => ItemKind::Modbus,
            Self::CustomAlarm { .. } => ItemKind::CustomAlarm,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Dali { id, .. }
            | Self::DaliBrightness { id, .. }
            | Self::DaliMotion { id, .. }
            | Self::DigitalInput { id, .. }
            | Self::ImpulseCounter { id, .. }
            | Self::System { id, .. }
            | Self::Modbus { id, .. }
            | Self::CustomAlarm { id, .. } => id,
        }
    }

    pub fn title(&self) -> &LocalizedText {
        match self {
            Self::Dali { title, .. }
            | Self::DaliBrightness { title, .. }
            | Self::DaliMotion { title, .. }
            | Self::DigitalInput { title, .. }
            | Self::ImpulseCounter { title, .. }
            | Self::System { title, .. }
            | Self::Modbus { title, .. }
            | Self::CustomAlarm { title, .. } => title,
        }
    }

    pub fn tags(&self) -> &[String] {
        match self {
            Self::Dali { tags, .. }
            | Self::DaliBrightness { tags, .. }
            | Self::DaliMotion { tags, .. }
            | Self::DigitalInput { tags, .. }
            | Self::ImpulseCounter { tags, .. }
            | Self::System { tags, .. }
            | Self::Modbus { tags, .. }
            | Self::CustomAlarm { tags, .. } => tags,
        }
    }

    /// Slave key (only `modbus`).
    pub fn slave_key(&self) -> Option<&str> {
        match self {
            Self::Modbus { slave, .. } => Some(slave),
            _ => None,
        }
    }

    /// Native index (only the native kinds).
    pub fn index(&self) -> Option<u32> {
        match self {
            Self::Dali { index, .. }
            | Self::DaliBrightness { index, .. }
            | Self::DaliMotion { index, .. }
            | Self::DigitalInput { index, .. }
            | Self::ImpulseCounter { index, .. } => Some(*index),
            _ => None,
        }
    }

    /// Unit of the item's primary series, if the item defines one.
    pub fn unit(&self) -> Option<&str> {
        match self {
            Self::Dali { unit, .. }
            | Self::DaliBrightness { unit, .. }
            | Self::DaliMotion { unit, .. }
            | Self::DigitalInput { unit, .. }
            | Self::ImpulseCounter { unit, .. }
            | Self::Modbus { unit, .. } => unit.as_deref(),
            Self::System { .. } | Self::CustomAlarm { .. } => None,
        }
    }

    pub fn control(&self) -> Option<&Control> {
        match self {
            Self::Dali { control, .. }
            | Self::DaliBrightness { control, .. }
            | Self::DaliMotion { control, .. }
            | Self::DigitalInput { control, .. }
            | Self::ImpulseCounter { control, .. }
            | Self::System { control, .. }
            | Self::Modbus { control, .. } => control.as_ref(),
            Self::CustomAlarm { .. } => None,
        }
    }

    pub fn generator(&self) -> Option<&GeneratorConfig> {
        match self {
            Self::Dali { generator, .. }
            | Self::DaliBrightness { generator, .. }
            | Self::DaliMotion { generator, .. }
            | Self::DigitalInput { generator, .. }
            | Self::ImpulseCounter { generator, .. }
            | Self::System { generator, .. }
            | Self::Modbus { generator, .. } => generator.as_ref(),
            Self::CustomAlarm { .. } => None,
        }
    }

    /// Register type of a Modbus item: explicit `registerType`, else `coil` for a `switch`
    /// control, else `holding`. `None` for other kinds.
    pub fn register_type(&self) -> Option<ModbusRegisterType> {
        match self {
            Self::Modbus {
                register_type,
                control,
                ..
            } => Some(match (register_type, control) {
                (Some(t), _) => *t,
                (None, Some(Control::Switch { .. })) => ModbusRegisterType::Coil,
                (None, _) => ModbusRegisterType::Holding,
            }),
            _ => None,
        }
    }
}

/// A custom alarm of a slave (`lw_demo_<id:02>_<name>`, D2). A slave carries any number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SlaveAlarmDefinition {
    /// `[A-Za-z0-9_-]`; used in the alarm type and as the fault target.
    pub name: String,
    pub text: String,
    pub severity: Severity,
}

/// A Modbus slave = one functional group (PRD §7.4, §11.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SlaveDefinition {
    pub key: String,
    /// ≤ 19 UTF-8 bytes (firmware buffer).
    pub name: String,
    /// Request interval in seconds (written to `lw_configModbusSlaves`).
    pub request_interval: u32,
    /// Custom alarms of this slave (D2).
    #[serde(default)]
    pub alarms: Vec<SlaveAlarmDefinition>,
}

/// `library/items/*.json` (`"type": "lw-demo-items"`).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ItemLibraryFile {
    #[serde(rename = "type")]
    pub file_type: String,
    pub schema_version: u32,
    #[serde(default)]
    pub slaves: Vec<SlaveDefinition>,
    pub items: Vec<LibraryItem>,
}

/// DALI group (→ `lw_configGroups`, firmware-core-v2.md §5.3). Used in demo packs and
/// as the device-side group entry edited by `set_dali_groups`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DaliGroup {
    /// 1–16.
    pub id: u32,
    pub name: String,
    /// Lamp (converter) numbers 1–64. A lamp is only measured if it is in a group.
    pub converters: Vec<u32>,
    /// 0–100 % (firmware integer).
    pub min_level: u32,
    /// 0–100 %; the level `lw_switchGroupMode` `on` switches to.
    pub max_level: u32,
    /// Lux threshold 0–200000 (firmware default 4500 when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lux_threshold: Option<u32>,
    /// Id of the master group (`slaveOf`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slave_of: Option<u32>,
    #[serde(default)]
    pub dynamic_control: bool,
    #[serde(default)]
    pub automatic_switch: bool,
}

/// Digital input mode (`lw_configDigitalInputs[].type`; `none` is rejected by the firmware).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum DigitalInputType {
    /// Switches DALI groups (`highLevelAction` / `lowLevelAction` / `groups`); sends `Value<NN>`.
    DaliAction,
    /// Counts pulses; sends `ImpulseCounter<NN>` (`initCount`, `increment`, `unit`).
    ImpulseCounter,
    /// Plain input; sends `Value<NN>`.
    Measurement,
}

/// Settings of one digital input that map 1:1 to `lw_configDigitalInputs[]`
/// (firmware-core-v2.md §5.4). Which fields apply depends on `type`:
/// `daliAction` → `highLevelAction`, `lowLevelAction`, `groups`;
/// `impulseCounter` → `initCount`, `increment`, `unit`; `measurement` → none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DigitalInputSettings {
    #[serde(rename = "type")]
    pub input_type: DigitalInputType,
    /// Only active inputs are sent by the firmware. Default `true`.
    #[serde(default = "default_true")]
    pub is_active: bool,
    /// `impulseCounter`: start value, 0–4294967295. Default 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub init_count: Option<u32>,
    /// `impulseCounter`: increment per pulse, 1–65535. Default 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub increment: Option<u32>,
    /// `impulseCounter`: unit, ≤ 19 bytes (the firmware truncates longer values).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// `daliAction`: action on high level: `on` | `off` | `auto` (lowercase; the firmware matches
    /// with a lowercase `strstr`, anything else fails `lw_fetchDigitalInputs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub high_level_action: Option<String>,
    /// `daliAction`: action on low level: `on` | `off` | `auto`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low_level_action: Option<String>,
    /// `daliAction`: group ids 1–16 the action applies to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<u32>,
}

fn default_true() -> bool {
    true
}

/// Digital input in a demo pack. The title is **not** stored here: the entry references
/// the `digitalInput` / `impulseCounter` item, whose German title is written to
/// `lw_configDigitalInputs[].title` (D1, single source of truth).
///
/// Deserialization is hand-written ([`DigitalInputConfigRaw`]) because `#[serde(flatten)]`
/// would disable `deny_unknown_fields`: a typo like `"groupz"` is an error, as everywhere else.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DigitalInputConfig {
    /// 1–10 (the firmware `id`). Optional in the file: omitted (0 after parsing) it is taken
    /// from the item's `index` when the library is loaded.
    pub index: u32,
    /// Id of the item with this `index` (`digitalInput` or `impulseCounter`).
    pub item_id: String,
    #[serde(flatten)]
    pub settings: DigitalInputSettings,
}

/// Digital input as stored on the device (`lw_configDigitalInputs[]`, incl. the extra
/// `title` field of D1). Read / edited via `get_device_config` / `set_digital_inputs`.
///
/// Deserialization is hand-written (see [`DigitalInputConfig`]): unknown fields are errors.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DigitalInputEntry {
    /// 1–10 (the firmware `id`).
    pub index: u32,
    /// Display title (D1); the floorplan widget label falls back to the input number if empty.
    pub title: String,
    #[serde(flatten)]
    pub settings: DigitalInputSettings,
}

/// Strict mirror of [`DigitalInputSettings`] fields + the `index` / `itemId` of a demo entry.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DigitalInputConfigRaw {
    #[serde(default)]
    index: u32,
    item_id: String,
    #[serde(rename = "type")]
    input_type: DigitalInputType,
    #[serde(default = "default_true")]
    is_active: bool,
    #[serde(default)]
    init_count: Option<u32>,
    #[serde(default)]
    increment: Option<u32>,
    #[serde(default)]
    unit: Option<String>,
    #[serde(default)]
    high_level_action: Option<String>,
    #[serde(default)]
    low_level_action: Option<String>,
    #[serde(default)]
    groups: Vec<u32>,
}

/// Strict mirror of [`DigitalInputSettings`] fields + the `index` / `title` of a device entry.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DigitalInputEntryRaw {
    index: u32,
    title: String,
    #[serde(rename = "type")]
    input_type: DigitalInputType,
    #[serde(default = "default_true")]
    is_active: bool,
    #[serde(default)]
    init_count: Option<u32>,
    #[serde(default)]
    increment: Option<u32>,
    #[serde(default)]
    unit: Option<String>,
    #[serde(default)]
    high_level_action: Option<String>,
    #[serde(default)]
    low_level_action: Option<String>,
    #[serde(default)]
    groups: Vec<u32>,
}

impl<'de> Deserialize<'de> for DigitalInputConfig {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let r = DigitalInputConfigRaw::deserialize(d)?;
        Ok(Self {
            index: r.index,
            item_id: r.item_id,
            settings: DigitalInputSettings {
                input_type: r.input_type,
                is_active: r.is_active,
                init_count: r.init_count,
                increment: r.increment,
                unit: r.unit,
                high_level_action: r.high_level_action,
                low_level_action: r.low_level_action,
                groups: r.groups,
            },
        })
    }
}

impl<'de> Deserialize<'de> for DigitalInputEntry {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let r = DigitalInputEntryRaw::deserialize(d)?;
        Ok(Self {
            index: r.index,
            title: r.title,
            settings: DigitalInputSettings {
                input_type: r.input_type,
                is_active: r.is_active,
                init_count: r.init_count,
                increment: r.increment,
                unit: r.unit,
                high_level_action: r.high_level_action,
                low_level_action: r.low_level_action,
                groups: r.groups,
            },
        })
    }
}

/// One step of a scenario (PRD §8): set a value, set a generator, or raise / clear a fault.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ScenarioStep {
    #[serde(rename_all = "camelCase")]
    Value {
        item_id: String,
        #[specta(type = specta_typescript::Number)]
        value: f64,
    },
    #[serde(rename_all = "camelCase")]
    Generator {
        item_id: String,
        generator: GeneratorConfig,
    },
    /// `true` raises, `false` clears.
    Fault {
        target: super::session::FaultTarget,
        active: bool,
    },
}

/// Named preset (PRD §8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scenario {
    pub id: String,
    pub name: LocalizedText,
    /// Applied automatically on session start.
    #[serde(default)]
    pub start: bool,
    pub set: Vec<ScenarioStep>,
}

/// `library/demos/*.json` (`"type": "lw-demo"`, PRD §11.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DemoPack {
    #[serde(rename = "type")]
    pub file_type: String,
    pub schema_version: u32,
    pub id: String,
    pub name: LocalizedText,
    pub description: LocalizedText,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub includes: Vec<String>,
    #[serde(default)]
    pub excludes: Vec<String>,
    /// 1–64. Optional in the file: omitted (0 after parsing) it is derived when the library is
    /// loaded as the highest converter number in `daliGroups` or selected lamp index.
    #[serde(default)]
    pub dali_lamp_count: u32,
    #[serde(default)]
    pub dali_groups: Vec<DaliGroup>,
    #[serde(default)]
    pub digital_inputs: Vec<DigitalInputConfig>,
    pub send_interval_sec: u32,
    #[serde(default)]
    pub scenarios: Vec<Scenario>,
    /// Paths relative to `library/`, e.g. `floorplans/gas-station/ground-floor.json`.
    #[serde(default)]
    pub floorplans: Vec<String>,
}

// ---------------------------------------------------------------------------------------------
// Floorplan layout files (`library/floorplans/<demoId>/<name>.json`, `"type": "lw-demo-floorplan"`)
// ---------------------------------------------------------------------------------------------
//
// Mirror of the plugin's `lw_floorplan` (docs/research/floorplan-plugin.md §1, §3) with **item
// references instead of Cumulocity names**: measurements, alarm types and Modbus targets are
// resolved at apply time (`payload::floorplan`). Library-internal, not part of IPC.

/// Widget state level (`ranges[].level`, boolean display `onLevel` / `offLevel`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WidgetLevel {
    Good,
    Warning,
    Alarm,
}

/// Value range of a sensor widget: `min` inclusive, `max` exclusive, `None` = open.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WidgetRange {
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    pub level: WidgetLevel,
}

/// `display.stateIcons` of a boolean sensor display.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StateIcons {
    pub on: String,
    pub off: String,
}

/// `display` of a sensor widget (copied 1:1 into the plugin widget).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum SensorDisplay {
    Number {},
    #[serde(rename_all = "camelCase")]
    Boolean {
        on_value: f64,
        off_value: f64,
        on_label: String,
        off_label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on_level: Option<WidgetLevel>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        off_level: Option<WidgetLevel>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state_icons: Option<StateIcons>,
    },
}

/// The measured item of a sensor widget. Resolves to `measurement {fragment, series}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WidgetMeasurement {
    pub item_id: String,
    /// Only for `dali` items: one of `Brightness` (default), `Lamp Status`, `Converter Status`,
    /// `Converter Runtime`. Other kinds have exactly one series.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
}

/// An alarm reference of a widget; resolves to one or more literal alarm types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum AlarmRef {
    /// All alarms that belong to an item: DALI lamp → every native lamp alarm; brightness /
    /// motion sensor → its native alarms; Modbus item → its slave's timeout alarm and **all**
    /// the slave's custom alarms; `customAlarm` → `lw_demo_<id>`; DI / system → none.
    #[serde(rename_all = "camelCase")]
    Item { item_id: String },
    /// `lw_modbus_<id:02>_timeoutError` of a slave.
    #[serde(rename_all = "camelCase")]
    SlaveTimeout { slave_key: String },
    /// One custom alarm of a slave: `lw_demo_<id:02>_<alarmName>`.
    #[serde(rename_all = "camelCase")]
    SlaveAlarm {
        slave_key: String,
        alarm_name: String,
    },
    /// A literal alarm type, e.g. `lw_daliBusPowerFail`, `lw_powerFailure`.
    #[serde(rename_all = "camelCase")]
    Type { alarm_type: String },
}

/// `valueType` of a Modbus control widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModbusValueType {
    Boolean,
    Number,
    Multi,
}

/// One option of a `multi` Modbus control.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModbusControlOption {
    /// Raw register value 0–65535, unique within the control.
    pub value: u32,
    pub label: String,
    pub icon: String,
}

/// `control` of a control widget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum WidgetControl {
    /// DALI group switch; `group` is an id of the demo's `daliGroups`.
    Dali {
        group: u32,
        /// At least two of `off` / `on` / `auto`, any order.
        modes: Vec<super::session::GroupMode>,
    },
    /// Write to a writable Modbus item. The target (`slaveAddress`, `registerAddress`, `coil` |
    /// `register`) and the readback measurement come from the item's slave / register assignment.
    /// Fields per `valueType`: `boolean` → `onLabel`, `offLabel`, `invert`; `number` → `min`,
    /// `max`, `step`, `unit`; `multi` → `options` (holding registers only).
    #[serde(rename_all = "camelCase")]
    Modbus {
        item_id: String,
        /// Add the item's measurement as `readback` (default `true`).
        #[serde(default = "default_true")]
        readback: bool,
        value_type: ModbusValueType,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on_label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        off_label: Option<String>,
        /// `boolean` only: ON writes 0, OFF writes 1.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        invert: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        options: Vec<ModbusControlOption>,
    },
}

/// One widget of a floorplan layout (plugin `WidgetBase` + kind fields, item references).
/// `x` / `y` are image pixels. `deviceId` does not exist: library floorplans are device floorplans.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum FloorplanWidget {
    #[serde(rename_all = "camelCase")]
    Sensor {
        id: String,
        x: f64,
        y: f64,
        #[serde(default)]
        locked: bool,
        icon: String,
        /// Default: the item's German title (DI: the `lw_configDigitalInputs` title).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        measurement: WidgetMeasurement,
        /// Shown unit; default: the series unit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        /// Default 1.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decimals: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display: Option<SensorDisplay>,
        #[serde(default)]
        ranges: Vec<WidgetRange>,
        /// Add the measured item's alarms (`AlarmRef::Item`) automatically (default `true`).
        #[serde(default = "default_true")]
        auto_alarms: bool,
        /// Further alarms to watch.
        #[serde(default)]
        alarms: Vec<AlarmRef>,
        /// Non-null switches stale detection on (the plugin uses interval + 5 min).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stale_after_min: Option<u32>,
    },
    #[serde(rename_all = "camelCase")]
    Alarm {
        id: String,
        x: f64,
        y: f64,
        #[serde(default)]
        locked: bool,
        icon: String,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        alarms: Vec<AlarmRef>,
    },
    #[serde(rename_all = "camelCase")]
    Control {
        id: String,
        x: f64,
        y: f64,
        #[serde(default)]
        locked: bool,
        icon: String,
        /// Default: the item's German title (Modbus) or the group name (DALI).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default)]
        confirm: bool,
        control: WidgetControl,
    },
}

impl FloorplanWidget {
    pub fn id(&self) -> &str {
        match self {
            Self::Sensor { id, .. } | Self::Alarm { id, .. } | Self::Control { id, .. } => id,
        }
    }

    pub fn icon(&self) -> &str {
        match self {
            Self::Sensor { icon, .. } | Self::Alarm { icon, .. } | Self::Control { icon, .. } => {
                icon
            }
        }
    }

    /// `(x, y)` in image pixels.
    pub fn position(&self) -> (f64, f64) {
        match self {
            Self::Sensor { x, y, .. } | Self::Alarm { x, y, .. } | Self::Control { x, y, .. } => {
                (*x, *y)
            }
        }
    }
}

/// `library/floorplans/<demoId>/<name>.json` (`"type": "lw-demo-floorplan"`, PRD §12).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FloorplanLayout {
    #[serde(rename = "type")]
    pub file_type: String,
    pub schema_version: u32,
    /// `lw_floorplan` MO `name`; re-applying replaces a floorplan of the same name.
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// c8y icon name; plugin default `map`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Image file next to this JSON (`.png`, `.jpg`, `.jpeg`, `.svg`).
    pub image: String,
    /// Image size in pixels (widget coordinates use it).
    pub image_width: u32,
    pub image_height: u32,
    /// 0.1–1; missing = 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_opacity: Option<f64>,
    #[serde(default)]
    pub widgets: Vec<FloorplanWidget>,
}

/// Short demo info for lists.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DemoSummary {
    pub id: String,
    pub name: LocalizedText,
    pub description: LocalizedText,
    pub tags: Vec<String>,
    /// Number of items selected by tags ∪ includes − excludes.
    pub item_count: u32,
    pub floorplan_count: u32,
}

/// The whole bundled library, as loaded at startup.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryInfo {
    pub version: String,
    pub demos: Vec<DemoSummary>,
    pub item_count: u32,
    pub slave_count: u32,
    /// Non-fatal problems found while loading (fatal ones fail with `AppError::Library`).
    pub warnings: Vec<String>,
}

/// Per-item generator overrides keyed by item id (used by `lw_demo.generators`).
pub type GeneratorOverrides = BTreeMap<String, GeneratorConfig>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn items_are_kind_tagged_and_strict() {
        let item: LibraryItem = serde_json::from_value(json!({
            "kind": "modbus", "id": "freezer_temp", "slave": "kuehlung",
            "title": {"de": "Tiefkühler", "en": "Freezer"}, "writable": false
        }))
        .unwrap();
        assert_eq!(item.kind(), ItemKind::Modbus);
        assert_eq!(item.register_type(), Some(ModbusRegisterType::Holding));

        let switch: LibraryItem = serde_json::from_value(json!({
            "kind": "modbus", "id": "ac_on", "slave": "klima", "writable": true,
            "title": {"de": "x", "en": "x"},
            "control": {"kind": "switch", "default": 0}
        }))
        .unwrap();
        assert_eq!(switch.register_type(), Some(ModbusRegisterType::Coil));

        // Missing per-kind field, unknown field, unknown kind: all rejected.
        for bad in [
            json!({"kind": "modbus", "id": "x", "title": {"de": "x", "en": "x"}}),
            json!({"kind": "customAlarm", "id": "x", "title": {"de": "x", "en": "x"}, "severity": "MAJOR"}),
            json!({"kind": "dali", "id": "l", "index": 1, "title": {"de": "x", "en": "x"}, "typo": 1}),
            json!({"kind": "nope", "id": "x"}),
        ] {
            assert!(
                serde_json::from_value::<LibraryItem>(bad.clone()).is_err(),
                "{bad}"
            );
        }

        let alarm: LibraryItem = serde_json::from_value(json!({
            "kind": "customAlarm", "id": "doorOpen", "title": {"de": "Tür", "en": "Door"},
            "severity": "MAJOR", "text": "Door open"
        }))
        .unwrap();
        assert_eq!(alarm.kind(), ItemKind::CustomAlarm);
        assert_eq!(serde_json::to_value(&alarm).unwrap()["kind"], "customAlarm");
    }

    #[test]
    fn scenario_steps_and_digital_inputs_round_trip() {
        let steps: Vec<ScenarioStep> = serde_json::from_value(json!([
            {"kind": "value", "itemId": "freezer_temp", "value": -18.5},
            {"kind": "generator", "itemId": "x", "generator": {"kind": "sine", "period": 60, "amplitude": 2}},
            {"kind": "fault", "target": {"kind": "customAlarm", "itemId": "doorOpen"}, "active": true}
        ]))
        .unwrap();
        assert!(matches!(steps[2], ScenarioStep::Fault { active: true, .. }));
        // `value` + `fault` in one step is not expressible any more.
        assert!(
            serde_json::from_value::<ScenarioStep>(json!({"kind": "value", "itemId": "x"}))
                .is_err()
        );

        let di: DigitalInputConfig = serde_json::from_value(json!({
            "index": 1, "itemId": "pump", "type": "daliAction",
            "highLevelAction": "on", "lowLevelAction": "off", "groups": [10, 2]
        }))
        .unwrap();
        assert_eq!(di.settings.input_type, DigitalInputType::DaliAction);
        assert!(di.settings.is_active);
        assert_eq!(di.settings.groups, vec![10, 2]);
        let back = serde_json::to_value(&di).unwrap();
        assert_eq!(back["type"], "daliAction");
        assert_eq!(back["itemId"], "pump");
    }

    #[test]
    fn slave_alarms_are_a_list() {
        let s: SlaveDefinition = serde_json::from_value(json!({
            "key": "kuehlung", "name": "Kuehlung", "requestInterval": 30,
            "alarms": [
                {"name": "co2Leakage", "text": "CO2", "severity": "CRITICAL"},
                {"name": "doorOpen", "text": "Tür", "severity": "MINOR"}
            ]
        }))
        .unwrap();
        assert_eq!(s.alarms.len(), 2);
        let none: SlaveDefinition =
            serde_json::from_value(json!({"key": "k", "name": "K", "requestInterval": 30}))
                .unwrap();
        assert!(none.alarms.is_empty());
    }

    #[test]
    fn write_types_map_to_plugin_and_operation() {
        assert_eq!(ModbusRegisterType::Coil.write_register_type(), Some("coil"));
        assert_eq!(
            ModbusRegisterType::Holding.write_register_type(),
            Some("holding")
        );
        assert_eq!(
            ModbusRegisterType::Holding.write_operation_type(),
            Some("register")
        );
        assert_eq!(
            ModbusRegisterType::Coil.write_operation_type(),
            Some("coil")
        );
        assert_eq!(ModbusRegisterType::Input.write_register_type(), None);
    }
}
