//! The `UndoableObjects` level-stack container (T62) and the minimal
//! components-side second stack (T63).
//!
//! Java anchors: `datastructures/UndoableObjects.java` (all 343 lines,
//! transliterated field-for-field). The jar spike
//! `rust/harness/oracle/UndoSpike.java` drove a 30-step scripted
//! sequence against the frozen oracle; its capture
//! (`/tmp/epic-t2-undo.out`, quoted step-by-step in the pins below) is
//! the ground truth for every observable here.
//!
//! ## Semantics (jar-pinned)
//!
//! - The map iterates DESCENDING id (Java: `ConcurrentSkipListMap` in
//!   `compareTo` natural order; `Item.compareTo = item.id - id`,
//!   Item.java:94-103). Callers pass the key type — the board uses
//!   `Reverse<ItemId>` — so "map order" here means the `BTreeMap`'s
//!   ascending key order, which the caller arranges to be descending id.
//! - Nodes live in an append-only slab; the map holds id → current
//!   node index. Old clones and dead nodes stay in the slab forever
//!   (Java garbage-collects them; nothing observable reads them).
//! - **delete dichotomy** (`UndoableObjects.java:115-124`):
//!   `level < stack_level` pushes the NODE onto the current delete
//!   list, else pushes `undo_object` (if non-null). A delete at
//!   stack level 0 (or of a no-undoObject node at the top level)
//!   pushes nothing — the deletion is unrecoverable.
//! - **undo** (:143-178): for map nodes at `level == stack_level`,
//!   swap in `undo_object` (setting `undo_object.redo_object = node`;
//!   `cancelled` gets the node's value ALWAYS, `restored` the old
//!   value), then re-put every node of `deleted_objects_stack[
//!   stack_level-1]` and restore their values. Created-new nodes stay
//!   in the map as invisible redo-only entries.
//! - **redo** (:183-226): guard is `stack_level >= stack.len()` (NOT
//!   `redo_possible`). Map nodes with `redo_object.level ==
//!   stack_level` swap forward (cancelled += old value, restored +=
//!   new); nodes AT the level count as restored. Then the delete-list
//!   walk: advance `while redo_object.level <= stack_level`, remove
//!   the final node from the map, and — the subtle part — remove its
//!   value from `restored` (first occurrence, order-preserving) if
//!   present, ELSE cancel it. Spike S13: the map-loop restored `3:c1`
//!   (c's live node) and the walk then advanced `c0_old → c-live` and
//!   removed `3:c1` from `restored`, yielding
//!   `cancelled=[3:c0, 2:b0, 1:a0] restored=[2:b1]`.
//! - **The walk never advances more than once** (spike S13 shows the
//!   one advance): a delete-list node's `redo_object` points at the
//!   then-live node (set at `save_for_undo` creation, :285), and the
//!   only way that TARGET can itself carry a `redo_object` is the
//!   popSnapshot re-link branch (:246-252), which is dead code (see
//!   below). A restored old node CAN re-enter a delete list with its
//!   creation link intact (undo restores it below `stack_level` and a
//!   later delete pushes it — the merge pin's `b_old` travels exactly
//!   this path), but the link still points at a node whose own
//!   `redo_object` is null, so the walk stops after one advance. The
//!   1-advance walk is pinned and the loop is kept verbatim for
//!   transliteration fidelity.
//! - **popSnapshot** (:232-267): calls `disable_redo` first. The
//!   re-link branch (:246-252, node at `stack_level-1` with
//!   `redo_object` at `stack_level`) is DEAD CODE in the frozen Java:
//!   the only ways `stack_level` can rise past such a link are
//!   `generateSnapshot` (whose `disable_redo` nulls `redo_object` on
//!   exactly the level-`stack_level` nodes that carry it) or `redo`
//!   (whose map-loop swaps the node out of the map). Ported verbatim
//!   anyway; the reachable `else` (decrement `level >= stack_level`)
//!   IS pinned — spike S18 demoted `6:f0` and `2:b1` from L1 to L0
//!   and they became visible in the iteration.
//! - **disableRedo** (:293-309) runs on EVERY mutator: truncate the
//!   delete stack to `stack_level`, REMOVE map nodes with
//!   `level > stack_level`, null `redo_object` on `level ==
//!   stack_level` nodes. Spike S16/S17: after undo then insert, the
//!   level-2 node `5:e0` vanished from the map and redo returned
//!   false.
//! - **readObject** (:54-63) skips nodes with `level > stack_level`.
//!
//! ## Documented divergences (none observable in the M2 gates)
//!
//! - Java equality in `restored.remove(...)` is OBJECT IDENTITY
//!   (`Storable` impls don't override `equals`); this port uses
//!   `T: PartialEq` VALUE equality. The two differ only when an old
//!   clone's value equals the live value at the walk moment — the
//!   board's snapshot values (`ItemEntry`) differ by construction
//!   whenever a save-for-undo happened (the save exists precisely
//!   because the value changed).
//! - `saveForUndo(object)` clones the PASSED Java object (the live
//!   instance, whose id matches the node's); the port clones the
//!   NODE's current value — identical state, since the map entry and
//!   the live instance share the id and the caller mutates only
//!   after saving.
//! - The `FRLogger.warn` calls (delete-miss in redo's walk,
//!   save-for-undo node-miss) are log-only in Java and are no-ops
//!   here (the D12 warnings surface is fed exclusively by
//!   `Wiring.java`; see the M1b Task 6 correction).

use std::cmp::Reverse;
use std::collections::BTreeMap;

/// Index into the node slab (`UndoableObjectNode` cross-links become
/// plain indices; Java object identity is not needed for any pinned
/// observable).
type NodeIdx = usize;

/// Java `UndoableObjectNode` (`UndoableObjects.java:328-342`).
#[derive(Clone, Debug)]
struct Node<K, T> {
    key: K,
    value: T,
    level: usize,
    undo_object: Option<NodeIdx>,
    redo_object: Option<NodeIdx>,
}

/// Java `UndoableObjects`: a level stack of value snapshots keyed by
/// `K` (the board passes `Reverse<ItemId>` so map order is descending
/// id, matching the Java `ConcurrentSkipListMap`).
#[derive(Clone, Debug)]
pub struct UndoableObjects<K, T> {
    /// Java `objects` — only the CURRENT node per key is live here
    /// (old clones stay in the slab, unreachable from the map).
    objects: BTreeMap<K, NodeIdx>,
    /// All nodes ever created, append-only.
    nodes: Vec<Node<K, T>>,
    /// Java `deletedObjectsStack` — one list per snapshot level.
    deleted_objects_stack: Vec<Vec<NodeIdx>>,
    /// Java `stackLevel`.
    stack_level: usize,
    /// Java `redoPossible`.
    redo_possible: bool,
}

impl<K: Ord + Copy, T: Clone + PartialEq> UndoableObjects<K, T> {
    /// Java `UndoableObjects()` — empty, level 0, redo disabled.
    #[must_use]
    pub fn new() -> Self {
        Self {
            objects: BTreeMap::new(),
            nodes: Vec::new(),
            deleted_objects_stack: Vec::new(),
            stack_level: 0,
            redo_possible: false,
        }
    }

    /// Java `stackLevel`.
    #[must_use]
    pub fn stack_level(&self) -> usize {
        self.stack_level
    }

    /// Java `redoPossible`.
    #[must_use]
    pub fn redo_possible(&self) -> bool {
        self.redo_possible
    }

    /// Java `insert` (:66-70): disable redo, node at the current
    /// level. An insert that REPLACES a same-key entry burns the old
    /// node silently (Java: `map.put` replaces the value — the old
    /// node becomes unreachable, same as here).
    pub fn insert(&mut self, key: K, value: T) {
        self.disable_redo();
        let idx = self.nodes.len();
        self.nodes.push(Node {
            key,
            value,
            level: self.stack_level,
            undo_object: None,
            redo_object: None,
        });
        self.objects.insert(key, idx);
    }

    /// Java `delete` (:76-128). Returns false when the key is not in
    /// the map. The dichotomy: `level < stack_level` pushes the node
    /// itself (recoverable by undo); otherwise pushes `undo_object`
    /// if non-null — a delete of a never-saved node at the top level
    /// (or any delete at level 0, where no delete list exists) is
    /// unrecoverable.
    pub fn delete(&mut self, key: &K) -> bool {
        self.disable_redo();
        let Some(&idx) = self.objects.get(key) else {
            return false;
        };
        if let Some(list) = self.deleted_objects_stack.last_mut() {
            if self.nodes[idx].level < self.stack_level {
                // push the node itself
                list.push(idx);
            } else if let Some(undo_idx) = self.nodes[idx].undo_object {
                // push the pre-change snapshot instead
                list.push(undo_idx);
            }
        }
        self.objects.remove(key);
        true
    }

    /// Java `generateSnapshot` (:131-136): push an empty delete list,
    /// `++stackLevel`.
    pub fn generate_snapshot(&mut self) {
        self.disable_redo();
        self.deleted_objects_stack.push(Vec::new());
        self.stack_level += 1;
    }

    /// Java `undo` (:143-178). `None` = the Java `false` return (no
    /// more undo possible); `Some((cancelled, restored))` carries the
    /// collections in EXACTLY the Java append order — map order for
    /// the swap phase, then delete-list order for the restore phase.
    /// That order is observable (Task 14's digest) and pinned here.
    pub fn undo(&mut self) -> Option<(Vec<T>, Vec<T>)> {
        if self.stack_level == 0 {
            return None;
        }
        let mut cancelled = Vec::new();
        let mut restored = Vec::new();
        // Map order (Java: ConcurrentSkipListMap values()). The put
        // during iteration is safe in Java; here the snapshot is
        // taken first and the puts applied after.
        let entries: Vec<(K, NodeIdx)> = self.objects.iter().map(|(&k, &i)| (k, i)).collect();
        for (_key, idx) in entries {
            if self.nodes[idx].level == self.stack_level {
                if let Some(undo_idx) = self.nodes[idx].undo_object {
                    self.nodes[undo_idx].redo_object = Some(idx);
                    let undo_key = self.nodes[undo_idx].key;
                    self.objects.insert(undo_key, undo_idx);
                    restored.push(self.nodes[undo_idx].value.clone());
                }
                cancelled.push(self.nodes[idx].value.clone());
            }
        }
        // restore the deleted objects (Java :170-177)
        let delete_list = self.deleted_objects_stack[self.stack_level - 1].clone();
        for idx in delete_list {
            let key = self.nodes[idx].key;
            self.objects.insert(key, idx);
            restored.push(self.nodes[idx].value.clone());
        }
        self.stack_level -= 1;
        self.redo_possible = true;
        Some((cancelled, restored))
    }

    /// Java `redo` (:183-226). `None` = the Java `false` return; the
    /// guard is `stack_level >= stack.len()` (NOT `redo_possible`).
    /// The delete-list walk advances the (at-most-one-link) redo
    /// chain, removes the final node from the map, and MUTATES
    /// `restored`: a value already present (first occurrence,
    /// order-preserving removal — Java `List.remove(Object)`) is
    /// dropped instead of being cancelled.
    pub fn redo(&mut self) -> Option<(Vec<T>, Vec<T>)> {
        if self.stack_level >= self.deleted_objects_stack.len() {
            return None;
        }
        self.stack_level += 1;
        let mut cancelled = Vec::new();
        let mut restored = Vec::new();
        let entries: Vec<(K, NodeIdx)> = self.objects.iter().map(|(&k, &i)| (k, i)).collect();
        for (key, idx) in entries {
            let redo_idx = self.nodes[idx].redo_object;
            if let Some(r) = redo_idx
                && self.nodes[r].level == self.stack_level
            {
                // replace the lower-level object by the current one
                self.objects.insert(key, r);
                cancelled.push(self.nodes[idx].value.clone());
                restored.push(self.nodes[r].value.clone());
                continue;
            }
            if self.nodes[idx].level == self.stack_level {
                // created on the current level — restorable by a later undo
                restored.push(self.nodes[idx].value.clone());
            }
        }
        // re-delete the objects deleted on this level (Java :209-224)
        let delete_list = self.deleted_objects_stack[self.stack_level - 1].clone();
        for deleted_idx in delete_list {
            let mut current = deleted_idx;
            while let Some(r) = self.nodes[current].redo_object {
                if self.nodes[r].level <= self.stack_level {
                    current = r;
                } else {
                    break;
                }
            }
            let key = self.nodes[current].key;
            let value = self.nodes[current].value.clone();
            // Java: FRLogger.warn("previous deleted object not found") on a
            // miss — log-only there, silent here (module docs).
            self.objects.remove(&key);
            match restored.iter().position(|v| *v == value) {
                Some(pos) => {
                    restored.remove(pos);
                }
                None => cancelled.push(value),
            }
        }
        Some((cancelled, restored))
    }

    /// Java `popSnapshot` (:232-267). The re-link branch
    /// (:246-252) is dead code in the frozen Java (module docs);
    /// ported verbatim. The `else` branch — decrementing nodes at
    /// `level >= stack_level` — fires on every pop that has
    /// top-level nodes and is pinned (spike S18).
    pub fn pop_snapshot(&mut self) -> bool {
        self.disable_redo();
        if self.stack_level == 0 {
            return false;
        }
        let entries: Vec<(K, NodeIdx)> = self.objects.iter().map(|(&k, &i)| (k, i)).collect();
        for (_key, idx) in entries {
            if self.nodes[idx].level == self.stack_level - 1 {
                let redo_idx = self.nodes[idx].redo_object;
                if let Some(r) = redo_idx
                    && self.nodes[r].level == self.stack_level
                {
                    self.nodes[r].undo_object = self.nodes[idx].undo_object;
                    if let Some(u) = self.nodes[idx].undo_object {
                        self.nodes[u].redo_object = self.nodes[idx].redo_object;
                    }
                }
            } else if self.nodes[idx].level >= self.stack_level {
                self.nodes[idx].level -= 1;
            }
        }
        // join the top delete list into the second-top (Java :258-271)
        let stack_size = self.deleted_objects_stack.len();
        if stack_size >= 2 {
            let from_list = self.deleted_objects_stack[stack_size - 1].clone();
            let to_list = &mut self.deleted_objects_stack[stack_size - 2];
            for idx in from_list {
                if self.nodes[idx].level < self.stack_level - 1 {
                    to_list.push(idx);
                } else if let Some(undo_idx) = self.nodes[idx].undo_object {
                    to_list.push(undo_idx);
                }
            }
        }
        self.deleted_objects_stack.pop();
        self.stack_level -= 1;
        true
    }

    /// Java `saveForUndo` (:273-290): clone the current value into an
    /// old node at the value's level and splice the cross-links, then
    /// promote the live node to `stack_level`. No-op when the key is
    /// not in the map (Java logs a warning and returns — module docs)
    /// or when the node is already at `stack_level`.
    pub fn save_for_undo(&mut self, key: &K) {
        self.disable_redo();
        let Some(&idx) = self.objects.get(key) else {
            // Java: FRLogger.warn("... object node not found") — spike S19.
            return;
        };
        if self.nodes[idx].level < self.stack_level {
            let old_idx = self.nodes.len();
            let old = Node {
                key: self.nodes[idx].key,
                value: self.nodes[idx].value.clone(),
                level: self.nodes[idx].level,
                undo_object: self.nodes[idx].undo_object,
                redo_object: Some(idx),
            };
            self.nodes.push(old);
            self.nodes[idx].undo_object = Some(old_idx);
            self.nodes[idx].level = self.stack_level;
        }
    }

    /// Mutable access to a live value — the caller-side "mutate after
    /// save_for_undo" path (the Java caller just assigns fields on
    /// the shared instance). The board's item mutations go through
    /// here.
    pub fn value_mut(&mut self, key: &K) -> Option<&mut T> {
        let &idx = self.objects.get(key)?;
        Some(&mut self.nodes[idx].value)
    }

    /// The visible value for a key (any node at or below the stack
    /// level; the map only ever holds current nodes, so this is the
    /// map entry — unlike iteration, single lookups have no
    /// redo-only entries to skip).
    #[must_use]
    pub fn get(&self, key: &K) -> Option<&T> {
        let &idx = self.objects.get(key)?;
        Some(&self.nodes[idx].value)
    }

    /// Whether the key is currently live in the map.
    #[must_use]
    pub fn contains(&self, key: &K) -> bool {
        self.objects.contains_key(key)
    }

    /// Java `startReadObject`/`readObject` (:44-63): iterate the
    /// visible values in map order (descending id for the board's
    /// `Reverse<ItemId>` keys), skipping redo-only nodes
    /// (`level > stack_level`).
    pub fn iter_visible(&self) -> impl Iterator<Item = (&K, &T)> {
        self.objects
            .iter()
            .filter(|entry| self.nodes[*entry.1].level <= self.stack_level)
            .map(|(k, &idx)| (k, &self.nodes[idx].value))
    }

    /// Java `disableRedo` (:293-309) — runs on every mutator.
    fn disable_redo(&mut self) {
        if !self.redo_possible {
            return;
        }
        self.redo_possible = false;
        // shorten the delete stack to stack_level
        self.deleted_objects_stack.truncate(self.stack_level);
        let entries: Vec<(K, NodeIdx)> = self.objects.iter().map(|(&k, &i)| (k, i)).collect();
        for (key, idx) in entries {
            if self.nodes[idx].level > self.stack_level {
                self.objects.remove(&key);
            } else if self.nodes[idx].level == self.stack_level {
                self.nodes[idx].redo_object = None;
            }
        }
    }
}

impl<K: Ord + Copy, T: Clone + PartialEq> Default for UndoableObjects<K, T> {
    fn default() -> Self {
        Self::new()
    }
}

/// The components-side second stack (T63, `Components.java:16`):
/// `Components` owns its own `UndoableObjects` instance, completely
/// independent of the board's item stack. Component ids are 1-BASED
/// indexes (`Components.java:140`, `componentArr.size() + 1` at add
/// time) and are never reused — the counter only grows, even across
/// undo (Java re-syncs `componentArr` from the undo list by INDEX,
/// `restoreComponentArrFromUndoList`, so the array slots stay 1:1
/// with ids forever).
///
/// This is the minimal container; the real `Component` (placement,
/// rotation, T68) is Task 3.
#[derive(Clone, Debug)]
pub struct ComponentsUndoStack<T> {
    undo_list: UndoableObjects<Reverse<u32>, T>,
    /// Java `componentArr` growth — the NEXT id is `count + 1`.
    count: u32,
}

impl<T: Clone + PartialEq> ComponentsUndoStack<T> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            undo_list: UndoableObjects::new(),
            count: 0,
        }
    }

    /// Java `Components.add`: id = `componentArr.size() + 1` — 1-based,
    /// monotonically growing, never reused.
    pub fn add(&mut self, value: T) -> u32 {
        self.count += 1;
        let id = self.count;
        self.undo_list.insert(Reverse(id), value);
        id
    }

    /// The number of components ever added (Java `componentArr.size()`
    /// — NOT the live count; undo restores values, not array slots).
    #[must_use]
    pub fn count(&self) -> u32 {
        self.count
    }

    pub fn undo(&mut self) -> Option<(Vec<T>, Vec<T>)> {
        self.undo_list.undo()
    }

    pub fn redo(&mut self) -> Option<(Vec<T>, Vec<T>)> {
        self.undo_list.redo()
    }

    pub fn generate_snapshot(&mut self) {
        self.undo_list.generate_snapshot();
    }

    pub fn pop_snapshot(&mut self) -> bool {
        self.undo_list.pop_snapshot()
    }

    pub fn save_for_undo(&mut self, id: u32) {
        self.undo_list.save_for_undo(&Reverse(id));
    }

    pub fn delete(&mut self, id: u32) -> bool {
        self.undo_list.delete(&Reverse(id))
    }

    /// Visible components in map order (descending id).
    pub fn iter_visible(&self) -> impl Iterator<Item = (u32, &T)> {
        self.undo_list.iter_visible().map(|(k, v)| (k.0, v))
    }
}

impl<T: Clone + PartialEq> Default for ComponentsUndoStack<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spike's `Rec`: (id, value). Display mirror `id:value` so a
    /// failing assertion reads like the capture. Rust equality here is
    /// value equality on the pair; every value in the scripted
    /// sequence is distinct at each comparison point, so it matches
    /// the Java identity-equality behavior (module docs).
    #[derive(Clone, PartialEq, Debug)]
    struct Rec {
        id: u32,
        value: String,
    }

    impl Rec {
        fn new(id: u32, value: &str) -> Self {
            Self {
                id,
                value: value.to_string(),
            }
        }
    }

    /// Renders like Java `Rec.toString()` (`id + ":" + value`) so
    /// assertion messages quote the capture verbatim.
    fn show(values: &[Rec]) -> String {
        values
            .iter()
            .map(|r| format!("{}:{}", r.id, r.value))
            .collect::<Vec<_>>()
            .join(", ")
    }

    type Stack = UndoableObjects<Reverse<u32>, Rec>;

    fn iter(stack: &Stack) -> Vec<Rec> {
        stack.iter_visible().map(|(_, v)| v.clone()).collect()
    }

    /// S1: fresh inserts at L0 iterate DESCENDING id — the capture's
    /// first line, `iter=[3:c0, 2:b0, 1:a0]`. An ascending port fails.
    #[test]
    fn s1_inserts_iterate_descending_id() {
        let mut s = Stack::new();
        for (id, v) in [(1, "a0"), (2, "b0"), (3, "c0")] {
            s.insert(Reverse(id), Rec::new(id, v));
        }
        assert_eq!(show(&iter(&s)), "3:c0, 2:b0, 1:a0");
        assert_eq!(s.stack_level(), 0);
    }

    /// S2–S7: snapshot, save+mutate, and the delete DICHOTOMY — all
    /// three sides. The capture:
    ///   S4 iter=[3:c0, 2:b1]        (a: level 0 < 1 → NODE pushed)
    ///   S6 iter=[3:c0, 2:b1]        (d: level 1 == 1, undo null →
    ///                                 NOTHING pushed)
    ///   S7 iter=[2:b1]              (c: level 1 == 1, undo non-null
    ///                                 → UNDOOBJECT pushed)
    #[test]
    fn s2_to_s7_delete_dichotomy() {
        let mut s = Stack::new();
        for (id, v) in [(1, "a0"), (2, "b0"), (3, "c0")] {
            s.insert(Reverse(id), Rec::new(id, v));
        }
        s.generate_snapshot(); // S2
        assert_eq!(s.stack_level(), 1);
        // S3: saveForUndo(b) then mutate the live value to b1
        s.save_for_undo(&Reverse(2));
        s.value_mut(&Reverse(2)).expect("b live").value = "b1".to_string();
        assert_eq!(show(&iter(&s)), "3:c0, 2:b1, 1:a0");
        // S4: delete(a) — level 0 < stackLevel 1 → node pushed
        assert!(s.delete(&Reverse(1)));
        assert_eq!(show(&iter(&s)), "3:c0, 2:b1");
        // S5: insert d at L1 (no undoObject)
        s.insert(Reverse(4), Rec::new(4, "d0"));
        assert_eq!(show(&iter(&s)), "4:d0, 3:c0, 2:b1");
        // S6: delete(d) — level 1 == stackLevel, undoObject null →
        // nothing pushed: d is unrecoverable
        assert!(s.delete(&Reverse(4)));
        assert_eq!(show(&iter(&s)), "3:c0, 2:b1");
        // S7: saveForUndo(c), mutate, delete — undoObject pushed
        s.save_for_undo(&Reverse(3));
        s.value_mut(&Reverse(3)).expect("c live").value = "c1".to_string();
        assert!(s.delete(&Reverse(3)));
        assert_eq!(show(&iter(&s)), "2:b1");

        // The dichotomy's OBSERVABLE: undoing to L0 restores the map
        // swap (`2:b0`) then the delete list in push order — a's NODE
        // (`1:a0`) and c's UNDOOBJECT (`3:c0`, the pre-mutation clone;
        // `<` flipped to `<=` pushes the mutated live node instead and
        // this shows `3:c1`).
        let (cancelled, restored) = s.undo().expect("undo to L0");
        assert_eq!(show(&cancelled), "2:b1");
        assert_eq!(show(&restored), "2:b0, 1:a0, 3:c0");
    }

    /// S8–S15: the second level, double undo, double redo. The
    /// capture (every collection quoted verbatim):
    ///   S11 undo -> cancelled=[5:e0]           restored=[2:b1]
    ///   S12 undo -> cancelled=[2:b1]           restored=[2:b0, 1:a0, 3:c0]
    ///   S13 redo -> cancelled=[3:c0, 2:b0, 1:a0] restored=[2:b1]
    ///   S14 redo -> cancelled=[2:b1]           restored=[5:e0]
    ///   S15 undo -> cancelled=[5:e0]           restored=[2:b1]
    /// S12's restored pins the ORDER: map-order swap first (`2:b0`),
    /// then delete-list push order (`1:a0` from S4 before `3:c0` from
    /// S7). S13's cancelled is the redo-chain-walk pin (see
    /// [`s13_redo_walk_advances_and_mutates_restored`]).
    #[test]
    fn s8_to_s15_double_undo_double_redo() {
        let mut s = Stack::new();
        for (id, v) in [(1, "a0"), (2, "b0"), (3, "c0")] {
            s.insert(Reverse(id), Rec::new(id, v));
        }
        s.generate_snapshot(); // S2
        s.save_for_undo(&Reverse(2));
        s.value_mut(&Reverse(2)).expect("b").value = "b1".to_string();
        s.delete(&Reverse(1)); // S4
        s.insert(Reverse(4), Rec::new(4, "d0"));
        s.delete(&Reverse(4)); // S6
        s.save_for_undo(&Reverse(3));
        s.value_mut(&Reverse(3)).expect("c").value = "c1".to_string();
        s.delete(&Reverse(3)); // S7
        s.generate_snapshot(); // S8
        s.insert(Reverse(5), Rec::new(5, "e0")); // S9
        assert_eq!(show(&iter(&s)), "5:e0, 2:b1");
        s.delete(&Reverse(2)); // S10: b1 node (L1 < 2) pushed
        assert_eq!(show(&iter(&s)), "5:e0");

        // S11: undo to L1 — e cancelled (still in map, INVISIBLE),
        // b1 restored from the delete list.
        let (cancelled, restored) = s.undo().expect("S11 undo");
        assert_eq!(show(&cancelled), "5:e0");
        assert_eq!(show(&restored), "2:b1");
        assert_eq!(show(&iter(&s)), "2:b1");
        // e is invisible but STILL MAP-RESIDENT (undo leaves its level-2
        // node in the map; only disable_redo removes it) — S14 brings it
        // back through the swap-forward branch.
        assert!(
            s.contains(&Reverse(5)),
            "e stays map-resident while invisible"
        );
        assert!(s.iter_visible().all(|(k, _)| k.0 != 5));

        // S12: undo to L0 — b1 swapped for b0 (cross-link), then the
        // S4/S7 delete list restored IN PUSH ORDER.
        let (cancelled, restored) = s.undo().expect("S12 undo");
        assert_eq!(show(&cancelled), "2:b1");
        assert_eq!(show(&restored), "2:b0, 1:a0, 3:c0");
        assert_eq!(show(&iter(&s)), "3:c0, 2:b0, 1:a0");
        assert_eq!(s.stack_level(), 0);

        // S13: redo to L1.
        let (cancelled, restored) = s.redo().expect("S13 redo");
        assert_eq!(show(&cancelled), "3:c0, 2:b0, 1:a0");
        assert_eq!(show(&restored), "2:b1");
        assert_eq!(show(&iter(&s)), "2:b1");

        // S14: redo to L2 — e restored via the level==stackLevel
        // branch; b1 re-deleted by the walk.
        let (cancelled, restored) = s.redo().expect("S14 redo");
        assert_eq!(show(&cancelled), "2:b1");
        assert_eq!(show(&restored), "5:e0");
        assert_eq!(show(&iter(&s)), "5:e0");
        assert_eq!(s.stack_level(), 2);

        // S15: undo back to L1.
        let (cancelled, restored) = s.undo().expect("S15 undo");
        assert_eq!(show(&cancelled), "5:e0");
        assert_eq!(show(&restored), "2:b1");
    }

    /// S13 in isolation — the redo delete-list WALK pin. c was
    /// saved-for-undo then deleted at L1 (its UNDOOBJECT `c0_old` was
    /// pushed, and `c0_old.redo_object` = c's live node by the
    /// save_for_undo creation link). During redo to L1 the map-loop
    /// swaps c0_old → c-live (cancelled `3:c0`, restored `3:c1`); the
    /// walk then ADVANCES c0_old → c-live, removes `3:c1` from
    /// `restored` (first occurrence) instead of cancelling it again.
    /// Flipping the walk condition `<=` to `<` (or dropping the
    /// advance) leaves `3:c1` in restored and cancels `3:c0` twice:
    /// cancelled=[3:c0, 2:b0, 1:a0, 3:c0], restored=[3:c1, 2:b1] —
    /// both fail this pin (verified by mutation before commit).
    #[test]
    fn s13_redo_walk_advances_and_mutates_restored() {
        let mut s = Stack::new();
        for (id, v) in [(1, "a0"), (2, "b0"), (3, "c0")] {
            s.insert(Reverse(id), Rec::new(id, v));
        }
        s.generate_snapshot();
        s.save_for_undo(&Reverse(2));
        s.value_mut(&Reverse(2)).expect("b").value = "b1".to_string();
        s.delete(&Reverse(1));
        s.save_for_undo(&Reverse(3));
        s.value_mut(&Reverse(3)).expect("c").value = "c1".to_string();
        s.delete(&Reverse(3));
        s.undo().expect("to L0");
        let (cancelled, restored) = s.redo().expect("S13 redo");
        assert_eq!(show(&cancelled), "3:c0, 2:b0, 1:a0");
        assert_eq!(show(&restored), "2:b1");
        // the walk removed 3:c1 — the live c left the map entirely
        assert!(!s.contains(&Reverse(3)));
        assert_eq!(show(&iter(&s)), "2:b1");
    }

    /// S16–S18: disableRedo's truncation via a mutation. The capture:
    ///   S16 iter=[6:f0, 2:b1]  (5:e0 at L2 REMOVED from the map)
    ///   S17 redo -> false      (stack truncated to stack_level)
    ///   S18 pop -> true, iter=[6:f0, 2:b1] (f, b1 demoted L1 → L0
    ///                                by the decrement branch)
    #[test]
    fn s16_to_s18_disable_redo_truncation() {
        let mut s = Stack::new();
        for (id, v) in [(1, "a0"), (2, "b0"), (3, "c0")] {
            s.insert(Reverse(id), Rec::new(id, v));
        }
        // the spike prefix through S15 (same script as s19_to_s26)
        s.generate_snapshot(); // S2
        s.save_for_undo(&Reverse(2));
        s.value_mut(&Reverse(2)).expect("b").value = "b1".to_string();
        s.delete(&Reverse(1)); // S4
        s.insert(Reverse(4), Rec::new(4, "d0")); // S5
        s.delete(&Reverse(4)); // S6
        s.save_for_undo(&Reverse(3)); // S7
        s.value_mut(&Reverse(3)).expect("c").value = "c1".to_string();
        s.delete(&Reverse(3));
        s.generate_snapshot(); // S8
        s.insert(Reverse(5), Rec::new(5, "e0")); // S9
        s.delete(&Reverse(2)); // S10
        s.undo(); // S11 -> L1
        s.undo(); // S12 -> L0
        s.redo(); // S13 -> L1
        s.redo(); // S14 -> L2
        s.undo(); // S15 -> L1
        assert!(s.redo_possible());

        // S16: any mutation disables redo: e (L2) leaves the map.
        s.insert(Reverse(6), Rec::new(6, "f0"));
        assert_eq!(show(&iter(&s)), "6:f0, 2:b1");
        assert!(!s.redo_possible());
        assert!(!s.contains(&Reverse(5)), "e removed by disable_redo");

        // S17: redo impossible — the stack was truncated to level 1.
        assert!(s.redo().is_none());
        assert_eq!(show(&iter(&s)), "6:f0, 2:b1");

        // S18: pop demotes f and b1 from L1 to L0 — they stay visible.
        assert!(s.pop_snapshot());
        assert_eq!(show(&iter(&s)), "6:f0, 2:b1");
        assert_eq!(s.stack_level(), 0);
    }

    /// S19–S26: the residual-state probes. a was re-deleted by S13's
    /// walk and its delete-list entries merged away by S18's pop, so
    /// every later `save_for_undo(a)` is a no-op (Java logs
    /// "object node not found" — spike S19/S21/S23). The undos of the
    /// empty snapshots S20/S22 return TRUE with EMPTY collections,
    /// and S26 hits the stack floor.
    #[test]
    fn s19_to_s26_missing_node_noop_and_empty_undos() {
        let mut s = Stack::new();
        for (id, v) in [(1, "a0"), (2, "b0"), (3, "c0")] {
            s.insert(Reverse(id), Rec::new(id, v));
        }
        // ... fast-forward through S4–S18 (same script as above)
        s.generate_snapshot();
        s.save_for_undo(&Reverse(2));
        s.value_mut(&Reverse(2)).expect("b").value = "b1".to_string();
        s.delete(&Reverse(1));
        s.save_for_undo(&Reverse(3));
        s.value_mut(&Reverse(3)).expect("c").value = "c1".to_string();
        s.delete(&Reverse(3));
        s.generate_snapshot();
        s.insert(Reverse(5), Rec::new(5, "e0"));
        s.delete(&Reverse(2));
        s.undo();
        s.undo();
        s.redo();
        s.redo();
        s.undo();
        s.insert(Reverse(6), Rec::new(6, "f0"));
        s.redo();
        s.pop_snapshot(); // S18, -> L0

        // S19: save_for_undo of a key not in the map — no panic, no
        // state change.
        s.save_for_undo(&Reverse(1));
        assert_eq!(show(&iter(&s)), "6:f0, 2:b1");
        // S20: snapshot -> L1
        s.generate_snapshot();
        assert_eq!(s.stack_level(), 1);
        // S21/S23: still no-ops
        s.save_for_undo(&Reverse(1));
        s.generate_snapshot(); // S22 -> L2
        s.save_for_undo(&Reverse(1));
        // S24: undo of the empty L2 — true, nothing cancelled/restored
        let (cancelled, restored) = s.undo().expect("S24 undo");
        assert!(cancelled.is_empty());
        assert!(restored.is_empty());
        // S25: pop back to L0
        assert!(s.pop_snapshot());
        // S26: nothing left to undo
        assert!(s.undo().is_none());
        assert_eq!(s.stack_level(), 0);
    }

    /// S27–S30: created-new nodes stay in the map as invisible
    /// redo-only entries — the readObject skip. The capture:
    ///   S28 iter=[7:g0, 6:f0, 2:b1]
    ///   S29 undo -> cancelled=[7:g0] restored=[]   iter=[6:f0, 2:b1]
    /// g is STILL IN THE MAP (a later redo would resurrect it), just
    /// skipped by `level > stack_level`.
    #[test]
    fn s27_to_s30_created_new_nodes_stay_invisible() {
        let mut s = Stack::new();
        for (id, v) in [(1, "a0"), (2, "b0"), (3, "c0")] {
            s.insert(Reverse(id), Rec::new(id, v));
        }
        // fast-forward the full script to its post-S26 residual state
        // (map: f at L0, b1 at L0; a and c long gone) — the exact
        // state the capture's S28 line describes
        s.generate_snapshot(); // S2
        s.save_for_undo(&Reverse(2));
        s.value_mut(&Reverse(2)).expect("b").value = "b1".to_string();
        s.delete(&Reverse(1)); // S4
        s.insert(Reverse(4), Rec::new(4, "d0")); // S5
        s.delete(&Reverse(4)); // S6
        s.save_for_undo(&Reverse(3)); // S7
        s.value_mut(&Reverse(3)).expect("c").value = "c1".to_string();
        s.delete(&Reverse(3));
        s.generate_snapshot(); // S8
        s.insert(Reverse(5), Rec::new(5, "e0")); // S9
        s.delete(&Reverse(2)); // S10
        s.undo(); // S11
        s.undo(); // S12
        s.redo(); // S13
        s.redo(); // S14
        s.undo(); // S15
        s.insert(Reverse(6), Rec::new(6, "f0")); // S16
        s.redo(); // S17 (none)
        s.pop_snapshot(); // S18
        s.save_for_undo(&Reverse(1)); // S19 (no-op)
        s.generate_snapshot(); // S20
        s.save_for_undo(&Reverse(1)); // S21 (no-op)
        s.generate_snapshot(); // S22
        s.save_for_undo(&Reverse(1)); // S23 (no-op)
        s.undo(); // S24
        s.pop_snapshot(); // S25
        assert!(s.undo().is_none()); // S26
        s.generate_snapshot(); // S27
        assert_eq!(show(&iter(&s)), "6:f0, 2:b1");
        s.insert(Reverse(7), Rec::new(7, "g0")); // S28 at L1
        assert_eq!(show(&iter(&s)), "7:g0, 6:f0, 2:b1");
        let (cancelled, restored) = s.undo().expect("S29 undo");
        assert_eq!(show(&cancelled), "7:g0");
        assert!(restored.is_empty());
        // g invisible at L1 but the map still holds it: get() would
        // find it through the CURRENT-node map (no redo-only entry
        // here — the map only ever holds current nodes; the skip is
        // purely the level filter in iteration).
        assert_eq!(show(&iter(&s)), "6:f0, 2:b1");
        // S29's undo popped the snapshot stack: L1 -> L0 (the spike's
        // stack level, NOT g's node level, which stays at 1)
        assert_eq!(s.stack_level(), 0);
    }

    /// The FULL S1–S30 script replayed end-to-end, asserting the
    /// capture's iteration line after every step. This is the master
    /// pin: any divergence from the jar sequence shows up as an iter
    /// mismatch at the exact step.
    #[test]
    fn full_spike_sequence_s1_to_s30() {
        let mut s = Stack::new();
        for (id, v) in [(1, "a0"), (2, "b0"), (3, "c0")] {
            s.insert(Reverse(id), Rec::new(id, v));
        }
        assert_eq!(show(&iter(&s)), "3:c0, 2:b0, 1:a0"); // S1
        s.generate_snapshot(); // S2
        assert_eq!(show(&iter(&s)), "3:c0, 2:b0, 1:a0");
        s.save_for_undo(&Reverse(2)); // S3
        s.value_mut(&Reverse(2)).expect("b").value = "b1".to_string();
        assert_eq!(show(&iter(&s)), "3:c0, 2:b1, 1:a0");
        s.delete(&Reverse(1)); // S4
        assert_eq!(show(&iter(&s)), "3:c0, 2:b1");
        s.insert(Reverse(4), Rec::new(4, "d0")); // S5
        assert_eq!(show(&iter(&s)), "4:d0, 3:c0, 2:b1");
        s.delete(&Reverse(4)); // S6
        assert_eq!(show(&iter(&s)), "3:c0, 2:b1");
        s.save_for_undo(&Reverse(3)); // S7
        s.value_mut(&Reverse(3)).expect("c").value = "c1".to_string();
        s.delete(&Reverse(3));
        assert_eq!(show(&iter(&s)), "2:b1");
        s.generate_snapshot(); // S8
        assert_eq!(show(&iter(&s)), "2:b1");
        s.insert(Reverse(5), Rec::new(5, "e0")); // S9
        assert_eq!(show(&iter(&s)), "5:e0, 2:b1");
        s.delete(&Reverse(2)); // S10
        assert_eq!(show(&iter(&s)), "5:e0");
        s.undo(); // S11
        assert_eq!(show(&iter(&s)), "2:b1");
        s.undo(); // S12
        assert_eq!(show(&iter(&s)), "3:c0, 2:b0, 1:a0");
        s.redo(); // S13
        assert_eq!(show(&iter(&s)), "2:b1");
        s.redo(); // S14
        assert_eq!(show(&iter(&s)), "5:e0");
        s.undo(); // S15
        assert_eq!(show(&iter(&s)), "2:b1");
        s.insert(Reverse(6), Rec::new(6, "f0")); // S16
        assert_eq!(show(&iter(&s)), "6:f0, 2:b1");
        assert!(s.redo().is_none()); // S17
        assert_eq!(show(&iter(&s)), "6:f0, 2:b1");
        s.pop_snapshot(); // S18
        assert_eq!(show(&iter(&s)), "6:f0, 2:b1");
        s.save_for_undo(&Reverse(1)); // S19 (no-op)
        assert_eq!(show(&iter(&s)), "6:f0, 2:b1");
        s.generate_snapshot(); // S20
        assert_eq!(show(&iter(&s)), "6:f0, 2:b1");
        s.save_for_undo(&Reverse(1)); // S21 (no-op)
        assert_eq!(show(&iter(&s)), "6:f0, 2:b1");
        s.generate_snapshot(); // S22
        assert_eq!(show(&iter(&s)), "6:f0, 2:b1");
        s.save_for_undo(&Reverse(1)); // S23 (no-op)
        assert_eq!(show(&iter(&s)), "6:f0, 2:b1");
        s.undo(); // S24
        assert_eq!(show(&iter(&s)), "6:f0, 2:b1");
        s.pop_snapshot(); // S25
        assert_eq!(show(&iter(&s)), "6:f0, 2:b1");
        assert!(s.undo().is_none()); // S26
        assert_eq!(show(&iter(&s)), "6:f0, 2:b1");
        s.generate_snapshot(); // S27
        assert_eq!(show(&iter(&s)), "6:f0, 2:b1");
        s.insert(Reverse(7), Rec::new(7, "g0")); // S28
        assert_eq!(show(&iter(&s)), "7:g0, 6:f0, 2:b1");
        s.undo(); // S29
        assert_eq!(show(&iter(&s)), "6:f0, 2:b1");
        assert_eq!(show(&iter(&s)), "6:f0, 2:b1"); // S30 (end)
    }

    /// T63: the components stack is independent — ids are 1-based and
    /// monotonically growing (`Components.java:140`), and undo/redo
    /// never resurrect an id. Java `restoreComponentArrFromUndoList`
    /// re-syncs slots BY INDEX, so a redone component lands back in
    /// its original slot.
    #[test]
    fn components_stack_ids_are_one_based_and_grow_monotonically() {
        let mut comps: ComponentsUndoStack<String> = ComponentsUndoStack::new();
        assert_eq!(comps.add("U1".to_string()), 1);
        assert_eq!(comps.add("U2".to_string()), 2);
        assert_eq!(comps.add("U3".to_string()), 3);
        assert_eq!(comps.count(), 3);

        comps.generate_snapshot();
        assert!(comps.delete(2));
        // visible: 3, 1 (descending id, 2 deleted)
        let visible: Vec<(u32, &String)> = comps.iter_visible().collect();
        assert_eq!(
            visible,
            vec![(3, &"U3".to_string()), (1, &"U1".to_string())]
        );

        let (cancelled, restored) = comps.undo().expect("undo");
        assert!(cancelled.is_empty());
        assert_eq!(restored, vec!["U2".to_string()]);

        // after an undo the counter has NOT shrunk: the next add is 4
        // (Java: componentArr keeps its slots; ids are never reused)
        assert_eq!(comps.add("U4".to_string()), 4);
        assert_eq!(comps.count(), 4);
    }

    /// S31–S37 (spike Phase 5, fresh instance — the plan-mandated
    /// popSnapshot MERGE pin): the TOP delete list's entries re-enter the
    /// second-top through the SAME node-vs-undoObject dichotomy as
    /// delete, evaluated against `stack_level - 1` (`UndoableObjects
    /// .java:249-263`). Setup parks BOTH sides in the top list at once:
    /// a (level 0, deleted at L2) and b's second-save old node `b_old1`
    /// (level 1, `undo_object` = the first-save clone `b_old` carrying
    /// `b0`). The merge keeps a as its NODE (`0 < 1`) and sends `b_old1`
    /// as its UNDOOBJECT (`1 !< 1`). The capture's S37
    /// `restored=[1:a0, 2:b0]` pins both sides AND the merge push order:
    /// flipping the merge `<` to `<=` lands `b_old1` itself and restores
    /// `2:b1` (mutation-verified before commit); dropping the node side
    /// loses `a` entirely. The undo right after the pop restores from
    /// the MERGED second-top list (at level 1, undo reads index
    /// `stack_level - 1` = 0 — where the merge landed).
    #[test]
    fn p5_pop_snapshot_merge_dichotomy_both_sides() {
        let mut s = Stack::new();
        s.insert(Reverse(1), Rec::new(1, "a0")); // S31
        s.insert(Reverse(2), Rec::new(2, "b0"));
        assert_eq!(show(&iter(&s)), "2:b0, 1:a0");
        s.generate_snapshot(); // S32 — b_old (b0, L0)
        s.save_for_undo(&Reverse(2));
        s.value_mut(&Reverse(2)).expect("b").value = "b1".to_string();
        assert_eq!(show(&iter(&s)), "2:b1, 1:a0");
        s.generate_snapshot(); // S33 — b_old1 (b1, L1), undo_object = b_old
        s.save_for_undo(&Reverse(2));
        s.value_mut(&Reverse(2)).expect("b").value = "b2".to_string();
        assert_eq!(show(&iter(&s)), "2:b2, 1:a0");
        assert!(s.delete(&Reverse(1))); // S34: a L0 < 2 -> NODE into top list
        assert_eq!(show(&iter(&s)), "2:b2");
        assert!(s.delete(&Reverse(2))); // S35: b live L2 == 2 -> UNDOOBJECT
        assert_eq!(show(&iter(&s)), "");
        assert!(s.pop_snapshot()); // S36: merge — a stays NODE; b_old1 -> b_old
        assert_eq!(show(&iter(&s)), "");
        assert_eq!(s.stack_level(), 1);
        let (cancelled, restored) = s.undo().expect("S37 undo"); // S37
        assert!(cancelled.is_empty());
        assert_eq!(show(&restored), "1:a0, 2:b0");
        assert_eq!(show(&iter(&s)), "2:b0, 1:a0");
        assert_eq!(s.stack_level(), 0);
    }
}
