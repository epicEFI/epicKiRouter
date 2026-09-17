//! Java `autoroute/path/Connection.java` — a routing connection ending
//! at the next fork or terminal item. The walk (`Connection.get`) is
//! the ripup resolver's cost unit: `MazeRipupResolver.checkRipup`
//! builds the `Connection` of the obstacle item and scores its detour.

use std::collections::BTreeSet;

use epic_geometry::point::Point;

use crate::drill::DrillEngine;

/// Java `Connection.DETOUR_ADD`.
const DETOUR_ADD: f64 = 100.0;

/// Java `Connection.DETOUR_ITEM_COST`.
const DETOUR_ITEM_COST: f64 = 0.1;

/// Java `Integer.MAX_VALUE` widened to f64 — the `getDetour` verdict
/// for an open-ended connection (Java auto-widens the int return).
const DETOUR_OPEN_ENDED: f64 = 2147483647.0;

/// Java `app.freerouting.autoroute.path.Connection` — describes a
/// routing connection ending at the next fork or terminal item.
///
/// Java's `itemList` is a `TreeSet<Item>` iterating ids DESCENDING;
/// here [`Connection::item_list`] is a `BTreeSet` in ASCENDING id
/// order — read it through [`Connection::items_descending`] where the
/// Java iteration order is observable (e.g. capture item-id strings).
#[derive(Clone, Debug)]
pub struct Connection {
    /// Java `startPoint` — `None` is the Java null: the connection
    /// ends in empty space on this side.
    pub start_point: Option<Point>,
    /// Java `startLayer`.
    pub start_layer: i32,
    /// Java `endPoint` — `None` is the Java null (empty-space end).
    pub end_point: Option<Point>,
    /// Java `endLayer`.
    pub end_layer: i32,
    /// Java `itemList` — ASCENDING item ids (Java's TreeSet order is
    /// DESCENDING; see [`Connection::items_descending`]).
    pub item_list: BTreeSet<u64>,
}

impl Connection {
    /// Java's `TreeSet<Item>` iteration order (ids DESCENDING).
    pub fn items_descending(&self) -> impl Iterator<Item = u64> + '_ {
        self.item_list.iter().rev().copied()
    }

    /// Java `Connection.get(Item)` — the connection this item belongs
    /// to; `None` is the Java null (the item is not routable, or — for
    /// the memo arm — see the deviation note below).
    ///
    /// Deviation from Java: the precalculated-connection memo
    /// (`item.getAutorouteInfo().getPrecalculatedConnection()` plus the
    /// tail `setPrecalculatedConnection` loop) is SKIPPED. The memo is
    /// value-identical to a fresh walk on an unchanged board — it
    /// caches THIS algorithm's own result — and the Rust engine
    /// recomputes at each call site; no capture row can distinguish
    /// memoized from recomputed (the oracle spike reads through fresh
    /// boards per run).
    ///
    /// The walk consumes the seam [`DrillEngine::item_normal_contacts`]
    /// whose contract is Java's `TreeSet` DESCENDING order — the
    /// contact order IS observable (the first-terminating contact
    /// becomes the start point).
    pub fn get<E: DrillEngine>(ctx: &mut E, item_key: u64) -> Option<Connection> {
        if !ctx.item_is_routable(item_key) {
            return None;
        }
        let contacts = ctx.item_normal_contacts(item_key);
        let mut connection_items: BTreeSet<u64> = BTreeSet::new();
        connection_items.insert(item_key);

        let mut start_point: Option<Point> = None;
        let mut start_layer = 0;
        let mut end_point: Option<Point> = None;
        let mut end_layer = 0;

        for contact in contacts {
            let mut current_item = contact;
            let Some(mut prev_contact_point) = ctx.normal_contact_point(item_key, current_item)
            else {
                // no unique contact point
                continue;
            };
            let mut prev_contact_layer = ctx.first_common_layer(item_key, current_item);
            let mut fork_found = false;
            if ctx.item_is_trace(item_key) {
                // Check, that there is only 1 contact at this location.
                // Only for pins and vias items of more than 1 connection
                // are collected
                let check_contacts =
                    ctx.trace_normal_contacts_at(item_key, &prev_contact_point, false);
                if check_contacts.len() != 1 {
                    fork_found = true;
                }
            }
            // Search from currentItem along the contacts
            // until the next fork or nonroute item.
            loop {
                if !ctx.item_is_routable(current_item) || fork_found {
                    // connection ends
                    if let Some(start) = start_point.as_ref() {
                        if !points_equal(&prev_contact_point, start) {
                            end_point = Some(prev_contact_point);
                            end_layer = prev_contact_layer;
                        }
                    } else {
                        start_point = Some(prev_contact_point);
                        start_layer = prev_contact_layer;
                    }
                    break;
                }
                connection_items.insert(current_item);
                let current_item_contacts = ctx.item_normal_contacts(current_item);
                // filter the contacts at the previous contact point,
                // because we were already there.
                // If then there is not exactly 1 new contact left, there is
                // a stub or a fork.
                let mut next_contact_point: Option<Point> = None;
                let mut next_contact_layer = -1;
                let mut next_contact: Option<u64> = None;
                for tmp_contact in current_item_contacts {
                    let tmp_contact_layer = ctx.first_common_layer(current_item, tmp_contact);
                    if tmp_contact_layer >= 0 {
                        let Some(tmp_contact_point) =
                            ctx.normal_contact_point(current_item, tmp_contact)
                        else {
                            // no unique contact point
                            fork_found = true;
                            break;
                        };
                        if prev_contact_layer != tmp_contact_layer
                            || !points_equal(&prev_contact_point, &tmp_contact_point)
                        {
                            next_contact_point = Some(tmp_contact_point);
                            next_contact_layer = tmp_contact_layer;
                            if next_contact.is_some() {
                                // second new contact found
                                fork_found = true;
                                break;
                            }
                            next_contact = Some(tmp_contact);
                        }
                    }
                }
                let Some(next_contact) = next_contact else {
                    break;
                };
                // Java quirk kept verbatim: the ITEM advances to the
                // FIRST new contact, but the point/layer were already
                // OVERWRITTEN by the SECOND new contact before the
                // fork break — on a fork the recorded endpoint is the
                // second contact's point while the walk continues at
                // the first (the capture pins the divergence).
                current_item = next_contact;
                prev_contact_point =
                    next_contact_point.expect("a recorded contact carries its point");
                prev_contact_layer = next_contact_layer;
            }
        }

        Some(Connection {
            start_point,
            start_layer,
            end_point,
            end_layer,
            item_list: connection_items,
        })
    }

    /// Java `Connection.traceLength()` (`Connection.java:133-141`) —
    /// the cumulative length of the traces in this connection. Java
    /// sums over the `itemList` `TreeSet<Item>` whose natural order is
    /// DESCENDING id (`Item.compareTo` is `item.id - id`), so the f64
    /// additions run high-id to low-id; mirror that walk exactly —
    /// ascending summation is exact for integer-valued lengths but a
    /// latent float-order divergence otherwise.
    pub fn trace_length<E: DrillEngine>(&self, ctx: &E) -> f64 {
        self.items_descending()
            .filter(|&key| ctx.item_is_trace(key))
            .map(|key| ctx.item_trace_length(key))
            .sum()
    }

    /// Java `Connection.getDetour()` — an estimation of the actual
    /// length of the connection divided by the minimal possible length.
    pub fn get_detour<E: DrillEngine>(&self, ctx: &E) -> f64 {
        let (Some(start), Some(end)) = (&self.start_point, &self.end_point) else {
            return DETOUR_OPEN_ENDED;
        };
        let min_trace_length = start.to_float().distance(&end.to_float());
        (self.trace_length(ctx) + DETOUR_ADD) / (min_trace_length + DETOUR_ADD)
            + DETOUR_ITEM_COST * (self.item_list.len() as f64 - 1.0)
    }
}

/// Java `Point.equals` for the walk's contact comparisons. The Rust
/// [`Point`] has no `PartialEq` (value equality is ill-defined across
/// the Int/Rational split), so the two Int points compare EXACTLY and
/// everything else falls to the float coordinates. Java's `equals` is
/// class-dispatched — a mixed Int/Rational pair is never equal there —
/// but mixed pairs do not arise (contact points are parse-grid
/// integers), so the float fallback is unreachable in practice.
fn points_equal(a: &Point, b: &Point) -> bool {
    match (a, b) {
        (Point::Int(x), Point::Int(y)) => x == y,
        _ => a.to_float() == b.to_float(),
    }
}
