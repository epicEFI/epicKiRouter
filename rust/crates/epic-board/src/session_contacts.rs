//! The T15/D23 `SessionContacts` provider: board-side drill-contact
//! data for the SES endpoint-snap rule.
//!
//! Java anchors — `SesWriter.snappedEndpoint` (`:426-453`) consumes
//! `wire.getStartContacts()/getEndContacts()` and, per contact that is
//! a `DrillItem`, applies three PROVIDER-side filters before the rule
//! ever sees it:
//!
//! 1. kind: only `DrillItem` (Pin/Via) — trace/area contacts `continue`;
//! 2. layer span: the contact must span the WIRE's layer
//!    (`layer < drill.firstLayer() || layer > drill.lastLayer()` →
//!    `continue`, `:435-437`);
//! 3. shape: `drill.getShape(layer - drill.firstLayer())` non-null
//!    (`:438-441`).
//!
//! THIS module is those filters (plus the center and
//! `borderDistance(center)` computations); the RULE — the `<= 0.5`
//! function-level quirk, the inradius `<=`, first-qualifying-wins — is
//! the pure function `epic_dsn::ses::writer::snapped_endpoint` (D23's
//! rule-side half). The split exists because the rule's branches are
//! unreachable through the real contacts path (see the writer's module
//! docs for the corpus-wide `snapFired=0` evidence): unit pins craft
//! drill vectors the live pipeline cannot produce.
//!
//! Ordering: [`SessionDrillContacts::endpoint_drills`] returns the
//! filtered drills in JAVA CONTACT ORDER — descending id (T60's
//! `contacts_descend_by_id` pin; jar-witnessed by the crafted
//! coincident via+pin endpoint, whose contact set printed
//! `contacts=[4:Via 2:Pin]`). The rule's first-qualifying-wins walk is
//! order-observable, so the provider must not re-sort.
//!
//! Pre-collection: the contact queries need `(&SearchTreeManager,
//! &mut Board)`, which the emit loop cannot hand out — so
//! [`SessionDrillContacts::collect`] snapshots every trace endpoint's
//! drill list up front (compute-on-demand under the hood, exactly like
//! the T10 seam it rides) and the emit loop then borrows the snapshot
//! immutably while the [`SesBoard`](epic_dsn::SesBoard) is borrowed for
//! the items themselves.

use crate::board::Board;
use crate::contacts::{end_contacts, start_contacts};
use crate::id::ItemId;
use crate::items::ItemData;
use crate::tree_manager::SearchTreeManager;
use epic_dsn::ses::writer::{EndpointDrill, SessionContacts};
use epic_geometry::float_point::FloatPoint;
use std::collections::HashMap;

/// The pre-collected snapshot: per trace id, the provider-filtered
/// drill data for the FIRST-corner side and the LAST-corner side.
///
/// Built by [`collect`](SessionDrillContacts::collect) from the T10
/// contacts seam; implements the epic-dsn trait the 3-arg writer takes.
pub struct SessionDrillContacts {
    start: HashMap<i32, Vec<EndpointDrill>>,
    end: HashMap<i32, Vec<EndpointDrill>>,
}

impl SessionDrillContacts {
    /// Snapshots every trace's endpoint drill data, provider-filtered
    /// and in contact (descending-id) order. Mirrors what Java's
    /// `snappedEndpoint` would query per wire — but once, up front, so
    /// the emit loop never needs `&mut Board`.
    #[must_use]
    pub fn collect(manager: &SearchTreeManager, board: &mut Board) -> Self {
        let mut provider = SessionDrillContacts {
            start: HashMap::new(),
            end: HashMap::new(),
        };
        // Snapshot the trace ids first — the contact queries need
        // `&mut Board`, so no iteration borrow may survive them.
        let trace_ids: Vec<ItemId> = board
            .iter_ascending()
            .filter(|entry| matches!(entry.data, ItemData::Trace { .. }))
            .map(|entry| entry.id)
            .collect();
        for id in trace_ids {
            // The clamp can never fire: `ItemId <= MAX_ID = 2^30 - 1`
            // (`id.rs`), far inside `i32`. If a future id source ever
            // exceeded `i32::MAX`, this would SILENTLY MERGE two traces'
            // contact lists under the same `i32::MAX` key — keep the
            // `MAX_ID` bound if this map stays keyed by `i32`.
            let key = i32::try_from(id.get()).unwrap_or(i32::MAX);
            let start_list = filtered_drills(manager, board, id, true);
            let end_list = filtered_drills(manager, board, id, false);
            provider.start.insert(key, start_list);
            provider.end.insert(key, end_list);
        }
        provider
    }
}

impl SessionContacts for SessionDrillContacts {
    fn endpoint_drills(&self, trace_id: i32, start_side: bool) -> &[EndpointDrill] {
        let side = if start_side { &self.start } else { &self.end };
        side.get(&trace_id).map_or(&[], |list| list.as_slice())
    }
}

/// One endpoint's filtered drill list — Java `snappedEndpoint`'s
/// contact loop (`:431-451`) up to (but excluding) the rule's own
/// comparisons. The contacts seam returns descending ids; the filters
/// preserve that order.
fn filtered_drills(
    manager: &SearchTreeManager,
    board: &mut Board,
    trace_id: ItemId,
    start_side: bool,
) -> Vec<EndpointDrill> {
    let Some(layer) = board.trace_layer(trace_id) else {
        return Vec::new();
    };
    let contacts = if start_side {
        start_contacts(manager, board, trace_id)
    } else {
        end_contacts(manager, board, trace_id)
    };
    let mut drills = Vec::new();
    for contact in contacts {
        // Filter 1 — kind: DrillItem only (`:432-434`).
        let is_drill = board
            .get(contact)
            .is_some_and(|entry| matches!(entry.data, ItemData::Pin { .. } | ItemData::Via { .. }));
        if !is_drill {
            continue;
        }
        // Filter 2 — layer span: the WIRE's layer inside the drill's
        // first..=last (`:435-437`).
        let (Some(first_layer), Some(last_layer)) = (
            board.drill_first_layer(contact),
            board.drill_last_layer(contact),
        ) else {
            continue;
        };
        if layer < first_layer || layer > last_layer {
            continue;
        }
        // Filter 3 — shape on the wire's layer non-null (`:438-441`);
        // then the center and the inradius bound (`:448-450`).
        let Some(shape) = board.drill_shape(contact, layer - first_layer) else {
            continue;
        };
        let Some(center_point) = board.drill_center(contact) else {
            continue;
        };
        let center: FloatPoint = center_point.to_float();
        let border_distance = shape.border_distance(&center);
        drills.push(EndpointDrill {
            center,
            border_distance,
        });
    }
    drills
}

/// Java-side reachability addendum: of the three provider filters,
/// only the KIND filter is live through the real contacts path —
/// - SPAN (`:435-437`) is structurally unreachable: the contact path
///   filters `currentItem.sharesLayer(this)` BEFORE the DrillItem
///   acceptance (`Trace.java:183-201`), and for a single-layer trace
///   on layer `w` and a drill spanning `[f, l]`,
///   `sharesLayer ⇔ f <= w <= l` — exactly the negation of the skip.
/// - SHAPE-NULL (`:438-441`) is unreachable too, EMPIRICALLY: a via
///   with shapes on layers 0 and 2 of a 3-layer board and a wire on
///   layer 1 yields jar `contacts=[]` (SesSnapOracle on the crafted
///   mid-null case, mirrored below) — the drill is not in the wire
///   layer's tree, so it is never a contact to begin with.
///
/// Both are kept verbatim because the jar is the contract; each is
/// pinned as a NEGATIVE witness (the wall), not a live branch.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::parse_board_from_text;

    /// The tree manager over the board's items — the T10 contacts-test
    /// fill (`insert_all_board_items`); the provider is order-stable
    /// across fills (the contacts seam returns descending ids either
    /// way).
    fn indexed(board: &mut Board) -> SearchTreeManager {
        let mut manager = SearchTreeManager::new();
        manager.insert_all_board_items(board);
        manager
    }

    /// The `trace_id` of the board's single trace item.
    fn only_trace(board: &Board) -> ItemId {
        board
            .iter_ascending()
            .find_map(|entry| match entry.data {
                ItemData::Trace { .. } => Some(entry.id),
                _ => None,
            })
            .expect("one trace item")
    }

    fn key(id: ItemId) -> i32 {
        i32::try_from(id.get()).unwrap_or(0)
    }

    const AT_CENTER_DSN: &str = r##"(pcb "at-center.dsn"
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
    (host_cad "T15-spike")
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu
      (type signal)
      (property
        (index 0)
      )
    )
    (layer B.Cu
      (type signal)
      (property
        (index 1)
      )
    )
    (boundary
      (path pcb 0  90000 95000  90000 110000  115000 110000  115000 95000  90000 95000)
    )
    (via "Via[0-1]_889:635_um")
    (rule
      (width 177.8)
      (clearance 152.4)
    )
  )
  (placement
    (component SPIKE:TwoPad
      (place U1 100000 100000 front 0 (PN "SPIKE"))
    )
  )
  (library
    (image SPIKE:TwoPad
      (pin Rect[A]Pad_1727.200000x1727.200000_um 1 -2540 0)
      (pin Rect[A]Pad_1727.200000x1727.200000_um 2 2540 0)
    )
    (padstack Rect[A]Pad_1727.200000x1727.200000_um
      (shape (rect F.Cu -863.6 -863.6 863.6 863.6))
      (shape (rect B.Cu -863.6 -863.6 863.6 863.6))
      (attach off)
    )
    (padstack "Via[0-1]_889:635_um"
      (shape (circle F.Cu 889))
      (shape (circle B.Cu 889))
      (attach off)
    )
  )
  (network
    (net SPIKE
      (pins U1-1 U1-2)
    )
    (class kicad_default SPIKE
      (circuit
        (use_via "Via[0-1]_889:635_um")
      )
      (rule
        (width 177.8)
        (clearance 152.4)
      )
    )
  )
  (wiring
    (via "Via[0-1]_889:635_um" 97460 100000 (net SPIKE) (type route))
    (wire (path F.Cu 406.4  97460 100000  102540 100000)(net SPIKE)(type route))
  )
)
"##;

    const BETWEEN_PADS_DSN: &str = r##"(pcb "between-pads.dsn"
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
    (host_cad "T15-spike")
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu
      (type signal)
      (property
        (index 0)
      )
    )
    (layer B.Cu
      (type signal)
      (property
        (index 1)
      )
    )
    (boundary
      (path pcb 0  90000 95000  90000 110000  115000 110000  115000 95000  90000 95000)
    )
    (via "Via[0-1]_889:635_um")
    (rule
      (width 177.8)
      (clearance 152.4)
    )
  )
  (placement
    (component SPIKE:TwoPad
      (place U1 100000 100000 front 0 (PN "SPIKE"))
    )
  )
  (library
    (image SPIKE:TwoPad
      (pin Rect[A]Pad_1727.200000x1727.200000_um 1 -2540 0)
      (pin Rect[A]Pad_1727.200000x1727.200000_um 2 2540 0)
    )
    (padstack Rect[A]Pad_1727.200000x1727.200000_um
      (shape (rect F.Cu -863.6 -863.6 863.6 863.6))
      (shape (rect B.Cu -863.6 -863.6 863.6 863.6))
      (attach off)
    )
    (padstack "Via[0-1]_889:635_um"
      (shape (circle F.Cu 889))
      (shape (circle B.Cu 889))
      (attach off)
    )
  )
  (network
    (net SPIKE
      (pins U1-1 U1-2)
    )
    (class kicad_default SPIKE
      (circuit
        (use_via "Via[0-1]_889:635_um")
      )
      (rule
        (width 177.8)
        (clearance 152.4)
      )
    )
  )
  (wiring
    (wire (path F.Cu 406.4  100000 100000  100000 104000)(net SPIKE)(type route))
  )
)
"##;

    const CHAIN_DSN: &str = r##"(pcb "at-center.dsn"
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
    (host_cad "T15-spike")
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu
      (type signal)
      (property
        (index 0)
      )
    )
    (layer B.Cu
      (type signal)
      (property
        (index 1)
      )
    )
    (boundary
      (path pcb 0  90000 95000  90000 110000  115000 110000  115000 95000  90000 95000)
    )
    (via "Via[0-1]_889:635_um")
    (rule
      (width 177.8)
      (clearance 152.4)
    )
  )
  (placement
    (component SPIKE:TwoPad
      (place U1 100000 100000 front 0 (PN "SPIKE"))
    )
  )
  (library
    (image SPIKE:TwoPad
      (pin Rect[A]Pad_1727.200000x1727.200000_um 1 -2540 0)
      (pin Rect[A]Pad_1727.200000x1727.200000_um 2 2540 0)
    )
    (padstack Rect[A]Pad_1727.200000x1727.200000_um
      (shape (rect F.Cu -863.6 -863.6 863.6 863.6))
      (shape (rect B.Cu -863.6 -863.6 863.6 863.6))
      (attach off)
    )
    (padstack "Via[0-1]_889:635_um"
      (shape (circle F.Cu 889))
      (shape (circle B.Cu 889))
      (attach off)
    )
  )
  (network
    (net SPIKE
      (pins U1-1 U1-2)
    )
    (class kicad_default SPIKE
      (circuit
        (use_via "Via[0-1]_889:635_um")
      )
      (rule
        (width 177.8)
        (clearance 152.4)
      )
    )
  )
  (wiring
    (via "Via[0-1]_889:635_um" 97460 100000 (net SPIKE) (type route))
    (wire (path F.Cu 406.4  97460 100000  102540 100000)(net SPIKE)(type route))
    (wire (path F.Cu 406.4  102540 100000  102540 105000)(net SPIKE)(type route))
  )
)
"##;

    const MID_NULL_DSN: &str = r##"(pcb mid-null.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
    (host_cad "T15-spike")
  )
  (resolution um 1)
  (unit um)
  (structure
    (layer F.Cu
      (type signal)
      (property
        (index 0)
      )
    )
    (layer In1.Cu
      (type signal)
      (property
        (index 1)
      )
    )
    (layer B.Cu
      (type signal)
      (property
        (index 2)
      )
    )
    (boundary
      (path pcb 0  0 0  200000 0  200000 200000  0 200000  0 0)
    )
    (via PAD_END
      (shape (circle F.Cu 600 0 0))
      (shape (circle B.Cu 600 0 0))
    )
    (rule
      (width 250)
      (clearance 14)
    )
  )
  (library
    (padstack PAD_END
      (shape (circle F.Cu 600 0 0))
      (shape (circle B.Cu 600 0 0))
      (attach off)
    )
  )
  (network
    (net MID)
    (class kicad_default MID
      (circuit
        (use_via PAD_END)
      )
      (rule
        (width 250)
        (clearance 14)
      )
    )
  )
  (wiring
    (via PAD_END 50000 50000 (net MID)(type route))
    (wire (path In1.Cu 250  50000 50000  90000 50000)(net MID)(type route))
  )
)
"##;

    /// Pins the provider's ordered output on the coincident endpoint:
    /// BOTH drills survive the filters, VIA (id 4, the higher id)
    /// FIRST — Java contact order (descending), the observable the
    /// snap's first-qualifying-wins walk depends on. Centers and
    /// border distances are the jar capture rows (dsn x 10 board
    /// scale; via circle radius 889/2 x 10 = 4445; pin rect half
    /// 863.6 x 10 = 8636).
    #[test]
    fn provider_orders_coincident_drills_descending() {
        let mut board = parse_board_from_text(AT_CENTER_DSN);
        let manager = indexed(&mut board);
        let trace = only_trace(&board);
        assert_eq!(trace.get(), 5, "P_CAPTURE trace=5 (jar row)");

        let provider = SessionDrillContacts::collect(&manager, &mut board);
        let start = provider.endpoint_drills(5, true);
        assert_eq!(start.len(), 2, "contacts=[4:Via 2:Pin] (jar row)");
        assert_eq!(start[0].center, FloatPoint::new(974_600.0, 1_000_000.0));
        assert_eq!(start[0].border_distance, 4445.0, "via inradius jar row");
        assert_eq!(start[1].center, FloatPoint::new(974_600.0, 1_000_000.0));
        assert_eq!(start[1].border_distance, 8636.0, "pin inradius jar row");

        let end = provider.endpoint_drills(5, false);
        assert_eq!(end.len(), 1, "contacts=[3:Pin] (jar row)");
        assert_eq!(end[0].center, FloatPoint::new(1_025_400.0, 1_000_000.0));
        assert_eq!(end[0].border_distance, 8636.0);
    }

    /// The WALL witness — mirrors the jar's between-pads capture
    /// (`contacts=[]` on both endpoints): an endpoint BETWEEN two pads
    /// is no pad's contact (drill acceptance needs endpoint ==
    /// center), so the provider yields nothing and the snap rule never
    /// sees a drill. This is the structural reason the inradius arm is
    /// dead upstream (the writer module docs).
    #[test]
    fn provider_between_pads_endpoint_has_no_drills() {
        let mut board = parse_board_from_text(BETWEEN_PADS_DSN);
        let manager = indexed(&mut board);
        let provider = SessionDrillContacts::collect(&manager, &mut board);
        let trace = only_trace(&board);
        assert!(
            provider.endpoint_drills(key(trace), true).is_empty(),
            "contacts=[] (jar row): the wall"
        );
        assert!(
            provider.endpoint_drills(key(trace), false).is_empty(),
            "contacts=[] (jar row): the wall, end side"
        );
    }

    /// The KIND filter — a trace-to-trace contact (the contacts seam
    /// returns the other trace) yields NO drill data (`:432-434`
    /// `instanceof DrillItem`). The chain board: a via at the wire
    /// junction, a second wire whose start corner sits on the first
    /// wire's end corner.
    #[test]
    fn provider_drops_trace_contacts() {
        let mut board = parse_board_from_text(CHAIN_DSN);
        let manager = indexed(&mut board);
        let traces: Vec<ItemId> = board
            .iter_ascending()
            .filter_map(|entry| match entry.data {
                ItemData::Trace { .. } => Some(entry.id),
                _ => None,
            })
            .collect();
        assert_eq!(traces.len(), 2, "jar row: traces=2");
        let provider = SessionDrillContacts::collect(&manager, &mut board);

        // Jar row: `trace=6 side=start contacts=[5:Trace 3:Pin]` — the
        // contact set holds a TRACE and a PIN; only the pin's drill
        // data may survive the kind filter (the trace drops), and the
        // pin's row keeps its exact center/inradius.
        let seam = start_contacts(&manager, &mut board, traces[1]);
        assert_eq!(
            seam.len(),
            2,
            "contrast: contacts=[5:Trace 3:Pin] (jar row)"
        );
        let drills = provider.endpoint_drills(key(traces[1]), true);
        assert_eq!(
            drills.len(),
            1,
            "the Trace contact drops, the Pin drill stays"
        );
        assert_eq!(drills[0].center, FloatPoint::new(1_025_400.0, 1_000_000.0));
        assert_eq!(drills[0].border_distance, 8636.0, "pin inradius jar row");

        // Jar row: `trace=5 side=end contacts=[6:Trace 3:Pin]`.
        let seam5 = end_contacts(&manager, &mut board, traces[0]);
        assert_eq!(
            seam5.len(),
            2,
            "contrast: contacts=[6:Trace 3:Pin] (jar row)"
        );
        let drills5 = provider.endpoint_drills(key(traces[0]), false);
        assert_eq!(drills5.len(), 1);
        assert_eq!(drills5[0].center, FloatPoint::new(1_025_400.0, 1_000_000.0));
    }

    /// The SHAPE-NULL wall — mirrors the jar's crafted mid-null
    /// capture (`contacts=[]` on both endpoints): a via whose padstack
    /// has shapes on F.Cu and B.Cu only, contacted by a wire on the
    /// middle layer, is not even a CONTACT (the drill is absent from
    /// the wire layer's tree), so `getShape(1)` never runs on a real
    /// contact. The filter stays ported verbatim; this pins the wall.
    #[test]
    fn provider_null_middle_shape_via_is_no_contact() {
        let mut board = parse_board_from_text(MID_NULL_DSN);
        let manager = indexed(&mut board);
        let trace = only_trace(&board);
        let seam_contacts = start_contacts(&manager, &mut board, trace);
        assert!(
            seam_contacts.is_empty(),
            "contacts=[] (jar row): the via is not in the layer-1 tree"
        );
        let provider = SessionDrillContacts::collect(&manager, &mut board);
        assert!(
            provider.endpoint_drills(key(trace), true).is_empty(),
            "no contact -> no drill data"
        );
    }

    /// A non-trace id and an unknown id must answer empty (the trait's
    /// total-function contract; Java would throw there, the provider
    /// cannot).
    #[test]
    fn provider_unknown_and_non_trace_ids_answer_empty() {
        let mut board = parse_board_from_text(AT_CENTER_DSN);
        let manager = indexed(&mut board);
        let provider = SessionDrillContacts::collect(&manager, &mut board);
        assert_eq!(
            provider.endpoint_drills(5, true).len(),
            2,
            "id 5 is the trace"
        );
        assert!(provider.endpoint_drills(9999, true).is_empty());
        assert!(
            provider.endpoint_drills(2, true).is_empty(),
            "pin id 2 is no trace"
        );
    }
}
