# Library JSON schemas

JSON Schema draft 2020-12, the normative spec of the library files (PRD §11.1, §18).

| Schema | Files | `type` |
|---|---|---|
| `tenants.schema.json` | `tenants.json` | `lw-tenants` |
| `items.schema.json` | `items/*.json` | `lw-demo-items` |
| `demo.schema.json` | `demos/*.json` | `lw-demo` |
| `floorplan.schema.json` | `floorplans/<demoId>/*.json` | `lw-demo-floorplan` |
| `common.schema.json` | shared `$defs` (texts, tags, controls, generators, effects, fault targets) | — |

`$id`s live under `https://schemas.lichtwart.io/demo-simulator/` (identifiers only, nothing is
fetched); cross-file `$ref`s are relative (`common.schema.json#/$defs/…`).

Checks:

- `npm run validate:library` (ajv) — schema check of `library/` and the test fixture library.
- `cargo test` — `library::schema_tests` (every file vs. schema, schema ⇄ serde agreement) and
  `library::validate` (semantic rules: unique ids, references, firmware limits).

Keep in sync with the serde model in `src-tauri/src/model/library.rs`: change both, then run
`npm run gen:types` and the checks above.
