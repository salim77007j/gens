//! Inspector — the right dock: reads and edits the REAL components of
//! the selected entity.
//!
//! Every field is a live view: DragValues mutate the entity's
//! `Transform` in the world mid-pass, the mesh/collider/body sections
//! mirror the actual `MeshSlot`/`AuthoredBody`/physics vocabulary, and
//! the script section shows the actual `ScriptComponent`. Where the
//! reference mockup shows fields the engine does not have (mass as a
//! number — mass derives from collider density), the panel shows the
//! real quantity instead and the omission is documented (ADR-0025).
//!
//! Rotation is authored in YXZ Euler degrees (editor convention) and
//! converted to the `Quat` the engine stores; the roundtrip is pinned
//! by tests.

use crate::{
    model::{AuthoredBody, MeshSlot, ShapeKind},
    shell::EditorState,
    theme,
};
use gens_ecs::prelude::*;
use gens_math::{Quat, Vec3};
use gens_physics::world::{BodyKind, ColliderShape};
use gens_scripting::ScriptComponent;

/// The right dock panel.
pub(crate) fn panel(ui: &mut egui::Ui, state: &mut EditorState, world: &mut World) {
    egui::Panel::right("inspector")
        .default_size(280.0)
        .resizable(true)
        .frame(
            egui::Frame::default()
                .fill(theme::PANEL_BG)
                .inner_margin(egui::Margin::same(6)),
        )
        .show(ui, |ui| {
            ui.label(egui::RichText::new("Inspector").strong());
            ui.separator();
            let Some(entity) = state.selected else {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new("Select an entity in the hierarchy")
                            .color(theme::TEXT_DIM),
                    );
                });
                return;
            };
            if world.get_entity(entity).is_err() {
                // The selection was despawned (Delete Selected) — clear it.
                state.selected = None;
                return;
            }
            identity_section(ui, world, entity);
            transform_section(ui, world, entity);
            mesh_section(ui, world, entity);
            body_section(ui, world, entity);
            script_section(ui, world, entity);
        });
}

/// Identity: name, active flag, optional tag.
fn identity_section(ui: &mut egui::Ui, world: &mut World, entity: Entity) {
    collapsible(ui, "Identity", true, |ui| {
        if let Some(name) = world.get::<Name>(entity) {
            let mut text = name.as_str().to_string();
            if ui
                .add(egui::TextEdit::singleline(&mut text).desired_width(ui.available_width()))
                .lost_focus()
                && !text.is_empty()
                && text != name.as_str()
            {
                world.entity_mut(entity).insert(Name::new(text));
            }
        }
        let mut active = world.get::<Active>(entity).is_some();
        if ui.checkbox(&mut active, "Active").changed() {
            crate::hierarchy::toggle_active(world, entity);
        }
        if let Some(tag) = world.get::<Tag>(entity) {
            let mut text = tag.0.clone();
            if ui
                .add(egui::TextEdit::singleline(&mut text).desired_width(ui.available_width()))
                .lost_focus()
                && text != tag.0
            {
                world.entity_mut(entity).insert(Tag::new(text));
            }
        }
    });
}

/// Transform: position / rotation (YXZ degrees) / scale.
fn transform_section(ui: &mut egui::Ui, world: &mut World, entity: Entity) {
    let Some(t) = world.get::<Transform>(entity).copied() else {
        return;
    };
    collapsible(ui, "Transform", true, |ui| {
        let mut pos = t.position;
        if vec3_row(ui, "Position", &mut pos) {
            world
                .entity_mut(entity)
                .insert(Transform { position: pos, ..t });
        }
        let mut euler = euler_degrees(t.rotation);
        if vec3_row_degrees(ui, "Rotation", &mut euler) {
            world.entity_mut(entity).insert(Transform {
                rotation: quat_from_degrees(euler),
                ..t
            });
        }
        let mut scale = t.scale;
        if vec3_row(ui, "Scale", &mut scale) {
            world.entity_mut(entity).insert(Transform { scale, ..t });
        }
    });
}

/// The authored mesh the viewport draws.
fn mesh_section(ui: &mut egui::Ui, world: &mut World, entity: Entity) {
    let Some(slot) = world.get::<MeshSlot>(entity).copied() else {
        return;
    };
    collapsible(ui, "Mesh", true, |ui| {
        let mut slot_now = slot;
        // Shape dropdown (index-based selection over ShapeKind::ALL).
        let mut shape_idx = ShapeKind::ALL
            .iter()
            .position(|k| *k == slot.shape)
            .unwrap_or(0);
        egui::ComboBox::from_label("Shape")
            .selected_text(ShapeKind::ALL[shape_idx].as_str())
            .show_ui(ui, |ui| {
                for (i, kind) in ShapeKind::ALL.iter().enumerate() {
                    ui.selectable_value(&mut shape_idx, i, kind.as_str());
                }
            });
        if ShapeKind::ALL[shape_idx] != slot.shape {
            slot_now.shape = ShapeKind::ALL[shape_idx];
        }
        // Tint (RGBA bytes, 0..=255).
        let mut tint = slot.tint;
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Tint");
            for (axis, label) in tint.iter_mut().zip(["R", "G", "B", "A"]) {
                changed |= ui
                    .add(
                        egui::DragValue::new(axis)
                            .speed(1.0)
                            .range(0..=255)
                            .prefix(format!("{label} ")),
                    )
                    .changed();
            }
        });
        if changed {
            slot_now.tint = tint;
        }
        if slot_now != slot {
            world.entity_mut(entity).insert(slot_now);
        }
    });
}

/// Physics body: kind, CCD, damping — the `BodySpec` vocabulary.
fn body_section(ui: &mut egui::Ui, world: &mut World, entity: Entity) {
    let Some(body) = world.get::<AuthoredBody>(entity).copied() else {
        return;
    };
    collapsible(ui, "Physics Body", true, |ui| {
        let mut b = body;
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Type");
            let kinds = [
                (BodyKind::Dynamic, "Dynamic"),
                (BodyKind::Fixed, "Fixed"),
                (BodyKind::KinematicPosition, "Kinematic"),
            ];
            let mut current = kinds.iter().position(|(k, _)| *k == b.kind).unwrap_or(0);
            egui::ComboBox::from_id_salt("body_kind")
                .selected_text(kinds[current].1)
                .show_ui(ui, |ui| {
                    for (i, (_, label)) in kinds.iter().enumerate() {
                        ui.selectable_value(&mut current, i, *label);
                    }
                });
            if kinds[current].0 != b.kind {
                b.kind = kinds[current].0;
                changed = true;
            }
        });
        changed |= ui.checkbox(&mut b.ccd, "Use CCD").changed();
        ui.horizontal(|ui| {
            ui.label("Linear Damping");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut b.linear_damping)
                        .speed(0.01)
                        .range(0.0..=f32::MAX),
                )
                .changed();
        });
        collider_fields(ui, &mut b, &mut changed);
        if changed {
            world.entity_mut(entity).insert(b);
        }
    });
}

/// Collider shape + material (friction / restitution / density).
fn collider_fields(ui: &mut egui::Ui, b: &mut AuthoredBody, changed: &mut bool) {
    collapsible(ui, "Collider", true, |ui| {
        let is_ball = matches!(b.collider, ColliderShape::Ball(_));
        let mut ball = is_ball;
        if ui
            .checkbox(&mut ball, "Ball collider (else cuboid)")
            .changed()
        {
            b.collider = if ball {
                ColliderShape::Ball(0.5)
            } else {
                ColliderShape::Cuboid(Vec3::splat(0.5))
            };
            *changed = true;
        }
        match &mut b.collider {
            ColliderShape::Ball(radius) => {
                ui.horizontal(|ui| {
                    ui.label("Radius");
                    *changed |= ui
                        .add(
                            egui::DragValue::new(radius)
                                .speed(0.02)
                                .range(0.01..=f32::MAX),
                        )
                        .changed();
                });
            }
            ColliderShape::Cuboid(half) => {
                *changed |= vec3_row(ui, "Half extents", half);
            }
        }
        ui.horizontal(|ui| {
            ui.label("Friction");
            *changed |= ui
                .add(
                    egui::DragValue::new(&mut b.friction)
                        .speed(0.01)
                        .range(0.0..=1.0),
                )
                .changed();
        });
        ui.horizontal(|ui| {
            ui.label("Restitution");
            *changed |= ui
                .add(
                    egui::DragValue::new(&mut b.restitution)
                        .speed(0.01)
                        .range(0.0..=1.0),
                )
                .changed();
        });
        ui.horizontal(|ui| {
            ui.label("Density");
            *changed |= ui
                .add(
                    egui::DragValue::new(&mut b.density)
                        .speed(0.01)
                        .range(0.01..=f32::MAX),
                )
                .changed();
        });
    });
}

/// The script binding (Phase 5 `ScriptComponent` vocabulary).
fn script_section(ui: &mut egui::Ui, world: &mut World, entity: Entity) {
    if world.get::<ScriptComponent>(entity).is_none() {
        return;
    }
    collapsible(ui, "Script", true, |ui| {
        // Read owned copies first (no overlapping world borrows).
        let (vars, requests) = {
            let script = world.get::<ScriptComponent>(entity).unwrap();
            (script.vars.clone(), script.requests.clone())
        };
        let mut script_name = world.get::<ScriptComponent>(entity).unwrap().script.clone();
        let mut entry = world.get::<ScriptComponent>(entity).unwrap().entry.clone();
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Script");
            changed |= ui
                .add(egui::TextEdit::singleline(&mut script_name).desired_width(140.0))
                .lost_focus();
        });
        ui.horizontal(|ui| {
            ui.label("Entry");
            changed |= ui
                .add(egui::TextEdit::singleline(&mut entry).desired_width(140.0))
                .lost_focus();
        });
        if changed {
            world.entity_mut(entity).insert(ScriptComponent {
                script: script_name,
                entry,
                vars,
                requests,
            });
        }
        ui.label(
            egui::RichText::new("Variables are edited in the data asset, not here.")
                .small()
                .color(theme::TEXT_DIM),
        );
    });
}

/// A labeled XYZ row; returns true when any axis changed.
fn vec3_row(ui: &mut egui::Ui, label: &str, v: &mut Vec3) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        for (axis, name) in v.as_mut().iter_mut().zip(["X", "Y", "Z"]) {
            changed |= ui
                .add(
                    egui::DragValue::new(axis)
                        .speed(0.05)
                        .prefix(format!("{name} ")),
                )
                .changed();
        }
    });
    changed
}

/// Same as [`vec3_row`] but editing degrees (display rounding only).
fn vec3_row_degrees(ui: &mut egui::Ui, label: &str, v: &mut [f32; 3]) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        for (axis, name) in v.iter_mut().zip(["X", "Y", "Z"]) {
            changed |= ui
                .add(
                    egui::DragValue::new(axis)
                        .speed(0.5)
                        .prefix(format!("{name} ")),
                )
                .changed();
        }
    });
    changed
}

fn collapsible(
    ui: &mut egui::Ui,
    title: &str,
    default_open: bool,
    add: impl FnOnce(&mut egui::Ui),
) {
    egui::CollapsingHeader::new(egui::RichText::new(title).strong())
        .default_open(default_open)
        .show(ui, add);
}

/// Rotation as YXZ Euler degrees (editor-facing convention).
pub fn euler_degrees(q: Quat) -> [f32; 3] {
    let (y, x, z) = q.to_euler(glam::EulerRot::YXZ);
    [x, y, z].map(f32::to_degrees)
}

/// Builds a rotation from YXZ Euler degrees (inverse of [`euler_degrees`]).
pub fn quat_from_degrees(deg: [f32; 3]) -> Quat {
    let [x, y, z] = deg.map(f32::to_radians);
    Quat::from_euler(glam::EulerRot::YXZ, y, x, z)
}

/// Applies an authored transform (helper so tests hit the same code the
/// panel writes through).
pub fn set_transform(
    world: &mut World,
    entity: Entity,
    position: Vec3,
    euler_deg: [f32; 3],
    scale: Vec3,
) {
    world.entity_mut(entity).insert(Transform {
        position,
        rotation: quat_from_degrees(euler_deg),
        scale,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn euler_roundtrip_is_stable() {
        for deg in [
            [0.0; 3],
            [90.0, 0.0, 0.0],
            [0.0, 180.0, 0.0],
            [15.0, -30.0, 60.0],
        ] {
            let back = euler_degrees(quat_from_degrees(deg));
            // Quaternion double-cover (documented since Phase 4): euler
            // representations may flip sign at the branch cut while
            // naming the SAME rotation — compare rotations, not text.
            let a = quat_from_degrees(deg);
            let b = quat_from_degrees(back);
            let dot = a.dot(b).abs();
            assert!(dot > 1.0 - 1e-5, "{deg:?} -> {back:?} (dot {dot})");
        }
    }

    #[test]
    fn set_transform_writes_real_components() {
        let mut world = World::new();
        let e = world.spawn_empty().id();
        set_transform(
            &mut world,
            e,
            Vec3::new(12.43, 4.28, -8.76),
            [0.0, 90.0, 0.0],
            Vec3::ONE,
        );
        let t = world.get::<Transform>(e).unwrap();
        assert!((t.position.x - 12.43).abs() < 1e-5);
        // A 90-degree Y rotation has a known sine component.
        let [x, y, _z] = euler_degrees(t.rotation);
        assert!((y - 90.0).abs() < 1e-3 && x.abs() < 1e-3);
    }

    #[test]
    fn authored_body_edits_reach_the_world() {
        let mut world = World::new();
        let e = world.spawn(AuthoredBody::default()).id();
        let mut b = *world.get::<AuthoredBody>(e).unwrap();
        b.ccd = true;
        b.kind = BodyKind::Fixed;
        b.collider = ColliderShape::Ball(1.25);
        b.friction = 0.6;
        b.restitution = 0.2;
        world.entity_mut(e).insert(b);
        let stored = world.get::<AuthoredBody>(e).unwrap();
        assert!(stored.ccd);
        assert_eq!(stored.kind, BodyKind::Fixed);
        assert_eq!(stored.collider, ColliderShape::Ball(1.25));
        assert_eq!(stored.friction, 0.6);
        assert_eq!(stored.restitution, 0.2);
    }

    #[test]
    fn mesh_tint_edits_reach_the_world() {
        let mut world = World::new();
        let e = world.spawn(MeshSlot::cube([200, 200, 205, 255])).id();
        let mut m = *world.get::<MeshSlot>(e).unwrap();
        m.shape = ShapeKind::Quad;
        m.tint = [90, 90, 100, 255];
        world.entity_mut(e).insert(m);
        let stored = world.get::<MeshSlot>(e).unwrap();
        assert_eq!(stored.shape, ShapeKind::Quad);
        assert_eq!(stored.tint, [90, 90, 100, 255]);
    }
}
