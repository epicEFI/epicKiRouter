//! The global plan (M6-T7) — per-net guides and the congestion-aware
//! net order (design §4.2 stage 2: per-net plans "regions/layers/
//! expected topology"; detail honoring starts at T8 — the T7 face is
//! the map, the guides, the planned ORDER, and pattern routing).
//!
//! All outputs are pure functions of board state (the T6 digest
//! discipline): fixed iteration orders, BTreeMap only, integer
//! arithmetic, no HashMap anywhere.

use std::collections::BTreeMap;

use epic_board::board::Board;
use epic_geometry::int_box::IntBox;
use epic_geometry::int_point::IntPoint;
use epic_geometry::point::Point;
use sha2::{Digest as _, Sha256};

use super::map::{CongestionMap, widen};

/// The expected topology class of a net's guide (design :70).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetTopology {
    /// Two terminals sharing one signal layer — the pattern-routing
    /// candidate (the cheap-net fast path).
    SameLayerPair,
    /// Two terminals on different signal layers (via required).
    LayerPair,
    /// Three or more terminals (the maze's territory).
    MultiTerminal,
    /// The net carries a plane (`contains_plane`) — stub+via routing,
    /// never pattern-routed.
    Plane,
    /// Fewer than two connectable items (nothing to plan).
    Open,
}

/// The per-net plan (design :70): the guide region, the preferred
/// signal layers, and the expected topology class.
#[derive(Clone, Debug)]
pub struct NetGuide {
    /// The net number (1-based).
    net_no: i32,
    /// The guide region: the terminal centers' bbox expanded by one
    /// cell side per side (saturating).
    region: IntBox,
    /// The guide region in cell units (inclusive ix0..=ix1, iy0..=iy1).
    region_cells: (usize, usize, usize, usize),
    /// The preferred signal layers, LEAST congested first (ascending
    /// total overflow, tie = ascending ordinal).
    layers: Vec<usize>,
    /// The expected topology class.
    topology: NetTopology,
}

impl NetGuide {
    /// The net number.
    #[must_use]
    pub fn net_no(&self) -> i32 {
        self.net_no
    }

    /// The guide region.
    #[must_use]
    pub fn region(&self) -> IntBox {
        self.region
    }

    /// The preferred signal layers (least congested first).
    #[must_use]
    pub fn layers(&self) -> &[usize] {
        &self.layers
    }

    /// The expected topology class.
    #[must_use]
    pub fn topology(&self) -> NetTopology {
        self.topology
    }

    /// The region's cell range (ix0, ix1, iy0, iy1) — the pin/face
    /// read for the pin bank.
    #[must_use]
    pub fn region_cells_debug(&self) -> (usize, usize, usize, usize) {
        self.region_cells
    }

    /// The guide's congestion severity: the sum of the cell overflow
    /// over the region's cells, all layers, no exclusion (the planning
    /// priority weight — a pure function of board state).
    fn severity(&self, map: &CongestionMap) -> u64 {
        let (ix0, ix1, iy0, iy1) = self.region_cells;
        let mut sum = 0u64;
        for layer in 0..map.total_overflow().len() {
            for iy in iy0..=iy1 {
                for ix in ix0..=ix1 {
                    sum += map.overflow(ix, iy, layer, None).max(0) as u64;
                }
            }
        }
        sum
    }
}

/// The global plan: the map, the per-net guides, and the planned net
/// order.
#[derive(Debug, Clone)]
pub struct GlobalPlan {
    map: CongestionMap,
    guides: Vec<NetGuide>,
    /// The planned order: net numbers, most congested guide first
    /// (descending severity, tie = ascending net number). Nets absent
    /// (0/1-terminal nets) route LAST, in ascending net order.
    order: Vec<i32>,
    /// net number -> planned rank (0 = first).
    rank: BTreeMap<i32, usize>,
}

impl GlobalPlan {
    /// Builds the global plan over the board (pure; module docs).
    #[must_use]
    pub fn build(board: &mut Board) -> Self {
        let map = CongestionMap::build(board);
        let overflow_per_layer: Vec<u64> = map.total_overflow().to_vec();
        let cell = map.cell_size();

        // The guides: one per net with two or more connectable items.
        let mut guides: Vec<NetGuide> = Vec::new();
        let max_net = board.rules().nets.max_net_number();
        for net_no in 1..=max_net {
            let items = board.get_connectable_items(net_no);
            if items.len() < 2 {
                continue;
            }
            let topology = if board
                .rules()
                .nets
                .get(net_no)
                .is_some_and(|net| net.contains_plane)
            {
                NetTopology::Plane
            } else if items.len() > 2 {
                NetTopology::MultiTerminal
            } else {
                // Two terminals: same-layer pair iff both single-layer
                // drill items on the SAME signal layer.
                let (a, b) = (items[0], items[1]);
                let mut span = |id: epic_board::id::ItemId| -> Option<(i32, i32)> {
                    let first = board.drill_first_layer(id)?;
                    let last = board.drill_last_layer(id)?;
                    Some((first, last))
                };
                match (span(a), span(b)) {
                    (Some((af, al)), Some((bf, bl))) if af == al && bf == bl && af == bf => {
                        NetTopology::SameLayerPair
                    }
                    _ => NetTopology::LayerPair,
                }
            };

            // The region: terminal centers' bbox + one cell per side.
            let mut region = IntBox {
                ll: IntPoint {
                    x: i32::MAX,
                    y: i32::MAX,
                },
                ur: IntPoint {
                    x: i32::MIN,
                    y: i32::MIN,
                },
            };
            for item in &items {
                if let Some(Point::Int(center)) = board.drill_center(*item) {
                    region.ll.x = region.ll.x.min(center.x);
                    region.ll.y = region.ll.y.min(center.y);
                    region.ur.x = region.ur.x.max(center.x);
                    region.ur.y = region.ur.y.max(center.y);
                }
            }
            let region = widen(&region, cell as i32);
            let region_cells = map.cell_range_of(&region);

            // The preferred layers: ascending total overflow, tie =
            // ascending ordinal.
            let mut layers: Vec<usize> = (0..overflow_per_layer.len()).collect();
            layers.sort_by_key(|&layer| (overflow_per_layer[layer], layer));
            guides.push(NetGuide {
                net_no,
                region,
                region_cells,
                layers,
                topology,
            });
        }

        // The planned order: descending severity, tie ascending net
        // number.
        let mut severity_of: BTreeMap<i32, u64> = BTreeMap::new();
        for guide in &guides {
            severity_of.insert(guide.net_no, guide.severity(&map));
        }
        let mut order: Vec<i32> = guides.iter().map(|guide| guide.net_no).collect();
        order.sort_by_key(|&net_no| {
            (
                std::cmp::Reverse(severity_of.get(&net_no).copied().unwrap_or(0)),
                net_no,
            )
        });

        // The rank map.
        let mut rank = BTreeMap::new();
        for (position, net_no) in order.iter().enumerate() {
            rank.insert(*net_no, position);
        }

        Self {
            map,
            guides,
            order,
            rank,
        }
    }

    /// The map.
    #[must_use]
    pub fn map(&self) -> &CongestionMap {
        &self.map
    }

    /// The guides (ascending net number — the build walk's order).
    #[must_use]
    pub fn guides(&self) -> &[NetGuide] {
        &self.guides
    }

    /// The planned order (most congested first).
    #[must_use]
    pub fn net_order(&self) -> &[i32] {
        &self.order
    }

    /// The planned rank of a net (None = routes last).
    #[must_use]
    pub fn rank_of(&self, net_no: i32) -> Option<usize> {
        self.rank.get(&net_no).copied()
    }

    /// The order digest: SHA-256 over the order vector's canonical
    /// rows `r<rank> net<net_no>` (the deterministic-order face).
    #[must_use]
    pub fn order_digest(&self) -> String {
        let mut hasher = Sha256::new();
        for (position, net_no) in self.order.iter().enumerate() {
            let row = format!("r{position} net{net_no}\n");
            hasher.update(row.as_bytes());
        }
        format!("{:x}", hasher.finalize())
    }
}
