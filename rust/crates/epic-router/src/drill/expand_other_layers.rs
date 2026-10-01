//! Java `MazeExpansionEngine.expandToOtherLayers`
//! (`MazeExpansionEngine.java:237-375`) + `checkLayerWithAnyMatchingVia`
//! (`:377-414`) — the via-mask layer-change logic: sweep the drill's
//! layer span for drillable layers, filter through the via masks, and
//! emit the layer-change maze elements.
//!
//! The DECISIONS (bounds sweeps, edge flags, mask gate, costs,
//! occupancy) are ported and pinned here; three deep probes are SEAMS
//! (see `SEAM.md`): `ViaLayerChecker` (T10 — Java
//! `ForcedViaInserter.checkLayer`), `DestinationDistance` (T8 — Java
//! `search.destinationDistance.calculate`), and the emission callback
//! (T6 — Java `search.mazeExpansionList.add`).

use epic_geometry::float_line::FloatLine;
use epic_geometry::float_point::FloatPoint;
use epic_geometry::point::Point;
use epic_geometry::tile_shape::TileShape;

use super::maze_search_element::Adjustment;
use super::{DrillEngine, ExpansionDrill, ViaRuleVia};
use crate::control::AutorouteControl;
use epic_board::items::shape_max_width;

/// Java `ForcedPadRouter.CheckDrillResult`
/// (`ForcedPadRouter.java:495-499`) — declaration order mirrored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckDrillResult {
    /// Java `DRILLABLE`.
    Drillable,
    /// Java `DRILLABLE_WITH_ATTACH_SMD`.
    DrillableWithAttachSmd,
    /// Java `NOT_DRILLABLE`.
    NotDrillable,
}

/// SEAM (T10): Java `ForcedViaInserter.checkLayer` (static, 12 args) —
/// the deep legality probe (shove/ripup simulation over the real
/// board). The port reduces the twelve arguments to the eight the
/// DECISION code consumes: the two recursion depths (`:399-400`) and
/// `ctrl.traceClearanceClassIndex` (`:403`) are T10's internal inputs,
/// passed through its own construction; `search.autorouteEngine.board`
/// (`:401`) is implicit in the checker's board access; and the
/// per-layer `ctrl.traceHalfWidth[layer]` (`:402`) is already folded
/// into `required_radius` by the caller — exactly Java's
/// `requiredRadius = Math.max(viaRadius, ctrl.traceHalfWidth[layer])`
/// at `:389`, which the port mirrors at its `checkLayer` call site.
/// `netNumbers int[]` (`:398`) collapses to one `i32` faithfully: this
/// call site always passes exactly one net number. T10 ports the
/// checker with the rest of ForcedPadRouter/ForcedViaInserter as one
/// unit.
pub trait ViaLayerChecker {
    /// Java `ForcedViaInserter.checkLayer(requiredRadius,
    /// clearanceClass, attachSmdAllowed, roomShape, location, layer,
    /// netNumbers, ...)` for one candidate via position.
    #[allow(clippy::too_many_arguments)] // the Java parameter list, reduced from 12 to 8
    fn check_layer(
        &mut self,
        required_radius: f64,
        clearance_class: i32,
        attach_smd_allowed: bool,
        room_shape: &TileShape,
        location: &Point,
        layer: i32,
        net_number: i32,
    ) -> CheckDrillResult;
}

/// SEAM (T8): Java `search.destinationDistance` — the target-distance
/// estimate folded into the sorting value. `calculate` is the
/// per-query estimate; `join` is Java
/// `destinationDistance.join(IntBox, int)` called by
/// `MazeSearchEngine.init` for every destination tree shape (the
/// heuristic's input accumulation — Java's `MoonMilOffline
/// .joinDestinationShape` equivalent). T8 owns the real heuristic.
pub trait DestinationDistance {
    fn calculate(&self, middle: &FloatPoint, layer: i32) -> f64;

    /// Java `DestinationDistance.join(IntBox shape, int layer)` — the
    /// T8 SEAM slot: the default is a no-op so the T5/T6 targetless
    /// distances are unaffected.
    fn join(&mut self, _shape: &epic_geometry::int_box::IntBox, _layer: i32) {}
}

/// The `MazeListElement` seed `expandToOtherLayers` emits (Java
/// `MazeListElement` ctor, `MazeListElement.java:54`, argument order
/// mirrored; see the trace there for the field mapping). T6 owns the
/// consumer (Java `search.mazeExpansionList.add` — a TreeSet whose
/// `add` override is fanout-aware and inert while `ctrl.isFanout` is
/// false).
#[derive(Clone, Debug, PartialEq)]
pub struct DrillMazeListElement {
    /// Java `door` = the drill itself (opaque key here).
    pub door_key: u64,
    /// Java `sectionNoOfDoor` = `toLayer - drill.firstLayer`.
    pub section_no_of_door: i32,
    /// Java `backtrackDoor` = the SAME drill.
    pub backtrack_door_key: u64,
    /// Java `sectionNoOfBacktrackDoor` = the FROM element's section.
    pub section_no_of_backtrack_door: i32,
    /// Java `expansionValue` = from-value + `addViaCosts[from][to]`.
    pub expansion_value: f64,
    /// Java `sortingValue` = expansionValue + destination distance.
    pub sorting_value: f64,
    /// Java `nextRoom` = `drill.roomArr[currentRoomIndex]`.
    pub next_room_key: u64,
    /// Java `shapeEntry` — carried through unchanged.
    pub shape_entry: FloatLine,
    /// Java `roomRipped`.
    pub room_ripped: bool,
    /// Java `adjustment` = `Adjustment.NONE`.
    pub adjustment: Adjustment,
    /// Java `alreadyChecked` = false.
    pub already_checked: bool,
}

/// Java `expandToOtherLayers` (`:237-375`). Java passes the whole
/// `MazeListElement`; the port takes the fields the body reads. The
/// emitted elements go to the `emit` callback (the T6 seam).
#[allow(clippy::too_many_arguments)]
pub fn expand_to_other_layers(
    ctx: &mut impl DrillEngine,
    ctrl: &AutorouteControl,
    drill_key: u64,
    drill: &ExpansionDrill,
    section_no_of_door: i32,
    expansion_value: f64,
    shape_entry: &FloatLine,
    checker: &mut impl ViaLayerChecker,
    destination_distance: &impl DestinationDistance,
    emit: &mut dyn FnMut(DrillMazeListElement),
) {
    let layer_count = ctrl.layer_count as i32;
    // Java initializers (`:239-240`): `viaUpperBound = -1` is DEAD in
    // Java too (both branches reassign) — mirrored verbatim.
    #[allow(unused_assignments)]
    let mut via_lower_bound = 0;
    #[allow(unused_assignments)]
    let mut via_upper_bound = -1;
    let from_layer = drill.first_layer + section_no_of_door;
    let mut smd_attached_on_component_side = false;
    let mut smd_attached_on_solder_side = false;
    let room_ripped;
    // Java: `roomArr[sectionNoOfDoor]` — a null slot would fail the
    // instanceof and NPE on `.getShape()` in the free-space branch; a
    // live drill always has the room.
    let door_room = drill.room_arr[section_no_of_door as usize]
        .expect("the from-room exists (Java NPEs on null)");
    if ctx.room_is_obstacle(door_room) {
        // The ripped-via branch (:246-261): re-expansion through a
        // ripped own-net via — its padstack span IS the via range.
        if !ctrl.ripup_allowed {
            return;
        }
        let Some(item_key) = ctx.room_obstacle_item_key(door_room) else {
            return;
        };
        if !ctx.item_is_via(item_key) {
            return;
        }
        let Some(padstack_no) = ctx.item_padstack_no(item_key) else {
            return;
        };
        // Java `ctrl.viaRule.containsPadstack` — reference identity in
        // Java; padstack-number equality here (numbers are unique).
        let contains = ctx
            .via_rule_vias()
            .iter()
            .any(|via| via.padstack_no == padstack_no);
        if !contains || ctx.item_clearance_class(item_key) != ctrl.via_clearance_class {
            return;
        }
        let (from, to) = ctx.padstack_layer_span(padstack_no);
        via_lower_bound = from;
        via_upper_bound = to;
        room_ripped = true;
    } else {
        room_ripped = false;
        let net_number = ctrl.net_number;
        let via_lower_limit = drill.first_layer.max(ctrl.via_lower_bound);
        let via_upper_limit = drill.last_layer.min(ctrl.via_upper_bound);
        // Downward sweep from the from-layer (:267-297).
        let mut current_layer = from_layer;
        loop {
            let room_key = drill.room_arr[(current_layer - drill.first_layer) as usize]
                .expect("room exists on every drill layer");
            let room_shape = ctx.room_shape(room_key);
            let drill_result = check_layer_with_any_matching_via(
                &mut *ctx,
                ctrl,
                drill,
                current_layer,
                &room_shape,
                checker,
                net_number,
            );
            if drill_result == CheckDrillResult::NotDrillable {
                via_lower_bound = current_layer + 1;
                break;
            } else if drill_result == CheckDrillResult::DrillableWithAttachSmd {
                if current_layer == 0 {
                    smd_attached_on_component_side = true;
                } else if current_layer == layer_count - 1 {
                    smd_attached_on_solder_side = true;
                }
            }
            if current_layer <= via_lower_limit {
                via_lower_bound = via_lower_limit;
                break;
            }
            current_layer -= 1;
        }
        if via_lower_bound > drill.first_layer {
            return;
        }
        // Upward sweep (:300-315).
        let mut current_layer = from_layer + 1;
        loop {
            if current_layer > via_upper_limit {
                via_upper_bound = via_upper_limit;
                break;
            }
            let room_key = drill.room_arr[(current_layer - drill.first_layer) as usize]
                .expect("room exists on every drill layer");
            let room_shape = ctx.room_shape(room_key);
            let drill_result = check_layer_with_any_matching_via(
                &mut *ctx,
                ctrl,
                drill,
                current_layer,
                &room_shape,
                checker,
                net_number,
            );
            if drill_result == CheckDrillResult::NotDrillable {
                via_upper_bound = current_layer - 1;
                break;
            } else if drill_result == CheckDrillResult::DrillableWithAttachSmd
                && current_layer == layer_count - 1
            {
                smd_attached_on_solder_side = true;
            }
            current_layer += 1;
        }
        if via_upper_bound < drill.last_layer {
            return;
        }
    }
    // The mask loop + emission (:317-374).
    let mut to_layer = via_lower_bound;
    while to_layer <= via_upper_bound {
        if to_layer != from_layer {
            let (current_first_layer, current_last_layer) = if to_layer < from_layer {
                (to_layer, from_layer)
            } else {
                (from_layer, to_layer)
            };
            let mut mask_found = false;
            for mask in &ctrl.via_infos {
                if current_first_layer >= mask.from_layer
                    && current_last_layer <= mask.to_layer
                    && mask.from_layer >= via_lower_bound
                    && mask.to_layer <= via_upper_bound
                {
                    // Java precedence: && binds tighter than || —
                    // `!(A && B || C && D) || E`.
                    let mask_ok = !(mask.from_layer == 0 && smd_attached_on_component_side
                        || mask.to_layer == layer_count - 1 && smd_attached_on_solder_side)
                        || mask.attach_smd_allowed;
                    if mask_ok {
                        mask_found = true;
                        break;
                    }
                }
            }
            if mask_found {
                let current_room_index = to_layer - drill.first_layer;
                if !drill
                    .maze_search_element(current_room_index as usize)
                    .is_occupied
                {
                    let new_expansion_value = expansion_value
                        + f64::from(
                            ctrl.add_via_costs[from_layer as usize].to_layer[to_layer as usize],
                        );
                    let shape_entry_middle = shape_entry.a.middle_point(&shape_entry.b);
                    let sorting_value = new_expansion_value
                        + destination_distance.calculate(&shape_entry_middle, to_layer);
                    emit(DrillMazeListElement {
                        door_key: drill_key,
                        section_no_of_door: current_room_index,
                        backtrack_door_key: drill_key,
                        section_no_of_backtrack_door: section_no_of_door,
                        expansion_value: new_expansion_value,
                        sorting_value,
                        next_room_key: drill.room_arr[current_room_index as usize]
                            .expect("room exists on every drill layer"),
                        shape_entry: *shape_entry,
                        room_ripped,
                        adjustment: Adjustment::None,
                        already_checked: false,
                    });
                }
            }
        }
        to_layer += 1;
    }
}

/// Java `MazeExpansionEngine.checkLayerWithAnyMatchingVia`
/// (`:377-414`): the rule-via ITERATION and radius arithmetic are
/// ported; the per-layer probe is the T10 seam
/// ([`ViaLayerChecker::check_layer`]).
pub fn check_layer_with_any_matching_via(
    ctx: &mut impl DrillEngine,
    ctrl: &AutorouteControl,
    drill: &ExpansionDrill,
    layer: i32,
    room_shape: &TileShape,
    checker: &mut impl ViaLayerChecker,
    net_number: i32,
) -> CheckDrillResult {
    let mut drillable_with_attach_smd = false;
    for via in ctx.via_rule_vias() {
        let ViaRuleVia {
            padstack_no,
            clearance_class,
            attach_smd_allowed,
        } = via;
        let (from, to) = ctx.padstack_layer_span(padstack_no);
        if layer < from || layer > to {
            continue;
        }
        // Java `viaShape == null ? 0 : 0.5 * viaShape.maxWidth()`.
        let via_radius = ctx
            .padstack_shape(padstack_no, layer)
            .map_or(0.0, |shape| 0.5 * shape_max_width(&shape));
        let required_radius = via_radius.max(f64::from(ctrl.trace_half_width[layer as usize]));
        // The check routes THROUGH the ctx (T11): the production engine
        // overrides [`DrillEngine::via_layer_check`] to run the real
        // `forced_via_inserter::check_layer` (which needs `&mut
        // manager`+`&mut board` — exactly what the ctx owns), while the
        // capture/test engines keep the default that consults `checker`
        // — capture behavior preserved byte-for-row.
        let result = ctx.via_layer_check(
            ctrl,
            checker,
            required_radius,
            clearance_class,
            attach_smd_allowed,
            room_shape,
            &drill.location,
            layer,
            net_number,
        );
        if result == CheckDrillResult::Drillable {
            return result;
        }
        if result == CheckDrillResult::DrillableWithAttachSmd {
            drillable_with_attach_smd = true;
        }
    }
    if drillable_with_attach_smd {
        CheckDrillResult::DrillableWithAttachSmd
    } else {
        CheckDrillResult::NotDrillable
    }
}
