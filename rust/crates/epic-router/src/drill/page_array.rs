//! Java `autoroute/drill/DrillPageArray.java` (121 lines) — the grid
//! of drill pages over the board bounds.
//!
//! Deviation: Java `emitDiagnostics` (`:110-120`) is omitted — the
//! Rust engine has no diagnostic sink yet.

use epic_geometry::int_box::IntBox;
use epic_geometry::int_point::IntPoint;
use epic_geometry::tile_shape::TileShape;

use super::DrillEngine;
use super::drill_page::DrillPage;
use super::expansion_drill::ExpansionDrill;

/// Java `AutorouteEngine.java:89-91` — the page-width formula LIVES AT
/// THE CONSTRUCTION SITE in Java (`maxDrillPageWidth = max((int)(5 *
/// board.rules.getDefaultViaDiameter()), 10000)`); ported next to the
/// array as its contract. The `(int)` cast TRUNCATES the double
/// product toward zero FIRST, then the max clamps to 10000 — Rust's
/// `as i32` saturates exactly like the Java cast for the
/// degenerate/infinite inputs (NaN -> 0).
#[must_use]
pub fn max_drill_page_width(default_via_diameter: f64) -> i32 {
    let as_int = (5.0 * default_via_diameter) as i32;
    as_int.max(10_000)
}

/// Java `DrillPageArray` — the page grid (`pages[j][i]`, row-major
/// like the Java 2-D array).
pub struct DrillPageArray {
    /// Java `bounds` — the grid frame (`board.boundingBox`).
    pub bounds: IntBox,
    /// Java `columnCount`.
    pub column_count: i32,
    /// Java `rowCount`.
    pub row_count: i32,
    /// Java `pageWidth`.
    pub page_width: i32,
    /// Java `pageHeight`.
    pub page_height: i32,
    /// Java `pages` — `pages[j][i]`, j the row, i the column.
    pub pages: Vec<Vec<DrillPage>>,
}

impl Default for DrillPageArray {
    /// The EMPTY grid — the construction placeholder for the engine's
    /// `mem::take` search window: `AutorouteEngine` owns the array, but
    /// the maze engine needs it mutably while the engine itself is the
    /// `DrillEngine` context, so the array swaps out for the search and
    /// is restored after. Nothing reads the engine's copy mid-search
    /// (the maze code goes through its own field), so the placeholder
    /// is never indexed; the real array is built in the engine ctor
    /// right after `Self::new` completes.
    fn default() -> Self {
        Self {
            bounds: IntBox::new(IntPoint::new(0, 0), IntPoint::new(0, 0)),
            column_count: 0,
            row_count: 0,
            page_width: 0,
            page_height: 0,
            pages: Vec::new(),
        }
    }
}

impl DrillPageArray {
    /// Java ctor (`:34-66`): `columnCount = ceil(length/maxPageWidth)`
    /// over doubles, then `pageWidth = ceil(length/columnCount)` —
    /// the SECOND division redistributes the remainder; the LAST
    /// column/row is snapped to `bounds.ur` (`:47-58`) so the grid
    /// always covers the board exactly. Layer arithmetic wraps like
    /// Java int overflow does (never reached at real board sizes).
    pub fn new(ctx: &impl DrillEngine, max_page_width: i32) -> Self {
        let bounds = ctx.board_bounds();
        let length = f64::from(bounds.ur.x.wrapping_sub(bounds.ll.x));
        let height = f64::from(bounds.ur.y.wrapping_sub(bounds.ll.y));
        let column_count = (length / f64::from(max_page_width)).ceil() as i32;
        let row_count = (height / f64::from(max_page_width)).ceil() as i32;
        let page_width = (length / f64::from(column_count)).ceil() as i32;
        let page_height = (height / f64::from(row_count)).ceil() as i32;
        let layer_count = ctx.layer_count();
        let mut pages = Vec::with_capacity(row_count.max(0) as usize);
        for j in 0..row_count {
            let mut row = Vec::with_capacity(column_count.max(0) as usize);
            for i in 0..column_count {
                let ll_x = bounds.ll.x.wrapping_add(i.wrapping_mul(page_width));
                let ur_x = if i == column_count - 1 {
                    bounds.ur.x
                } else {
                    ll_x.wrapping_add(page_width)
                };
                let ll_y = bounds.ll.y.wrapping_add(j.wrapping_mul(page_height));
                let ur_y = if j == row_count - 1 {
                    bounds.ur.y
                } else {
                    ll_y.wrapping_add(page_height)
                };
                row.push(DrillPage::new(
                    IntBox::new(IntPoint::new(ll_x, ll_y), IntPoint::new(ur_x, ur_y)),
                    layer_count,
                ));
            }
            pages.push(row);
        }
        Self {
            bounds,
            column_count,
            row_count,
            page_width,
            page_height,
            pages,
        }
    }

    /// Java `overlappingPages(shape)` (`:76-97`) — the (row, column)
    /// coordinates of all pages with a 2-dimensional overlap, in the
    /// Java scan order (j outer, i inner). The loop bounds mirror the
    /// Java double arithmetic EXACTLY: `minJ = floor(...)` is a cast,
    /// `maxJ`/`maxI` stay DOUBLES and the loop test is the strict
    /// `j < maxJ` — a probe shape whose box edge lands exactly on a
    /// page boundary produces `maxJ` integral and stops short of that
    /// boundary row (`ceil` would over-scan one page and index out of
    /// the grid when the shape touches `bounds.ur`).
    #[must_use]
    pub fn overlapping_pages(&self, shape: &TileShape) -> Vec<(i32, i32)> {
        let shape_box = shape.bounding_box().intersection(&self.bounds);
        let min_j = (f64::from(shape_box.ll.y.wrapping_sub(self.bounds.ll.y))
            / f64::from(self.page_height))
        .floor() as i32;
        let max_j =
            f64::from(shape_box.ur.y.wrapping_sub(self.bounds.ll.y)) / f64::from(self.page_height);
        let min_i = (f64::from(shape_box.ll.x.wrapping_sub(self.bounds.ll.x))
            / f64::from(self.page_width))
        .floor() as i32;
        let max_i =
            f64::from(shape_box.ur.x.wrapping_sub(self.bounds.ll.x)) / f64::from(self.page_width);
        let mut result = Vec::new();
        let mut j = min_j;
        while f64::from(j) < max_j {
            let mut i = min_i;
            while f64::from(i) < max_i {
                let page = &self.pages[j as usize][i as usize];
                if shape.intersection(&page.shape_tile()).dimension() > 1 {
                    result.push((j, i));
                }
                i += 1;
            }
            j += 1;
        }
        result
    }

    /// Java `invalidate(shape)` (`:68-73`) — drops the memoized drills
    /// of every overlapping page.
    pub fn invalidate(&mut self, shape: &TileShape) {
        for (j, i) in self.overlapping_pages(shape) {
            self.pages[j as usize][i as usize].invalidate();
        }
    }

    /// Java `reset()` (`:100-107`).
    pub fn reset(&mut self) {
        for row in &mut self.pages {
            for page in row {
                page.reset();
            }
        }
    }

    // ---- T6 accessors (the maze engine resolves doors to grid cells) ----

    /// The page at `(row, column)` — Java holds the object reference.
    #[must_use]
    pub fn page(&self, row: i32, column: i32) -> &DrillPage {
        &self.pages[row as usize][column as usize]
    }

    /// The mutable page at `(row, column)`.
    pub fn page_mut(&mut self, row: i32, column: i32) -> &mut DrillPage {
        &mut self.pages[row as usize][column as usize]
    }

    /// Java `page.getShape()`.
    #[must_use]
    pub fn page_shape(&self, row: i32, column: i32) -> TileShape {
        self.page(row, column).shape_tile()
    }

    /// The memoized drills of the page — Java's live
    /// `page.drills` list (never read before `get_drills` populated
    /// it: the maze engine only reaches drills through
    /// [`DrillPage::get_drills`]).
    #[must_use]
    pub fn page_drills(&self, row: i32, column: i32) -> &[ExpansionDrill] {
        self.page(row, column)
            .drills()
            .expect("drills are memoized before the maze engine reads them")
    }

    /// The `d`-th drill of the page.
    #[must_use]
    pub fn page_drill(&self, row: i32, column: i32, d: usize) -> &ExpansionDrill {
        &self.page_drills(row, column)[d]
    }

    /// The mutable `d`-th drill of the page.
    pub fn page_drill_mut(&mut self, row: i32, column: i32, d: usize) -> &mut ExpansionDrill {
        &mut self.pages[row as usize][column as usize]
            .drills_mut()
            .expect("drills are memoized before the maze engine reads them")[d]
    }

    /// Java `drill.getShape()` for the `d`-th drill.
    #[must_use]
    pub fn page_drill_shape(&self, row: i32, column: i32, d: usize) -> TileShape {
        self.page_drill(row, column, d).get_shape().clone()
    }
}
