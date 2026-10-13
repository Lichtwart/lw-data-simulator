//! Resolving a device setup (`lw_demo` + library) into items and slaves, slave / register
//! assignment, the device's DALI groups / digital inputs (`lw_demo`, D10), digests and versions.
//!
//! Shared by apply, upgrade, export, `update_demo_setup`, the fault catalog and the session, so
//! every place resolves a device exactly the same way (DECISIONS D6).

use std::collections::BTreeMap;

use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::library::{Library, select};
use crate::model::{
    DaliGroup, DemoPack, DeviceConfig, DigitalInputEntry, LibraryItem, LwDemoState,
    ModbusRegisterType, SlaveAssignment, SlaveDefinition,
};
use crate::payload::config::digital_input_entry;
use crate::payload::naming::{assign_registers, assign_slaves};

/// Items and slaves of a device setup (owned, so a session can keep them).
#[derive(Debug, Clone, Default)]
pub struct ResolvedSetup {
    /// Library selection (`tags ∪ includes − excludes`, library order) followed by the device's
    /// custom items (minus `excludes`).
    pub items: Vec<LibraryItem>,
    /// Slaves referenced by the selected Modbus items (order of first use) with their assignment.
    pub slaves: Vec<(SlaveDefinition, SlaveAssignment)>,
    /// Assignments that are not selected right now (see `LwDemoState::retired_slaves`): store
    /// them back with `slaves` so numbers are never reused.
    pub retired: Vec<SlaveAssignment>,
}

impl ResolvedSetup {
    pub fn item_refs(&self) -> Vec<&LibraryItem> {
        self.items.iter().collect()
    }

    pub fn slave_refs(&self) -> Vec<(&SlaveDefinition, &SlaveAssignment)> {
        self.slaves.iter().map(|(d, a)| (d, a)).collect()
    }

    pub fn item(&self, id: &str) -> Option<&LibraryItem> {
        self.items.iter().find(|i| i.id() == id)
    }
}

/// Selected items of a setup: library selection + custom items (not excluded).
pub fn select_items(library: &Library, demo: &LwDemoState) -> Vec<LibraryItem> {
    let mut items: Vec<LibraryItem> =
        select::select_by(&library.items, &demo.tags, &demo.includes, &demo.excludes)
            .into_iter()
            .cloned()
            .collect();
    for c in &demo.custom_items {
        if !demo.excludes.iter().any(|e| e == c.id()) && !items.iter().any(|i| i.id() == c.id()) {
            items.push(c.clone());
        }
    }
    items
}

/// Slave definition by key: library first, then the device's custom slaves.
pub fn slave_definition<'a>(
    library: &'a Library,
    demo: &'a LwDemoState,
    key: &str,
) -> Option<&'a SlaveDefinition> {
    library
        .slave(key)
        .or_else(|| demo.custom_slaves.iter().find(|s| s.key == key))
}

/// Slave keys used by `items`, in order of first use.
pub fn slave_keys(items: &[LibraryItem]) -> Vec<&str> {
    let mut keys: Vec<&str> = Vec::new();
    for k in items.iter().filter_map(LibraryItem::slave_key) {
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    keys
}

/// Slave ids / addresses and register addresses for `items`, keeping every existing assignment
/// (D6: never renumbered; new slaves count down from 20 / 240, new registers take the lowest
/// free address of their type). `retired` are assignments of slaves / registers that are not
/// selected (any more): their numbers stay taken and come back when the item is selected again.
///
/// Returns `(active, retired)`: the assignments for `items`, and everything of `existing` /
/// `retired` that is not selected, to be stored in `LwDemoState::retired_slaves`.
pub fn assign_all(
    existing: &[SlaveAssignment],
    retired: &[SlaveAssignment],
    items: &[LibraryItem],
) -> AppResult<(Vec<SlaveAssignment>, Vec<SlaveAssignment>)> {
    // Everything ever assigned, per slave key.
    let mut pool: Vec<SlaveAssignment> = existing.to_vec();
    for r in retired {
        match pool.iter_mut().find(|p| p.key == r.key) {
            Some(p) => {
                for reg in &r.registers {
                    if !p
                        .registers
                        .iter()
                        .any(|x| x.item_id == reg.item_id && x.register_type == reg.register_type)
                    {
                        p.registers.push(reg.clone());
                    }
                }
            }
            None => pool.push(r.clone()),
        }
    }
    let keys = slave_keys(items);
    let mut slaves = assign_slaves(&pool, &keys)?;
    for s in &mut slaves {
        let regs: Vec<(&str, ModbusRegisterType)> = items
            .iter()
            .filter(|i| i.slave_key() == Some(s.key.as_str()))
            .filter_map(|i| i.register_type().map(|t| (i.id(), t)))
            .collect();
        s.registers = assign_registers(&s.registers, &regs)?;
    }
    let mut out_retired = Vec::new();
    for p in &pool {
        match slaves.iter().find(|s| s.key == p.key) {
            None => out_retired.push(p.clone()),
            Some(active) => {
                let registers: Vec<_> =
                    p.registers
                        .iter()
                        .filter(|r| {
                            !active.registers.iter().any(|a| {
                                a.item_id == r.item_id && a.register_type == r.register_type
                            })
                        })
                        .cloned()
                        .collect();
                if !registers.is_empty() {
                    out_retired.push(SlaveAssignment {
                        registers,
                        ..p.clone()
                    });
                }
            }
        }
    }
    Ok((slaves, out_retired))
}

/// Resolve a setup: select items, look up slave definitions, assign (keeping `demo.slaves` and
/// `demo.retired_slaves`).
pub fn resolve(library: &Library, demo: &LwDemoState) -> AppResult<ResolvedSetup> {
    let items = select_items(library, demo);
    let (assignments, retired) = assign_all(&demo.slaves, &demo.retired_slaves, &items)?;
    let mut slaves = Vec::with_capacity(assignments.len());
    for a in assignments {
        let def = slave_definition(library, demo, &a.key)
            .ok_or_else(|| AppError::NotFound(format!("slave {}", a.key)))?
            .clone();
        slaves.push((def, a));
    }
    Ok(ResolvedSetup {
        items,
        slaves,
        retired,
    })
}

impl ResolvedSetup {
    /// Store the assignments (active and retired) in `state`.
    pub fn store_assignments(&self, state: &mut LwDemoState) {
        state.slaves = self.slaves.iter().map(|(_, a)| a.clone()).collect();
        state.retired_slaves = self.retired.clone();
    }
}

// ───────────────────────────── export round trip (finding 14) ─────────────────────────────

/// What [`drop_library_duplicates`] did.
#[derive(Debug, Clone, Default)]
pub struct CustomDedupe {
    /// Ids of custom slaves / items that the library has now.
    pub now_in_library: Vec<String>,
    /// Entries whose library version differs from the custom one (the library one is used).
    pub warnings: Vec<String>,
}

/// Custom slaves / items whose id now exists in the library (an exported pack was released) are
/// dropped from the device: the library version is used. Equal ones silently, different ones
/// with a warning. An item that was selected only as a custom item is added to `includes`, so
/// the device does not lose it. `known`: only entries that were custom on the device before
/// (a patch that newly introduces a colliding id is an error, not a release).
pub fn drop_library_duplicates(
    library: &Library,
    state: &mut LwDemoState,
    known: Option<&std::collections::BTreeSet<String>>,
) -> CustomDedupe {
    let is_known = |id: &str| known.is_none_or(|k| k.contains(id));
    let mut out = CustomDedupe::default();
    let mut keep_items = Vec::new();
    for c in std::mem::take(&mut state.custom_items) {
        match library.item(c.id()).filter(|_| is_known(c.id())) {
            None => keep_items.push(c),
            Some(l) => {
                if digest(l) != digest(&c) {
                    out.warnings.push(format!(
                        "custom item {} is in the library now; the library version is used",
                        c.id()
                    ));
                }
                out.now_in_library.push(c.id().to_owned());
                let id = c.id().to_owned();
                let selected = select::select_by(
                    &library.items,
                    &state.tags,
                    &state.includes,
                    &state.excludes,
                )
                .iter()
                .any(|i| i.id() == id);
                if !selected && !state.excludes.contains(&id) {
                    state.includes.push(id);
                }
            }
        }
    }
    state.custom_items = keep_items;
    let mut keep_slaves = Vec::new();
    for c in std::mem::take(&mut state.custom_slaves) {
        match library.slave(&c.key).filter(|_| is_known(&c.key)) {
            None => keep_slaves.push(c),
            Some(l) => {
                if digest(l) != digest(&c) {
                    out.warnings.push(format!(
                        "custom slave {} is in the library now; the library version is used",
                        c.key
                    ));
                }
                out.now_in_library.push(c.key.clone());
            }
        }
    }
    state.custom_slaves = keep_slaves;
    out
}

// ───────────────────────────── demo-pack level digests (finding 12) ─────────────────────────────

/// Fields of a demo pack that are copied to the device at apply time.
pub const DEMO_FIELDS: &[&str] = &[
    "tags",
    "includes",
    "excludes",
    "daliLampCount",
    "daliGroups",
    "digitalInputs",
    "sendIntervalSec",
];

/// The demo pack's digital inputs as the device stores them (`lw_demo.digitalInputs`, with the
/// item's German title).
pub fn demo_inputs(demo: &DemoPack, items: &[&LibraryItem]) -> AppResult<Vec<DigitalInputEntry>> {
    demo.digital_inputs
        .iter()
        .map(|c| digital_input_entry(c, items))
        .collect()
}

/// Digests of the demo pack's values as they are written to the device.
pub fn demo_digests(
    demo: &DemoPack,
    items: &[&LibraryItem],
) -> AppResult<BTreeMap<String, String>> {
    let inputs = demo_inputs(demo, items)?;
    let groups = &demo.dali_groups;
    Ok(device_digests(
        &demo.tags,
        &demo.includes,
        &demo.excludes,
        demo.dali_lamp_count,
        demo.send_interval_sec,
        groups,
        &inputs,
    ))
}

/// Digests of the same fields as the device has them now.
pub fn current_digests(state: &LwDemoState, config: &DeviceConfig) -> BTreeMap<String, String> {
    device_digests(
        &state.tags,
        &state.includes,
        &state.excludes,
        state.dali_lamp_count,
        state.send_interval_sec,
        &config.groups,
        &config.digital_inputs,
    )
}

fn device_digests(
    tags: &[String],
    includes: &[String],
    excludes: &[String],
    lamps: u32,
    interval: u32,
    groups: &[DaliGroup],
    inputs: &[DigitalInputEntry],
) -> BTreeMap<String, String> {
    [
        ("tags", digest(&tags)),
        ("includes", digest(&includes)),
        ("excludes", digest(&excludes)),
        ("daliLampCount", digest(&lamps)),
        ("daliGroups", digest(&canonical_groups(groups))),
        ("digitalInputs", digest(&canonical_inputs(inputs))),
        ("sendIntervalSec", digest(&interval)),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect()
}

/// Firmware default `luxThreshold` of a group.
const DEFAULT_LUX_THRESHOLD: u32 = 4500;

/// Groups as digests compare them: the firmware default lux threshold filled in. Older app
/// versions stored groups in `lw_configGroups` and digested them as they came back from there;
/// the same form keeps their `appliedDemo` digests valid (D10).
fn canonical_groups(groups: &[DaliGroup]) -> Vec<DaliGroup> {
    groups
        .iter()
        .map(|g| DaliGroup {
            lux_threshold: Some(g.lux_threshold.unwrap_or(DEFAULT_LUX_THRESHOLD)),
            ..g.clone()
        })
        .collect()
}

/// Digital inputs as digests compare them: only the fields of the input's type, counter
/// defaults filled in, the unit cut to the firmware's 19 bytes (the form older app versions read
/// back from `lw_configDigitalInputs`, see [`canonical_groups`]).
fn canonical_inputs(inputs: &[DigitalInputEntry]) -> Vec<DigitalInputEntry> {
    use crate::model::DigitalInputType;
    use crate::payload::config::{DI_UNIT_MAX_LEN, truncate_bytes};
    inputs
        .iter()
        .map(|e| {
            let s = &e.settings;
            let mut c = e.clone();
            let (counter, action) = match s.input_type {
                DigitalInputType::ImpulseCounter => (true, false),
                DigitalInputType::DaliAction => (false, true),
                DigitalInputType::Measurement => (false, false),
            };
            c.settings.init_count = counter.then(|| s.init_count.unwrap_or(0));
            c.settings.increment = counter.then(|| s.increment.unwrap_or(1));
            c.settings.unit = s
                .unit
                .as_deref()
                .filter(|_| counter)
                .map(|u| truncate_bytes(u, DI_UNIT_MAX_LEN).to_owned());
            if !action {
                c.settings.high_level_action = None;
                c.settings.low_level_action = None;
                c.settings.groups = Vec::new();
            }
            c
        })
        .collect()
}

// ───────────────────────────── groups and digital inputs (D10) ─────────────────────────────

/// DALI groups and digital inputs of a device: `lw_demo.daliGroups` / `lw_demo.digitalInputs`.
/// A device set up by an older app version (which kept them in `lw_config*`, no longer read,
/// D10) has neither: the library demo's groups / inputs are used then (none without a demo).
pub fn device_config(library: &Library, state: &LwDemoState) -> DeviceConfig {
    let demo = state.demo_id.as_deref().and_then(|d| library.demo(d));
    let groups = match &state.dali_groups {
        Some(g) => g.clone(),
        None => demo.map(|d| d.dali_groups.clone()).unwrap_or_default(),
    };
    let digital_inputs = match (&state.digital_inputs, demo) {
        (Some(i), _) => i.clone(),
        (None, Some(d)) => {
            let items = select_items(library, state);
            let refs: Vec<&LibraryItem> = items.iter().collect();
            d.digital_inputs
                .iter()
                .filter_map(|c| digital_input_entry(c, &refs).ok())
                .collect()
        }
        (None, None) => Vec::new(),
    };
    DeviceConfig {
        groups,
        digital_inputs,
    }
}

/// Every `lw_config*` fragment of a managed object, set to `null`: the patch that removes them
/// (D10: the simulator no longer writes or reads them; older app versions did).
pub fn stale_config_fragments(mo: &Value) -> serde_json::Map<String, Value> {
    mo.as_object()
        .map(|o| {
            o.keys()
                .filter(|k| k.starts_with("lw_config"))
                .map(|k| (k.clone(), Value::Null))
                .collect()
        })
        .unwrap_or_default()
}

// ───────────────────────────── digests / versions ─────────────────────────────

/// Stable short digest (FNV-1a 64, hex) of a value's canonical JSON. Used to detect changed
/// library items / floorplans between the applied and the bundled library (PRD §14).
pub fn digest<T: serde::Serialize>(value: &T) -> String {
    let json = serde_json::to_string(value).unwrap_or_default();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in json.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Digest per selected library item (custom items are device-local and not tracked).
pub fn item_digests(library: &Library, items: &[LibraryItem]) -> BTreeMap<String, String> {
    items
        .iter()
        .filter(|i| library.item(i.id()).is_some())
        .map(|i| (i.id().to_owned(), digest(i)))
        .collect()
}

/// `MAJOR.MINOR.PATCH` (a pre-release suffix is ignored); `None` if unparsable.
pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    Some((
        parts.next()??,
        parts.next().flatten().unwrap_or(0),
        parts.next().flatten().unwrap_or(0),
    ))
}

/// `device` is older than `bundled` (both SemVer). Unparsable versions are never "older".
pub fn is_older_version(device: &str, bundled: &str) -> bool {
    match (parse_version(device), parse_version(bundled)) {
        (Some(d), Some(b)) => d < b,
        _ => false,
    }
}

/// `lw_demo` of a device needs an upgrade: it has a demo that exists in the bundled library
/// and its library version is older.
pub fn upgrade_available(library: &Library, demo: Option<&LwDemoState>) -> bool {
    demo.is_some_and(|d| {
        d.demo_id
            .as_deref()
            .is_some_and(|id| library.demo(id).is_some())
            && d.library_version
                .as_deref()
                .is_some_and(|v| is_older_version(v, &library.version))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_older_version("1.4.0", "1.10.0"));
        assert!(!is_older_version("1.10.0", "1.4.0"));
        assert!(!is_older_version("1.4.0", "1.4.0"));
        assert!(is_older_version("0.9", "1.0.0"));
        assert!(!is_older_version("garbage", "1.0.0"));
    }

    #[test]
    fn stale_config_fragments_are_nulled() {
        let mo = json!({ "name": "d", "lw_demo": {}, "lw_configCore": {}, "lw_configGroups": [],
                         "lw_configAlarmBlacklist": [], "c8y_IsDevice": {} });
        let p = stale_config_fragments(&mo);
        let keys: Vec<&str> = p.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            vec![
                "lw_configAlarmBlacklist",
                "lw_configCore",
                "lw_configGroups"
            ]
        );
        assert!(p.values().all(Value::is_null));
        assert!(stale_config_fragments(&json!({ "name": "d" })).is_empty());
    }

    /// Groups / inputs come from `lw_demo`; a device without them gets the library demo's.
    #[test]
    fn device_config_falls_back_to_the_library_demo() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library");
        let lib = crate::library::loader::load(&root, "0.0.0").unwrap();
        let demo = &lib.demos[0];
        let mut st = crate::device::demo_state::empty_state();
        st.demo_id = Some(demo.id.clone());
        st.tags.clone_from(&demo.tags);
        st.includes.clone_from(&demo.includes);
        st.excludes.clone_from(&demo.excludes);
        let c = device_config(&lib, &st);
        assert_eq!(c.groups, demo.dali_groups);
        assert_eq!(c.digital_inputs.len(), demo.digital_inputs.len());
        assert_eq!(c.digital_inputs[0].title, "Zapfsäule 1");
        st.dali_groups = Some(Vec::new());
        st.digital_inputs = Some(Vec::new());
        let c = device_config(&lib, &st);
        assert!(c.groups.is_empty() && c.digital_inputs.is_empty());
        let raw = device_config(&lib, &crate::device::demo_state::empty_state());
        assert!(raw.groups.is_empty() && raw.digital_inputs.is_empty());
    }

    /// Devices applied by an older app version carry `appliedDemo` digests of groups / inputs as
    /// they came back from `lw_config*` (defaults filled in): the same values still match (D10).
    #[test]
    fn group_and_input_digests_match_the_old_round_trip_form() {
        use crate::model::{DigitalInputSettings, DigitalInputType};
        let group = |lux| DaliGroup {
            id: 1,
            name: "Shop".into(),
            converters: vec![1, 2],
            min_level: 0,
            max_level: 100,
            lux_threshold: lux,
            slave_of: None,
            dynamic_control: false,
            automatic_switch: false,
        };
        let input = |t, init, inc, unit: Option<&str>| DigitalInputEntry {
            index: 3,
            title: "Gas".into(),
            settings: DigitalInputSettings {
                input_type: t,
                is_active: true,
                init_count: init,
                increment: inc,
                unit: unit.map(str::to_owned),
                high_level_action: None,
                low_level_action: None,
                groups: vec![],
            },
        };
        let d = |g: Vec<DaliGroup>, i: Vec<DigitalInputEntry>| {
            device_digests(&[], &[], &[], 4, 30, &g, &i)
        };
        let library_form = d(
            vec![group(None)],
            vec![input(
                DigitalInputType::ImpulseCounter,
                None,
                None,
                Some("m3"),
            )],
        );
        let old_round_trip = d(
            vec![group(Some(4500))],
            vec![input(
                DigitalInputType::ImpulseCounter,
                Some(0),
                Some(1),
                Some("m3"),
            )],
        );
        assert_eq!(library_form, old_round_trip);
        // A real change still shows.
        let changed = d(
            vec![group(Some(100))],
            vec![input(DigitalInputType::Measurement, None, None, None)],
        );
        assert_ne!(changed["daliGroups"], library_form["daliGroups"]);
        assert_ne!(changed["digitalInputs"], library_form["digitalInputs"]);
    }

    #[test]
    fn digest_is_stable_and_sensitive() {
        assert_eq!(digest(&json!({"a": 1})), digest(&json!({"a": 1})));
        assert_ne!(digest(&json!({"a": 1})), digest(&json!({"a": 2})));
    }
}
