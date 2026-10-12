//! Scene hierarchy — the left dock: a live, searchable view of the
//! actual ECS tree.
//!
//! The tree is rebuilt from the world every frame (engine scene sizes
//! make this trivially cheap): roots are entities without `ChildOf`,
//! children follow the parent's `Children` insertion order, unnamed
//! entities get a stable fallback label and the eye toggle flips the
//! REAL `Active` component (hidden = not active in simulation).
//!
//! With a search string the filter keeps matching nodes plus every
//! ancestor needed to reach them, auto-expanding the path — the
//! reference editor's search behavior.

use crate::{shell::EditorState, theme};
use gens_ecs::ecs::hierarchy::ChildOf;
use gens_ecs::prelude::*;
use gens_scripting::ScriptComponent;
use std::collections::{BTreeMap, BTreeSet};

use crate::model::{AuthoredBody, MeshSlot};

/// One flattened row of the hierarchy tree.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The row's entity.
    pub entity: Entity,
    /// Indentation depth (roots at 0).
    pub depth: u32,
    /// Display name (`Name` component or a stable fallback).
    pub name: String,
    /// `true` when the entity carries `Active` (eye icon state).
    pub visible: bool,
    /// Whether expandable children exist.
    pub has_children: bool,
    /// Whether children are currently shown (trees open by default;
    /// `collapsed` tracks user-folded branches).
    pub expanded: bool,
}

#[derive(Default)]
struct NodeInfo {
    name: String,
    parent: Option<Entity>,
    children: Vec<Entity>,
    active: bool,
}

type Nodes = BTreeMap<Entity, NodeInfo>;

/// Snapshot of every entity's identity + relations, keyed by entity.
///
/// Entities carrying none of the editor-visible component vocabulary
/// are skipped — the same convention the save format uses (ADR-0021).
/// This is what keeps bevy's internal seed entity (`0v0`, no
/// components — pinned since Phase 1) out of the tree.
fn collect(world: &World) -> Nodes {
    let mut nodes: Nodes = BTreeMap::new();
    for entity_ref in world.iter_entities() {
        // bevy 0.20 iter_entities yields EntityRef; the id is what the
        // tree (and every component lookup) keys on.
        let entity = entity_ref.id();
        let named = world.get::<Name>(entity).is_some();
        let has_content = named
            || world.get::<Transform>(entity).is_some()
            || world.get::<Active>(entity).is_some()
            || world.get::<Tag>(entity).is_some()
            || world.get::<ChildOf>(entity).is_some()
            || world.get::<MeshSlot>(entity).is_some()
            || world.get::<AuthoredBody>(entity).is_some()
            || world.get::<ScriptComponent>(entity).is_some();
        if !has_content {
            continue;
        }
        let info = nodes.entry(entity).or_default();
        info.name = world
            .get::<Name>(entity)
            .map(|n| n.as_str().to_string())
            .unwrap_or_else(|| format!("Entity {entity:?}"));
        info.active = world.get::<Active>(entity).is_some();
        if let Some(parent) = world.get::<ChildOf>(entity).map(|link| link.parent()) {
            nodes.entry(parent).or_default().children.push(entity);
            if let Some(info) = nodes.get_mut(&entity) {
                info.parent = Some(parent);
            }
        }
    }
    nodes
}

/// Flattens the world into visible rows (search-aware).
///
/// Order: roots in entity-id order (the ADR-0021 doctrine — archetype
/// iteration order is not deterministic), children in insertion order
/// (pinned by bevy's `Children` component). Same build sequence, same
/// tree, byte for byte.
pub fn build_tree(world: &World, search: &str, collapsed: &BTreeSet<Entity>) -> Vec<Row> {
    let nodes = collect(world);
    let roots: Vec<Entity> = nodes
        .iter()
        .filter(|(_, info)| info.parent.is_none())
        .map(|(entity, _)| *entity)
        .collect();
    let needle = search.trim().to_lowercase();
    let mut rows = Vec::new();
    for root in roots {
        walk(&nodes, root, 0, &needle, collapsed, &mut rows);
    }
    rows
}

/// Depth-first walk; returns whether this subtree survived the filter.
fn walk(
    nodes: &Nodes,
    entity: Entity,
    depth: u32,
    needle: &str,
    collapsed: &BTreeSet<Entity>,
    out: &mut Vec<Row>,
) -> bool {
    let Some(info) = nodes.get(&entity) else {
        return false;
    };
    let filtering = !needle.is_empty();
    let self_match = filtering && info.name.to_lowercase().contains(needle);

    // Children first: a subtree survives if the node matches OR any
    // descendant does.
    let open = if filtering {
        // While filtering, paths to matches are force-open.
        true
    } else {
        // Trees open by default: a branch hides its children only when
        // the user explicitly collapsed it.
        !collapsed.contains(&entity)
    };
    let mut child_kept = false;
    let mut child_rows: Vec<Row> = Vec::new();
    // Children are only walked when the branch is actually open — the
    // non-filtering tree shows exactly the expanded frontier.
    if open {
        for &child in &info.children {
            if walk(nodes, child, depth + 1, needle, collapsed, &mut child_rows) {
                child_kept = true;
            }
        }
    }
    let keep = self_match || child_kept || !filtering;
    if keep {
        let expanded_now = if filtering {
            (child_kept || self_match) && !info.children.is_empty()
        } else {
            !collapsed.contains(&entity) && !info.children.is_empty()
        };
        out.push(Row {
            entity,
            depth,
            name: info.name.clone(),
            visible: info.active,
            has_children: !info.children.is_empty(),
            expanded: expanded_now,
        });
        out.append(&mut child_rows);
    }
    keep
}

/// The left dock panel: header, search field, tree rows.
pub(crate) fn panel(ui: &mut egui::Ui, state: &mut EditorState, world: &mut World) {
    egui::Panel::left("hierarchy")
        .default_size(230.0)
        .resizable(true)
        .frame(
            egui::Frame::default()
                .fill(theme::PANEL_BG)
                .inner_margin(egui::Margin::same(4)),
        )
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Scene Hierarchy").strong());
            });
            ui.add(
                egui::TextEdit::singleline(&mut state.search)
                    .hint_text("Search nodes...")
                    .desired_width(ui.available_width()),
            );
            ui.separator();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let rows = build_tree(world, &state.search, &state.collapsed);
                    for row in rows {
                        draw_row(ui, state, world, &row);
                    }
                });
        });
}

const ROW_HEIGHT: f32 = 18.0;
/// Expand-toggle width (keeps collapsed rows aligned with expanded ones).
const TOGGLE_W: f32 = 16.0;

fn draw_row(ui: &mut egui::Ui, state: &mut EditorState, world: &mut World, row: &Row) {
    ui.horizontal(|ui| {
        ui.add_space(4.0 + 12.0 * row.depth as f32);
        // Expand toggle (aligned even for leaf rows).
        if row.has_children {
            let glyph = if row.expanded { "-" } else { "+" };
            if ui
                .add(
                    egui::Button::new(egui::RichText::new(glyph).color(theme::TEXT_DIM))
                        .fill(egui::Color32::TRANSPARENT)
                        .min_size(egui::vec2(TOGGLE_W, ROW_HEIGHT)),
                )
                .clicked()
            {
                if row.expanded {
                    state.collapsed.insert(row.entity);
                } else {
                    state.collapsed.remove(&row.entity);
                }
            }
        } else {
            ui.allocate_exact_size(egui::vec2(TOGGLE_W, ROW_HEIGHT), egui::Sense::hover());
        }
        // Node kind bullet (folder-ish dim, entities accent when visible).
        ui.label(
            egui::RichText::new(if row.visible { "\u{25CF}" } else { "\u{25CB}" }).color(
                if row.visible {
                    theme::ACCENT
                } else {
                    theme::TEXT_DIM
                },
            ),
        );
        let selected = state.selected == Some(row.entity);
        let text = egui::RichText::new(&row.name).color(if selected {
            theme::TEXT
        } else {
            theme::TEXT_DIM
        });
        let response = ui.selectable_label(selected, text);
        // Layout capture for scripted pixel-accurate replay (P4-6 lesson).
        state.row_rects.insert(row.name.clone(), response.rect);
        if response.clicked() {
            state.selected = Some(row.entity);
        }
        // Visibility eye = the REAL Active component (right-aligned).
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let glyph = if row.visible { "v" } else { "x" };
            if ui
                .add(
                    egui::Button::new(egui::RichText::new(glyph).color(theme::TEXT_DIM))
                        .fill(egui::Color32::TRANSPARENT)
                        .min_size(egui::vec2(16.0, ROW_HEIGHT)),
                )
                .clicked()
            {
                toggle_active(world, row.entity);
            }
        });
    });
}

/// Flips the entity's `Active` component — hiding an entity in the
/// hierarchy really deactivates it in the world.
pub fn toggle_active(world: &mut World, entity: Entity) {
    if world.get::<Active>(entity).is_some() {
        world.entity_mut(entity).remove::<Active>();
    } else {
        world.entity_mut(entity).insert(Active);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gens_ecs::hierarchy::attach_children;

    fn named(world: &mut World, name: &str) -> Entity {
        world.spawn((Name::new(name), Active)).id()
    }

    fn names_of(rows: &[Row]) -> Vec<String> {
        rows.iter().map(|r| r.name.clone()).collect()
    }

    #[test]
    fn tree_reflects_real_relations() {
        let mut world = World::new();
        let world_e = named(&mut world, "World");
        let light = named(&mut world, "DirectionalLight");
        let station = named(&mut world, "SciFi_Station");
        attach_children(&mut world, world_e, &[light]);

        let rows = build_tree(&world, "", &BTreeSet::new());
        // Exactly the three named entities — no seed entity, no phantoms.
        assert_eq!(rows.len(), 3);
        // Root order is entity-id order (ADR-0021), not spawn order.
        let root_entities: Vec<Entity> = rows.iter().map(|r| r.entity).collect();
        let mut expected_roots = vec![world_e, station];
        expected_roots.sort();
        let light_idx = rows
            .iter()
            .position(|r| r.name == "DirectionalLight")
            .unwrap();
        assert_eq!(rows[light_idx].entity, light);
        assert_eq!(rows[light_idx].depth, 1);
        // Light's parent is whichever root comes first in row order.
        let light_parent = rows[light_idx - 1].entity;
        assert_eq!(light_parent, world_e);
        let roots_only: Vec<Entity> = rows
            .iter()
            .filter(|r| r.depth == 0)
            .map(|r| r.entity)
            .collect();
        assert_eq!(roots_only, expected_roots);
        assert_eq!(root_entities.len(), 3);
        assert!(rows.iter().any(|r| r.name == "World" && r.has_children));
        assert!(
            rows.iter()
                .any(|r| r.name == "SciFi_Station" && !r.has_children)
        );
    }

    #[test]
    fn expansion_flattens_children() {
        let mut world = World::new();
        let station = named(&mut world, "SciFi_Station");
        let bay = named(&mut world, "DockingBay");
        let ship = named(&mut world, "Spaceship");
        attach_children(&mut world, station, &[bay]);
        attach_children(&mut world, bay, &[ship]);

        // Trees open by default: full chain visible with no user folds.
        let rows = build_tree(&world, "", &BTreeSet::new());
        assert_eq!(
            names_of(&rows),
            vec!["SciFi_Station", "DockingBay", "Spaceship"]
        );
        assert_eq!(rows[2].depth, 2);
        assert!(rows[0].expanded && rows[1].expanded && !rows[2].expanded);
        // Folding DockingBay hides exactly its subtree.
        let mut collapsed = BTreeSet::new();
        collapsed.insert(bay);
        let rows = build_tree(&world, "", &collapsed);
        assert_eq!(names_of(&rows), vec!["SciFi_Station", "DockingBay"]);
        assert_eq!(rows[1].depth, 1);
        assert!(rows[0].expanded && !rows[1].expanded);
    }

    #[test]
    fn search_keeps_matches_and_ancestors() {
        let mut world = World::new();
        let station = named(&mut world, "SciFi_Station");
        let bay = named(&mut world, "DockingBay");
        let ship = named(&mut world, "Spaceship");
        attach_children(&mut world, station, &[bay]);
        attach_children(&mut world, bay, &[ship]);

        let rows = build_tree(&world, "spaceship", &BTreeSet::new());
        assert_eq!(
            names_of(&rows),
            vec!["SciFi_Station", "DockingBay", "Spaceship"]
        );
        assert!(
            rows.iter().all(|r| !r.has_children || r.expanded),
            "filter paths must be force-open"
        );
        // Case-insensitive.
        let rows = build_tree(&world, "SCIFI", &BTreeSet::new());
        assert_eq!(names_of(&rows), vec!["SciFi_Station"]);
        // No match at all -> empty tree.
        let rows = build_tree(&world, "terrain", &BTreeSet::new());
        assert!(rows.is_empty());
    }

    #[test]
    fn visibility_maps_active_component() {
        let mut world = World::new();
        let shown = named(&mut world, "Shown");
        let hidden = named(&mut world, "Hidden");
        world.entity_mut(hidden).remove::<Active>();
        let rows = build_tree(&world, "", &BTreeSet::new());
        assert_eq!(rows.len(), 2, "seed entity must not appear");
        let shown_row = rows.iter().find(|r| r.name == "Shown").unwrap();
        let hidden_row = rows.iter().find(|r| r.name == "Hidden").unwrap();
        assert!(shown_row.visible && !hidden_row.visible);
        // toggle_active flips the real component.
        toggle_active(&mut world, hidden);
        assert!(world.get::<Active>(hidden).is_some());
        toggle_active(&mut world, shown);
        assert!(world.get::<Active>(shown).is_none());
    }

    #[test]
    fn componentless_entities_are_filtered() {
        let mut world = World::new();
        world.spawn_empty(); // like bevy's internal seed entity
        let named_only = named(&mut world, "Real");
        let _ = named_only;
        let rows = build_tree(&world, "", &BTreeSet::new());
        assert_eq!(names_of(&rows), vec!["Real"]);
        // An entity with content but no name gets a stable fallback label.
        world.spawn(Transform::from_position(Vec3::ZERO));
        let rows = build_tree(&world, "", &BTreeSet::new());
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter().any(|r| r.name.starts_with("Entity ")),
            "unnamed entity needs a stable fallback label"
        );
    }
}
