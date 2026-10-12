# Phase 3 review — demo library content

Scope: `library/items/*.json`, `library/demos/*.json`, `library/floorplans/*/ground-floor.{json,svg}`.
Checked against PRD §7, §8, §11, §12, §17, `docs/DECISIONS.md`, `AGENTS.md` (Library format, Shipped
demos), `docs/research/prototype-inventory.md` (the 135-item table), `docs/research/floorplan-plugin.md`,
`docs/research/firmware-core-v2.md`, the sim engine (`src-tauri/src/sim/{core,scenario,scheduler}.rs`)
and the plugin source (`davaco-demo/src/app/floorplan`, `node_modules/@c8y/style` icon set).

Tools run:
- `npm run validate:library`: **25 files checked, 0 invalid**.
- `cargo test --lib library::`: **34 passed** (this includes `loads_repo_library`, no load warnings).
- A script that compares the inventory table with the library (ids, client tags) and computes
  each demo's selection. Result: all 135 prototype items are there, and every item's client tags
  match the inventory exactly. Selections are 72 / 69 / 66 items, matching the AGENTS.md table.
- Floorplans drawn with the plugin's real widget geometry and opened in Chrome (chrome-devtools).
  Circles are 96 px wide, 64 px high, anchored at the centre with `translate(-50%,-32px)`, title
  below. Controls are at least 150 px wide with a 36 px header (`widget-anchor.ts`,
  `control-shell.styles.less`). **Widgets do not scale with the image** (`canvas.component.ts:110-112`).
  The drawing assumes the image is fitted at a scale of 0.83 (a panel about 1000 px wide).

Engine facts the scenario findings depend on:
- A scenario `value` step sets the item's manual override, so its generator stops
  (`core.rs:518-543`). Only a `generator` step clears the override again (`core.rs:799-821`).
- A scenario never undoes anything by itself.
- A slave timeout stops that slave's measurements (PRD §7.4).
- Generators step every 5 s (`scheduler.rs:25`).

Severity:
- **High**: the demo visibly misbehaves in a sales demo.
- **Medium**: a real content defect or a clear realism gap.
- **Low**: polish.
- **Info**: no action needed, recorded for the record.

---

## High

### 1. `library/demos/*.json` `scenarios[id=normal].set`: "Normalbetrieb" does not undo the other scenarios
Value steps freeze generators, and faults stay active until someone clears them. So after any demo
scenario, a click on "Normalbetrieb" leaves the device broken. What stays wrong:
- **gas-station**:
  - `cooling-fault` leaves `cooler_temp` stuck at 11 °C (only `freezer_temp` gets its generator
    back) and `fuse_freezer` active.
  - `peak-load` leaves `en_ev1..3_peak` at 30 A, `current_peak_total` on randomWalk 250 (red on
    the floorplan), `ev_power_alarm` active, and `ev_slot_1..3` = 1.
  - `night` leaves `lux-1` at 0 lx in daytime (its dailyProfile is frozen), and `crew_occ` /
    `support_occ` = 0.
- **retail**: the same as gas-station, and additionally `en_clr_alarm` and `fuse_cooler` stay
  active, and `cooler_temp` stays at 11.
- **restaurant**:
  - `kitchen-fault` leaves `ac_kit_alarm` active and `ac_kit_act` stuck at 32 °C.
  - `lunch-peak` leaves `en_kit_peak` (30), `en_din_peak` (24) and `ac_din_act` (26) frozen, and
    `current_peak_total` on 250.
  - `night` leaves `lux-1` stuck at 0.

**Fix:** make `normal` a full reset:
- a `generator` step restoring the library generator for every item that any other scenario sets
  (`cooler_temp`, `freezer_temp`, `lux-1`, `en_ev1..3_peak`, `current_peak_total` (its dailyProfile),
  `en_kit_peak`, `en_din_peak`, `ac_kit_act`, `ac_din_act`);
- `fault … active:false` for every fault that any scenario raises (`fuse_freezer`, `fuse_cooler`,
  `en_clr_alarm`, `ev_power_alarm`, `ac_kit_alarm`, …);
- daytime values for the DIs.

A validation rule would stop this from coming back: "every target that a non-start scenario
changes must also appear in the start scenario".

### 2. `library/demos/retail.json` `scenarios[id=cooling-fault]`: the slave timeout hides the temperature rise
The scenario sets `freezer_temp = -9` and `cooler_temp = 11`, and in the same step activates
`slaveTimeout kuehlung`. A timeout stops all `kuehlung` measurements, so the new temperatures are
never sent. The floorplan keeps showing the old green −18 °C / 3 °C (and goes stale after 15 min),
and the warning/alarm ranges never turn red. gas-station's `cooling-fault` does it correctly
(no timeout).
**Fix:** remove the `slaveTimeout` step from `cooling-fault`. If a communication loss should be
shown, put it in its own scenario (`comm-loss`, "Kommunikationsausfall Kühlung").

### 3. `library/floorplans/*/ground-floor.json` `widgets[].x/y`: widgets overlap each other and cover room labels
The widgets are fixed screen size, so on a 1200×700 image the clusters overlap at any realistic
zoom. They still overlap at scale 1.0, because controls are ≥150 px wide but placed 60–125 px apart.
Seen in the browser:
- **gas-station and retail top row** (y 80–135):
  - `w-dali-wc` (90,80), `w-dali-staff` (215,80) and `w-ac-crew-mode` (300,80) overlap each other
    and cover `w-co2` (360,85). retail has `w-ac-wc-set` (160,120) on top of `w-wc-leak`.
  - `w-ac-mgr-set` (505,85) sits under `w-fuses` (440,85).
  - `w-co2` / `w-fuses` overlap `w-energy` / `w-peak` vertically (only 50 px apart; a circle plus
    its title is about 100 px).
  - The room labels "Personal", "Technikraum", "Trockenlager" and "WC" (y=150) are covered by
    the circles at y 130–135.
- **gas-station Tankanlage**:
  - `w-pump-release` (950,215) covers the "Tankanlage" label and both tank titles.
  - `w-tank-super` / `w-tank-diesel` (y 170) cover the "Super" / "Diesel" captions.
- **restaurant**:
  - `w-ac-kit-act` (215,230), `w-ac-kit-set` (290,230) and `w-ac-kit-mode` (350,230) are stacked.
  - `w-dali-kitchen` (330,160) sits over "Ausgabe".
  - `w-dali-dining` (500,110) overlaps `w-ac-din-set` (600,140), `w-ac-din-mode` (670,140) and
    `w-ac-din-act` (740,140).
  - `w-order-occ` (860,100) and `w-orderpanels` (920,100) overlap, only 60 px apart.
  - `w-kit-kwh` / `w-kit-peak` (60/130, 250) cover "Küche".
  - `w-crew-occ` / `w-support-occ` cover "Personal".

**Fix:** pick one of these:
- (a) Double the image to 2400×1400 (same drawing scaled ×2, x/y ×2). Fixed-size widgets then get
  twice the room.
- (b) Re-place the widgets with a minimum spacing of about 170 px between controls, about 110 px
  between circles horizontally and about 120 px vertically. Move the controls into the empty
  areas (Verkaufsraum, Tankstellendach, Gastraum, Drive-Through).

Also move the room labels to the top-left corner of each room, where the circles do not reach.
Re-check in the plugin at 100 % zoom.

---

## Medium

### 4. `library/demos/restaurant.json` `scenarios[id=kitchen-fault]`: the timeout hides `ac_kit_act = 32`
This is the same pattern as #2. `slaveTimeout klima_kueche` stops the readout, so the 32 °C value
never reaches Cumulocity. On top of that, `w-ac-kit-act` has no `ranges`, so even a sent 32 °C
would not turn red. The alarms are still visible: grill, fryer, fuse, and the timeout plus
`ac_kit_alarm` through `autoAlarms`.
**Fix:**
- Drop the `slaveTimeout` step, or drop `ac_kit_act`, and keep `slaveAlarm ac_kit_alarm`.
- Give `w-ac-kit-act` and `w-ac-din-act` ranges, e.g. `<26 good`, `26–30 warning`, `≥30 alarm`.

### 5. `library/items/energy.json` `items[id=en_kit_peak].generator.points` and `library/floorplans/restaurant/ground-floor.json` `widgets[id=w-kit-peak].ranges`: the floorplan shows an alarm during normal operation
The profile reaches 28 A at 11:00, 12:00 and 17:00, and the widget's `alarm` range starts at 28
(min is inclusive). So on a normal day the "Spitzenstrom Küche" widget turns red at lunch and in
the evening, with no fault active.
**Fix:** either move the range (warning 24–30, alarm ≥30) or cap the profile at about 25 A. Keep
≥30 for `lunch-peak` (which sets 30).

### 6. `library/floorplans/*/ground-floor.json`: lighting cannot be seen on any floorplan
- There is no DALI lamp sensor widget in any of the three floorplans. DALI control widgets show
  only the group mode, which comes from `lw_daliGroupChange`. A scenario that sets lamp
  brightness sends no group event.
- So `night` (lamps 80→10 %, restaurant 5 %) changes nothing visible on the floorplan except lux
  and occupancy.
- None of the 13 native lamp alarms (`lw_lampFailure_1_NN_*`, `lw_converter_1_NN_lost`, …) is
  watched by any widget.

For a Lichtwart demo, lighting is the main story.
**Fix:** add 1–2 lamp sensor widgets per DALI group, e.g.:

```json
{"kind":"sensor","measurement":{"itemId":"light-11"},"icon":"light-bulb","decimals":0}
```

Their `autoAlarms` (default true) bring in the lamp's native alarms. Place them in the room the
lamp belongs to; the Totem box (gas-station / retail) is currently empty.

### 7. `library/demos/*.json` `scenarios`: no scenario shows a lighting fault
The engine supports `fault` targets `daliLamp {lamp, alarmType}` and `daliBus {alarmType}`, but
no shipped scenario uses them. Every fault scenario is about cooling, kitchen or EV. The
lamp-failure story has to be clicked together by hand in the fault menu.
**Fix:** add a scenario `lamp-fault` ("Leuchtenausfall") to each demo. Example for gas-station /
retail:
- `{"kind":"fault","target":{"kind":"daliLamp","lamp":11,"alarmType":"lw_lampFailure_1_11_generic"},"active":true}`
- and for example `lw_converter_1_13_lost` on lamp 13.

Undo both in `normal` (see #1). The floorplan widgets from #6 make it visible.

### 8. `library/demos/gas-station.json` `daliGroups[id=1]` and `library/items/dali-lights.json`: the gas station has no canopy lights
- Group 1 is named "Tankstellendach" (canopy) but contains `light-1` "Totem Licht" and `light-3`
  "Eingangsschriftzug".
- The floorplan control `w-dali-canopy` "Dach und Totem" sits on the canopy, which has no lamps.
- The inventory's gas-station reasoning assumed "canopy-like outside lights".

**Fix:**
- Add `light-15..light-18` titled "Tankstellendach 1–4" / "Canopy 1–4" with tags `gas-station`,
  `category:lighting`, `zone:outside`. Lamps 15–64 are free; see #26.
- Make group 1 "Tankstellendach" = [15,16,17,18] and group 5 "Werbung" (signage) = [1,3].
- Update `normal` / `night`, the floorplan, and the AGENTS.md table ("13 of 18 / 5").

### 9. `library/demos/{gas-station,retail}.json` `daliGroups[id=2]`: the sales floor has only vestibule lights
"Shop" / "Verkaufsraum" = [11,12] = "Vorraum Nord" and "Vorraum Ost". A store has no actual
sales-floor lighting, which looks thin in a lighting-company demo.
**Fix:**
- Add for example `light-19..light-22` "Verkaufsraum 1–4" / "Sales floor 1–4" (tags `retail`,
  `gas-station`) to group 2.
- Optionally move the vestibules into their own group "Eingang".

### 10. `library/demos/restaurant.json` (selection via the `restaurant` tag): the davaco restaurant lost its cold chain but keeps the fuses for it
D4 says "the prototype's davaco items are the restaurant demo". The prototype had Freezer & Cooler
(`freezer_temp`, `cooler_temp`, `freezer_occ`, `cooler_occ`, the `en_frz_*` / `en_clr_*`
meters). With the inventory's tag rule, `restaurant` selects none of them. Yet it does select
`fuse_freezer`, `fuse_cooler`, `fuse_dry_storage`, `fuse_restroom` and `fuse_vestibules`, which
protect equipment the demo does not have.

A quick-service restaurant without freezer temperatures is also a weak demo: HACCP cold-chain
monitoring is the most convincing restaurant use case.

**Fix:**
- Add `restaurant` to the tags of the 6 `kuehlung` registers and of `freezer_occ` / `cooler_occ`.
  DI 4 and 5 are free in this demo.
- Add DI entries 4 and 5 and a freezer/cooler widget in the floorplan's Trockenlager/Kühlung
  area.
- Add a cold-chain fault to `kitchen-fault`, or as its own scenario.
- Alternatively, drop `restaurant` from the fuse tags. Either way, update inventory §3 and the
  AGENTS.md counts.

### 11. `library/floorplans/*/ground-floor.json`: important alarms are missing from the floorplans
- **`dry_leak_alarm`** (Trockenlager: Wasseraustritt) is not referenced by any widget in any
  demo. The DI widgets `w-dry-leak` have no alarms of their own (`autoAlarms` adds nothing for a
  DI).
  - **Fix:** add `"alarms":[{"kind":"item","itemId":"dry_leak_alarm"}]` to `w-dry-leak` in all
    three floorplans.
- **restaurant `w-fuses`** lists 16 fuses but leaves out `fuse_freezer` and `fuse_cooler`, which
  are selected in this demo.
  - **Fix:** add them, or remove the items from the demo (#10).
- **gas-station / retail**:
  - The `klima_wc` / `klima_trocken` / `klima_manager` / `klima_crew` timeouts and `ac_*_alarm`
    are not watched. Only control widgets exist for AC, and controls carry no alarms.
  - The `energie_innen` and `energie_aussen` slave alarms (`en_cms_alarm`, `en_wc_alarm`,
    `en_sign_alarm`, `en_totem_alarm`, …) are not watched either.
  - **Fix:**
    - add one `alarm` widget "Klima" with
      `alarms:[{kind:"item",itemId:"ac_wc_act"},{kind:"item",itemId:"ac_dry_act"},{kind:"item",itemId:"ac_mgr_act"},{kind:"item",itemId:"ac_crew_act"}]`
      (each item ref brings in that slave's timeout and all its alarms);
    - add one "Unterverteilung" (sub-distribution) widget with `en_cms_kwh`, `en_sign_kwh`.
- **restaurant**: the `energie_aussen` alarms are not watched.
  - **Fix:** add `{kind:"item",itemId:"en_sign_kwh"}` to an energy alarm widget.

### 12. `library/items/fuel.json` `items[id=tank_super|tank_diesel].generator`: tank levels swing absurdly, and no scenario uses the fuel system
- `randomWalk {spread 15, step 1}` steps every 5 s. A fuel tank level then moves up and down
  ±12 %/min and runs across its whole 50–80 % band within minutes. On a chart this looks like
  noise; a real tank drains slowly and jumps up on delivery.
- The floorplan ranges (`<15` alarm, `15–30` warning) are never reached.
- No scenario raises `tank_low` / `tank_leak` or locks `pump_release`, so the gas station's
  unique part is never demonstrated.

**Fix:**
- Use `randomWalk {center 65, spread 15, step 0.05}`, or a `sine {period 86400, amplitude 15, center 60}`.
- Add a scenario `tank-fault` ("Störung Tankanlage") to gas-station:
  - `tank_diesel` = 12;
  - `slaveAlarm tankanlage tank_low` active;
  - optionally `tank_leak` + `pump_release` = 0.

  Undo it in `normal`.

### 13. `library/items/digital-inputs.json` `items[id=restroom_leak|dry_leak]` and `library/demos/*.json`: water-leak inputs raise nothing, and no scenario uses them
Switching "Wasser WC" on turns the DI widget red, but it raises no Cumulocity alarm. `restroom_leak`
has no alarm at all. `dry_leak_alarm` is a separate custom alarm that has to be fired by hand.
**Fix:**
- Give both DI items an `effects` alarm, e.g.
  `{"kind":"alarm","type":"lw_demo_restroom_leak","severity":"MAJOR","text":"WC: Wasseraustritt","threshold":1}`,
  and the same for `dry_leak` (or retire `dry_leak_alarm`, which is then redundant).
- Use the leak in a scenario, e.g. "Wasserschaden" (water damage).
- Check that the effect alarm types are written into the widget's alarms.

---

## Low

### 14. `library/items/energy.json` `slaves[key=energie_innen].alarms`: shared slave alarms for registers that are not selected
gas-station and retail select only `en_cms_*` / `en_wc_*` from `energie_innen`, but the slave still
carries `en_kit_alarm`, `en_ord_alarm` and `en_din_alarm` ("Energie Küche / Bestellung /
Gastraum: Störung"). restaurant carries `en_wc_alarm` without the WC meter.

Effects:
- **Fault menu**: the gas-station fault menu offers a kitchen energy alarm.
- **Floorplan, restaurant**: `w-kit-kwh` gets `en_wc_alarm` in its alarm types through
  `autoAlarms`. Harmless, but noise.
- **Floorplan, gas-station / retail**: these alarms are not on the floorplan, because no widget
  references `energie_innen`.

All other slaves are either fully selected or fully unselected per demo, so `energie_innen` is
the only case.
**Fix:**
- Split the slave into `energie_shop` (cms, wc) and `energie_kueche` (kit, ord, din): 14 slaves,
  at most 13 per demo.
- Or have the selection drop slave alarms whose `name` is an item id that is not selected
  (engine change).

### 15. `library/items/energy.json` `items[id=energy_total].generator.ratePerHour`: the total counts slower than its sub-meters
- gas-station / retail: the selected sub-meters add up to 27.3 kWh/h (frz 3 + clr 2 + cms 1.2 +
  wc 0.6 + sign 1.5 + totem 1 + ev 5/6/7), but the total meter counts 25 kWh/h.
- Sub-meters start at 1 kWh while the total starts at 1000.
- Anyone who compares the numbers sees that they do not add up.

**Fix:**
- Set `energy_total` to about 40 kWh/h, which covers the sub-meters plus unmetered load; with
  restaurant at about 10 kWh/h metered, 40 is still plausible.
- Give the sub-meters realistic start values, e.g. `control.default` 2000–15000.

### 16. `library/items/{energy,cooling,ev-charging}.json` `generator.dailyProfile`: one generic day shape for all three businesses
- Every peak-current profile is the same plateau (07:00–18:00 flat, a small dip at 14–15).
- A drive-through kitchen (`en_kit_peak`, `en_din_peak`, `en_ord_peak`) should show a lunch peak
  (11:30–13:30) and a dinner peak (17:30–20:00).
- A gas station's shop should show commuter peaks (07–09, 16–18).
- The three EV chargers have **identical** profiles (three overlapping lines on a chart), and they
  draw 5–6 A at night while `ev_slot_*` = 0.

**Fix:**
- Shape the restaurant profiles with two peaks.
- Vary the EV profiles: offset them by 1–2 h and scale them 0.8 / 1.0 / 1.1.
- Set the night EV current to 0–1 A.

### 17. `library/demos/*.json` `scenarios[id=normal]`: occupancy is always "Frei"
DIs cannot have generators, and `normal` sets no DI. So by day every occupancy widget (Personalraum,
Ladeplatz, Bestellung, Tiefkühler belegt) shows "Frei". The demo looks empty.
**Fix:** in `normal`, set a daytime pattern, e.g. `crew_occ` 1, `ev_slot_2` 1, `order_occ` 1.

### 18. `library/demos/*.json` `scenarios[id=normal]` and `daliGroups[id=1]`: outside lights at 80 % in daylight
`normal` sets totem / lettering / drive panel to 80 % while `lux-1` reads up to 33 000 lx. Group 1
has `luxThreshold 5000` + `automaticSwitch true`, but the engine does not simulate lux switching
(there is no hit for `automatic_switch` / `lux_threshold` in `src-tauri/src/sim`).
**Fix:**
- Set the outdoor lamps to 0 (or about 30 for signage) in `normal`. `night` already sets 100.
- Or note in the demo description that the lux logic is not simulated.

### 19. `library/items/hvac.json` `items[id=ac_*_act].control`: undocumented change from the inventory
- The inventory row 17 etc. says `0..80 step 1, int16`; the library has `0..40 step 0.5, float32`.
- 32–176 °F converts to 0–80 °C. For a room-temperature sensor 0–40 is more sensible, and
  `kitchen-fault` (32) still fits.
- Same for freezer / cooler: inventory int16 step 1 → library float32 step 0.5.

**Fix:** none in the library. Note the deviation in `prototype-inventory.md` §2.4, or in the PR
text.

The other °F→°C conversions are correct:
- setpoint 61–75 °F → 16–24, default 70 °F → 21;
- actual default 72 °F → 22;
- freezer −4..32 °F → −20..0, default 0 °F → −18;
- cooler 23..61 °F → −5..16, default 38 °F → 3.

All units are ≤7 bytes and metric: `°C` (3 bytes), `kWh`, `A`, `%`.

### 20. German / English titles (`library/items/*.json` `title`, floorplan `title`)
German:
- `light-5` "Gastraum Raum" → "Gastraum Decke".
- `light-6` "Küche Licht" → "Küchenlicht"; `light-7` → "Spülküchenlicht". The "X Licht" pattern
  reads like a translation.
- `order_occ` "Bestellung belegt" → "Bestellsäule belegt".
- `restroom_leak` "Wasser WC" → "Wassermelder WC"; `dry_leak` → "Wassermelder Trockenlager".
- Floorplan `w-dry-leak` "Wasser Trocken" → "Wasser Lager".
- `fuse_drive` "Sicherung Drive" → "Sicherung Drive-In".
- "Support" / "Presenter" are kept English in German. That is acceptable QSR jargon, but be
  consistent.

English:
- Capitalisation is mixed: "Fuse Kitchen", "Fuse Dry storage", "Energy Kitchen" vs "Energy
  freezer", "Peak current Outside signage". Use sentence case everywhere ("Kitchen fuse",
  "Kitchen energy").
- `restroom_leak` "Restroom water" → "Restroom water leak".

Umlauts are used correctly in all German titles and texts (Kühlzelle, Störung, Außen, Ölwechsel).

### 21. `library/floorplans/*/ground-floor.json` `widgets[].locked`: no widget is locked
`locked` is missing everywhere, and the plugin defaults it to `false`. A presenter who clicks
around can drag widgets, and a save then persists the change.
**Fix:** set `"locked": true` on all sensor and alarm widgets. Controls can stay unlocked if
preferred.

### 22. `library/floorplans/*/ground-floor.json` DI sensor `title`s override the DI config title
Examples:
- `w-ev-1..3` "Ladeplatz N" (config: "Ladeplatz N belegt");
- `w-dry-leak` "Wasser Trocken" (config: "Wasser Trockenlager");
- `w-crew-occ` "Personalraum".

AGENTS.md: a literal title means later DI title edits never reach the floorplan.
**Fix:** drop `title` on the DI widgets and let the config title flow through, or shorten the DI
item titles themselves.

### 23. `library/floorplans/*/ground-floor.svg`: drawn equipment without a widget
These boxes in the drawings have no widget:
- gas-station: "Säule 1–4" (pumps), "Totem";
- retail: "Totem";
- restaurant: "Flurry" (its alarm sits on the "Fritteuse" widget `w-fry`), "Drive-Panel" (lamp 2),
  "Spülküche" (lamp 7), "Ausgabefenster";
- the empty `<rect x="1060" y="460" …>` in the restaurant drawing has no label.

**Fix:**
- Put lamp sensors on Totem / Drive-Panel (#6).
- Split `w-fry` into Fritteuse (`fry_oil_exchange`) and Flurry (`flurry_cleaning`), the latter at
  (325,90).
- Remove or label the stray rect.

### 24. `library/floorplans/*/ground-floor.json` icons: valid, but could be clearer
All icon names used are valid in the plugin's icon set (`@c8y/style` `dlt-c8y-icon-*` and the icon
selector lists): map, light-bulb, sun-o, snowflake-o, thermometer-half, user, tint,
air-conditioner, fire, rotate-right, air-quality, bell, bolt, plug, warning, power-off.

Clearer choices:
- `car` for the EV slots (instead of `user`);
- `charging-battery` for `w-ev-alarm` (instead of `bolt`, which `w-energy` already uses);
- `gas` for the tanks and `w-tank-alarm` (instead of `tint` / `warning`; `tint` also stands for
  water leaks);
- `fan` for the AC mode controls.

**Fix:** optional icon swap.

### 25. `library/items/{cooling,energy,ev-charging,hvac,fuel}.json` `slaves[].name`: ASCII slave names next to umlaut group names
Slave names are ASCII ("Kuehlung", "Ladesaeule 1", "Klima Kueche", "Energie Aussen"), while the
DALI group names use umlauts ("Außen", "Küche"). The slave name is visible in the timeout alarm
text `Modbus 20-240 Timeout Error Kuehlung`.

The firmware limit is a 20-byte buffer, i.e. 19 bytes (`MBM_modbus_master.h:30-34`), and the
umlaut forms fit: Kühlung 8, Ladesäule 1 12, Klima Küche 12, Energie Außen 14 bytes. But
`firmware-core-v2.md` says nothing about non-ASCII handling in the parser or display (UNVERIFIED).
The PRD examples are ASCII.

**Fix:** keep ASCII, which is the safe choice for a config that real Cores could also read, and
state the rule in AGENTS.md ("slave names ASCII; titles and texts with umlauts"). Switch only
after it has been checked on hardware.

---

## Info

### 26. `library/items/dali-lights.json` `items[id=light-15..light-64]`: lamps without a client tag are fine
They are never selected by tag, only through `includes` or custom demos, and their generic titles
("Licht 15") are fine for that. They are the natural place for the canopy and sales-floor lamps
of #8 and #9.

### 27. `library/demos/{gas-station,retail}.json` `daliLampCount: 14` with only 9 lamps grouped
Lamps 2 and 4–7 exist on the bus but are in no group. The engine does not send them
(`lamp_sendable`, `core.rs:445-450`), exactly like the firmware (FW §1.2). Correct. If #8/#9 add
lamps, raise `daliLampCount` to match.

### 28. Migration completeness: complete
All 135 prototype rows are covered:

| Prototype items | Count | Where they are |
|---|---|---|
| Lights | 14 | `dali` items `light-1`…`light-14` |
| Native lamp alarms | 13 | implicit with the DALI items |
| Outside brightness + its sensor-lost alarm | 1 + 1 | `daliBrightness` item `lux-1` |
| Binary inputs | 10 | digital inputs, DI 01–10 |
| Modbus registers | 46 | 13 slaves |
| Slave alarms | 22 | slave `alarms[]` |
| Device-level alarms | 28 | `customAlarm` items (10 equipment/safety + 18 fuses) |

The 6 AC modes are multi-effect items (event `lw_demo_ac_*_mode` + inventory property; a
documented break from the prototype's event type).

Extra items not in the prototype: `rssi`, `board_temp`, the fuel slave (3 registers, 2 alarms) and
`light-15..64`. No prototype item is missing, and the client tags match the inventory 1:1.

### 29. Checks
- `npm run validate:library`: 25 files, 0 invalid.
- `cargo test --lib library::`: 34 passed (including `loads_repo_library` with no warnings).
- The AGENTS.md "Shipped demos" table matches the computed selections:
  - gas-station 72 items, 12 slaves;
  - retail 69 items, 11 slaves;
  - restaurant 66 items, 5 slaves;
  - lamps / groups / DIs 9/4/9, 9/4/9, 10/4/5.

  All slaves have ≤ 10 registers.

---

**Count:** High 3 · Medium 10 · Low 12 · Info 4.


## Resolution

All 29 findings are handled (library validation: 0 invalid; `cargo test --lib library::` 34 passed, no load
warnings; `npx biome check library` clean). Changes are in `library/items`, `library/demos`,
`library/floorplans` (JSON and SVG) and `AGENTS.md`.

- **1, 17, 18**: every `normal` is now generated as a full reset: generator step for each item another scenario
  touches, `active:false` for every fault, daytime DIs (crew 1, EV slot 2 = 1, order 1), outdoor lamps 0 / 30 %
  in daylight, 100 % at night. Proposed rule (not implemented): `engine-requests.md`, "Validation proposals" 1.
- **2, 4**: `cooling-fault` and `kitchen-fault` no longer time out the slave they show. New `comm-loss`
  scenarios (Kühlung / Klima Küche). `w-ac-kit-act` / `w-ac-din-act` got ranges (<26 / 26-30 / ≥30).
- **3**: all three floorplans were re-laid out and redrawn (1680-1810 x 1074-1177 px) as room cells sized for the
  plugin's real widget sizes (circle 96 x ~100, control ≥150 x 36), labels top-left under which widgets start,
  nothing overlaps. Rendered with real widget rectangles in headless Chrome and checked by eye. Note: widgets
  do not scale with the image, so overlap-freeness holds at image scale ≥ ~0.8; below that (small panel) the
  plugin's zoom is the user's lever. Doubling the image does not help at fit-zoom, so it was not used.
- **5, 16**: `en_kit_peak` capped at 27 A with lunch and dinner peaks, widget ranges 24 / 30; restaurant profiles
  with two peaks; commuter shape for `current_peak_total`; EV profiles offset and scaled, ≤1 A at night.
- **6, 7, 23**: lamp sensor widgets in every room/group (light-1..22 as relevant), Totem / Drive-Panel covered
  by lamp sensors, `lamp-fault` scenario in all demos (`daliLamp` generic failure + converter lost). Flurry
  split from Fritteuse. The stray rect was dropped with the redraw.
- **8, 9, 27**: gas-station: lamps 15-18 canopy (group 1 Tankstellendach), group 5 Werbung (1, 3), group 6
  Eingang; lamps 19-22 sales floor in group 2 (also retail, group 5 Eingang); `daliLampCount` 22.
- **10, 11**: restaurant now has the cold chain (`restaurant` tag on the six `kuehlung` registers, `freezer_occ`,
  `cooler_occ`, DI 4/5, widgets, `cooling-fault`, fuses in `w-fuses`) and `en_wc_*`. New "Klima" and
  "Unterverteilung" alarm widgets watch AC and sub-meter slave alarms; `w-dry-leak` / `w-wc-leak` watch the
  leak alarms. **Inventory §3 must be updated** (restaurant now selects the freezer/cooler items).
- **12**: tank walk step 0.05; `tank-fault` scenario (diesel 12 %, tank_low + tank_leak, pumps locked).
- **13**: `restroom_leak` and `dry_leak` have an alarm effect (`lw_demo_restroom_leak`,
  `lw_demo_dry_leak_alarm`); `dry_leak_alarm` custom item retired (same alarm type). `water-damage` scenario.
  Effect alarms are not in `autoAlarms` (engine request 3), so the widgets list them as `type` refs.
- **14**: `energie_innen` split into `energie_shop` (cms, wc) and `energie_kueche` (kit, ord, din).
- **15**: `energy_total` 40 kWh/h, start 90000; sub-meter starts 1500-12400.
- **19**: no library change; the 0-40 °C float32 deviation from inventory §2.4 is intentional (to be noted in
  the inventory).
- **20, 21, 22, 24, 25**: titles fixed (German and English sentence case, "Drive-In"), sensor/alarm widgets
  `locked`, DI widgets take the config title, icons car / charging-battery / gas / fan; slave names stay ASCII
  (rule added to AGENTS.md).
- **26, 28, 29**: nothing to change; AGENTS.md "Shipped demos" updated: gas-station 17 of 22 lamps / 6 groups /
  9 DI / 79 items; retail 13 of 22 / 5 / 9 / 72; restaurant 10 / 4 / 7 / 75 (slaves 12 / 11 / 7).
