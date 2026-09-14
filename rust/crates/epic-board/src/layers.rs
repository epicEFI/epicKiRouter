//! The board-side layer structure — a port of
//! `board/model/structure/LayerStructure.java` (all 82 lines) and
//! `board/model/structure/Layer.java`.
//!
//! ## Board-side vs parser-side (do not conflate)
//!
//! Java has TWO `LayerStructure` classes. The PARSER one
//! (`io.specctra.parser.LayerStructure`, ported in
//! `epic_dsn::layer_structure`) resolves layer NAMES during the read and
//! owns the Electra outline fallback (a query containing "Top" resolves to
//! layer 0 — T34, jar-pinned there). THIS module is the BOARD-side query
//! surface: `getNo(name)` is an EXACT, case-SENSITIVE `equals` scan with
//! NO fallback (`LayerStructure.java:19-26`) — by the time the board
//! exists, every name has already been resolved at parse time.
//!
//! What MUST stay identical between the two are the SIGNAL-RENUMBERING
//! semantics (`signalLayerCount` / `getSignalLayerNo` /
//! `getSignalLayer`), which the parser-side port also carries
//! (`epic_dsn::layer_structure::LayerStructure::signal_layer_count` —
//! same count-of-`isSignal` walk). This module adds the board-only
//! `get_signal_layer` / `get_signal_layer_no` / `get_layer_no` trio with
//! Java's exact quirks (below).
//!
//! The board-side `Layer` (`Layer.java`) carries ONLY `name` + `isSignal`
//! — the parser-side `net_names` plane list does not exist board-side
//! (plane membership lives on the net, `rules.Net.containsPlane`).

/// Java `board.model.structure.Layer` (`Layer.java:8-26`): `name` +
/// `isSignal` only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layer {
    /// Java `name`.
    pub name: String,
    /// Java `isSignal` — true for routing layers; power/ground layers
    /// are false.
    pub is_signal: bool,
}

impl Layer {
    /// Java `Layer(String, boolean)`.
    #[must_use]
    pub fn new(name: &str, is_signal: bool) -> Self {
        Self {
            name: name.to_string(),
            is_signal,
        }
    }
}

/// Java `board.model.structure.LayerStructure` (`LayerStructure.java`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayerStructure {
    /// Java `layers` (public final array, physical order: index 0 is the
    /// component side).
    pub layers: Vec<Layer>,
}

impl LayerStructure {
    /// Java `LayerStructure(Layer[])`.
    #[must_use]
    pub fn new(layers: Vec<Layer>) -> Self {
        Self { layers }
    }

    /// Java `getNo(String)` (`:19-26`): the index of the named layer,
    /// EXACT case-sensitive compare, first match, -1 on a miss. (No
    /// Electra fallback — that is parser-side only; module docs.)
    #[must_use]
    pub fn get_no(&self, name: &str) -> i32 {
        for (index, layer) in self.layers.iter().enumerate() {
            if name == layer.name {
                return index as i32;
            }
        }
        -1
    }

    /// Java `signalLayerCount()` (`:39-47`): count of `isSignal` layers.
    /// Same walk as `epic_dsn::layer_structure` (jar-pinned there, T34f).
    #[must_use]
    pub fn signal_layer_count(&self) -> i32 {
        self.layers.iter().filter(|layer| layer.is_signal).count() as i32
    }

    /// Java `getSignalLayer(int no)` (`:50-61`): the `no`-th (0-based)
    /// signal layer in physical order. QUIRK: an out-of-range `no`
    /// returns the LAST layer — Java's `return layers[layers.length - 1]`
    /// tail (`:60`) — which also makes an EMPTY structure panic there;
    /// this port returns `None` for the empty case (the board guarantees
    /// at least one layer) and keeps the last-layer fallback for the
    /// non-empty out-of-range case, mirroring the reachable behavior.
    #[must_use]
    pub fn get_signal_layer(&self, no: i32) -> Option<&Layer> {
        let mut found = 0;
        for layer in &self.layers {
            if layer.is_signal {
                if no == found {
                    return Some(layer);
                }
                found += 1;
            }
        }
        self.layers.last()
    }

    /// Java `getSignalLayerNo(Layer)` (`:64-75`): the number of signal
    /// layers with a SMALLER physical index than the given layer. The
    /// Java signature compares by object identity; the port takes the
    /// layer's physical INDEX (the identity a board-side caller holds).
    /// -1 when the index is out of range (Java's loop-miss return).
    #[must_use]
    pub fn get_signal_layer_no(&self, layer_index: usize) -> i32 {
        let mut found = 0;
        for (index, layer) in self.layers.iter().enumerate() {
            if index == layer_index {
                return found;
            }
            if layer.is_signal {
                found += 1;
            }
        }
        -1
    }

    /// Java `getLayerNo(int signalLayerNo)` (`:78-81`): the physical
    /// index of the `signalLayerNo`-th signal layer. Inherits
    /// [`LayerStructure::get_signal_layer`]'s last-layer fallback, so an
    /// out-of-range signal number yields the LAST layer's index (Java
    /// `getNo` of `layers[len-1]` = `len-1`; an empty structure yields
    /// Java's `getNo` miss = -1 here).
    #[must_use]
    pub fn get_layer_no(&self, signal_layer_no: i32) -> i32 {
        match self.get_signal_layer(signal_layer_no) {
            Some(layer) => self.get_no(&layer.name),
            None => -1,
        }
    }

    /// The conversion from the parse-time IR (`SesBoard.layers`): a
    /// board-side layer keeps only name + isSignal (module docs — the
    /// parser-side plane `net_names` are not board-side state).
    #[must_use]
    pub fn from_ir(layers: &epic_dsn::layer_structure::LayerStructure) -> Self {
        Self {
            layers: layers
                .layers
                .iter()
                .map(|layer| Layer::new(&layer.name, layer.is_signal))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two signal layers + one power layer (the board-side analogue of
    /// the epic-dsn T34 test table).
    fn mixed() -> LayerStructure {
        LayerStructure::new(vec![
            Layer::new("F.Cu", true),
            Layer::new("GND", false),
            Layer::new("B.Cu", true),
        ])
    }

    /// `LayerStructure.java:19-26`: EXACT case-sensitive name lookup —
    /// no Electra fallback (a "TopLayer"-style query that the PARSER-side
    /// structure answers 0 must MISS here). This is the discrimination
    /// between the two Java classes: pinning `get_no("TopLayer") == -1`
    /// fails any port that copied the parser fallback in.
    #[test]
    fn get_no_is_exact_with_no_electra_fallback() {
        let layers = mixed();
        assert_eq!(layers.get_no("F.Cu"), 0);
        assert_eq!(layers.get_no("GND"), 1);
        assert_eq!(layers.get_no("B.Cu"), 2);
        assert_eq!(layers.get_no("f.cu"), -1, "case-sensitive");
        assert_eq!(layers.get_no("TopLayer"), -1, "no parser-side fallback");
        assert_eq!(layers.get_no("missing"), -1);
    }

    /// `:39-47` `signalLayerCount` = 2 (matches the epic-dsn
    /// `signal_layer_count` semantics — the one behavior the two classes
    /// genuinely share).
    #[test]
    fn signal_layer_count_skips_power_layers() {
        assert_eq!(mixed().signal_layer_count(), 2);
        assert_eq!(
            LayerStructure::new(Vec::new()).signal_layer_count(),
            0,
            "empty"
        );
    }

    /// `:50-61` `getSignalLayer(no)`: signal layers in physical order —
    /// getSignalLayer(0)=F.Cu, (1)=B.Cu, and the QUIRK: out-of-range
    /// returns the LAST layer (Java `layers[length-1]` tail), so (2) is
    /// B.Cu and (-1) is B.Cu too, NOT an error.
    #[test]
    fn get_signal_layer_walks_signals_and_falls_back_to_last() {
        let layers = mixed();
        assert_eq!(
            layers.get_signal_layer(0).map(|l| l.name.as_str()),
            Some("F.Cu")
        );
        assert_eq!(
            layers.get_signal_layer(1).map(|l| l.name.as_str()),
            Some("B.Cu")
        );
        assert_eq!(
            layers.get_signal_layer(2).map(|l| l.name.as_str()),
            Some("B.Cu"),
            "out-of-range -> LAST layer (the Java tail)"
        );
        assert_eq!(
            layers.get_signal_layer(-1).map(|l| l.name.as_str()),
            Some("B.Cu"),
            "negative -> LAST layer as well"
        );
        assert!(
            LayerStructure::new(Vec::new())
                .get_signal_layer(0)
                .is_none()
        );
    }

    /// `:64-75` `getSignalLayerNo`: the count of signal layers with a
    /// SMALLER number than the given layer (verified against the Java
    /// walk: `layers[i] == layer` returns BEFORE counting layer i). F.Cu
    /// -> 0; GND (power) still counts the F.Cu before it -> 1; B.Cu ->
    /// 1 (GND is not a signal); out of range -> -1.
    #[test]
    fn get_signal_layer_no_counts_signals_strictly_before() {
        let layers = mixed();
        assert_eq!(layers.get_signal_layer_no(0), 0);
        assert_eq!(
            layers.get_signal_layer_no(1),
            1,
            "power layer: F.Cu counted"
        );
        assert_eq!(
            layers.get_signal_layer_no(2),
            1,
            "B.Cu: only F.Cu precedes it"
        );
        assert_eq!(layers.get_signal_layer_no(3), -1, "out of range");
    }

    /// `:78-81` `getLayerNo(signalLayerNo)`: signal index -> physical
    /// index (F.Cu->0, B.Cu->2), with the inherited last-layer fallback
    /// (signalLayerNo 7 -> last layer B.Cu -> 2).
    #[test]
    fn get_layer_no_maps_signal_index_to_physical() {
        let layers = mixed();
        assert_eq!(layers.get_layer_no(0), 0);
        assert_eq!(layers.get_layer_no(1), 2);
        assert_eq!(layers.get_layer_no(7), 2, "fallback to the last layer");
    }

    /// The IR conversion drops exactly the parser-only plane net names
    /// and keeps name/isSignal per layer.
    #[test]
    fn from_ir_keeps_name_and_signal_flag_only() {
        let ir = epic_dsn::layer_structure::LayerStructure::new(vec![
            epic_dsn::layer_structure::Layer::new("F.Cu", 0, true),
            epic_dsn::layer_structure::Layer::with_net_names(
                "GND",
                1,
                false,
                vec!["GND".to_string()],
            ),
        ]);
        let board_layers = LayerStructure::from_ir(&ir);
        assert_eq!(board_layers.layers.len(), 2);
        assert_eq!(board_layers.layers[0], Layer::new("F.Cu", true));
        assert_eq!(board_layers.layers[1], Layer::new("GND", false));
        assert_eq!(board_layers.signal_layer_count(), 1);
    }
}
