//! Pin items — the Board-level shape/center delegates (M2 Task 4).
//!
//! The pin GEOMETRY (the 7-step `Pin.getShape` chain, the mirrored
//! layer span, the center correction) was ported in Task 3 and lives
//! in [`crate::components`] ([`crate::components::pin_shape`],
//! [`crate::components::pin_center`], [`pin_first_layer`] — module
//! docs there quote the spike captures). This module REUSES that
//! resolution — it does not re-derive it — and adds only the
//! ITEM-side surface: resolving an arena entry's
//! `(component_id, pin_index)` into the Task 3 calls.
//!
//! The delegates use only public Board accessors ([`Board::get`],
//! [`Board::components`], [`Board::library`]) — item modules cannot
//! touch the arena's private fields, by design (the [`ItemEntry`]
//! docs: derived state lives Board-side, never in the entry).
//!
//! Evidence: the chain-level captures are pinned where the chain
//! lives (`SYNTH_BACKPIN_SHAPE` in [`crate::items::drill`], the
//! `MLPIN`/`SPIKE3_MIRRORFIRST` rows in [`crate::components`]); the
//! fixture test here runs the bm08 jar capture through the FULL
//! board path (`DRILL kind=pin id=41 ... firstLayer=0 lastLayer=1
//! minWidth=15240.0` — the min width reads the pin shapes per layer,
//! so the delegate is width-constrained by a jar number).

use crate::id::ItemId;
use crate::items::{BoardShape, ItemData};

impl crate::board::Board {
    /// Java `Pin.getShape(index)` (`Pin.java:165-242`) for a PIN ITEM
    /// — the full Task 3 chain ([`crate::components::pin_shape`]) for
    /// the arena entry's `(component_id, pin_index)`. `None` for a
    /// non-pin item or an unresolvable pin.
    #[must_use]
    pub fn pin_shape(&self, id: ItemId, index: i32) -> Option<BoardShape> {
        let entry = self.get(id)?;
        let ItemData::Pin { pin_index, .. } = &entry.data else {
            return None;
        };
        let component_id = u32::try_from(entry.component_id).ok()?;
        crate::components::pin_shape(
            self.components(),
            self.library(),
            component_id,
            *pin_index,
            index,
        )
    }

    /// The RAW padstack bounding box the width faces read — Java
    /// `Pin.getMaxWidth`/`getMinWidth` resolve
    /// `getPadstackLayer(layer - firstLayer())` and take the
    /// PADSTACK's shape there (NO pin rotation/translation), so the
    /// mirror-side mapping is the only transform in play. `None` is
    /// Java's warn-and-0 face (a null shape slot; log-only).
    fn pin_raw_padstack_bbox_on_layer(
        &self,
        id: ItemId,
        layer: i32,
    ) -> Option<epic_geometry::int_box::IntBox> {
        let entry = self.get(id)?;
        let ItemData::Pin { padstack_no, .. } = &entry.data else {
            return None;
        };
        let component_id = u32::try_from(entry.component_id).ok()?;
        let component = self.components().get(component_id)?;
        let padstack = self.library().padstack(*padstack_no)?;
        let first_layer = crate::components::pin_first_layer(component, padstack);
        let padstack_layer =
            crate::components::padstack_layer(component, padstack, layer - first_layer);
        // Java `Padstack.getShape` (`Padstack.java:128-133`) answers
        // null (its warn row is log-only) for an out-of-range layer;
        // the port maps that onto None → the callers' Java warn-and-0
        // arms.
        let shape = if padstack_layer < 0 {
            None
        } else {
            padstack.get_shape(padstack_layer as usize)
        }?;
        Some(shape.bounding_box())
    }

    /// Java `Pin.getMaxWidth(layer)` (`Pin.java:517-531`): the largest
    /// width of the pin shape on `layer` — the raw padstack shape's
    /// bounding box `maxWidth()`. Java's null-shape warn arms return 0.
    #[must_use]
    pub fn pin_max_width_on_layer(&self, id: ItemId, layer: i32) -> f64 {
        self.pin_raw_padstack_bbox_on_layer(id, layer)
            .map_or(0.0, |bbox| bbox.max_width())
    }

    /// Java `Pin.getMinWidth(layer)` (`Pin.java:488-505`): the smallest
    /// width of the pin shape on `layer` — the raw padstack shape's
    /// bounding box `minWidth()`. Java's null-shape warn arms return 0.
    #[must_use]
    pub fn pin_min_width_on_layer(&self, id: ItemId, layer: i32) -> f64 {
        self.pin_raw_padstack_bbox_on_layer(id, layer)
            .map_or(0.0, |bbox| bbox.min_width())
    }

    /// Java `Pin.getTraceNeckdownHalfwidth(layer)` (`Pin.java:511-515`):
    /// `(int) Math.max(0.5 * getMinWidth(layer) - 1, 1)` — the neckdown
    /// half width a trace needs to enter/leave a pin narrower than the
    /// trace. The double→int cast truncates (positive domain), the
    /// Rust `as` cast saturates identically here.
    #[must_use]
    pub fn pin_trace_neckdown_halfwidth(&self, id: ItemId, layer: i32) -> i32 {
        let result = (0.5 * self.pin_min_width_on_layer(id, layer) - 1.0).max(1.0);
        result as i32
    }

    /// Java `DrillItem.isOnLayer(layer)` (`DrillItem.java:157-159`) for
    /// a PIN — `firstLayer() <= layer <= lastLayer()` over the pin's
    /// (mirrored) layer span. `false` for a non-pin item.
    #[must_use]
    pub fn pin_is_on_layer(&self, id: ItemId, layer: i32) -> bool {
        let Some(entry) = self.get(id) else {
            return false;
        };
        let ItemData::Pin { padstack_no, .. } = &entry.data else {
            return false;
        };
        let Some(component_id) = u32::try_from(entry.component_id).ok() else {
            return false;
        };
        let Some(component) = self.components().get(component_id) else {
            return false;
        };
        let Some(padstack) = self.library().padstack(*padstack_no) else {
            return false;
        };
        let first_layer = crate::components::pin_first_layer(component, padstack);
        let last_layer = crate::components::pin_last_layer(component, padstack);
        layer >= first_layer && layer <= last_layer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Board;
    use epic_dsn::reader::{DsnReadResult, read_board};

    /// The bm08 fixture path — the outline-keepout pin fixture and the
    /// smallest tier-A board with pins worth sampling (`DRILLS vias=0
    /// pins=40`).
    const BM08: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../scripts/benchmark/fixtures/DAC2020_boards/DAC2020_bm08.dsn"
    );

    /// Parses bm08 through the epic-dsn reader and converts it.
    fn bm08_board() -> Board {
        let bytes = std::fs::read(BM08).expect("bm08 fixture present");
        let mut ses = epic_dsn::ses_board::SesBoard::new();
        match read_board(bytes.as_slice(), &mut ses) {
            DsnReadResult::Success { .. } => {}
            other => panic!("expected Success, got {other:?}"),
        }
        Board::from_ses_board(&ses)
    }

    /// The first pin item id on the descending walk (bm08's highest
    /// live ids are the pins; the outline is id 1).
    fn first_pin_id(board: &Board) -> ItemId {
        board
            .iter_descending()
            .find(|entry| matches!(entry.data, ItemData::Pin { .. }))
            .map(|entry| entry.id)
            .expect("bm08 has 40 pins")
    }

    /// The bm08 pin capture through the BOARD path — capture lines
    /// (`/tmp/epic-t4-items-bm08.out`):
    /// ```text
    /// DRILLS vias=0 pins=40
    /// DRILL kind=pin id=41 center=1560830 -1010920
    ///   padstack=Oval[A]Pad_3048x1524_um psFrom=0 psTo=1 psLayerCount=2
    ///   placedFront=true firstLayer=0 lastLayer=1 attach=false
    ///   minWidth=15240.0
    /// ```
    /// `Board::pin_center` and the drill span/min-width methods
    /// (memoized Board-side, [`crate::board::Board`]) must reproduce
    /// all of it for id 41: the center is `location + relative
    /// location` with no pad-shape correction (the oval pad contains
    /// it), the span is the front-side padstack span 0..1, and the
    /// min width 15240.0 (1524 um x10) is the oval's short axis read
    /// through [`Board::pin_shape`] per signal layer.
    #[test]
    fn bm08_pin_center_span_and_min_width_match_the_capture() {
        let mut board = bm08_board();
        let id = ItemId::new(41);
        let entry = board.get(id).expect("pin id 41");
        let ItemData::Pin { padstack_no, .. } = &entry.data else {
            panic!("id 41 is a pin");
        };
        assert_eq!(
            board
                .library()
                .padstack(*padstack_no)
                .expect("padstack")
                .name,
            "Oval[A]Pad_3048x1524_um",
            "DRILL padstack=Oval[A]Pad_3048x1524_um"
        );
        // pin_index is whatever the entry carries (not in the capture);
        // it only has to RESOLVE through the package below.

        // The span and min width through the memoized Board methods.
        assert_eq!(board.drill_first_layer(id), Some(0), "firstLayer=0");
        assert_eq!(board.drill_last_layer(id), Some(1), "lastLayer=1");
        assert_eq!(
            board.drill_min_width(id),
            Some(15_240.0),
            "minWidth=15240.0 — through Board::pin_shape per layer"
        );

        // The center (capture: center=1560830 -1010920) — pinned
        // anchor-blind too: an untranslated component location or a
        // mirrored rel location misses.
        match board.pin_center(id).expect("center resolves") {
            epic_geometry::point::Point::Int(center) => {
                assert_eq!((center.x, center.y), (1_560_830, -1_010_920));
            }
            other => panic!("expected an integer center, got {other:?}"),
        }

        // The shapes exist on both layers of the span and are tiles
        // (an oval pad converts through the convex reader); OOB shape
        // indices yield None (Java would array-bound-throw).
        for index in 0..2 {
            assert!(
                board.pin_shape(id, index).is_some(),
                "shape {index} over the 0..1 span"
            );
        }
        assert_eq!(board.pin_shape(id, 2), None, "past the padstack span");
        assert_eq!(board.pin_shape(id, -1), None, "negative index");

        // The FIRST pin on the descending walk is the HIGHEST id —
        // the capture's id 41 (the pins were inserted after the
        // outline id 1; `DRILL kind=pin id=40` follows).
        assert_eq!(first_pin_id(&board).get(), 41, "DRILLS pins=40, top id 41");
        // And a second pin from the capture (id 40, same padstack):
        let id40 = ItemId::new(40);
        let center40 = board.pin_center(id40).expect("pin 40 resolves");
        match center40 {
            epic_geometry::point::Point::Int(center) => {
                assert_eq!(
                    (center.x, center.y),
                    (1_560_830, -1_036_320),
                    "DRILL kind=pin id=40 center=1560830 -1036320"
                );
            }
            other => panic!("expected an integer center, got {other:?}"),
        }
        assert_eq!(board.drill_min_width(id40), Some(15_240.0), "same padstack");

        // A non-pin id (the outline) resolves to None on every pin
        // surface — kind discrimination, not a panic.
        let outline = ItemId::new(1);
        assert_eq!(board.pin_shape(outline, 0), None);
        assert_eq!(board.pin_center(outline), None);
        assert_eq!(board.drill_first_layer(outline), None);
        assert_eq!(board.drill_min_width(outline), None);
    }
}
