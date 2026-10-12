//! Demo item selection, search and filter chips (PRD §11.4, §11.5).

use crate::model::{DemoPack, LibraryItem, SlaveDefinition};

/// Items of a demo: items carrying any of the demo's `tags`, plus `includes`, minus `excludes`.
/// Library order, no duplicates.
pub fn select_items<'a>(items: &'a [LibraryItem], demo: &DemoPack) -> Vec<&'a LibraryItem> {
    select_by(items, &demo.tags, &demo.includes, &demo.excludes)
}

/// Selection rule `tags ∪ includes − excludes` for any setup (demo pack or `lw_demo`).
pub fn select_by<'a>(
    items: &'a [LibraryItem],
    tags: &[String],
    includes: &[String],
    excludes: &[String],
) -> Vec<&'a LibraryItem> {
    let mut out: Vec<&LibraryItem> = Vec::new();
    for item in items {
        let id = item.id();
        if excludes.iter().any(|e| e == id) || out.iter().any(|o| o.id() == id) {
            continue;
        }
        let tagged = item.tags().iter().any(|t| tags.contains(t));
        if tagged || includes.iter().any(|i| i == id) {
            out.push(item);
        }
    }
    out
}

/// Full-text search over title (de + en), id, tags and slave name (Modbus items).
/// Case-insensitive; the query is split on whitespace and all words must match (AND).
/// An empty query matches everything.
pub fn search<'a>(
    items: &'a [LibraryItem],
    slaves: &[SlaveDefinition],
    query: &str,
) -> Vec<&'a LibraryItem> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    items
        .iter()
        .filter(|item| {
            let slave_name = item
                .slave_key()
                .and_then(|k| slaves.iter().find(|s| s.key == k))
                .map(|s| s.name.as_str());
            let mut haystack = vec![
                item.title().de.as_str(),
                item.title().en.as_str(),
                item.id(),
            ];
            haystack.extend(item.tags().iter().map(String::as_str));
            haystack.extend(slave_name);
            let haystack: Vec<String> = haystack.into_iter().map(str::to_lowercase).collect();
            words
                .iter()
                .all(|w| haystack.iter().any(|h| h.contains(w.as_str())))
        })
        .collect()
}

/// Tag namespace (PRD §11.5): plain client tags, `category:*`, `zone:*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TagNamespace {
    /// Client / demo tag, e.g. `gas-station`.
    Client,
    /// `category:<value>`.
    Category,
    /// `zone:<value>`.
    Zone,
}

/// Namespace and value of a tag; `None` for an unknown `prefix:` namespace.
pub fn parse_tag(tag: &str) -> Option<(TagNamespace, &str)> {
    match tag.split_once(':') {
        None => Some((TagNamespace::Client, tag)),
        Some(("category", v)) => Some((TagNamespace::Category, v)),
        Some(("zone", v)) => Some((TagNamespace::Zone, v)),
        Some(_) => None,
    }
}

/// Filter chips built from the tags of `items`: full tag strings, sorted, unique, per namespace.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilterChips {
    pub client: Vec<String>,
    pub category: Vec<String>,
    pub zone: Vec<String>,
}

pub fn filter_chips<'a>(items: impl IntoIterator<Item = &'a LibraryItem>) -> FilterChips {
    let mut chips = FilterChips::default();
    for item in items {
        for tag in item.tags() {
            let list = match parse_tag(tag) {
                Some((TagNamespace::Client, _)) => &mut chips.client,
                Some((TagNamespace::Category, _)) => &mut chips.category,
                Some((TagNamespace::Zone, _)) => &mut chips.zone,
                None => continue,
            };
            if !list.contains(tag) {
                list.push(tag.clone());
            }
        }
    }
    chips.client.sort();
    chips.category.sort();
    chips.zone.sort();
    chips
}

/// Whether `item` passes the active chips: within a namespace any chip matches (OR), across
/// namespaces all must match (AND). No active chips = everything passes.
pub fn matches_chips(item: &LibraryItem, active: &[String]) -> bool {
    [
        TagNamespace::Client,
        TagNamespace::Category,
        TagNamespace::Zone,
    ]
    .iter()
    .all(|ns| {
        let mut chips = active
            .iter()
            .filter(|c| parse_tag(c).is_some_and(|(n, _)| n == *ns))
            .peekable();
        chips.peek().is_none() || chips.any(|c| item.tags().contains(c))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::LocalizedText;
    use serde_json::json;

    fn item(id: &str, de: &str, en: &str, tags: &[&str]) -> LibraryItem {
        serde_json::from_value(json!({
            "kind": "dali", "id": id, "index": 1,
            "title": {"de": de, "en": en}, "tags": tags
        }))
        .unwrap()
    }

    fn modbus(id: &str, slave: &str, tags: &[&str]) -> LibraryItem {
        serde_json::from_value(json!({
            "kind": "modbus", "id": id, "slave": slave,
            "title": {"de": "Wert", "en": "Value"}, "tags": tags
        }))
        .unwrap()
    }

    fn demo(tags: &[&str], includes: &[&str], excludes: &[&str]) -> DemoPack {
        let s = |v: &[&str]| v.iter().map(|x| (*x).to_owned()).collect();
        DemoPack {
            file_type: "lw-demo".into(),
            schema_version: 1,
            id: "d".into(),
            name: LocalizedText::default(),
            description: LocalizedText::default(),
            tags: s(tags),
            includes: s(includes),
            excludes: s(excludes),
            dali_lamp_count: 4,
            dali_groups: vec![],
            digital_inputs: vec![],
            send_interval_sec: 30,
            scenarios: vec![],
            floorplans: vec![],
        }
    }

    fn ids(v: &[&LibraryItem]) -> Vec<String> {
        v.iter().map(|i| i.id().to_owned()).collect()
    }

    #[test]
    fn selection_is_tags_union_includes_minus_excludes() {
        let items = vec![
            item("a", "A", "A", &["gas-station"]),
            item("b", "B", "B", &["retail"]),
            item("c", "C", "C", &["gas-station", "retail"]),
            item("d", "D", "D", &[]),
        ];
        let sel = select_items(&items, &demo(&["gas-station"], &["b", "b"], &["c"]));
        assert_eq!(ids(&sel), vec!["a", "b"]);
        // An excluded include stays excluded; order = library order.
        let sel = select_items(&items, &demo(&["retail"], &["d", "a"], &["a"]));
        assert_eq!(ids(&sel), vec!["b", "c", "d"]);
        assert!(select_items(&items, &demo(&[], &[], &[])).is_empty());
    }

    #[test]
    fn search_ands_words_over_titles_ids_tags_and_slave_names() {
        let items = vec![
            item(
                "light-1",
                "Licht Zapfsäule",
                "Pump light",
                &["gas-station", "zone:forecourt"],
            ),
            item(
                "light-2",
                "Licht Shop",
                "Shop light",
                &["retail", "zone:shop"],
            ),
            modbus("freezer_temp", "kuehlung", &["category:cooling"]),
        ];
        let slaves = vec![SlaveDefinition {
            key: "kuehlung".into(),
            name: "Kuehlung Markt".into(),
            request_interval: 30,
            alarms: vec![],
        }];
        let q = |q: &str| ids(&search(&items, &slaves, q));
        assert_eq!(q("licht"), vec!["light-1", "light-2"]);
        assert_eq!(q("LICHT shop"), vec!["light-2"]);
        assert_eq!(q("pump forecourt"), vec!["light-1"]);
        assert_eq!(q("zapfsäule"), vec!["light-1"]);
        assert_eq!(q("markt"), vec!["freezer_temp"]);
        assert_eq!(q("cooling freezer"), vec!["freezer_temp"]);
        assert!(q("licht markt").is_empty());
        assert_eq!(q("  ").len(), 3);
    }

    #[test]
    fn chips_by_namespace() {
        let items = vec![
            item("a", "A", "A", &["retail", "category:light", "zone:shop"]),
            item(
                "b",
                "B",
                "B",
                &["gas-station", "category:light", "zone:forecourt"],
            ),
            item("c", "C", "C", &["gas-station", "category:tank", "odd:x"]),
        ];
        let chips = filter_chips(&items);
        assert_eq!(chips.client, vec!["gas-station", "retail"]);
        assert_eq!(chips.category, vec!["category:light", "category:tank"]);
        assert_eq!(chips.zone, vec!["zone:forecourt", "zone:shop"]);

        let active = |v: &[&str]| v.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
        let pass = |chips: &[&str]| {
            items
                .iter()
                .filter(|i| matches_chips(i, &active(chips)))
                .map(|i| i.id())
                .collect::<Vec<_>>()
        };
        assert_eq!(pass(&[]), vec!["a", "b", "c"]);
        assert_eq!(pass(&["gas-station"]), vec!["b", "c"]);
        // OR within a namespace, AND across.
        assert_eq!(
            pass(&["category:light", "category:tank"]),
            vec!["a", "b", "c"]
        );
        assert_eq!(pass(&["gas-station", "category:light"]), vec!["b"]);
        assert_eq!(pass(&["retail", "zone:forecourt"]), Vec::<&str>::new());
        assert_eq!(parse_tag("zone:shop"), Some((TagNamespace::Zone, "shop")));
        assert_eq!(parse_tag("odd:x"), None);
    }
}
