//! Synthetic-world unit pins for the drill subsystem (no capture rows
//! here — these pin STRUCTURE the synthetic harness controls; the
//! oracle-backed literal rows live in `pins.rs`). The world is a
//! minimal [`DrillEngine`] over hand-seeded items/rooms/tree entries.

use std::sync::atomic::AtomicBool;

use epic_board::items::BoardShape;
use epic_geometry::circle::Circle;
use epic_geometry::float_line::FloatLine;
use epic_geometry::float_point::FloatPoint;
use epic_geometry::int_box::IntBox;
use epic_geometry::int_octagon::IntOctagon;
use epic_geometry::int_point::IntPoint;
use epic_geometry::line::Line;
use epic_geometry::point::Point;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::tile_shape::TileShape;

use super::expansion_drill::ExpansionDrill;
use super::page_array::{DrillPageArray, max_drill_page_width};
use super::{
    CheckDrillResult, DrillEngine, DrillMazeListElement, ViaLayerChecker, ViaRuleVia,
    check_layer_with_any_matching_via, expand_to_other_layers,
};
use crate::control::{
    AngleRestriction, AutorouteControl, ExpansionCostFactor, RouterSettingsIr, ViaCost, ViaMask,
};
use crate::expansion::door::ExpansionDoor;
use crate::expansion::{NeighbourEngine, TargetItemExpansionDoor, TreeEntry};
use epic_index::SearchTreeVariant;
use epic_index::complete_shape::IncompleteRoom;

// ---- the synthetic world ------------------------------------------------

/// Item keys live at `ITEM_BASE + id` (rooms at `ROOM_BASE + id`) so
/// item/room key spaces never collide.
const ITEM_BASE: u64 = 1 << 40;
const ROOM_BASE: u64 = 2 << 40;

#[derive(Clone)]
struct SynthItem {
    id: i32,
    shapes: Vec<TileShape>,
    is_pin: bool,
    is_via: bool,
    drillable: bool,
    pin_drill_allowed: bool,
    /// The scripted STORED center — a via's own or a pin's, mirroring
    /// `Board::drill_center` (the shared field IS the Java
    /// `DrillItem.getCenter` shape: Some for a drill item, None for a
    /// plain item).
    stored_center: Option<Point>,
    padstack_no: i32,
    clearance_class: i32,
}

#[derive(Clone)]
struct SynthRoom {
    id: i32,
    layer: i32,
    obstacle: bool,
    shape: TileShape,
    obstacle_item: Option<u64>,
}

#[derive(Clone)]
struct SynthPadstack {
    no: i32,
    span: (i32, i32),
    shape: Option<BoardShape>,
}

struct SynthWorld {
    bounds: IntBox,
    layer_count: i32,
    items: Vec<SynthItem>,
    rooms: Vec<SynthRoom>,
    padstacks: Vec<SynthPadstack>,
    /// The raw tree leaves: `(object_key, shape_index, layer)` —
    /// deliberately UNSORTED (the walk order comes from
    /// `java_ordered_entries`).
    entries: Vec<(u64, u32, i32)>,
    /// The pre-seeded `overlappingItems` answers: `(item_key, layer)`
    /// in exactly the order the engine returns them (Java TreeSet:
    /// DESCENDING item id).
    item_overlaps: Vec<(u64, i32)>,
    complete_calls: Vec<(bool, i32)>,
    /// The `generateRoomIdNo` call count (the probe-oracle burn-count
    /// discipline: recomputation burns ids).
    id_burns: usize,
    complete_result: Vec<u64>,
    /// When set, a completion at layer L returns the SINGLE room
    /// `room_key(base + L)` (Java completes exactly one room per
    /// layer); `None` falls back to the static `complete_result`.
    complete_room_base: Option<i32>,
    stop: Option<AtomicBool>,
    rule_vias: Vec<ViaRuleVia>,
    /// The `ItemAutorouteInfo.startInfo` flags (`item_is_destination`
    /// is the negation; T6 seam).
    start_infos: std::collections::BTreeSet<u64>,
    /// The synthetic traces: `key -> length`. Presence IS the
    /// `item_is_trace` answer, so a world defaults to trace-free and
    /// existing pins are unaffected.
    trace_lengths: std::collections::HashMap<u64, f64>,
}

fn box_tile(x0: i32, y0: i32, x1: i32, y1: i32) -> TileShape {
    TileShape::RegularTileShape(RegularTileShape::IntBox(IntBox::new(
        IntPoint::new(x0, y0),
        IntPoint::new(x1, y1),
    )))
}

impl SynthWorld {
    fn new(bounds: IntBox, layer_count: i32) -> Self {
        Self {
            bounds,
            layer_count,
            items: Vec::new(),
            rooms: Vec::new(),
            padstacks: Vec::new(),
            entries: Vec::new(),
            item_overlaps: Vec::new(),
            complete_calls: Vec::new(),
            id_burns: 0,
            complete_result: Vec::new(),
            complete_room_base: None,
            stop: None,
            start_infos: std::collections::BTreeSet::new(),
            rule_vias: Vec::new(),
            trace_lengths: std::collections::HashMap::new(),
        }
    }

    fn item_key(id: i32) -> u64 {
        ITEM_BASE + id as u64
    }

    fn room_key(id: i32) -> u64 {
        ROOM_BASE + id as u64
    }

    fn item(&self, key: u64) -> &SynthItem {
        self.items
            .iter()
            .find(|item| Self::item_key(item.id) == key)
            .expect("item exists")
    }

    fn room(&self, key: u64) -> &SynthRoom {
        self.rooms
            .iter()
            .find(|room| Self::room_key(room.id) == key)
            .expect("room exists")
    }

    fn padstack(&self, no: i32) -> &SynthPadstack {
        self.padstacks
            .iter()
            .find(|padstack| padstack.no == no)
            .expect("padstack exists")
    }

    /// Adds a plain (non-pin, non-via, not drillable) item with tree
    /// shapes; every shape becomes a tree entry on `layer`.
    fn add_item(&mut self, id: i32, layer: i32, shapes: Vec<TileShape>) {
        self.entries.extend(
            shapes
                .iter()
                .enumerate()
                .map(|(idx, _)| (Self::item_key(id), idx as u32, layer)),
        );
        self.items.push(SynthItem {
            id,
            shapes,
            is_pin: false,
            is_via: false,
            drillable: false,
            pin_drill_allowed: false,
            stored_center: None,
            padstack_no: 0,
            clearance_class: 1,
        });
    }

    fn add_pin(&mut self, id: i32, layer: i32, drill_allowed: bool, center: Option<Point>) {
        if center.is_some() {
            self.item_overlaps.push((Self::item_key(id), layer));
        }
        self.items.push(SynthItem {
            id,
            shapes: Vec::new(),
            is_pin: true,
            is_via: false,
            drillable: false,
            pin_drill_allowed: drill_allowed,
            stored_center: center,
            padstack_no: 0,
            clearance_class: 1,
        });
    }

    fn add_via(
        &mut self,
        id: i32,
        layer: i32,
        padstack_no: i32,
        clearance_class: i32,
        center: Point,
    ) {
        self.items.push(SynthItem {
            id,
            shapes: Vec::new(),
            is_pin: false,
            is_via: true,
            drillable: false,
            pin_drill_allowed: false,
            stored_center: Some(center),
            padstack_no,
            clearance_class,
        });
        self.entries.push((Self::item_key(id), 0, layer));
    }

    fn add_room(&mut self, id: i32, layer: i32, shape: TileShape) {
        self.rooms.push(SynthRoom {
            id,
            layer,
            obstacle: false,
            shape,
            obstacle_item: None,
        });
        self.entries.push((Self::room_key(id), 0, layer));
    }

    fn add_padstack(&mut self, no: i32, span: (i32, i32), shape: Option<BoardShape>) {
        self.padstacks.push(SynthPadstack { no, span, shape });
    }

    fn set_rule_vias(&mut self, vias: Vec<ViaRuleVia>) {
        self.rule_vias = vias;
    }
}

impl NeighbourEngine for SynthWorld {
    fn net_number(&self) -> i32 {
        1
    }

    fn generate_room_id_no(&mut self) -> i32 {
        self.id_burns += 1;
        (self.rooms.len() + 1) as i32
    }

    fn board_bounding_octagon(&self) -> IntOctagon {
        IntOctagon::EMPTY
    }

    fn add_incomplete_expansion_room(
        &mut self,
        _shape: TileShape,
        _layer: i32,
        _contained_shape: TileShape,
    ) -> u64 {
        0
    }

    fn remove_all_doors(&mut self, _room_key: u64) {}

    fn add_complete_free_space_room(&mut self, _shape: TileShape, _layer: i32, _id: i32) -> u64 {
        0
    }

    fn overlapping_entries(&mut self, shape: &TileShape, layer: i32) -> Vec<TreeEntry> {
        self.entries
            .iter()
            .filter(|(key, idx, entry_layer)| {
                (layer == -1 || *entry_layer == layer)
                    && self
                        .tree_shape(*key, *idx)
                        .expect("entry shape")
                        .intersection(shape)
                        .dimension()
                        >= 0
            })
            .map(|(key, idx, _)| TreeEntry {
                object_key: *key,
                shape_index_in_object: *idx,
            })
            .collect()
    }

    fn object_id(&self, object_key: u64) -> i32 {
        if object_key >= ROOM_BASE {
            self.room(object_key).id
        } else {
            self.item(object_key).id
        }
    }

    fn is_trace_obstacle(&self, _object_key: u64, _net_number: i32) -> bool {
        false
    }

    fn tree_shape(&self, object_key: u64, shape_index: u32) -> Option<TileShape> {
        if object_key >= ROOM_BASE {
            Some(self.room(object_key).shape.clone())
        } else {
            self.item(object_key)
                .shapes
                .get(shape_index as usize)
                .cloned()
        }
    }

    fn complete_shape(
        &mut self,
        _room_shape: Option<&TileShape>,
        _contained: Option<&TileShape>,
        _layer: i32,
        _ignore_object: Option<u64>,
        _ignore_shape: Option<&TileShape>,
    ) -> Vec<IncompleteRoom> {
        Vec::new()
    }

    fn tree_object_room(&self, object_key: u64) -> Option<u64> {
        (object_key >= ROOM_BASE).then_some(object_key)
    }

    fn is_item(&self, object_key: u64) -> bool {
        object_key < ROOM_BASE
    }

    fn item_is_routable(&self, _object_key: u64) -> bool {
        false
    }

    fn item_is_connectable(&self, _object_key: u64) -> bool {
        false
    }

    fn item_contains_net(&self, _object_key: u64, _net_number: i32) -> bool {
        false
    }

    fn item_shares_net(&self, _first_key: u64, _second_key: u64) -> bool {
        false
    }

    fn item_is_polyline_trace(&self, _object_key: u64) -> bool {
        false
    }

    fn item_expansion_room(&mut self, _object_key: u64, _shape_index: u32) -> Option<u64> {
        None
    }

    fn trace_connection_shape(&self, _object_key: u64, _shape_index: u32) -> Option<TileShape> {
        None
    }

    fn trace_first_or_last_parallel(
        &self,
        _item_key: u64,
        _index_in_item: u32,
        _door_line: &Line,
    ) -> Option<bool> {
        None
    }

    fn room_shape(&self, room_key: u64) -> TileShape {
        self.room(room_key).shape.clone()
    }

    fn room_layer(&self, room_key: u64) -> i32 {
        self.room(room_key).layer
    }

    fn room_id(&self, room_key: u64) -> i32 {
        self.room(room_key).id
    }

    fn room_is_incomplete(&self, _room_key: u64) -> bool {
        false
    }

    fn room_is_obstacle(&self, room_key: u64) -> bool {
        self.room(room_key).obstacle
    }

    fn room_is_complete_free_space(&self, _room_key: u64) -> bool {
        true
    }

    fn room_contained_shape(&self, _room_key: u64) -> Option<TileShape> {
        None
    }

    fn room_obstacle_item_key(&self, room_key: u64) -> Option<u64> {
        self.room(room_key).obstacle_item
    }

    fn room_obstacle_index_in_item(&self, _room_key: u64) -> Option<u32> {
        Some(0)
    }

    fn room_has_door_to(&self, _room_key: u64, _other_room_id: i32) -> bool {
        false
    }

    fn room_doors(&self, _room_key: u64) -> Vec<ExpansionDoor> {
        Vec::new()
    }

    fn room_key_of_id(&self, _id: i32) -> Option<u64> {
        None
    }

    fn room_key_of_door(&self, _id: i32, _door: &ExpansionDoor) -> Option<u64> {
        None
    }

    fn set_incomplete_shape(
        &mut self,
        _room_key: u64,
        _shape: TileShape,
        _contained_shape: TileShape,
    ) {
    }

    fn set_room_shape(&mut self, _room_key: u64, _shape: TileShape) {}

    fn attach_door(&mut self, _room_key: u64, _door: ExpansionDoor) {}

    fn add_target_door(&mut self, _room_key: u64, _door: TargetItemExpansionDoor) {}

    fn set_net_dependent(&mut self, _room_key: u64) {}

    // ---- T6 seams (scripted stubs; this world tests the drill-layer
    // logic against a SCRIPTED completion, not the production one —
    // the oracle-replay Harness in `pins.rs` owns that verification) ----
    fn tree_variant(&self) -> SearchTreeVariant {
        SearchTreeVariant::Generic
    }

    fn remove_incomplete_room(&mut self, room_key: u64) {
        self.rooms.retain(|r| Self::room_key(r.id) != room_key);
    }

    fn flush_completed_inserts(&mut self, _accepted: Option<u64>) {}

    fn room_target_doors(&self, _room_key: u64) -> Vec<TargetItemExpansionDoor> {
        Vec::new()
    }

    fn room_obstacle_doors_calculated(&self, _room_key: u64) -> bool {
        false
    }

    fn set_room_doors_calculated(&mut self, _room_key: u64) {}
}

impl DrillEngine for SynthWorld {
    fn board_bounds(&self) -> IntBox {
        self.bounds
    }

    fn layer_count(&self) -> i32 {
        self.layer_count
    }

    fn stop_flag(&self) -> Option<&AtomicBool> {
        self.stop.as_ref()
    }

    fn overlapping_items(&self, _shape: &TileShape, layer: i32) -> Vec<u64> {
        self.item_overlaps
            .iter()
            .filter(|(_, entry_layer)| *entry_layer == layer)
            .map(|(key, _)| *key)
            .collect()
    }

    fn item_is_drillable(&self, item_key: u64, _net_number: i32) -> bool {
        self.item(item_key).drillable
    }

    fn item_is_pin(&self, item_key: u64) -> bool {
        self.item(item_key).is_pin
    }

    fn pin_drill_allowed(&self, item_key: u64) -> bool {
        self.item(item_key).pin_drill_allowed
    }

    fn pin_center(&self, item_key: u64) -> Option<Point> {
        self.item(item_key).stored_center.clone()
    }

    fn via_center(&self, item_key: u64) -> Option<Point> {
        // The scripted STORED center — the via's own (every add_via
        // scripts one), mirroring `Board::drill_center`'s
        // via arm. Honored contract face (quality-review T17a-2 Q-4):
        // Some for a live scripted via, None only for a center-less
        // plain item (the trait's defensive shape). Observed by
        // `synth_via_center_answers_the_scripted_stored_center`.
        self.item(item_key).stored_center.clone()
    }

    fn item_is_via(&self, item_key: u64) -> bool {
        self.item(item_key).is_via
    }

    fn item_padstack_no(&self, item_key: u64) -> Option<i32> {
        let no = self.item(item_key).padstack_no;
        (no > 0).then_some(no)
    }

    fn item_clearance_class(&self, item_key: u64) -> i32 {
        self.item(item_key).clearance_class
    }

    fn padstack_layer_span(&self, padstack_no: i32) -> (i32, i32) {
        self.padstack(padstack_no).span
    }

    fn padstack_shape(&self, padstack_no: i32, layer: i32) -> Option<BoardShape> {
        let padstack = self.padstack(padstack_no);
        if layer >= padstack.span.0 && layer <= padstack.span.1 {
            padstack.shape.clone()
        } else {
            None
        }
    }

    fn via_rule_vias(&self) -> Vec<ViaRuleVia> {
        self.rule_vias.clone()
    }

    fn complete_expansion_room(
        &mut self,
        room_shape: Option<&TileShape>,
        _contained_shape: &TileShape,
        layer: i32,
    ) -> Vec<u64> {
        self.complete_calls.push((room_shape.is_none(), layer));
        if let Some(base) = self.complete_room_base {
            vec![Self::room_key(base + layer)]
        } else {
            self.complete_result.clone()
        }
    }

    // ---- T6 seams (scripted stubs) ----
    fn item_is_destination(&self, item_key: u64) -> bool {
        !self.start_infos.contains(&item_key)
    }

    fn set_item_start_info(&mut self, item_key: u64, start: bool) {
        if start {
            self.start_infos.insert(item_key);
        } else {
            self.start_infos.remove(&item_key);
        }
    }

    fn item_tree_shape_count(&self, item_key: u64) -> i32 {
        self.item(item_key).shapes.len() as i32
    }

    fn pin_neckdown_half_width(&self, _item_key: u64, _layer: i32) -> f64 {
        0.0
    }

    fn layer_is_signal(&self, _layer: i32) -> bool {
        true
    }

    fn drill_hits_foreign_conduction(
        &self,
        _location: &Point,
        _layer: i32,
        _net_number: i32,
    ) -> bool {
        false
    }

    fn pin_nearest_trace_exit_corner(
        &self,
        _item_key: u64,
        _from_point: &FloatPoint,
        _trace_half_width: i32,
        _layer: i32,
    ) -> Option<FloatPoint> {
        None
    }

    fn item_is_trace(&self, item_key: u64) -> bool {
        self.trace_lengths.contains_key(&item_key)
    }

    fn item_trace_half_width(&self, _item_key: u64) -> i32 {
        0
    }

    fn clearance_compensation_value(&self, _clearance_class: i32, _layer: i32) -> i32 {
        0
    }

    fn item_shape_layer(&self, item_key: u64, shape_index: u32) -> i32 {
        self.entries
            .iter()
            .find(|(key, index, _)| *key == item_key && *index == shape_index)
            .map(|(_, _, layer)| *layer)
            .unwrap_or(-1)
    }

    fn item_tree_shape_on_layer(&self, item_key: u64, layer: i32) -> Option<TileShape> {
        let item = self.item(item_key);
        let on_layer = self
            .entries
            .iter()
            .find(|(key, index, l)| {
                *key == item_key && *l == layer && (*index as usize) < item.shapes.len()
            })
            .map(|(_, index, _)| *index)?;
        item.shapes.get(on_layer as usize).cloned()
    }

    fn trace_angle_restriction(&self) -> AngleRestriction {
        AngleRestriction::FortyfiveDegree
    }

    // ---- T7 seams (scripted stubs; the synthetic worlds carry no
    // traces or contact topology — the oracle-replay Harness in
    // `pins.rs` owns the real reads) ----
    fn item_normal_contacts(&mut self, _item_key: u64) -> Vec<u64> {
        Vec::new()
    }

    fn trace_normal_contacts_at(
        &mut self,
        _item_key: u64,
        _point: &Point,
        _ignore_net: bool,
    ) -> Vec<u64> {
        Vec::new()
    }

    fn trace_start_contacts(&mut self, _item_key: u64) -> Vec<u64> {
        Vec::new()
    }

    fn trace_end_contacts(&mut self, _item_key: u64) -> Vec<u64> {
        Vec::new()
    }

    fn normal_contact_point(&mut self, _first_key: u64, _second_key: u64) -> Option<Point> {
        None
    }

    fn first_common_layer(&mut self, _first_key: u64, _second_key: u64) -> i32 {
        -1
    }

    fn item_is_user_fixed(&self, _item_key: u64) -> bool {
        false
    }

    fn item_is_shove_fixed(&self, _item_key: u64) -> bool {
        false
    }

    fn item_trace_length(&self, item_key: u64) -> f64 {
        self.trace_lengths.get(&item_key).copied().unwrap_or(0.0)
    }

    fn item_trace_polyline(&self, _item_key: u64) -> Option<epic_geometry::polyline::Polyline> {
        None
    }

    fn overlapping_objects_ignore_nets(
        &self,
        _shape: &TileShape,
        _layer: i32,
        _ignore_nets: &[i32],
    ) -> Vec<u64> {
        Vec::new()
    }
}

// ---- the scripted checker / distance stubs ------------------------------

/// Returns scripted results per `(layer, clearance_class)`, with a
/// default for unscripted pairs; records every call.
struct ScriptedChecker {
    script: Vec<(i32, i32, CheckDrillResult)>,
    default_result: CheckDrillResult,
    calls: Vec<(f64, i32, bool, i32, i32)>,
}

impl ScriptedChecker {
    fn new(default_result: CheckDrillResult) -> Self {
        Self {
            script: Vec::new(),
            default_result,
            calls: Vec::new(),
        }
    }

    fn script(&mut self, layer: i32, class: i32, result: CheckDrillResult) -> &mut Self {
        self.script.push((layer, class, result));
        self
    }
}

impl ViaLayerChecker for ScriptedChecker {
    fn check_layer(
        &mut self,
        required_radius: f64,
        clearance_class: i32,
        attach_smd_allowed: bool,
        _room_shape: &TileShape,
        location: &Point,
        layer: i32,
        _net_number: i32,
    ) -> CheckDrillResult {
        let Point::Int(location) = location else {
            panic!("drill location is an IntPoint");
        };
        self.calls.push((
            required_radius,
            clearance_class,
            attach_smd_allowed,
            layer,
            location.x,
        ));
        self.script
            .iter()
            .find(|(script_layer, script_class, _)| {
                *script_layer == layer && *script_class == clearance_class
            })
            .map_or(self.default_result, |(_, _, result)| *result)
    }
}

struct FixedDistance(f64);

impl super::DestinationDistance for FixedDistance {
    fn calculate(&self, _middle: &FloatPoint, _layer: i32) -> f64 {
        self.0
    }
}

// ---- shared builders ----------------------------------------------------

fn square_bounds(x0: i32, y0: i32, side: i32) -> IntBox {
    IntBox::new(IntPoint::new(x0, y0), IntPoint::new(x0 + side, y0 + side))
}

/// An `AutorouteControl` with only the fields the drill code reads:
/// 4 layers, `viaUpperBound` = layer count, asymmetric via-cost rows
/// (`[from][to]`: row 0 = 90..93, row 1 = 10..13, row 2 = 20..23,
/// row 3 = 30..33) so a transposed cost lookup reads the wrong value.
fn test_ctrl(via_upper_bound: i32, via_infos: Vec<ViaMask>) -> AutorouteControl {
    let layer_count = 4usize;
    let cost_rows: [Vec<i32>; 4] = [
        vec![90, 91, 92, 93],
        vec![10, 11, 12, 13],
        vec![20, 21, 22, 23],
        vec![30, 31, 32, 33],
    ];
    AutorouteControl {
        settings: RouterSettingsIr {
            trace_costs: Vec::new(),
            via_costs: 1,
            vias_allowed: true,
            bend_costs: Vec::new(),
            layer_active: vec![true; layer_count],
            automatic_neckdown: false,
            start_ripup_costs: 1,
            fanout: Default::default(),
        },
        trace_costs: vec![
            ExpansionCostFactor {
                horizontal: 1.0,
                vertical: 1.0,
            };
            layer_count
        ],
        bend_costs: vec![0.0; layer_count],
        with_neckdown: false,
        layer_active: vec![true; layer_count],
        layer_count,
        trace_half_width: vec![50; layer_count],
        compensated_trace_half_width: vec![50; layer_count],
        via_radii: vec![0.0; layer_count],
        add_via_costs: cost_rows
            .iter()
            .map(|row| ViaCost {
                to_layer: row.clone(),
            })
            .collect(),
        trace_clearance_class_index: 1,
        vias_allowed: true,
        attach_smd_allowed: false,
        min_normal_via_cost: 0.0,
        ripup_allowed: false,
        ripup_costs: 1000,
        ripup_pass_no: 1,
        is_fanout: false,
        fanout_start_pin_name: None,
        fanout_start_pin_center: None,
        fanout_start_pin_layer: -1,
        remove_unconnected_vias: true,
        push_shove: false,
        via_rule: None,
        net_number: 1,
        via_clearance_class: 2,
        via_infos,
        via_lower_bound: 0,
        via_upper_bound,
        max_via_radius: 0.0,
        tidy_region_width: i32::MAX,
        pull_tight_accuracy: 500,
        max_shove_trace_recursion_depth: 20,
        max_shove_via_recursion_depth: 5,
        max_spring_over_recursion_depth: 5,
        min_cheap_via_cost: 0.0,
        coupling: None,
    }
}

/// A 4-layer drill spanning layers 0..=3 with one free-space room per
/// layer (`room_arr = [101, 102, 103, 104]`).
fn drill_with_rooms() -> ExpansionDrill {
    let mut drill = ExpansionDrill::new(
        box_tile(0, 0, 100, 100),
        Point::Int(IntPoint::new(50, 50)),
        0,
        3,
    );
    for (index, room_id) in [101, 102, 103, 104].iter().enumerate() {
        drill.room_arr[index] = Some(SynthWorld::room_key(*room_id));
    }
    drill
}

fn rooms_world() -> SynthWorld {
    let mut world = SynthWorld::new(square_bounds(0, 0, 100), 4);
    for (layer, room_id) in [101, 102, 103, 104].iter().enumerate() {
        world.add_room(*room_id, layer as i32, box_tile(0, 0, 100, 100));
    }
    world
}

fn shape_entry() -> FloatLine {
    FloatLine::new(
        FloatPoint { x: 10.0, y: 20.0 },
        FloatPoint { x: 30.0, y: 60.0 },
    )
}

fn emit_keys(emitted: &[DrillMazeListElement]) -> Vec<i32> {
    emitted
        .iter()
        .map(|element| element.section_no_of_door)
        .collect()
}

// ---- the pins -----------------------------------------------------------

/// Java `AutorouteEngine.java:89-91`: `max((int)(5 * d), 10000)` —
/// the double product is TRUNCATED toward zero first, THEN clamped.
/// The 2000.3 row kills a round-first mutant (10001, not 10002); the
/// 1500.0 row kills a clamp-first mutant (would read max(7500) wrong
/// way); NaN truncates to 0 like the Java cast.
#[test]
fn max_drill_page_width_truncates_then_clamps() {
    assert_eq!(
        max_drill_page_width(2000.0),
        10_000,
        "5*2000 = 10000, clamp is a no-op"
    );
    assert_eq!(max_drill_page_width(1500.0), 10_000, "7500 clamps up");
    assert_eq!(
        max_drill_page_width(2050.0),
        10_250,
        "5*2050 above the clamp"
    );
    assert_eq!(
        max_drill_page_width(2000.3),
        10_001,
        "10001.5 TRUNCATES (Java int cast), a round mutant gives 10002"
    );
    assert_eq!(
        max_drill_page_width(f64::NAN),
        10_000,
        "(int) NaN = 0, clamped"
    );
}

/// Ctor (`DrillPageArray.java:34-66`): `ceil(10000/4000) = 3`
/// columns/rows, then `pageWidth = ceil(10000/3) = 3334` — the
/// redistribution division — and the LAST column/row snapped to
/// `bounds.ur` so the grid covers the board exactly.
#[test]
fn page_grid_arithmetic_snaps_last_row_column() {
    let world = SynthWorld::new(square_bounds(0, 0, 10_000), 2);
    let array = DrillPageArray::new(&world, 4000);
    assert_eq!(array.column_count, 3);
    assert_eq!(array.row_count, 3);
    assert_eq!(array.page_width, 3334, "ceil(10000/3)");
    assert_eq!(array.page_height, 3334);
    let cell = |j: usize, i: usize| {
        let page = &array.pages[j][i];
        (
            page.shape.ll.x,
            page.shape.ll.y,
            page.shape.ur.x,
            page.shape.ur.y,
        )
    };
    assert_eq!(cell(0, 0), (0, 0, 3334, 3334));
    assert_eq!(cell(0, 1), (3334, 0, 6668, 3334));
    assert_eq!(
        cell(0, 2),
        (6668, 0, 10_000, 3334),
        "last column snapped to ur.x"
    );
    assert_eq!(
        cell(2, 0),
        (0, 6668, 3334, 10_000),
        "last row snapped to ur.y"
    );
    assert_eq!(
        cell(2, 2),
        (6668, 6668, 10_000, 10_000),
        "corner page double-snapped"
    );
}

/// `overlappingPages` (`:76-97`): `maxJ`/`maxI` stay DOUBLES and the
/// loop test is the strict `j < maxJ` — with pageWidth 4000 over a
/// 12000 board, a probe covering the whole board has `maxJ = 3.0`
/// EXACTLY; an inclusive (`<=`) mutant would scan row/column 3 and
/// panic on the missing grid row. The dim>1 filter row pins the
/// dimension check (a line-thin overlap is not an overlap).
#[test]
fn overlapping_pages_strict_upper_bound_at_exact_integer() {
    let world = SynthWorld::new(square_bounds(0, 0, 12_000), 2);
    let array = DrillPageArray::new(&world, 4000);
    // Whole-board probe: exact-integer max bounds on BOTH axes.
    let coords = array.overlapping_pages(&box_tile(0, 0, 12_000, 12_000));
    let mut expected = Vec::new();
    for j in 0..3 {
        for i in 0..3 {
            expected.push((j, i));
        }
    }
    assert_eq!(
        coords, expected,
        "all 9 pages, no row/column 3 (a <= mutant panics)"
    );
    // Exact-boundary inner probe: rows [4000, 8000] -> exactly row 1.
    assert_eq!(
        array.overlapping_pages(&box_tile(0, 4000, 12_000, 8000)),
        vec![(1, 0), (1, 1), (1, 2)],
        "min = floor(4000/4000) = 1, max = 2.0 strict"
    );
    // Edge-touching probe: [8000, 12000] -> exactly column 2.
    assert_eq!(
        array.overlapping_pages(&box_tile(8000, 0, 12_000, 4000)),
        vec![(0, 2)],
        "the exact-integer upper edge is EXCLUDED"
    );
    // Full-width bottom strip: maxJ = 4000/4000 = 1.0 EXACT again —
    // row 0 only (an inclusive mutant scans row 1 and panics).
    assert_eq!(
        array.overlapping_pages(&box_tile(0, 0, 12_000, 4000)),
        vec![(0, 0), (0, 1), (0, 2)],
        "row bound excluded at the exact-integer edge"
    );
}

/// Java `ExpansionDrill.getId()` (`:127-130`):
/// `31 * (31 * location.getId() + firstLayer) + lastLayer` over
/// wrapping Java ints. The point id is chosen so `31 * pid` overflows
/// i32 — the expected value is the low 32 bits of the i64
/// evaluation, so a debug-panicking (non-wrapping) mutant dies.
#[test]
fn expansion_drill_get_id_wraps_like_java_int() {
    let location = Point::Int(IntPoint::new(1_000_000, 2_000_000));
    let pid = location.get_id();
    assert!(
        i64::from(pid) * 31 * 31 > i32::MAX as i64,
        "precondition: 31 * (31 * pid) overflows i32 (pid = {pid})"
    );
    let drill = ExpansionDrill::new(box_tile(0, 0, 10, 10), location, 0, 3);
    let first_layer = 0i64;
    let last_layer = 3i64;
    let expected = ((31i64) * ((31i64) * i64::from(pid) + first_layer) + last_layer) as i32;
    assert_eq!(drill.get_id(), expected, "low 32 bits (Java int overflow)");
}

/// `getDrills` cutout walk: entries skipped by the filters must NOT
/// update `prevObstacleShape` (the skip happens BEFORE the
/// assignment, `DrillPage.java:87-96`). Item 30 is drillable (shape
/// T), item 20 carries the SAME shape T, item 10 carries O. Correct
/// port: the drillable entry leaves `prev = EMPTY`, so T IS cut, then
/// O — holes {T, O}. A mutant that updates `prev` on skipped entries
/// drops the T cutout (prev = T contains T) and cuts only O — a
/// DIFFERENT decomposition (pinned by the exact piece boxes below).
#[test]
fn get_drills_skipped_entries_do_not_update_prev_shape() {
    let mut world = SynthWorld::new(square_bounds(0, 0, 3000), 1);
    let t_shape = box_tile(0, 0, 1000, 1000);
    world.add_item(30, 0, vec![t_shape.clone()]);
    world.items[0].drillable = true; // skipped by the isDrillable filter
    world.add_item(20, 0, vec![t_shape.clone()]);
    world.add_item(10, 0, vec![box_tile(2000, 0, 3000, 1000)]);
    // Completion yields one free-space room per layer.
    world.complete_result = vec![SynthWorld::room_key(501)];
    world.add_room(501, 0, box_tile(0, 0, 3000, 3000));

    let mut array = DrillPageArray::new(&world, 10_000);
    let page = &mut array.pages[0][0];
    let drills = page.get_drills(&mut world, 1, false);
    let boxes: Vec<(i32, i32, i32, i32)> = drills
        .iter()
        .map(|drill| {
            let bb = drill.get_shape().bounding_box();
            (bb.ll.x, bb.ll.y, bb.ur.x, bb.ur.y)
        })
        .collect();
    println!("SKIPPED_PREV boxes = {boxes:?} len = {}", drills.len());
    assert_eq!(
        boxes,
        [(1000, 0, 2000, 1000), (0, 1000, 3000, 3000)],
        "the exact decomposition of page minus the two cutouts: BOTH holes are cut \
         (a prev-update-on-skip mutant cuts only O and cannot produce the bottom-middle piece)"
    );
}

/// The duplicate-shape dedup: two non-drillable items with the SAME
/// shape X produce the SAME drill set as a single X (Java skips the
/// second because `prev.contains(current)`). Contrast witness: the
/// reference world with one X, same O — piece lists must match
/// exactly. (The prev-vs-union distinction is region-invisible in
/// the split output — see the module doc; THIS pin is the regression
/// net for the literal port.)
#[test]
fn get_drills_duplicate_shapes_do_not_duplicate_drills() {
    let build = |with_dup: bool| {
        let mut world = SynthWorld::new(square_bounds(0, 0, 3000), 1);
        let x_shape = box_tile(0, 0, 1000, 1000);
        world.add_item(30, 0, vec![x_shape.clone()]);
        if with_dup {
            world.add_item(20, 0, vec![x_shape.clone()]);
        }
        world.add_item(10, 0, vec![box_tile(2000, 0, 3000, 1000)]);
        world.complete_result = vec![SynthWorld::room_key(501)];
        world.add_room(501, 0, box_tile(0, 0, 3000, 3000));
        world
    };
    let mut single = build(false);
    let mut dup = build(true);
    let mut single_array = DrillPageArray::new(&single, 10_000);
    let mut dup_array = DrillPageArray::new(&dup, 10_000);
    let single_page = &mut single_array.pages[0][0];
    let dup_page = &mut dup_array.pages[0][0];
    let single_drills = single_page.get_drills(&mut single, 1, false);
    let dup_drills = dup_page.get_drills(&mut dup, 1, false);
    let key = |drills: &[super::ExpansionDrill]| {
        drills
            .iter()
            .map(|drill| {
                let bb = drill.get_shape().bounding_box();
                (bb.ll.x, bb.ll.y, bb.ur.x, bb.ur.y)
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        key(single_drills),
        key(dup_drills),
        "the duplicate X is skipped"
    );
    assert!(!single_drills.is_empty(), "both worlds decompose the page");
}

/// `calcPinCenterInDrill` (`:49-60`): the overlapping-items walk has
/// NO break — the LAST match wins, i.e. the LOWEST-id pin (the
/// engine returns items in Java TreeSet order: DESCENDING id). Pins
/// 5 and 3 BOTH overlap on layer 0 and both centers sit inside the
/// piece; the location is pin 3's center. A first-match mutant picks
/// pin 5's center and dies. The second world (pin on layer 1 only)
/// pins the layer-0-miss fallback to the LAST layer.
#[test]
fn get_drills_attach_smd_pin_center_last_match_wins() {
    let mut world = SynthWorld::new(square_bounds(0, 0, 3000), 2);
    world.add_pin(5, 0, true, Some(Point::Int(IntPoint::new(2000, 2000))));
    world.add_pin(3, 0, true, Some(Point::Int(IntPoint::new(800, 900))));
    // No tree rooms: every layer goes through the completion seam,
    // which hands back ONE room per layer (501 / 502).
    world.complete_room_base = Some(501);

    let mut array = DrillPageArray::new(&world, 10_000);
    let page = &mut array.pages[0][0];
    let drills = page.get_drills(&mut world, 1, true);
    assert_eq!(drills.len(), 1, "no cutouts: one convex piece");
    assert_eq!(
        drills[0].location,
        Point::Int(IntPoint::new(800, 900)),
        "the LAST (lowest-id) matching pin wins"
    );
    // The completion seam saw the point-degenerate search box (Java
    // `TileShape.getInstance(location)` -> the null-room arm) on both
    // layers, in order.
    assert_eq!(world.complete_calls, vec![(true, 0), (true, 1)]);

    // Fallback: no pin overlaps layer 0 -> the LAST layer is probed.
    let mut world = SynthWorld::new(square_bounds(0, 0, 3000), 2);
    world.add_pin(7, 1, true, Some(Point::Int(IntPoint::new(400, 400))));
    world.complete_room_base = Some(501);
    let mut array = DrillPageArray::new(&world, 10_000);
    let page = &mut array.pages[0][0];
    let drills = page.get_drills(&mut world, 1, true);
    assert_eq!(
        drills[0].location,
        Point::Int(IntPoint::new(400, 400)),
        "layer-0 miss falls back to the LAST layer"
    );
}

/// The memoization trap (`DrillPage.java:65`): the key is
/// `(drills == null || netNumber changed)` — `attachSmd` is NOT in
/// the key. First call with `attach = false` (pin 3 stays a cutout
/// obstacle -> the page splits around its hole); the second call
/// with `attach = true` returns the STALE drills (same piece count —
/// a key-includes-attach mutant recomputes into the hole-free single
/// piece and dies on the count). A NET change DOES recompute (after
/// `invalidate`, net 2): the hole-free single piece anchors at the
/// pin center.
#[test]
fn get_drills_memoization_ignores_attach_smd_but_not_net() {
    let mut world = SynthWorld::new(square_bounds(0, 0, 3000), 2);
    // Pin 3 is drill-allowed and overlapping on layer 0; it also
    // carries an obstacle shape that becomes a cutout when
    // attach=false.
    world.add_pin(3, 0, true, Some(Point::Int(IntPoint::new(800, 900))));
    world.items[0].shapes = vec![box_tile(500, 500, 1100, 1300)];
    world.entries.push((SynthWorld::item_key(3), 0, 0));
    world.complete_result = vec![SynthWorld::room_key(501), SynthWorld::room_key(502)];
    world.add_room(501, 0, box_tile(0, 0, 3000, 3000));
    world.add_room(502, 1, box_tile(0, 0, 3000, 3000));

    let mut array = DrillPageArray::new(&world, 10_000);
    let page = &mut array.pages[0][0];
    // Each observation is copied out while the `&mut page` borrow is
    // alive (get_drills returns a slice tied to it).
    let (stale_len, stale_first) = {
        let stale = page.get_drills(&mut world, 1, false);
        (
            stale.len(),
            (stale[0].location.clone(), stale[0].first_layer),
        )
    };
    assert!(
        stale_len >= 4,
        "a page with an interior rectangular hole needs >= 4 convex pieces (got {stale_len})"
    );
    let (cached_len, cached_first) = {
        let cached = page.get_drills(&mut world, 1, true);
        (
            cached.len(),
            (cached[0].location.clone(), cached[0].first_layer),
        )
    };
    assert_eq!(
        cached_len, stale_len,
        "attachSmd is NOT in the memo key: the stale piece count comes back"
    );
    assert_eq!(cached_first, stale_first, "the stale drills themselves");
    page.invalidate();
    let fresh = page.get_drills(&mut world, 2, true);
    assert_eq!(
        fresh.len(),
        1,
        "recomputed with attach=true: the pin is no cutout"
    );
    assert_eq!(
        fresh[0].location,
        Point::Int(IntPoint::new(800, 900)),
        "a net-number change recomputes (and now attach=true anchors the pin)"
    );
}

/// Java `reset()` (`DrillPage.java:153-164`) KEEPS the drills memo:
/// it resets each memoized drill's maze-search elements and the
/// page's own elements; only `invalidate()` (`:170-172`) nulls the
/// memo. Pin: compute the drills, dirty a page maze element, reset,
/// then `get_drills` for the SAME net — the MEMOIZED drills come back
/// (same count and anchor locations), the page element is back to its
/// initializer, and the completion seam sees ZERO calls and ZERO
/// room-id burns (an invalidate-in-reset mutant recomputes: non-empty
/// `complete_calls`, fresh burns). The drill-element reset arm is the
/// same loop shape (each memoized `ExpansionDrill::reset`, Java
/// `ExpansionDrill.java:120-123`); its state is not directly writable
/// through the read-only drills slice, so the page-element arm is the
/// observable witness here.
///
/// The world seeds NO tree room — a pre-seeded complete room lets
/// `calculate_expansion_rooms` short-circuit through the
/// overlapping-complete-room reuse and the completion seam never
/// fires, which made the first draft's zero-calls witness VACUOUS
/// (the invalidate-in-reset mutant survived it; caught by the
/// mutation re-run). With an empty tree, every drill computation is
/// forced through the seam.
#[test]
fn page_reset_keeps_the_drills_memo() {
    let mut world = SynthWorld::new(square_bounds(0, 0, 3000), 1);
    world.complete_result = vec![SynthWorld::room_key(501)];
    let mut array = DrillPageArray::new(&world, 10_000);
    let page = &mut array.pages[0][0];
    let (len, first_location) = {
        let drills = page.get_drills(&mut world, 1, false);
        assert!(!drills.is_empty(), "the page decomposes");
        (drills.len(), drills[0].location.clone())
    };
    // Dirty a page maze element (what a partial layer-change pass
    // leaves behind) and reset.
    page.maze_search_element_mut(0).is_occupied = true;
    page.reset();
    world.complete_calls.clear();
    world.id_burns = 0;
    let drills = page.get_drills(&mut world, 1, false);
    assert_eq!(drills.len(), len, "the memoized drills come back");
    assert_eq!(
        drills[0].location, first_location,
        "the SAME memoized drills (no recomputation)"
    );
    assert!(
        !page.maze_search_element(0).is_occupied,
        "reset cleared the page's maze elements"
    );
    assert!(
        world.complete_calls.is_empty(),
        "reset does NOT invalidate: no completion calls"
    );
    assert_eq!(world.id_burns, 0, "reset does NOT invalidate: no id burn");
}

/// `calculateExpansionRooms` (`:55-92`): a completion that does not
/// yield EXACTLY one room rejects the whole drill — the drill is NOT
/// added and `getDrills` returns an empty (but memoized) list.
#[test]
fn get_drills_rejects_drill_when_completion_yields_not_exactly_one_room() {
    let mut world = SynthWorld::new(square_bounds(0, 0, 3000), 2);
    world.complete_result = Vec::new(); // both completions yield zero rooms
    let mut array = DrillPageArray::new(&world, 10_000);
    let page = &mut array.pages[0][0];
    let drills = page.get_drills(&mut world, 1, false);
    assert!(drills.is_empty(), "every drill is rejected");
    // Java-faithful: the completion is attempted layer by layer and
    // the drill is REJECTED at the first layer that fails (the
    // single piece never reaches layer 1).
    assert_eq!(
        world.complete_calls,
        vec![(true, 0)],
        "fail-fast on the first non-single-room completion"
    );
}

/// `expandToOtherLayers` free-space path (`:263-315`, `:317-374`):
/// all four layers drillable, from-layer 1 -> the downward sweep
/// checks layers [1, 0], the upward sweep [2, 3]; emissions to
/// layers {0, 2, 3} with the asymmetric per-row via costs (row 1 =
/// 10..13) and the destination distance folded into the sorting
/// value. The occupancy check silences layer 2 in the second half.
#[test]
fn expand_to_other_layers_sweeps_costs_and_emits() {
    let mut world = rooms_world();
    world.add_padstack(
        7,
        (0, 3),
        Some(BoardShape::Circle(Circle::new(IntPoint::new(0, 0), 400))),
    );
    world.set_rule_vias(vec![ViaRuleVia {
        padstack_no: 7,
        clearance_class: 4,
        attach_smd_allowed: false,
    }]);
    let ctrl = test_ctrl(
        4,
        vec![ViaMask {
            from_layer: 0,
            to_layer: 3,
            attach_smd_allowed: false,
        }],
    );
    let drill = drill_with_rooms();
    let mut checker = ScriptedChecker::new(CheckDrillResult::Drillable);
    let mut emitted = Vec::new();
    expand_to_other_layers(
        &mut world,
        &ctrl,
        777,
        &drill,
        1,
        100.0,
        &shape_entry(),
        &mut checker,
        &FixedDistance(2.5),
        &mut |element| emitted.push(element),
    );
    // Sweeps: downward [1, 0], upward [2, 3]; radius = max(400, 50).
    assert_eq!(
        checker.calls.iter().map(|call| call.3).collect::<Vec<_>>(),
        vec![1, 0, 2, 3],
        "downward then upward sweep layers"
    );
    assert!(
        checker
            .calls
            .iter()
            .all(|call| (call.0 - 400.0).abs() < f64::EPSILON && call.1 == 4 && !call.2),
        "required radius 0.5 * 800 vs trace half width 50; class and attach forwarded"
    );
    assert_eq!(
        emit_keys(&emitted),
        vec![0, 2, 3],
        "to == from skipped; occupancy off here"
    );
    let first = &emitted[0];
    assert_eq!(
        (first.expansion_value, first.sorting_value),
        (110.0, 112.5),
        "cost row [from=1][to=0] = 10, +2.5 distance for sorting"
    );
    assert_eq!(emitted[1].expansion_value, 112.0, "row [1][2] = 12");
    assert_eq!(emitted[2].expansion_value, 113.0, "row [1][3] = 13");
    for element in &emitted {
        assert_eq!(element.door_key, 777);
        assert_eq!(element.backtrack_door_key, 777);
        assert_eq!(element.section_no_of_backtrack_door, 1);
        assert!(!element.room_ripped);
        assert_eq!(element.adjustment, super::Adjustment::None);
        assert!(!element.already_checked);
        assert_eq!(
            element.shape_entry,
            shape_entry(),
            "carried through unchanged"
        );
    }
    assert_eq!(
        emitted[0].next_room_key,
        SynthWorld::room_key(101),
        "nextRoom = roomArr[to - firstLayer]"
    );
    assert_eq!(emitted[0].section_no_of_door, 0);
    assert_eq!(emitted[2].section_no_of_door, 3);
}

/// The occupancy gate (`:349-353`): a maze search element marked
/// occupied silences exactly its to-layer; other layers still emit.
#[test]
fn expand_to_other_layers_occupancy_gates_emission() {
    let mut world = rooms_world();
    world.add_padstack(
        7,
        (0, 3),
        Some(BoardShape::Circle(Circle::new(IntPoint::new(0, 0), 400))),
    );
    world.set_rule_vias(vec![ViaRuleVia {
        padstack_no: 7,
        clearance_class: 4,
        attach_smd_allowed: false,
    }]);
    let ctrl = test_ctrl(
        4,
        vec![ViaMask {
            from_layer: 0,
            to_layer: 3,
            attach_smd_allowed: false,
        }],
    );
    let mut drill = drill_with_rooms();
    drill.maze_search_element_mut(2).is_occupied = true; // to-layer 2 (index 2 - firstLayer 0)
    let mut checker = ScriptedChecker::new(CheckDrillResult::Drillable);
    let mut emitted = Vec::new();
    expand_to_other_layers(
        &mut world,
        &ctrl,
        777,
        &drill,
        1,
        100.0,
        &shape_entry(),
        &mut checker,
        &FixedDistance(2.5),
        &mut |element| emitted.push(element),
    );
    assert_eq!(emit_keys(&emitted), vec![0, 3], "to-layer 2 is occupied");
}

/// The ctrl via-range clamp (`:279-315` + the `:314` early return):
/// `viaUpperBound = 2` caps the upward sweep at layer 2, and then
/// `viaUpperBound (2) < drill.lastLayer (3)` aborts the WHOLE
/// expansion — the drill cannot change layers when its span leaves
/// the ctrl via range. A mutant sweeping to `drill.lastLayer` (or
/// skipping the post-sweep clamp check) emits and dies.
#[test]
fn expand_to_other_layers_ctrl_upper_bound_clamps_the_sweep() {
    let mut world = rooms_world();
    world.add_padstack(
        7,
        (0, 3),
        Some(BoardShape::Circle(Circle::new(IntPoint::new(0, 0), 400))),
    );
    world.set_rule_vias(vec![ViaRuleVia {
        padstack_no: 7,
        clearance_class: 4,
        attach_smd_allowed: false,
    }]);
    let ctrl = test_ctrl(
        2,
        vec![ViaMask {
            from_layer: 0,
            to_layer: 2,
            attach_smd_allowed: false,
        }],
    );
    let drill = drill_with_rooms();
    let mut checker = ScriptedChecker::new(CheckDrillResult::Drillable);
    let mut emitted = Vec::new();
    expand_to_other_layers(
        &mut world,
        &ctrl,
        777,
        &drill,
        1,
        100.0,
        &shape_entry(),
        &mut checker,
        &FixedDistance(2.5),
        &mut |element| emitted.push(element),
    );
    assert!(
        emitted.is_empty(),
        "viaUpperBound (2) < lastLayer (3): the whole expansion aborts"
    );
    assert_eq!(
        checker.calls.iter().map(|call| call.3).collect::<Vec<_>>(),
        vec![1, 0, 2],
        "downward [1, 0] plus upward [2]; the sweep never probed layer 3"
    );
}

/// The NotDrillable sweep tightening (`:268-297`): a not-drillable
/// layer BELOW the from-layer raises `viaLowerBound` past
/// `firstLayer` -> early return (zero emissions, row A); a
/// not-drillable layer ABOVE lowers `viaUpperBound` and truncates
/// the emission range (row B: only to-layer 0 emits).
#[test]
fn expand_to_other_layers_not_drillable_tightens_bounds() {
    let build = || {
        let mut world = rooms_world();
        world.add_padstack(
            7,
            (0, 3),
            Some(BoardShape::Circle(Circle::new(IntPoint::new(0, 0), 400))),
        );
        world.set_rule_vias(vec![ViaRuleVia {
            padstack_no: 7,
            clearance_class: 4,
            attach_smd_allowed: false,
        }]);
        let ctrl = test_ctrl(
            4,
            vec![ViaMask {
                from_layer: 0,
                to_layer: 3,
                attach_smd_allowed: false,
            }],
        );
        let drill = drill_with_rooms();
        (world, ctrl, drill)
    };
    // Row A: layer 0 not drillable -> viaLowerBound = 1 > firstLayer -> return.
    let (mut world, ctrl, drill) = build();
    let mut checker = ScriptedChecker::new(CheckDrillResult::NotDrillable);
    checker.script(1, 4, CheckDrillResult::Drillable);
    let mut emitted = Vec::new();
    expand_to_other_layers(
        &mut world,
        &ctrl,
        777,
        &drill,
        1,
        100.0,
        &shape_entry(),
        &mut checker,
        &FixedDistance(2.5),
        &mut |element| emitted.push(element),
    );
    assert!(
        emitted.is_empty(),
        "viaLowerBound left the drill span: early return"
    );
    assert_eq!(checker.calls.len(), 2, "downward [1, 0], no upward sweep");

    // Row B: layer 2 not drillable -> viaUpperBound = 1 < lastLayer 3
    // -> the post-sweep check aborts the whole expansion (the probe
    // count separates this row from row A's early return).
    let (mut world, ctrl, drill) = build();
    let mut checker = ScriptedChecker::new(CheckDrillResult::Drillable);
    checker.script(2, 4, CheckDrillResult::NotDrillable);
    let mut emitted = Vec::new();
    expand_to_other_layers(
        &mut world,
        &ctrl,
        777,
        &drill,
        1,
        100.0,
        &shape_entry(),
        &mut checker,
        &FixedDistance(2.5),
        &mut |element| emitted.push(element),
    );
    assert!(
        emitted.is_empty(),
        "viaUpperBound (1) < lastLayer (3): abort"
    );
    assert_eq!(
        checker.calls.len(),
        3,
        "downward [1, 0] plus the aborted upward [2]"
    );
}

/// The mask gate truth table (`:329-341`) — the combinations where
/// the alternatives DIFFER (pin-failure mode 7/9). All layers are
/// DRILLABLE_WITH_ATTACH_SMD from-layer 1, so both edge flags set
/// (component side via the downward sweep reaching layer 0, solder
/// side via the upward sweep reaching layer 3):
/// * row A: full-range mask with `attach_smd_allowed = false` —
///   every to-layer blocked (zero emissions);
/// * row B: the SAME world with `attach_smd_allowed = true` — all
///   three emit (the contrast witness);
/// * row C: an EDGE-FREE mask (1..2) with attach=false still emits
///   to-layer 2 — the edge flags never bite away from the edges.
#[test]
fn expand_to_other_layers_mask_gate_truth_table() {
    let build = || {
        let mut world = rooms_world();
        world.add_padstack(
            7,
            (0, 3),
            Some(BoardShape::Circle(Circle::new(IntPoint::new(0, 0), 400))),
        );
        world.set_rule_vias(vec![ViaRuleVia {
            padstack_no: 7,
            clearance_class: 4,
            attach_smd_allowed: false,
        }]);
        let drill = drill_with_rooms();
        (world, drill)
    };
    // Rows A + B: both flags set, full-range mask.
    for (attach_allowed, expected) in [(false, Vec::<i32>::new()), (true, vec![0, 2, 3])] {
        let (mut world, drill) = build();
        let ctrl = test_ctrl(
            4,
            vec![ViaMask {
                from_layer: 0,
                to_layer: 3,
                attach_smd_allowed: attach_allowed,
            }],
        );
        let mut checker = ScriptedChecker::new(CheckDrillResult::DrillableWithAttachSmd);
        let mut emitted = Vec::new();
        expand_to_other_layers(
            &mut world,
            &ctrl,
            777,
            &drill,
            1,
            100.0,
            &shape_entry(),
            &mut checker,
            &FixedDistance(2.5),
            &mut |element| emitted.push(element),
        );
        assert_eq!(
            emit_keys(&emitted),
            expected,
            "attach_smd_allowed = {attach_allowed}"
        );
    }
    // Row C: edge-free mask, attach=false, both flags set.
    let (mut world, drill) = build();
    let ctrl = test_ctrl(
        4,
        vec![ViaMask {
            from_layer: 1,
            to_layer: 2,
            attach_smd_allowed: false,
        }],
    );
    let mut checker = ScriptedChecker::new(CheckDrillResult::DrillableWithAttachSmd);
    let mut emitted = Vec::new();
    expand_to_other_layers(
        &mut world,
        &ctrl,
        777,
        &drill,
        1,
        100.0,
        &shape_entry(),
        &mut checker,
        &FixedDistance(2.5),
        &mut |element| emitted.push(element),
    );
    assert_eq!(
        emit_keys(&emitted),
        vec![2],
        "only the in-mask to-layer 2 emits"
    );
    // Row D: the disjunct discriminator — mask (0, 2) with
    // attach=false and BOTH edge flags set. The from-0 disjunct is
    // TRUE (mask.from == 0 && smdComp) while the to-last disjunct is
    // FALSE (2 != 3), so the Java disjunction `!(A || C) || attach`
    // blocks the mask on EVERY to-layer (zero emissions) — but the
    // &&-flattened mutant `!(A && C) || attach` reads !(false) = true
    // and lets to-layers {0, 2} through. Rows A-C coincide under both
    // readings; THIS row is what kills the ||->&& mutant inside this
    // pin (verified by mutation).
    let (mut world, drill) = build();
    let ctrl = test_ctrl(
        4,
        vec![ViaMask {
            from_layer: 0,
            to_layer: 2,
            attach_smd_allowed: false,
        }],
    );
    let mut checker = ScriptedChecker::new(CheckDrillResult::DrillableWithAttachSmd);
    let mut emitted = Vec::new();
    expand_to_other_layers(
        &mut world,
        &ctrl,
        777,
        &drill,
        1,
        100.0,
        &shape_entry(),
        &mut checker,
        &FixedDistance(2.5),
        &mut |element| emitted.push(element),
    );
    assert!(
        emitted.is_empty(),
        "row D: the from-0 disjunct alone blocks the (0, 2) no-attach mask"
    );
}

/// The component-edge flag fires ONLY at layer 0 (Java `:283-286`),
/// and when set it blocks EVERY to-layer of any mask starting at 0
/// (`maskOk = !(mask.from == 0 && smdComp || ...) || attach`):
/// * world A — layer 0 attached: the full-range mask is dead, ZERO
///   emissions (a never-set flag mutant emits and dies);
/// * world B — layer 1 attached (a NON-edge layer): no flag, all of
///   {0, 2, 3} emits (a flag-on-any-attached-layer mutant blocks
///   everything and dies).
#[test]
fn expand_to_other_layers_component_edge_flag_bites_only_at_layer_zero() {
    let run = |attached_layer: i32| {
        let mut world = rooms_world();
        world.add_padstack(
            7,
            (0, 3),
            Some(BoardShape::Circle(Circle::new(IntPoint::new(0, 0), 400))),
        );
        world.set_rule_vias(vec![ViaRuleVia {
            padstack_no: 7,
            clearance_class: 4,
            attach_smd_allowed: false,
        }]);
        let ctrl = test_ctrl(
            4,
            vec![ViaMask {
                from_layer: 0,
                to_layer: 3,
                attach_smd_allowed: false,
            }],
        );
        let drill = drill_with_rooms();
        let mut checker = ScriptedChecker::new(CheckDrillResult::Drillable);
        checker.script(attached_layer, 4, CheckDrillResult::DrillableWithAttachSmd);
        let mut emitted = Vec::new();
        expand_to_other_layers(
            &mut world,
            &ctrl,
            777,
            &drill,
            1,
            100.0,
            &shape_entry(),
            &mut checker,
            &FixedDistance(2.5),
            &mut |element| emitted.push(element),
        );
        (
            emit_keys(&emitted),
            checker.calls.iter().map(|call| call.3).collect::<Vec<_>>(),
        )
    };
    let (keys, layers) = run(0);
    assert!(
        keys.is_empty(),
        "layer 0 attached: the from-0 mask blocks every to-layer"
    );
    assert_eq!(layers, vec![1, 0, 2, 3], "both sweeps ran");
    let (keys, _) = run(1);
    assert_eq!(
        keys,
        vec![0, 2, 3],
        "layer 1 attached is NOT the component edge: nothing is blocked"
    );
}

/// The ripped-via branch (`:246-261`): gate rows —
/// ripupAllowed=false (no emissions, no probes), the positive row
/// (padstack span becomes the via range, roomRipped=true, NO
/// checkLayer probes), a wrong clearance class, a padstack outside
/// the rule, and a non-via obstacle. Each negative gate flips the
/// output relative to the positive row (mode 9).
#[test]
fn expand_to_other_layers_ripped_via_branch_gates() {
    let build = |obstacle_via: bool, item_class: i32, rule_padstack: i32| {
        let mut world = rooms_world();
        world.add_padstack(5, (0, 3), None);
        world.add_padstack(9, (0, 3), None);
        // Item 11: the ripped via sitting in the from-room (center
        // scripted per the stored-center contract; these gate rows
        // never read it).
        world.add_via(11, 1, 5, item_class, Point::Int(IntPoint::new(50, 50)));
        // Swap the from-room (102) to an obstacle room over item 11.
        world.rooms[1].obstacle = true;
        world.rooms[1].obstacle_item = Some(SynthWorld::item_key(11));
        let _ = obstacle_via;
        world.set_rule_vias(vec![ViaRuleVia {
            padstack_no: rule_padstack,
            clearance_class: 2,
            attach_smd_allowed: false,
        }]);
        let ctrl = test_ctrl(
            4,
            vec![ViaMask {
                from_layer: 0,
                to_layer: 3,
                attach_smd_allowed: false,
            }],
        );
        let mut drill = drill_with_rooms();
        drill.room_arr[1] = Some(SynthWorld::room_key(202)); // the obstacle room
        world.rooms.push(SynthRoom {
            id: 202,
            layer: 1,
            obstacle: true,
            shape: box_tile(0, 0, 100, 100),
            obstacle_item: Some(SynthWorld::item_key(11)),
        });
        (world, ctrl, drill)
    };
    // Row 1: ripup forbidden -> immediate return.
    let (mut world, mut ctrl, drill) = build(true, 2, 5);
    ctrl.ripup_allowed = false;
    let mut checker = ScriptedChecker::new(CheckDrillResult::Drillable);
    let mut emitted = Vec::new();
    expand_to_other_layers(
        &mut world,
        &ctrl,
        777,
        &drill,
        1,
        100.0,
        &shape_entry(),
        &mut checker,
        &FixedDistance(2.5),
        &mut |element| emitted.push(element),
    );
    assert!(emitted.is_empty(), "ripupAllowed=false gates everything");
    assert!(checker.calls.is_empty(), "no probes in the obstacle branch");

    // Row 2 (positive): the padstack span IS the via range, no
    // completion probes, roomRipped set on every emission.
    let (mut world, ctrl, drill) = build(true, 2, 5);
    let mut ctrl = ctrl;
    ctrl.ripup_allowed = true;
    let mut checker = ScriptedChecker::new(CheckDrillResult::Drillable);
    let mut emitted = Vec::new();
    expand_to_other_layers(
        &mut world,
        &ctrl,
        777,
        &drill,
        1,
        100.0,
        &shape_entry(),
        &mut checker,
        &FixedDistance(2.5),
        &mut |element| emitted.push(element),
    );
    assert_eq!(
        emit_keys(&emitted),
        vec![0, 2, 3],
        "the full padstack span emits"
    );
    assert!(
        checker.calls.is_empty(),
        "the obstacle branch never probes checkLayer"
    );
    assert!(
        emitted.iter().all(|element| element.room_ripped),
        "roomRipped = true"
    );

    // Row 3: clearance class mismatch -> gated.
    let (mut world, ctrl, drill) = build(true, 3, 5);
    let mut ctrl = ctrl;
    ctrl.ripup_allowed = true;
    let mut checker = ScriptedChecker::new(CheckDrillResult::Drillable);
    let mut emitted = Vec::new();
    expand_to_other_layers(
        &mut world,
        &ctrl,
        777,
        &drill,
        1,
        100.0,
        &shape_entry(),
        &mut checker,
        &FixedDistance(2.5),
        &mut |element| emitted.push(element),
    );
    assert!(
        emitted.is_empty(),
        "the via's class must match ctrl.viaClearanceClass"
    );

    // Row 4: padstack not in the via rule -> gated.
    let (mut world, ctrl, drill) = build(true, 2, 9);
    let mut ctrl = ctrl;
    ctrl.ripup_allowed = true;
    let mut checker = ScriptedChecker::new(CheckDrillResult::Drillable);
    let mut emitted = Vec::new();
    expand_to_other_layers(
        &mut world,
        &ctrl,
        777,
        &drill,
        1,
        100.0,
        &shape_entry(),
        &mut checker,
        &FixedDistance(2.5),
        &mut |element| emitted.push(element),
    );
    assert!(
        emitted.is_empty(),
        "the via's padstack must be in ctrl.viaRule"
    );

    // Row 5: the obstacle is not a via -> gated.
    let (mut world, ctrl, drill) = build(false, 2, 5);
    let mut ctrl = ctrl;
    ctrl.ripup_allowed = true;
    world.items[0].is_via = false; // item 11 becomes a plain obstacle
    let mut checker = ScriptedChecker::new(CheckDrillResult::Drillable);
    let mut emitted = Vec::new();
    expand_to_other_layers(
        &mut world,
        &ctrl,
        777,
        &drill,
        1,
        100.0,
        &shape_entry(),
        &mut checker,
        &FixedDistance(2.5),
        &mut |element| emitted.push(element),
    );
    assert!(
        emitted.is_empty(),
        "only vias re-expand through the obstacle branch"
    );
}

/// The stored-center contract on the SYNTH impl (quality-review
/// T17a-2 Q-4, closing the QR-1 survival): `via_center` answers the
/// via's OWN scripted stored center — Some for every live scripted
/// via, never None (the trait doc's "None is unreachable for a live
/// via"), None only for a center-less plain item (the defensive
/// shape). Two vias at DISTINCT centers kill the constant-center and
/// wrong-item mutants alongside the reviewer's unconditional-panic
/// QR-1. (The full `end_points_matching` replay stays with P4/P5 —
/// SynthWorld carries no net/polyline model, and the via arm's other
/// faces are already pinned through the two impls callers actually
/// use.)
#[test]
fn synth_via_center_answers_the_scripted_stored_center() {
    let mut world = SynthWorld::new(square_bounds(0, 0, 3000), 2);
    world.add_via(11, 1, 5, 2, Point::Int(IntPoint::new(50, 50)));
    world.add_via(12, 1, 5, 2, Point::Int(IntPoint::new(700, 200)));
    world.add_item(13, 0, vec![box_tile(2000, 2000, 3000, 3000)]);
    assert_eq!(
        DrillEngine::via_center(&world, SynthWorld::item_key(11)),
        Some(Point::Int(IntPoint::new(50, 50))),
        "via 11 answers its own stored center"
    );
    assert_eq!(
        DrillEngine::via_center(&world, SynthWorld::item_key(12)),
        Some(Point::Int(IntPoint::new(700, 200))),
        "via 12 answers its own stored center (not via 11's)"
    );
    assert_eq!(
        DrillEngine::via_center(&world, SynthWorld::item_key(13)),
        None,
        "a center-less plain item answers None (the defensive shape)"
    );
}

/// The cutout walk follows the Java tree-set iteration order (object
/// id DESCENDING, `java_ordered_entries`), and the HOLE order feeds
/// `PolylineArea.splitToConvex` — the piece LIST sequence depends on
/// it even though the covered region does not (the cutout union is
/// commutative). Mode-9 contrast: two corner holes whose ids give
/// different cut orders, entries inserted ASCENDING (10 then 20) so a
/// pass-through or an ascending-sort mutant cuts 10's hole first and
/// produces a different sequence.
#[test]
fn get_drills_cutout_order_follows_java_tree_set_order() {
    let mut world = SynthWorld::new(square_bounds(0, 0, 3000), 1);
    world.add_item(10, 0, vec![box_tile(2000, 2000, 3000, 3000)]);
    world.add_item(20, 0, vec![box_tile(0, 0, 1000, 1000)]);
    world.complete_result = vec![SynthWorld::room_key(501)];
    world.add_room(501, 0, box_tile(0, 0, 3000, 3000));

    let mut array = DrillPageArray::new(&world, 10_000);
    let page = &mut array.pages[0][0];
    let drills = page.get_drills(&mut world, 1, false);
    let boxes: Vec<(i32, i32, i32, i32)> = drills
        .iter()
        .map(|drill| {
            let bb = drill.get_shape().bounding_box();
            (bb.ll.x, bb.ll.y, bb.ur.x, bb.ur.y)
        })
        .collect();
    println!("TREE_SET_ORDER boxes = {boxes:?}");
    // Verbatim DESC-order decomposition (hole 20 first, hole 10
    // second). The holes sit in OPPOSITE corners, so the cut order
    // reshapes the intermediate piece list differently — an
    // ascending-sort (or insertion-order pass-through) mutant cuts
    // hole 10 first and cannot reproduce this sequence.
    assert_eq!(
        boxes,
        [
            (1000, 0, 3000, 1000),
            (2000, 1000, 3000, 2000),
            (0, 1000, 2000, 3000)
        ],
        "the cutout walk runs id DESCENDING (Java tree-set order)"
    );
}

/// MAJOR-1: the hole-cutout simplify belongs to the DIVIDE piece's
/// dynamic type — `IntBox.cutout` (`IntBox.java:688-695`) simplifies
/// each result, `IntOctagon.cutout` (`IntOctagon.java:1059-1061`) and
/// `Simplex.cutout` (`Simplex.java:695-697`) do NOT. The walk: an
/// octagonal hole (item 30 — the [1000,3000]^2 block with all four
/// corners cut by 300, the exact shape family a DSN
/// `(shape (octagon ...))` padstack parses to, so the sequence is
/// Java-reachable) is cut FIRST (id-descending walk) from the IntBox
/// page. There the divide IS an IntBox, so both readings simplify and
/// the four corner pieces survive as true `IntOctagon`s (rows 4-7).
/// The second hole (item 20, box [0,1000,1300,1300]) then cuts the
/// lower-left corner piece — an `IntOctagon` DIVIDE — down to the
/// box-like remainder [0,0,1300,1000]: Java keeps it an `IntOctagon`
/// (unsimplified; the next round's `cutoutFrom` would dispatch the
/// octagon arm), while the unconditional-simplify mutant re-typed it
/// `IntBox` — the type tags below flip on row 5, which the bounding
/// boxes alone cannot see.
#[test]
fn octagon_divide_keeps_results_unsimplified() {
    let mut world = SynthWorld::new(square_bounds(0, 0, 4000), 1);
    // [1000,3000]^2 with all four corners cut by 300: diagonals
    // raised/lowered from the degenerate box values
    // (lx-uy, rx-ly, lx+ly, rx+uy) = (-2000, 2000, 2000, 6000).
    let octagon = IntOctagon::new(1000, 1000, 3000, 3000, -1700, 1700, 2300, 5700);
    world.add_item(
        30,
        0,
        vec![TileShape::RegularTileShape(RegularTileShape::IntOctagon(
            octagon,
        ))],
    );
    world.complete_result = vec![SynthWorld::room_key(501)];
    // Cut SECOND (lower id): a box over the LL corner piece's diagonal
    // band — the remainder below y=1000 is box-like.
    world.add_item(20, 0, vec![box_tile(0, 1000, 1300, 1300)]);
    let mut array = DrillPageArray::new(&world, 10_000);
    let page = &mut array.pages[0][0];
    let drills = page.get_drills(&mut world, 1, false);
    let rows: Vec<(bool, i32, i32, i32, i32)> = drills
        .iter()
        .map(|drill| {
            let shape = drill.get_shape();
            let is_oct = matches!(
                shape,
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(_))
            );
            let bb = shape.bounding_box();
            (is_oct, bb.ll.x, bb.ll.y, bb.ur.x, bb.ur.y)
        })
        .collect();
    assert_eq!(
        rows,
        [
            (false, 0, 1300, 1000, 2700),
            (false, 3000, 1300, 4000, 2700),
            (false, 1300, 0, 2700, 1000),
            (false, 1300, 3000, 2700, 4000),
            (true, 0, 2700, 1300, 4000),
            (true, 0, 0, 1300, 1000),
            (true, 2700, 0, 4000, 1300),
            (true, 2700, 2700, 4000, 4000),
        ],
        "the type-tagged decomposition: row 5 is the box-like remainder of an \
         octagon divide and must STAY an IntOctagon"
    );
}

/// MINOR-1a: the `prev_obstacle_shape` update fires for EVERY item
/// entry, including entries SKIPPED by the contains-dedup
/// (`DrillPage.java:87-96`: the assignment is after the if). Two
/// id-descending walks pin the literal output:
///
/// * run [S1, S2⊂S1, S3⊆S1, S3≠S2]: S1 cut, S2 skipped (prev = S2),
///   S3 vs S2 not contained → cut — the holes are {S1, S3} with S3's
///   hole NESTED in S1's, so the second cut is a geometric no-op and
///   the output is page minus S1. The move-update-inside mutant cuts
///   {S1} only — the SAME output (the nesting makes every
///   skipped-shape hole a subset of an already-removed hole, so the
///   mutant is output-equivalent by construction; verified by
///   mutation run — see the module history). The pin fixes the
///   correct-walk literal.
/// * the doc's [Y1, Y2, Y1] same-shape run: the second Y1 is
///   deduplicated against the PREVIOUS shape only (Y2 ⊉ Y1 → cut
///   again, a no-op nested hole) — output page minus (Y1 ∪ Y2). A
///   dedup-against-all-holes mutant produces the same geometry here
///   for the same nesting reason; the run pins the walk's literal
///   behavior regardless.
#[test]
fn get_drills_contains_skips_still_update_prev_shape() {
    // Run A: [S1, S2 ⊂ S1, S3 ⊆ S1, S3 ≠ S2].
    let mut world = SynthWorld::new(square_bounds(0, 0, 3000), 1);
    world.add_item(30, 0, vec![box_tile(0, 0, 3000, 1000)]);
    world.add_item(20, 0, vec![box_tile(0, 0, 1000, 1000)]);
    world.add_item(10, 0, vec![box_tile(2000, 0, 3000, 1000)]);
    world.complete_result = vec![SynthWorld::room_key(501)];
    let mut array = DrillPageArray::new(&world, 10_000);
    let page = &mut array.pages[0][0];
    let drills = page.get_drills(&mut world, 1, false);
    let boxes: Vec<(i32, i32, i32, i32)> = drills
        .iter()
        .map(|drill| {
            let bb = drill.get_shape().bounding_box();
            (bb.ll.x, bb.ll.y, bb.ur.x, bb.ur.y)
        })
        .collect();
    println!("CONTAINS_SKIP run A boxes = {boxes:?}");
    assert_eq!(
        boxes,
        [(0, 1000, 3000, 3000)],
        "S2 skipped with prev = S2, S3 still cut (nested no-op): page minus S1"
    );

    // Run B: [Y1, Y2, Y1] — the same-shape dedup is prev-only.
    let mut world = SynthWorld::new(square_bounds(0, 0, 3000), 1);
    world.add_item(30, 0, vec![box_tile(0, 0, 1000, 3000)]);
    world.add_item(20, 0, vec![box_tile(1000, 0, 2000, 3000)]);
    world.add_item(10, 0, vec![box_tile(0, 0, 1000, 3000)]);
    world.complete_result = vec![SynthWorld::room_key(501)];
    let mut array = DrillPageArray::new(&world, 10_000);
    let page = &mut array.pages[0][0];
    let drills = page.get_drills(&mut world, 1, false);
    let boxes: Vec<(i32, i32, i32, i32)> = drills
        .iter()
        .map(|drill| {
            let bb = drill.get_shape().bounding_box();
            (bb.ll.x, bb.ll.y, bb.ur.x, bb.ur.y)
        })
        .collect();
    println!("CONTAINS_SKIP run B boxes = {boxes:?}");
    assert_eq!(
        boxes,
        [(2000, 0, 3000, 3000)],
        "the duplicate Y1 cuts a nested no-op hole: page minus (Y1 union Y2)"
    );
}

/// MINOR-1b: the attach-SMD pin probe tries the FIRST layer, then the
/// LAST (`DrillPage.java:100-107` order) — a world with drill-allowed
/// pins on BOTH edge layers (different centers) pins the order: the
/// layer-0 center (400, 400) wins; the swap-last/first mutant anchors
/// at the layer-1 center (2000, 2000) and dies.
#[test]
fn get_drills_attach_smd_first_layer_probe_comes_first() {
    let mut world = SynthWorld::new(square_bounds(0, 0, 3000), 2);
    world.add_pin(7, 0, true, Some(Point::Int(IntPoint::new(400, 400))));
    world.add_pin(5, 1, true, Some(Point::Int(IntPoint::new(2000, 2000))));
    world.complete_room_base = Some(501);

    let mut array = DrillPageArray::new(&world, 10_000);
    let page = &mut array.pages[0][0];
    let drills = page.get_drills(&mut world, 1, true);
    assert_eq!(drills.len(), 1, "no cutouts: one convex piece");
    assert_eq!(
        drills[0].location,
        Point::Int(IntPoint::new(400, 400)),
        "the FIRST-layer probe runs before the LAST-layer probe"
    );
}

/// MINOR-1c (generality guard, NOT a capture row): Java's only
/// `expandToOtherLayers` call site passes drills whose `firstLayer` is
/// always 0 (page drills span the whole stack), so the
/// `fromLayer = drill.firstLayer + sectionNoOfDoor` offset
/// (`MazeExpansionEngine.java:242`) is never exercised against a
/// nonzero base there. The port is total, so the pin runs a synthetic
/// first_layer = 1 drill: from section 1 the from-layer is 2, the
/// sweeps probe [2, 1] then [3], and `nextRoom`/`sectionNoOfDoor`
/// index `roomArr` relative to the drill's own base (to-layer 1 →
/// index 0, to-layer 3 → index 2). A `from_layer = section_no_of_door`
/// mutant probes layer 1 first and dies on the probe sequence.
#[test]
fn expand_to_other_layers_from_layer_offsets_by_drill_first_layer() {
    let mut world = SynthWorld::new(square_bounds(0, 0, 100), 4);
    for (layer, room_id) in [(1, 101), (2, 102), (3, 103)] {
        world.add_room(room_id, layer, box_tile(0, 0, 100, 100));
    }
    world.add_padstack(
        7,
        (0, 3),
        Some(BoardShape::Circle(Circle::new(IntPoint::new(0, 0), 400))),
    );
    world.set_rule_vias(vec![ViaRuleVia {
        padstack_no: 7,
        clearance_class: 4,
        attach_smd_allowed: false,
    }]);
    let mut drill = ExpansionDrill::new(
        box_tile(0, 0, 100, 100),
        Point::Int(IntPoint::new(50, 50)),
        1,
        3,
    );
    for (index, room_id) in [101, 102, 103].iter().enumerate() {
        drill.room_arr[index] = Some(SynthWorld::room_key(*room_id));
    }
    // The mask must fit the SWEPT range [1, 3] (a (0, 3) mask fails
    // the `mask.from >= viaLowerBound` filter at viaLowerBound = 1).
    let ctrl = test_ctrl(
        4,
        vec![ViaMask {
            from_layer: 1,
            to_layer: 3,
            attach_smd_allowed: false,
        }],
    );
    let mut checker = ScriptedChecker::new(CheckDrillResult::Drillable);
    let mut emitted = Vec::new();
    expand_to_other_layers(
        &mut world,
        &ctrl,
        777,
        &drill,
        1, // section_no_of_door: the from-room is roomArr[1] (layer 2)
        100.0,
        &shape_entry(),
        &mut checker,
        &FixedDistance(2.5),
        &mut |element| emitted.push(element),
    );
    assert_eq!(
        checker.calls.iter().map(|call| call.3).collect::<Vec<_>>(),
        vec![2, 1, 3],
        "from-layer 2 (= firstLayer 1 + section 1): downward [2, 1], upward [3]"
    );
    assert_eq!(
        emit_keys(&emitted),
        vec![0, 2],
        "emissions to to-layers 1 and 3 (the from-layer 2 emits nothing); \
         sections are drill-relative (to - firstLayer)"
    );
    assert_eq!(
        emitted
            .iter()
            .map(|element| element.next_room_key)
            .collect::<Vec<_>>(),
        vec![SynthWorld::room_key(101), SynthWorld::room_key(103),],
        "nextRoom indexes roomArr by the drill-relative section"
    );
}

/// `checkLayerWithAnyMatchingVia` (`:377-414`): the rule-via span
/// filter, the radius arithmetic (`0.5 * maxWidth` vs
/// `traceHalfWidth[layer]`, null shape -> 0), the DRILLABLE
/// short-circuit (a later rule via is never probed), and the
/// attach-SMD accumulation verdict.
#[test]
fn check_layer_with_any_matching_via_radius_span_and_short_circuit() {
    let mut world = SynthWorld::new(square_bounds(0, 0, 100), 4);
    world.add_padstack(
        7,
        (0, 3),
        Some(BoardShape::Circle(Circle::new(IntPoint::new(0, 0), 400))),
    );
    world.add_padstack(8, (1, 2), None); // null shape at its span layers
    world.set_rule_vias(vec![
        ViaRuleVia {
            padstack_no: 7,
            clearance_class: 4,
            attach_smd_allowed: false,
        },
        ViaRuleVia {
            padstack_no: 8,
            clearance_class: 5,
            attach_smd_allowed: true,
        },
    ]);
    let ctrl = test_ctrl(4, Vec::new());
    let drill = drill_with_rooms();
    let room_shape = box_tile(0, 0, 100, 100);

    // Layer 1: via 7 not drillable (radius 400), via 8 attach-SMD
    // (null shape -> radius 0 -> required 50) -> WITH_ATTACH_SMD.
    let mut checker = ScriptedChecker::new(CheckDrillResult::NotDrillable);
    checker.script(1, 5, CheckDrillResult::DrillableWithAttachSmd);
    let result = check_layer_with_any_matching_via(
        &mut world,
        &ctrl,
        &drill,
        1,
        &room_shape,
        &mut checker,
        1,
    );
    assert_eq!(result, CheckDrillResult::DrillableWithAttachSmd);
    assert_eq!(
        checker.calls,
        vec![(400.0, 4, false, 1, 50), (50.0, 5, true, 1, 50)],
        "radii, classes, attach flags, layers and the drill location forwarded"
    );

    // Layer 3: only via 7 spans it -> plain NotDrillable.
    let mut checker = ScriptedChecker::new(CheckDrillResult::NotDrillable);
    let result = check_layer_with_any_matching_via(
        &mut world,
        &ctrl,
        &drill,
        3,
        &room_shape,
        &mut checker,
        1,
    );
    assert_eq!(result, CheckDrillResult::NotDrillable);
    assert_eq!(checker.calls.len(), 1, "the out-of-span via 8 is filtered");

    // Short-circuit: via 7 DRILLABLE on layer 1 -> via 8 never probed.
    let mut checker = ScriptedChecker::new(CheckDrillResult::NotDrillable);
    checker.script(1, 4, CheckDrillResult::Drillable);
    let result = check_layer_with_any_matching_via(
        &mut world,
        &ctrl,
        &drill,
        1,
        &room_shape,
        &mut checker,
        1,
    );
    assert_eq!(result, CheckDrillResult::Drillable);
    assert_eq!(checker.calls.len(), 1, "DRILLABLE returns immediately");
}

/// T7 quality round, M2-survivor kill: `Connection::trace_length`
/// must fold the per-trace lengths in JAVA's order — the
/// `TreeSet<Item>` DESCENDING-id walk (`Item.compareTo` is
/// `item.id - id`). THREE traces, because two cannot discriminate:
/// f64 addition is commutative, so a 2-addend fold is order-free.
/// At 1e16 the ULP is 2, so the descending fold `((1e16 + 1) + 1)`
/// ties and rounds to even = 1e16 both times, while the ascending
/// fold adds the small pair first: `((1 + 1) + 1e16) = 1e16 + 2`
/// exactly. The ascending-order mutant answers 1e16 + 2 and dies.
#[test]
fn connection_trace_length_folds_descending_like_java() {
    use std::collections::BTreeSet;

    use crate::path::Connection;

    let mut world = SynthWorld::new(IntBox::new(IntPoint::new(0, 0), IntPoint::new(100, 100)), 4);
    for (key, length) in [(1u64, 1.0f64), (2, 1.0), (3, 10_000_000_000_000_000.0)] {
        world.trace_lengths.insert(key, length);
    }
    let connection = Connection {
        start_point: None,
        start_layer: 0,
        end_point: None,
        end_layer: 0,
        item_list: BTreeSet::from([1, 2, 3]),
    };
    assert_eq!(
        connection.trace_length(&world),
        10_000_000_000_000_000.0,
        "the descending-id fold — Java's TreeSet order"
    );
    assert_eq!(
        connection.trace_length(&world),
        (10_000_000_000_000_000.0f64 + 1.0) + 1.0,
        "bit-identical to Java's descending expression"
    );
    assert_ne!(
        connection.trace_length(&world),
        (1.0f64 + 1.0) + 10_000_000_000_000_000.0,
        "the ascending-order mutant (1e16 + 2) is distinct — this pin kills it"
    );
}
