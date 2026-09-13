//! Port of `io.specctra.parser.Layer` / `LayerStructure`: the layer table
//! read from a DSN structure scope.
//!
//! T34 lives in [`LayerStructure::get_no`]: after an exact name scan fails,
//! the Electra fallback classifies the *query* name (not the layer names) —
//! any query containing `"Top"` resolves to layer 0, any containing
//! `"Bottom"` to the last layer, case-sensitively
//! (`LayerStructure.java:31-45`). Jar session `/tmp/epic-layers.jsh`, output
//! `/tmp/epic-layers.out` (2026-09-12): on layers `[TopLayer, BottomLayer,
//! Top]`, `getNo("Top")` is 2 (exact match beats the substring fallback that
//! would answer 0) while `getNo("Bottom")` is 2 (no exact layer, query
//! contains "Bottom" -> `length-1`); on an EMPTY structure `getNo("Top")` is
//! still 0 and `getNo("Bottom")` is -1 (`length-1` underflow).
//!
//! The second Java constructor (`LayerStructure(board.LayerStructure)`,
//! `:22-28`) mirrors a board-side layer table and lands with epic-board (M2).

/// Java `parser/Layer.java` (`:8-45`). `no` is the physical layer number
/// starting with 0 at the component side; the shared placeholders
/// [`Layer::PCB`] and [`Layer::SIGNAL`] use -1. `net_names` is populated for
/// power planes (`(use_net ...)` in the layer scope).
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    /// Java `name`.
    pub name: String,
    /// Java `no`.
    pub no: i32,
    /// Java `isSignal`: true for signal layers, false for power/ground
    /// layers.
    pub is_signal: bool,
    /// Java `netNames`.
    pub net_names: Vec<String>,
}

impl Layer {
    /// The boundary/outline pseudo-layer `Layer.PCB` (`Layer.java:11`).
    pub const PCB_NAME: &'static str = "pcb";
    /// The all-signal-layers pseudo-layer `Layer.SIGNAL` (`Layer.java:14`).
    pub const SIGNAL_NAME: &'static str = "signal";

    /// Java 3-arg constructor (`Layer.java:40-45`): empty net list.
    pub fn new(name: &str, no: i32, is_signal: bool) -> Self {
        Self {
            name: name.to_string(),
            no,
            is_signal,
            net_names: Vec::new(),
        }
    }

    /// Java 4-arg constructor (`Layer.java:27-32`): with plane net names.
    pub fn with_net_names(name: &str, no: i32, is_signal: bool, net_names: Vec<String>) -> Self {
        Self {
            name: name.to_string(),
            no,
            is_signal,
            net_names,
        }
    }

    /// Java `Layer.PCB` static constant (`Layer.java:11`): the
    /// boundary/outline pseudo-layer (name "pcb", `no` -1, not a signal
    /// layer).
    ///
    /// Java compares shape layers by IDENTITY (`==` against the two static
    /// constants — `Shape.java:82-84`, `Structure.java:314-320`,
    /// `Library.java:208`), so only the shared constants can ever match. Rust
    /// has no object identity: value [`PartialEq`] stands in for it, and the
    /// port relies on every use site building the pseudo-layers through these
    /// canonical constructors (jar session `/tmp/epic-t3-part-a.jsh`, output
    /// `/tmp/epic-t3-part-a.out`: "LAYER pcb name=pcb no=-1 isSignal=false
    /// netNames=0"). Never hand-roll `Layer::new("pcb", ..)`.
    pub fn pcb() -> Self {
        Layer::new(Layer::PCB_NAME, -1, false)
    }

    /// Java `Layer.SIGNAL` static constant (`Layer.java:14`): the
    /// all-signal-layers pseudo-layer (name "signal", `no` -1) — see
    /// [`Layer::pcb`] for the identity-vs-value rationale (jar, same session:
    /// "LAYER signal name=signal no=-1 isSignal=true netNames=0").
    pub fn signal() -> Self {
        Layer::new(Layer::SIGNAL_NAME, -1, true)
    }
}

/// Java `parser/LayerStructure.java` (`:8-69`): the ordered layer table.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerStructure {
    /// Java `layers` (public final array).
    pub layers: Vec<Layer>,
}

impl LayerStructure {
    /// Java `LayerStructure(Collection<Layer>)` (`:13-19`): keeps the
    /// collection order (file order of the `(layer ...)` scopes).
    pub fn new(layers: Vec<Layer>) -> Self {
        Self { layers }
    }

    /// Java `getNo` (`:31-45`): number of the named layer, or -1.
    ///
    /// 1. exact (case-sensitive) name match wins, first in file order;
    /// 2. else the Electra outline fallback on the QUERY name: containing
    ///    "Top" -> 0, containing "Bottom" -> last layer index
    ///    (`layers.len() as i32 - 1`, which is -1 on an empty table);
    /// 3. else -1.
    pub fn get_no(&self, name: &str) -> i32 {
        for (index, layer) in self.layers.iter().enumerate() {
            if name == layer.name {
                return index as i32;
            }
        }
        // check for special layers of the Electra autorouter used for the
        // outline (LayerStructure.java:37-44) — the QUERY name is classified
        if name.contains("Top") {
            return 0;
        }
        if name.contains("Bottom") {
            // signed arithmetic: an empty table yields -1, not usize underflow
            return self.layers.len() as i32 - 1;
        }
        -1
    }

    /// Java `signalLayerCount` (`:47-55`).
    pub fn signal_layer_count(&self) -> i32 {
        self.layers.iter().filter(|layer| layer.is_signal).count() as i32
    }

    /// Java `containsPlane` (`:57-68`): true if a NON-signal layer lists
    /// `net_name` among its net names (case-sensitive).
    pub fn contains_plane(&self, net_name: &str) -> bool {
        self.layers
            .iter()
            .any(|layer| !layer.is_signal && layer.net_names.iter().any(|name| name == net_name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Jar session `/tmp/epic-layers.jsh`, output `/tmp/epic-layers.out`,
    /// layers [Top, Inner1, Bottom] ("T34a" lines).
    fn three_layers() -> LayerStructure {
        LayerStructure::new(vec![
            Layer::new("Top", 0, true),
            Layer::new("Inner1", 1, true),
            Layer::new("Bottom", 2, true),
        ])
    }

    /// Canonical pseudo-layer singletons (jar session
    /// `/tmp/epic-t3-part-a.jsh`, output `/tmp/epic-t3-part-a.out`,
    /// 2026-09-12): "LAYER pcb name=pcb no=-1 isSignal=false netNames=0",
    /// "LAYER signal name=signal no=-1 isSignal=true netNames=0", "LAYER
    /// identity distinct=true" — the Java constants are distinct objects
    /// with no=-1 and empty net-name lists. All shape/structure use sites
    /// must build them via these constructors so value equality stands in
    /// for Java's `==` identity compares (`Shape.java:82-84` et al.).
    #[test]
    fn canonical_pseudo_layer_singletons() {
        assert_eq!(
            Layer::pcb(),
            Layer {
                name: "pcb".to_string(),
                no: -1,
                is_signal: false,
                net_names: Vec::new()
            }
        );
        assert_eq!(
            Layer::signal(),
            Layer {
                name: "signal".to_string(),
                no: -1,
                is_signal: true,
                net_names: Vec::new()
            }
        );
        assert_ne!(Layer::pcb(), Layer::signal());
    }

    /// T34a: exact matches hit their file-order index; "T34a
    /// getNo(Inner1)=1".
    #[test]
    fn exact_match_wins_in_file_order() {
        let layers = three_layers();
        assert_eq!(layers.get_no("Top"), 0);
        assert_eq!(layers.get_no("Inner1"), 1);
        assert_eq!(layers.get_no("Bottom"), 2);
    }

    /// T34a: "getNo(TopLayer)=0" (query contains "Top"), "getNo(bottom
    /// copper)=-1" and "getNo(TOP)=-1" and "getNo(copper top)=-1" (the
    /// fallback is CASE-SENSITIVE on the query), "getNo(TopBottom)=0" (the
    /// "Top" check runs before "Bottom"), "getNo(missing)=-1".
    #[test]
    fn electra_fallback_is_case_sensitive_on_the_query() {
        let layers = three_layers();
        assert_eq!(layers.get_no("TopLayer"), 0);
        assert_eq!(layers.get_no("bottom copper"), -1);
        assert_eq!(layers.get_no("TOP"), -1);
        assert_eq!(layers.get_no("copper top"), -1);
        assert_eq!(layers.get_no("TopBottom"), 0);
        assert_eq!(layers.get_no("missing"), -1);
    }

    /// T34b on layers [TopLayer, BottomLayer, Top]: "getNo(Top)=2" — the
    /// exact match at index 2 beats the substring fallback (which would
    /// answer 0); "getNo(TopLayer)=0" exact; "getNo(Bottom)=2" — no exact
    /// "Bottom" layer, query contains "Bottom" -> length-1 = 2;
    /// "getNo(BottomLayer)=1" exact.
    #[test]
    fn exact_beats_substring_fallback() {
        let layers = LayerStructure::new(vec![
            Layer::new("TopLayer", 0, true),
            Layer::new("BottomLayer", 1, true),
            Layer::new("Top", 2, true),
        ]);
        assert_eq!(layers.get_no("Top"), 2);
        assert_eq!(layers.get_no("TopLayer"), 0);
        assert_eq!(layers.get_no("Bottom"), 2);
        assert_eq!(layers.get_no("BottomLayer"), 1);
    }

    /// T34c on lowercase layers [top, bottom]: exact hits "getNo(top)=0" /
    /// "getNo(bottom)=1"; capitalized queries take the fallback
    /// ("getNo(Top)=0", "getNo(Bottom)=1"); "getNo(xTop)=0".
    #[test]
    fn lowercase_layers_still_fall_back() {
        let layers = LayerStructure::new(vec![
            Layer::new("top", 0, true),
            Layer::new("bottom", 1, true),
        ]);
        assert_eq!(layers.get_no("top"), 0);
        assert_eq!(layers.get_no("Top"), 0);
        assert_eq!(layers.get_no("bottom"), 1);
        assert_eq!(layers.get_no("Bottom"), 1);
        assert_eq!(layers.get_no("xTop"), 0);
    }

    /// T34d on an EMPTY table: "getNo(Top)=0" (the fallback runs with no
    /// layers at all), "getNo(Bottom)=-1" (length-1 underflows to -1 — the
    /// Rust port must use signed arithmetic, not usize wraparound),
    /// "getNo(missing)=-1".
    #[test]
    fn empty_table_fallback_quirk() {
        let layers = LayerStructure::new(Vec::new());
        assert_eq!(layers.get_no("Top"), 0);
        assert_eq!(layers.get_no("Bottom"), -1);
        assert_eq!(layers.get_no("missing"), -1);
    }

    /// T34a "signalLayerCount=3" for three signal layers.
    #[test]
    fn signal_layer_count_all_signal() {
        assert_eq!(three_layers().signal_layer_count(), 3);
    }

    /// T34f "signalLayerCount=2" when a power layer is present (T34e setup
    /// counts 2 signal + 1 power).
    #[test]
    fn signal_layer_count_skips_power() {
        let layers = LayerStructure::new(vec![
            Layer::new("Top", 0, true),
            Layer::with_net_names(
                "GND",
                1,
                false,
                vec!["VCC".to_string(), "GND_NET".to_string()],
            ),
            Layer::new("Bottom", 2, true),
        ]);
        assert_eq!(layers.signal_layer_count(), 2);
    }

    /// T34e/T34f: "containsPlane(VCC)=true", "containsPlane(GND_NET)=true",
    /// "containsPlane(vcc)=false" (case-sensitive), "containsPlane(Top)=false"
    /// (the signal layer's name/net list is never consulted);
    /// all-signal layers with net names -> "containsPlane(VCC) all-signal=
    /// false" (only NON-signal layers can contain a plane).
    #[test]
    fn contains_plane_checks_only_power_layers() {
        let mixed = LayerStructure::new(vec![
            Layer::new("Top", 0, true),
            Layer::with_net_names(
                "GND",
                1,
                false,
                vec!["VCC".to_string(), "GND_NET".to_string()],
            ),
            Layer::new("Bottom", 2, true),
        ]);
        assert!(mixed.contains_plane("VCC"));
        assert!(mixed.contains_plane("GND_NET"));
        assert!(!mixed.contains_plane("vcc"));
        assert!(!mixed.contains_plane("Top"));

        let all_signal = LayerStructure::new(vec![
            Layer::with_net_names("Top", 0, true, vec!["VCC".to_string()]),
            Layer::new("Bottom", 1, true),
        ]);
        assert!(!all_signal.contains_plane("VCC"));
    }
}
