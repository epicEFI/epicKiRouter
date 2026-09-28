//! Java `autoroute/maze/MazeRipupResolver.java` — resolves whether the
//! maze expansion may rip up an obstacle and calculates its cost.
//! Ported as an inherent [`MazeSearchEngine`] impl (Java holds the
//! resolver as a `search`-backed object; the engine carries the ctrl
//! and the stateful random generator the resolver reads).

use epic_geometry::float_point::FloatPoint;
use epic_geometry::line::Line;
use epic_geometry::point::Point;
use epic_geometry::polyline::Polyline;

use crate::drill::{DestinationDistance, DrillEngine, ViaLayerChecker};
use crate::maze::list_element::{ExpandableObject, MazeListElement};
use crate::maze::search_engine::{ALREADY_RIPPED_COSTS, MazeSearchEngine};

/// Java `MazeRipupResolver.FANOUT_COST_CONSTANT`.
const FANOUT_COST_CONSTANT: f64 = 20000.0;

/// M6-T9 (`router.push_shove`, default OFF — RUST-ONLY, no Java
/// counterpart): the per-search push-and-shove budget — how many
/// obstacle rooms may WAIVE their rip-up charge after a successful
/// shove probe, per maze search (one engine = one connection search =
/// one insertion). TUNING: 2 keeps the first crossing displacement and
/// its knock-on neighbor cheap while bounding the shove cascade a
/// single route can trigger; the exact-exhaustion face is pinned at
/// this value (DNR-16) and every load-bearing constant here is killed
/// by its pin in the T9 mutation round.
pub(crate) const PUSH_SHOVE_ROOM_BUDGET: i32 = 2;

/// M6-T9: the shove-before-rip composition predicate — pure, all
/// inputs read at the call site, no hidden state (the budget is
/// threaded explicitly so the pin worlds can hit the exact boundaries
/// DNR-16 demands). Fires only when the flag is ON, the shove probe
/// just verified the room shovable, a rip-up charge is actually
/// pending (`> 0` — `ALREADY_RIPPED_COSTS` and the unrippable `-1`
/// never waive), and the per-search budget holds. The OFF path answers
/// false on its first clause and the caller keeps `ripup_costs`
/// untouched — the default decision tree is byte-identical.
pub(crate) fn push_shove_waive_ripup(
    push_shove_on: bool,
    shoved: bool,
    ripup_costs: i32,
    budget_left: i32,
) -> bool {
    push_shove_on && shoved && ripup_costs > 0 && budget_left > 0
}

/// The CHECK_RIPUP observability payload — the fields of the Java
/// `FRLogger.trace("CHECK_RIPUP ...")` line (`MazeRipupResolver.java
/// :173-195`), carried to the capture pins. Java logs on EVERY
/// arithmetic-path return; the gate arms return `-1` before the log,
/// which the `None` trace mirrors (the ALREADY_RIPPED discriminator
/// pins exactly that log absence).
// The payload fields are read by the capture pins (`maze/pins.rs`);
// the non-test build only constructs them — dead_code from clippy's
// view, Java-log parity by design.
#[allow(dead_code)]
pub(crate) struct RipupTrace {
    /// Java `connectionItems` — the connection's item ids as Java's
    /// `TreeSet` DESCENDING string (`"[18,16,15]"`).
    pub connection_item_ids: String,
    /// Java `halfWidth=` — the costFactor (trace half width or the
    /// via-contact max).
    pub half_width: f64,
    /// Java `ripupCosts=` — `ctrl.ripupCosts` at the call.
    pub ripup_costs: i32,
    /// Java `traceLength=` — the connection's cumulative trace length.
    pub trace_length: f64,
    /// Java `minTraceLength=` — the start-to-end distance (0 when the
    /// connection is open-ended).
    pub min_trace_length: f64,
    /// Java `itemCount=` — the connection's item count.
    pub item_count: i32,
    /// Java `detour=` — AFTER the randomize factor (the capture's
    /// `detour` column is the pre-division value).
    pub detour: f64,
    /// Java `result=`.
    pub result: i32,
}

impl<E: DrillEngine, D: DestinationDistance, V: ViaLayerChecker> MazeSearchEngine<'_, E, D, V> {
    /// Java `MazeRipupResolver.checkRipup` (`:72-197`) — checks whether
    /// the next room can be ripped and returns its cost, or -1 when it
    /// cannot be ripped.
    pub(crate) fn check_ripup(
        &mut self,
        element: &MazeListElement,
        obstacle_item: u64,
        door_is_small: bool,
    ) -> i32 {
        self.check_ripup_traced(element, obstacle_item, door_is_small)
            .0
    }

    /// [`Self::check_ripup`] with the CHECK_RIPUP payload (the pins
    /// replay the capture rows through it; the engine wiring drops the
    /// trace).
    pub(crate) fn check_ripup_traced(
        &mut self,
        element: &MazeListElement,
        obstacle_item: u64,
        door_is_small: bool,
    ) -> (i32, Option<RipupTrace>) {
        if !self.ctx.item_is_routable(obstacle_item) {
            return (-1, None);
        }
        if door_is_small && !self.enter_through_small_door(element, obstacle_item) {
            return (-1, None);
        }

        // Java `listElement.door.otherRoom(listElement.nextRoom)` —
        // the room we CAME FROM. `DrillPage`/`ExpansionDrill` answer
        // Java null (DrillPage.java:185-187, ExpansionDrill.java:105-
        // 107), target-door elements carry no next room (unreachable
        // here). Only an OBSTACLE room yields a previous item.
        let previous_item: Option<u64> = match &element.door {
            ExpandableObject::RoomDoor(room_door) => {
                let next_room_key = element
                    .next_room_key
                    .expect("checkRipup runs on an element parked inside its next room");
                let next_room_id = self.ctx.room_id(next_room_key);
                room_door
                    .other_room_id(next_room_id)
                    .and_then(|id| self.ctx.room_key_of_door(id, room_door))
                    .filter(|key| self.ctx.room_is_obstacle(*key))
                    .and_then(|key| self.ctx.room_obstacle_item_key(key))
            }
            _ => None,
        };
        let room_was_shoved = element.adjustment != crate::drill::Adjustment::None;
        if room_was_shoved {
            if previous_item.is_some_and(|item| {
                item != obstacle_item && self.ctx.item_shares_net(item, obstacle_item)
            }) {
                return (-1, None);
            }
        } else if previous_item == Some(obstacle_item) {
            return (ALREADY_RIPPED_COSTS, None);
        }

        let mut fanout_via_cost_factor = 1.0;
        let mut cost_factor: f64 = 1.0;
        let preserve_fanout_protection = !self.ctrl.remove_unconnected_vias
            && self.ctrl.ripup_costs <= self.ctrl.settings.start_ripup_costs * 2;
        if self.ctx.item_is_trace(obstacle_item) {
            cost_factor = f64::from(self.ctx.item_trace_half_width(obstacle_item));
            if preserve_fanout_protection {
                fanout_via_cost_factor = calc_fanout_via_ripup_cost_factor(self.ctx, obstacle_item);
            }
        } else if self.ctx.item_is_via(obstacle_item) {
            let mut look_if_fanout_via = preserve_fanout_protection;
            let contact_list = self.ctx.item_normal_contacts(obstacle_item);
            let mut contact_count: i32 = 0;
            for current_contact in contact_list {
                if !self.ctx.item_is_trace(current_contact)
                    || self.ctx.item_is_user_fixed(current_contact)
                {
                    return (-1, None);
                }
                contact_count += 1;
                cost_factor =
                    cost_factor.max(f64::from(self.ctx.item_trace_half_width(current_contact)));
                if look_if_fanout_via && !self.ctrl.is_fanout {
                    let current_fanout_via_cost_factor =
                        calc_fanout_via_ripup_cost_factor(self.ctx, current_contact);
                    if current_fanout_via_cost_factor > 1.0 {
                        fanout_via_cost_factor = current_fanout_via_cost_factor;
                        look_if_fanout_via = false;
                    }
                }
            }
            if fanout_via_cost_factor <= 1.0 {
                cost_factor *= 0.5 * f64::from(contact_count.saturating_sub(1));
            }
        }

        let mut ripup_cost = f64::from(self.ctrl.ripup_costs) * cost_factor;
        let mut detour = 1.0;
        let mut trace_length = 0.0;
        let mut min_trace_length = 0.0;
        let mut item_count: i32 = 0;
        let mut connection_item_ids = "[]".to_string();
        if fanout_via_cost_factor <= 1.0
            && !self.ctrl.is_fanout
            && let Some(obstacle_connection) = crate::path::Connection::get(self.ctx, obstacle_item)
        {
            detour = obstacle_connection.get_detour(self.ctx);
            trace_length = obstacle_connection.trace_length(self.ctx);
            item_count = i32::try_from(obstacle_connection.item_list.len())
                .expect("connection item count fits i32");
            if let (Some(start), Some(end)) = (
                &obstacle_connection.start_point,
                &obstacle_connection.end_point,
            ) {
                min_trace_length = start.to_float().distance(&end.to_float());
            }
            connection_item_ids = format!(
                "[{}]",
                obstacle_connection
                    .items_descending()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
        // The randomize gate is INDEPENDENT of the fanout gate — the
        // capture's pass-4/7 rows carry BOTH the fanout factor and a
        // random factor. The draw consumes the generator state ONLY on
        // randomized passes (stateful across checkRipup calls).
        let randomize = self.ctrl.ripup_pass_no >= 4 && self.ctrl.ripup_pass_no % 3 != 0;
        if randomize {
            let random_number = self.random_generator.next_double();
            let random_factor = 0.5 + random_number * random_number;
            detour *= random_factor;
        }
        ripup_cost /= detour;
        ripup_cost *= fanout_via_cost_factor;
        // Java `(int)` cast and the Rust `as` both truncate toward zero
        // and saturate at the i32 bounds; negatives cannot arise.
        let mut result = (ripup_cost as i32).max(1);
        let max_ripup_costs = i32::MAX / 100;
        result = result.min(max_ripup_costs);

        (
            result,
            Some(RipupTrace {
                connection_item_ids,
                half_width: cost_factor,
                ripup_costs: self.ctrl.ripup_costs,
                trace_length,
                min_trace_length,
                item_count,
                detour,
                result,
            }),
        )
    }

    /// Java `MazeRipupResolver.checkLeavingRippedItem` (`:200-213`) —
    /// checks entering a thick room from a via or trace through a small
    /// door after ripup.
    pub(crate) fn check_leaving_ripped_item(&mut self, element: &MazeListElement) -> bool {
        let ExpandableObject::RoomDoor(current_door) = &element.door else {
            return false;
        };
        let next_room_key = element
            .next_room_key
            .expect("checkLeavingRippedItem reads the element's next room");
        let next_room_id = self.ctx.room_id(next_room_key);
        let Some(from_room_key) = current_door
            .other_room_id(next_room_id)
            .and_then(|id| self.ctx.room_key_of_door(id, current_door))
        else {
            // Java `fromRoom == null` fails the instanceof arm.
            return false;
        };
        if !self.ctx.room_is_obstacle(from_room_key) {
            return false;
        }
        let current_item = self
            .ctx
            .room_obstacle_item_key(from_room_key)
            .expect("an obstacle room carries its obstacle item");
        if !self.ctx.item_is_routable(current_item) {
            return false;
        }
        self.enter_through_small_door(element, current_item)
    }

    /// Java `MazeRipupResolver.enterThroughSmallDoor` (`:219-268`) —
    /// checks whether a door can be entered while ignoring the obstacle
    /// item and its directly connected items.
    pub(crate) fn enter_through_small_door(
        &mut self,
        element: &MazeListElement,
        ignore_item: u64,
    ) -> bool {
        // Java `listElement.door.getDimension()`: room doors carry the
        // dimension field; drills and drill pages answer 2 (the false
        // arm). Target-door elements have no next room and never reach
        // the ripup callers.
        let door_dimension = element.door.dimension();
        if door_dimension != 1 {
            return false;
        }
        let door_shape = self.element_door_shape(element);
        let mut door_line: Option<Line> = None;
        let mut prev_corner = door_shape.corner_approx(0).unwrap_or(FloatPoint::ZERO);
        let corner_count = door_shape.border_line_count();
        for i in 1..corner_count as i32 {
            let next_corner = door_shape.corner_approx(i).unwrap_or(FloatPoint::ZERO);
            if next_corner.distance_square(&prev_corner) > 1.0 {
                door_line = Some(door_shape.border_line(i - 1));
                break;
            }
            prev_corner = next_corner;
        }
        let Some(door_line) = door_line else {
            return false;
        };

        let door_center = door_shape.centre_of_gravity().round();
        let next_room_key = element
            .next_room_key
            .expect("enterThroughSmallDoor reads the element's next room");
        let current_layer = self.ctx.room_layer(next_room_key);
        // Java `int checkRadius = compensatedTraceHalfWidth[layer] +
        // TRACE_WIDTH_TOLERANCE` (the Java constant is the INT 2); the
        // Rust seam const is f64, and the f64 sum truncates back to
        // Java's int for the offset shape (half widths are integers).
        let check_radius =
            f64::from(self.ctrl.compensated_trace_half_width[current_layer as usize])
                + crate::expansion::TRACE_WIDTH_TOLERANCE;
        let lines = vec![
            door_line.translate(check_radius),
            Line::new_with_direction(
                Point::get_instance(door_center.x, door_center.y),
                door_line.direction().clone().turn_45_degree(2),
            ),
            door_line.translate(-check_radius),
        ];
        let check_polyline = Polyline::new(lines);
        // Java passes a null offset shape straight into the tree query
        // (NPE discipline); a healthy door shape always offsets.
        let check_shape = check_polyline
            .offset_shape(check_radius as i32, 0)
            .expect("the small-door check polyline offsets to a shape");
        let overlapping_objects = self.ctx.overlapping_objects_ignore_nets(
            &check_shape,
            current_layer,
            &[self.ctrl.net_number],
        );

        // ORDER-INDEPENDENCE: the tree's iteration order is not
        // reproducible, but the verdict does not depend on it — the
        // loop is a universal conjunction (every object must share the
        // net and carry ignore_item as a normal contact), so any
        // failing object yields false no matter where it is visited.
        for current_object in overlapping_objects {
            // Java skips non-Item tree objects (rooms) and the ignored
            // item itself (reference equality).
            if !self.ctx.is_item(current_object) || current_object == ignore_item {
                continue;
            }
            if !self.ctx.item_shares_net(current_object, ignore_item) {
                return false;
            }
            if !self
                .ctx
                .item_normal_contacts(current_object)
                .contains(&ignore_item)
            {
                return false;
            }
        }
        true
    }

    /// Java `listElement.door.getShape()` — the door shape. Room doors
    /// derive it from their endpoint rooms (the engine's door values
    /// carry no shape; the derivation is the T6-verified
    /// `shape_between` of the live endpoint shapes).
    fn element_door_shape(
        &self,
        element: &MazeListElement,
    ) -> epic_geometry::tile_shape::TileShape {
        match &element.door {
            ExpandableObject::RoomDoor(room_door) => {
                let (_, _, first_shape, second_shape) = self.door_endpoint_shapes(room_door);
                crate::expansion::ExpansionDoor::shape_between(&first_shape, &second_shape)
            }
            // Unreachable in the ripup callers (the dimension-1 gate
            // fails for drills/pages; target-door elements have no next
            // room) — the Java interface shape would be the object's
            // own shape.
            _ => panic!("enterThroughSmallDoor only runs on room doors"),
        }
    }
}

/// Java `MazeRipupResolver.calcFanoutViaRipupCostFactor` (`:35-66`) —
/// the cost factor protecting a fanout via at a trace end: a
/// single-layer pin contact or a 2-corner shove-fixed trace contact
/// yields `((halfWidth / length)^2) * 20000` clamped to at least 1;
/// the START contacts are probed before the END contacts and the first
/// protecting contact wins.
pub(crate) fn calc_fanout_via_ripup_cost_factor<E: DrillEngine>(
    ctx: &mut E,
    trace_key: u64,
) -> f64 {
    for i in 0..2 {
        let contacts = if i == 0 {
            ctx.trace_start_contacts(trace_key)
        } else {
            ctx.trace_end_contacts(trace_key)
        };
        if contacts.len() != 1 {
            continue;
        }
        let current_trace_contact = contacts[0];
        let mut protect_fanout_via = false;
        if ctx.item_is_pin(current_trace_contact) && ctx.pin_drill_allowed(current_trace_contact) {
            // Java `firstLayer() == lastLayer()` — the single-layer
            // (SMD) pin test, which is exactly `Pin.drillAllowed()`.
            protect_fanout_via = true;
        } else if ctx.item_is_polyline_trace(current_trace_contact)
            && ctx.item_is_shove_fixed(current_trace_contact)
            && ctx.item_trace_corner_count(current_trace_contact) == 2
        {
            protect_fanout_via = true;
        }

        if protect_fanout_via {
            let mut fanout_via_cost_factor =
                f64::from(ctx.item_trace_half_width(trace_key)) / ctx.item_trace_length(trace_key);
            fanout_via_cost_factor *= fanout_via_cost_factor;
            fanout_via_cost_factor *= FANOUT_COST_CONSTANT;
            return fanout_via_cost_factor.max(1.0);
        }
    }
    1.0
}

#[cfg(test)]
mod push_shove_tests {
    //! The M6-T9 waiver-predicate pins: every conjunction clause at its
    //! exact closed-form boundary, both directions (DNR-16) — the
    //! killing mutants of the T9 mutation round (M-PRED-*).

    use super::{PUSH_SHOVE_ROOM_BUDGET, push_shove_waive_ripup};

    /// The flag clause: OFF answers false even with everything else
    /// maximally favorable — the default-off byte-identity face.
    #[test]
    fn push_shove_waive_flag_off_never_fires() {
        assert!(
            !push_shove_waive_ripup(false, true, 100, PUSH_SHOVE_ROOM_BUDGET),
            "OFF: the first clause gates the whole waiver"
        );
    }

    /// The shoved clause: an unsuccessful probe never waives.
    #[test]
    fn push_shove_waive_requires_shoved() {
        assert!(!push_shove_waive_ripup(true, false, 100, 2));
    }

    /// The charge clause at its exact boundary ±1: `ripup_costs == 0`
    /// (nothing to waive — ALREADY_RIPPED_COSTS and the unrippable -1
    /// never reach the predicate anyway) does not waive; `1` does.
    /// A `>= 0` mutant dies on the 0 row; dropping the clause dies on
    /// the 0 row too (waive would fire with no charge pending).
    ///
    /// NIT-Q3 note (M7-T2): the `1` row is UNREACHABLE IN PRODUCTION —
    /// `ALREADY_RIPPED_COSTS == 1` short-circuits the enclosing block
    /// (`search_engine.rs`: `if ripup_costs != ALREADY_RIPPED_COSTS &&
    /// next_room_is_thick`) before the waiver site, so the smallest
    /// charge that can reach the predicate is 2. The row stays in the
    /// pin as the clause's unit-level boundary (the predicate, not the
    /// call site, owns the `> 0` semantics); it is NOT an observed
    /// engine state — a capture row showing charge `1` at the waiver
    /// would indicate a call-site regression, not a working engine.
    #[test]
    fn push_shove_waive_charge_boundary() {
        assert!(
            !push_shove_waive_ripup(true, true, 0, 2),
            "zero charge: nothing to waive"
        );
        assert!(
            push_shove_waive_ripup(true, true, 1, 2),
            "one unit of charge: waives"
        );
        assert!(
            !push_shove_waive_ripup(true, true, -1, 2),
            "the unrippable -1 never waives"
        );
        assert!(!push_shove_waive_ripup(true, true, -99, 2));
    }

    /// The budget clause at its exact boundary ±1: `budget_left == 0`
    /// (exhausted) does not waive; `1` (the last unit) does. A `>= 0`
    /// mutant dies on the 0 row.
    #[test]
    fn push_shove_waive_budget_boundary() {
        assert!(
            !push_shove_waive_ripup(true, true, 100, 0),
            "exhausted budget: no waiver"
        );
        assert!(
            push_shove_waive_ripup(true, true, 100, 1),
            "the last budget unit waives"
        );
    }

    /// The budget CONST literal (the tuning value): the ±1 mutation
    /// round (M-BUDGET) kills here and at the engine pin's
    /// conservation face.
    #[test]
    fn push_shove_room_budget_literal() {
        assert_eq!(PUSH_SHOVE_ROOM_BUDGET, 2, "the T9 tuning value");
    }
}
