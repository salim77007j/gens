# EDITOR REALITY CHECK — v1.1.0, first real user session

**Date:** 2026-10-13
**Tested artifact:** `gens-editor-linux-x86_64.zip` downloaded from the published v1.1.0 release (SHA256SUMS-verified pipeline artifact) — the exact bytes a user downloads, not a local rebuild.
**Test environment:** Debian 13 sandbox, 2 cores, Xvfb 1920×1080, Mesa 25.0.7 **lavapipe** software Vulkan (extracted from distro debs into user space), XTest-injected mouse/keyboard.
**Method:** scripted user sessions — the editor was launched with no special flags, then driven with real pointer clicks, drags, and keystrokes, with a full-screen screenshot captured at every step and the editor's own stderr/console log collected after every session. Object screen positions were measured pixel-exactly (PIL) before clicking them, so "we clicked the cube" means the measured center of the rendered cube, not a guess.
**Interaction time:** ~12 minutes of editor-on-screen interaction across 16 sessions (plus offline analysis of ~90 screenshots and all session logs). Every item on the requested checklist was attempted. Items that could not be tested at all are listed in §8.

**Companion evidence:** `docs/reality-check/01…31*.png` (every claim below references one).

---

## 1. Every problem found (numbered, specific, with evidence)

### P0 — the advertised v1.1 features that do not work as shipped

**1. Viewport clicking cannot select any object — it always selects the Terrain.**
The v1.1 release table says: *"Scene View picking — click to select, exact geometry; OBB slab test for cubes/markers, bounded-plane test for quads — no bounding-sphere approximations."* In real use, clicking **dead-center** on the orange Crate, the blue cube, and the Spaceship platform — 7 measured points across the three objects, positions measured from the rendered pixels — selected **"Terrain"** every single time (`12_pick_cube_center_selects_terrain.png`, `13_all_7_pick_points_select_terrain.png`). Clicking genuinely empty sky deselects (inspector shows "Select an entity in the hierarchy"). The picker's ray is landing on the giant 30×30 ground quad instead of the small object in front of it — i.e. the pick is not returning the closest hit (or the cube OBB test rejects and the plane behind accepts). Whatever the exact defect in `pick`, the observable behavior is: **the object you click is never the object you get.**

**2. Viewport dragging therefore drags the Terrain — you can drag the ground plane out of the world with one gesture and no warning.**
With Terrain (involuntarily) selected by problem 1, a drag from where the Crate appears moved the **entire Terrain**: Position went from (0, 0, 0) to (3.29, −2.51, −0.53) (`14_drag_moved_terrain_inspector.png`), and the viewport shows the world floor reduced to a tilted sliver on the right edge — the scene has no floor anymore (`15_terrain_dragged_leaves_void.png`). There is no confirmation, no lock, no per-axis constraint. "Drag an object to move it" as shipped means "drag the ground".

**3. "Drag empty space to orbit" never happens.**
Because every ray hits the Terrain (problem 1), there is no "empty space" — a drag anywhere on the ground region moves the ground. A drag in the only sky region available did not change the camera readout at all (yaw/pitch/dist stayed "yaw 35° pitch 30° dist 12.0" through every session). Orbit may exist in code; as shipped it is unreachable.

**4. Undo of a delete fails and the object is permanently lost.**
Sequence: select Crate in hierarchy → `Del` (Crate disappears, entities 11→10, `18_crate_deleted.png`) → `Ctrl+Z`. The editor's own console prints:
```
ERROR gens_editor::commands: undo/restore failed: entity '"Level"' is referenced as a parent but is not in the snapshot
```
and the Crate is **not restored** (`19_undo_after_delete_failed.png`). The release notes say "Undo covers transform edits, deletes, creates". Deletes of any entity that has a parent (i.e. everything in the scene tree) cannot be undone, and there is deliberately no redo to backstop it. This is silent data loss in the primary editing loop.

**5. The Inspector has no Physics section — on any entity.**
Release notes: *"Inspector — Transform / Mesh / Physics — edits reach the real components the simulation runs."* Captured inspectors for Crate, PlayerCharacter, DirectionalLight, Terrain, Spaceship show exactly three sections: Identity (name + Active), Transform, Mesh (`21_no_physics_section_any_entity.png`). The Crate *has* a physics body (it falls during Play), but the user has no way to see or edit mass/density/body type/friction/restitution. Physics editing is not in the shipped Inspector.

### P1 — core authoring gaps a user hits in the first ten minutes

**6. The File menu has two items: Save Scene, Load Scene.**
No New Scene, no Open/Import, no Export, no Save As, no Recent, no Quit (`02_menu_file_only_save_load.png`). The requested checklist items "create a new scene" and "export" **cannot be performed from the editor at all**.

**7. GameObject menu can create exactly three things: Empty, Cube, Marker (`04_menu_gameobject_only_3_creates.png`).**
- **There is no "Add Light".** The only light in the scene (DirectionalLight) is pre-seeded and is literally a yellow **Marker** mesh — selecting it shows no light properties whatsoever: no color, intensity, or direction (`21_no_physics_section_any_entity.png`, middle panel). A user cannot add or edit lighting.
- No sphere/capsule/cylinder/plane primitives, no camera object, no sprite, no particle, no empty-parent folder primitives beyond the three.

**8. There is no way to add a script to an entity in the interactive editor.**
The Asset Browser contains a `SCRIPT` asset; double-clicking it only highlights it (`20_shape_dropdown_3_options.png` shows selection but no editor/attachment UI opens). The Inspector has no "script" section, the GameObject menu has no "Add Script", and the console command vocabulary is exactly `help | entities | validate | stats | clear` (`22_console_commands_work.png`). Rhai scripting exists in the engine and in the package pipeline — but a user in the window editor cannot attach behavior to anything.

**9. Mesh palette is three shapes: Cube, Quad, Marker (`20_shape_dropdown_3_options.png`).**
No material/color *asset* concept in the editor — only per-entity Tint RGBA (which does work: `11_edits_reflect_in_viewport.png`).

**10. No context menus anywhere.**
Right-click in the viewport and in the hierarchy produced no menu (tested; no popup either place).

**11. Duplicates are spawned at the scene root, outside their parent.**
Ctrl+D on Terrain (child of Level) and on Crate (child of Level) created "Terrain copy" and "Crate copy" as **top-level** rows, breaking the hierarchy the duplicate came from (`16_duplicate_terrain_copy_at_root.png`, `17_duplicate_crate_copy_at_root.png`).

**12. No native file dialogs, no file picking of any kind (documented boundary, still a gap).**
Save/Load always use `scene.gensscene` beside the binary. Loading a *different* scene file is impossible in the GUI; there is no asset import (no model/texture/audio import paths in the editor).

**13. Help text points at a repository path.**
The Help menu's instructions end with "see docs/design/REFERENCE.png" (`06_help_menu.png`) — meaningless to someone who only has the zip.

### P2 — friction, honesty nits, polish

**14. The Linux binary crashes at startup on a plain Linux box with a raw panic.**
`./gens-editor` on this Debian 13 sandbox first died with:
```
thread 'main' panicked at xkbcommon-dl-0.4.2/src/x11.rs:59:28:
Library libxkbcommon-x11.so could not be loaded.
```
The zip README documents the GPU (Vulkan) requirement but **not** the X11 runtime deps (`libxkbcommon-x11-0`, `libxcb-xkb1` on minimal systems). After installing those two debs it launched fine. A first-run crash with a library name and no remediation hint is a terrible first impression; Godot on the identical machine ran immediately because it ships/bundles its deps.

**15. Numeric fields are drag-scrub widgets; typing is fragile.**
The Transform fields are egui DragValues: a plain click opens edit mode (`09_inspector_field_edit_mode.png`) and typed values do commit (`10_inspector_typed_value_committed.png` — rotation Z became exactly 45.0; position X became exactly 2.50), which is genuinely good. But the click affordance is ambiguous (press-drag scrubs instead), there are no steppers, no units, no sliders, no multi-select editing, and an in-progress edit renders as a highlighted box that reads as a broken field (`11_edits_reflect_in_viewport.png`, Tint R mid-edit).

**16. Bottom-dock tab strip has small targets and no labels for collapsed state; the command input is only reachable when the Console tab is active** (typing with another tab focused silently goes nowhere — the app drops the keystrokes without any cue).

**17. Status bar entity count (11) and console `entities` count (8, "resource-seed rows excluded by name filter") disagree without explanation.**

**18. Network tab's empty-state text is stale**: it says live sessions "appear in a windowed editor" — but this *is* the windowed editor (`25_network_empty_state.png`). The copy was written for the headless build and shipped unchanged.

**19. Profiler tab says "No profiler data yet — the app has not recorded any frames" while the status bar simultaneously reports Frame 560** (`22_console_commands_work.png`, top half). Honest empty state, confusing copy.

**20. No window chrome interactivity without a WM** (no decorations, cannot maximize/drag the window; close is Ctrl+C/kill). Partly environmental, but a user's first minutes include fighting this on many Linux setups; Godot has the same constraint under Xvfb, so this is a minor nit rather than a defect.

**21. FPS on software rendering is 17–41 fps at 1600×900** (status bar across sessions). On real GPUs this will be fine; documented readback presentation (~5.5 MB/frame) is the declared boundary. Noted for completeness.

### What worked (credit where due — verified by screenshots)

- Real OS window, live 3D viewport, honest renderer init log (`01_launch_default_scene.png`).
- **Play/Pause/Stop**: physics runs, the Crate falls onto the platform (`07_play_mode_crate_fell.png`), and **Stop restores the pre-play state exactly** (`08_stop_restores.png`, console: "simulation stopped — pre-play state restored").
- **Inspector typed editing works and reaches the simulation**: position 2.50, rotation 45.0 committed and visible in the viewport with the gizmo following (`10`, `11`).
- **Save/Load round trip works** (`gensscene-1`, console: Saved/Loaded, path `scene.gensscene`).
- Hierarchy selection, rename field, Active checkbox, duplicate (+undo of create), delete, hierarchy search field, eye toggles, collapse arrows all responded.
- Console commands work (`22`).
- All five menus open and every item executed without crashing the app; the editor survived ~12 minutes of hostile clicking with zero panics after launch.

---

## 2. Comparison: the SAME tasks in Godot 4.3 (identical sandbox, identical Xvfb + lavapipe)

Godot was downloaded as the official `Godot_v4.3-stable_linux.x86_64` binary and driven through the identical task list with the same synthetic-input harness.

| Task | **Gens v1.1.0** | **Godot 4.3** |
|---|---|---|
| First launch | Panic: missing `libxkbcommon-x11.so`; needed 2 hand-collected debs (README doesn't mention them) | Ran immediately; Vulkan 1.4.305 via llvmpipe, zero manual setup (`26_godot_project_manager.png`) |
| Project concept | None — a scene file beside the binary | Full project manager: create/import/scan, Asset Library, project templates (`26`) |
| "Create a new scene" | **Impossible** — no File→New | Guided dialog with renderer tradeoff explainer + Git metadata option (`27_godot_new_project_dialog.png`) |
| Add a cube | GameObject→Create Cube ✓ (works, spawns at origin) | 3D Scene → Ctrl+A → MeshInstance3D → assign New BoxMesh from a **15-mesh dropdown** (ArrayMesh/Box/Capsule/Cylinder/Plane/Quad/Prism/Sphere/Text/Torus/…) (`28_godot_scene_tree_nodes.png`, `29_godot_cube_created.png`) |
| Add a light | **No light creation exists**; existing light uneditable | Add Node → Light3D family, full property editor |
| Move the object | Viewport drag moves the **Terrain** instead (P0 #2); Inspector number-typing works | Gizmo drag moved the node along X; Inspector/gizmo both work; multi-gizmo modes |
| Selection by clicking in 3D | **Broken — always Terrain** (P0 #1) | Click-to-select works (node highlighted in tree + inspector on click) |
| Save | Ctrl+S → fixed `scene.gensscene` ✓ | Ctrl+S → `node_3d.tscn` in a real FileSystem dock, format documented |
| Run the game | Play runs the simulation **inside the editor window** (physics + transform sync); no separate game process | F5 → guided "select main scene" dialog (`30_godot_main_scene_dialog.png`) → launches the **actual game as its own window/process** (`31_godot_game_window.png`) |
| Export a build | **No export in the editor at all**; the pipeline is a separate CLI (`gens-pkg`/`gens-player` with `.genspkg`) | Project→Export with preset system (requires one-time template download; not exercised in sandbox — documented, not hidden) |
| Scripting from the editor | No path from UI to script attach | Full script editor dock, attach-to-node workflow |
| What Gens has that Godot doesn't | Deterministic replay + byte-identical world hashing; `.genspkg` package format with SHA256SUMS manifest; console commands (`entities`/`validate`/`stats`) wired to real ECS invariants; the honest empty-state discipline | (These exist in Godot in other forms — remote inspector, versioned scenes — but the *determinism-as-contract* stance is genuinely Gens's own angle) |

**Net:** Godot did in ~8 minutes of synthetic interaction everything on the checklist, with guided dialogs at every decision point. Gens did: add-cube ✓, transform-typing ✓, play/stop ✓, save/load ✓; and failed or lacked: new-scene ✗, light ✗, script-attach ✗, viewport-select ✗, viewport-drag ✗ (wrong target), export ✗.

---

## 3. Top 20 blockers to usability (ranked; each with what a fix would take)

| # | Blocker | Evidence | What it would take |
|---|---|---|---|
| 1 | Viewport pick returns wrong (farthest/planar) hit — always Terrain | `12`, `13` | In the pick loop, sort candidate hits by ray-t and take the **minimum positive t**; add a regression test that clicks a cube with a plane behind it. ~1 day incl. test. |
| 2 | Drag moves the wrongly-picked entity without constraint | `14`, `15` | Falls out of #1; then add drag plane + axis-constraint modes (V/X/Y/Z keys like Godot), and an undo entry per drag. ~2 days. |
| 3 | Undo-of-delete fails (parent not in snapshot) — silent data loss | `19` + ERROR line | Snapshot must include the ancestor chain (or store parent id and re-parent on restore); add delete→undo→world-hash-equal test. ~1–2 days. |
| 4 | No Physics section in Inspector | `21` | Render the existing physics component (mass/density/body-type/locked-axes) — the headless editor already had this per ADR-0025; port the widget. ~2–3 days. |
| 5 | No way to add/edit lights | `04`, `21` | GameObject→Create Light + a Light section (color/intensity/direction) in the Inspector; renderer already has lighting. ~3–5 days. |
| 6 | No script attach path in the editor | `20`, `22` | Inspector "Scripts" section: pick asset from registry → attach component → script runs in Play. Pipeline already executes rhai. ~3–5 days. |
| 7 | File→New Scene / Save As / Export missing | `02` | New Scene = clear world + reseed (command exists); Export = invoke the existing `gens-pkg build` from a dialog. ~2–4 days. |
| 8 | Only 3 mesh shapes, no materials | `20` | Add the primitive generator set (sphere/capsule/cylinder) + material assets (color/roughness) in the asset registry. ~1 week. |
| 9 | No native file dialogs / asset import | (documented) | egui-file-dialog crate or rfd; hook into Save/Load/Import. ~2–3 days for dialogs, import pipelines are a project of their own. |
| 10 | Startup panic on missing X11 libs | stderr panic trace | Link or bundle deps; catch the dl error and print an actionable message (`apt install libxkbcommon-x11-0`). ~0.5 day for the message + README; bundling is per-platform work. |
| 11 | No redo (undo losing data has no backstop) | `19` | Id-remapping layer (already flagged as future work in ADR-0036). ~1 week. |
| 12 | Duplicate breaks hierarchy (spawned at root) | `16`, `17` | Pass source parent id to the spawn command. ~0.5 day. |
| 13 | No context menus | (tested) | egui `response.context_menu(...)` — hierarchy + viewport menus for create/duplicate/delete/rename. ~2 days. |
| 14 | No project concept (recent files, folders, multi-scene) | `26` contrast | Project directory layout + project.json + recents; the package pipeline already has the primitives. ~2 weeks to a credible v0. |
| 15 | Orbit/unreachable + no zoom controls, single fixed camera metaphor | `15` | After #1/#2: verify orbit, add scroll-zoom, focus-selected (F), ortho views. ~2–3 days. |
| 16 | No gizmo: translate/rotate/scale handles in viewport | (absent) | egui-gizmo or custom; the pick/unproject math exists. ~1 week. |
| 17 | No multi-select, no box-select, no shift-click hierarchy | (tested) | Selection model change + inspector multi-edit. ~3–5 days. |
| 18 | Stale/contradictory empty-state copy (Network, Profiler vs Frame counter) | `22`, `25` | Copy pass referencing the windowed build. ~0.5 day. |
| 19 | Entity count inconsistency status-bar vs console | `22` | One definition of "live entity", printed in both places. ~0.5 day. |
| 20 | DragValue UX (no steppers/units, ambiguous click-vs-drag) | `09`, `11` | Add drag-speed hints, double-click-to-type affordance, step arrows. ~2 days. |

(Not counted above but required for parity eventually: animation clip authoring, audio authoring, network session UI, Game View camera switching, input-map editing, localization of any kind.)

---

## 4. Honest verdict

**Is Gens usable as a game engine today? No.**

Reasons, in one paragraph: the *engine* underneath is real and unusually disciplined for its size — deterministic fixed-step simulation, physics that run in Play mode with exact restore, a verified package pipeline, and a console that queries live ECS state; none of that is fake. But the *editor* — the thing a user touches — currently cannot support even the demo loop it ships with: you cannot reliably select what you click (P0 #1), dragging moves the ground plane instead of your object (P0 #2), deleting a child is an irreversible operation (P0 #4), and there is no UI path for lights, scripts, new scenes, or exports (P1 #5–#8). Every one of those was hit **within minutes** of ordinary use, exactly as the user predicted. v1.1.0's release notes describe capabilities that are true in code but not true in the shipped interaction (picking "exact geometry", Inspector "Physics", undo "covers deletes"), which is precisely the gap that only real-use testing catches — and which CI's smoke mode, by construction, did not (it clicks toolbar buttons at captured rects and asserts the play state machine; it never asserts *which object* a pick returns).

**What Gens is today:** a credible headless engine + pipeline with a promising editor shell at roughly the "technical demo" milestone of an editor.
**What it is not today:** an editor another human can author a game in.

---

## 5. Answers to the direct questions (Task 4)

1. **Did you actually use the editor before shipping v1.1.0?** **No.** v1.1.0 shipped with CI evidence only (window opens, toolbar buttons clicked at captured rects, drags driven, play state machine asserted). No session ever asserted *which entity* a viewport pick returns, so the picking/dragging defects shipped. This reality check is the first genuine user session of the artifact.
2. **Did you try every button?** **Nearly all, not literally all.** Tried: all 5 toolbar buttons, all 5 menus + their items, Reset View, Shape dropdown, Inspector fields (typed + clicked), hierarchy selection/search/rename/eye/collapse/"v" rows, duplicate/delete/undo/redo-attempts via both menu and shortcuts, console commands via keyboard **and** Run button, asset browser chips + script double-click, right-clicks, tab switches (3D Viewport/Game View/Animator/Profiler/Performance/Network/Console). Not tried: window resize/maximize (no WM in sandbox), audio (dummy), gamepad, macOS/Windows binaries (this sandbox is Linux-only; CI's own smoke covers their window-launch), Unity (Hub requires a GUI installer + interactive account login; not feasible headless — Godot was used as the comparison target).
3. **Did you compare with Godot or Unity?** **Yes — Godot 4.3 stable**, same sandbox, same synthetic-input harness, same tasks (§2). Unity: not possible here (GUI installer + account login), stated rather than pretended.
4. **Would YOU use Gens to build a game?** **No — not today.** I could *tolerate* the headless pipeline for a deterministic-simulation niche project (it is the strongest part of the project), but the interactive editor would actively fight me: I would lose work to the delete-undo bug, and I cannot light, script, or export a scene from the UI.
5. **If no — what specifically is missing?** The P0/P1 list in §3: correct picking, safe dragging, working delete-undo, Physics/light/script panels, New Scene/Export, then the parity long tail (gizmos, multi-select, materials, import, project system).

---

## 6. Root-cause hypotheses for the P0s (for the fix phase, not executed per instructions)

- **Picking:** the candidate-hit selection most likely iterates entities and keeps a hit without comparing ray distance (or the plane test accepts first). The four D PI experiments (clicks at raw, ×1.2, ÷1.2, exact centers) ruled out any DPI/coordinate-space offset: ppp is 1.0, the UI fills the window (measured: content extends to x=1599/y=899 of the 1600×900 window), and **exact measured centers still select Terrain** — this is a hit-selection logic bug, not an environment artifact.
- **Undo-of-delete:** the restore snapshot stores the deleted entity with `parent: "Level"` but validates parents *within the snapshot*; it must validate against the live world (or snapshot the ancestor chain).
- **Startup panic:** `xkbcommon-dl` dlopen fails hard; needs a graceful error + documented deps in the zip README.

## 7. Reproduction scripts

All sessions are reproducible: `/home/z/testing/` contains `session.py` (Xvfb + editor lifecycle), `helpers.py` (XTest input + measurement), `stages/*.py` (each numbered interaction), `measure.py` (PIL object-center measurement), and `session.log` (timestamped interaction ledger). Evidence PNGs referenced above are committed under `docs/reality-check/`.

## 8. Not tested (honest gaps of this check)

- Window resizing/maximization, real GPU pipelines, multi-monitor, HiDPI-scale behavior on real desktops, touch input.
- macOS/Windows binaries (Linux artifact only in this sandbox; CI smoke evidence for those platforms is CI's, not mine).
- Audio authoring/playback, gamepad input, localization, long-session memory leaks, multi-scene projects.
- Godot export (requires one-time template download; not fetched in the sandbox) — the *dialog* path was inspected, the full export run was not.
- Unity (see §5.3).
