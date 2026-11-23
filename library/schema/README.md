# Library JSON schemas

JSON Schema draft 2020-12, the normative spec of the library files (PRD §11.1, §18).

| Schema | Files | `type` |
|---|---|---|
| `items.schema.json` | `items/*.json` | `lw-demo-items` |
| `demo.schema.json` | `demos/*.json` | `lw-demo` |
| `floorplan.schema.json` | `floorplans/<demoId>/*.json` | `lw-demo-floorplan` |
| `common.schema.json` | shared `$defs` (texts, tags, controls, generators, effects, fault targets) | — |

`$id`s live under `https://schemas.lichtwart.io/demo-simulator/` (identifiers only, nothing is
fetched); cross-file `$ref`s are relative (`common.schema.json#/$defs/…`).

## Texts in two languages (DECISIONS D12)

A demo device has a content language (`lw_demo.language`, `de` | `en`, absent = `de`); everything
it gets from the library is named in that language. Fields that were already `{de, en}` stay so
(item `title`, select option `label`, demo `name` / `description`, scenario `name` /
`description`). These human-readable fields take **either** a plain string (used for both
languages, what older files have) **or** `{"de": …, "en": …}` (`common.schema.json#/$defs/text`,
`optionalText`):

| File | Field |
|---|---|
| `items/*.json` | `slaves[].name`, `slaves[].alarms[].text`, `items[].text` (`customAlarm`), `items[].effects[].text` (`event`, `alarm`) |
| `demos/*.json` | `daliGroups[].name` |
| `floorplans/*/*.json` | `name`, `description`, `widgets[].title` (sensor, control), `widgets[].text` / `note` (alarm), `widgets[].display.onLabel` / `offLabel` (sensor), `widgets[].control.onLabel` / `offLabel` / `options[].label` (Modbus control) |

Rules (`cargo test`, `library::validate`) hold in **each** language: non-empty where the field is
required; a `{de, en}` text needs both languages even where the field is optional; a slave name
is 1–19 UTF-8 **bytes** and ASCII; floorplan names are unique per demo across both languages (an
apply matches a floorplan by its name in either language). Units and ids are not translated; firmware alarm texts are English already.

Checks:

- `npm run validate:library` (ajv) — schema check of `library/` and the test fixture library.
- `cargo test` — `library::schema_tests` (every file vs. schema, schema ⇄ serde agreement) and
  `library::validate` (semantic rules: unique ids, references, firmware limits).

Keep in sync with the serde model in `src-tauri/src/model/library.rs`: change both, then run
`npm run gen:types` and the checks above.
