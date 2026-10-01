import app.freerouting.datastructures.UndoableObjects;

/**
 * M2 Task 2 jar spike: pins the exact UndoableObjects level-stack
 * semantics (T62) by driving a scripted sequence and printing the
 * observable surface after every step:
 *   - stack level (tracked by the script, the field is private)
 *   - readObject iteration (startReadObject/readObject loop)
 *   - cancelled/restored collections returned by undo/redo
 *   - return booleans of undo/redo/popSnapshot
 *
 * The Storable impl mirrors Item.compareTo (Item.java:94-103):
 * `item.id - id` — DESCENDING id under ConcurrentSkipListMap natural
 * order. clone() copies (Java saveForUndo clones the PASSED object).
 *
 * Run (JDK 25, from the repo root):
 *   ~/.jdks/jdk-25.0.4.1+1/bin/java -cp build/libs/freerouting-current-executable.jar \
 *       rust/harness/oracle/UndoSpike.java > /tmp/epic-t2-undo.out
 */
public final class UndoSpike {

  /** Storable mirror of Item: (id, value), compareTo descending id. */
  static final class Rec implements UndoableObjects.Storable {
    final int id;
    String value;

    Rec(int id, String value) {
      this.id = id;
      this.value = value;
    }

    @Override
    @SuppressWarnings("UnqualifiedMethodAccess")
    public int compareTo(Object other) {
      // Item.java:94-103 verbatim shape: item.id - id.
      if (other instanceof Rec rec) {
        return rec.id - id;
      }
      return 1;
    }

    @Override
    public Object clone() {
      return new Rec(id, value);
    }

    @Override
    public String toString() {
      return id + ":" + value;
    }
  }

  private final UndoableObjects objects = new UndoableObjects();
  private int step = 0;

  private void state() {
    StringBuilder sb = new StringBuilder("    iter=[");
    // startReadObject()/readObject(it) — the exact loop shape from
    // UndoableObjects.java:46-63 (readObject returns null at the end).
    java.util.Iterator<UndoableObjects.UndoableObjectNode> it = objects.startReadObject();
    while (true) {
      UndoableObjects.Storable next = objects.readObject(it);
      if (next == null) {
        break;
      }
      if (sb.length() > "    iter=[".length()) {
        sb.append(", ");
      }
      sb.append(next);
    }
    sb.append(']');
    System.out.println(sb);
  }

  private void step(String what) {
    step++;
    System.out.println("S" + step + " " + what);
    state();
  }

  private void stepUndoRedo(String what) {
    step++;
    System.out.println("S" + step + " " + what);
    java.util.LinkedList<UndoableObjects.Storable> cancelled = new java.util.LinkedList<>();
    java.util.LinkedList<UndoableObjects.Storable> restored = new java.util.LinkedList<>();
    boolean result;
    if (what.startsWith("undo")) {
      result = objects.undo(cancelled, restored);
    } else {
      result = objects.redo(cancelled, restored);
    }
    System.out.println("    -> " + result + " cancelled=" + cancelled + " restored=" + restored);
    state();
  }

  private Rec insert(int id, String value) {
    Rec rec = new Rec(id, value);
    objects.insert(rec);
    return rec;
  }

  private void run() {
    // ---- Phase 1: basics + the delete dichotomy (both sides + null) ----
    Rec a = insert(1, "a0");
    Rec b = insert(2, "b0");
    Rec c = insert(3, "c0");
    step("insert a=1:a0 b=2:b0 c=3:c0   [L0]");
    objects.generateSnapshot();
    step("generateSnapshot              [L1]");
    objects.saveForUndo(b);
    b.value = "b1";
    step("saveForUndo(b); b.value=b1");
    objects.delete(a);
    step("delete(a)   // a L0 < stack L1 -> NODE pushed (dichotomy side 1)");
    Rec d = insert(4, "d0");
    step("insert d=4:d0                 [L1, no undoObject]");
    objects.delete(d);
    step("delete(d)   // d L1 == stack, undoObject null -> NOTHING pushed");
    objects.saveForUndo(c);
    c.value = "c1";
    objects.delete(c);
    step("saveForUndo(c); c.value=c1; delete(c)  // == level, undoObject NON-null -> UNDOOBJECT pushed");

    // ---- Phase 2: second level, double undo, double redo ----
    objects.generateSnapshot();
    step("generateSnapshot              [L2]");
    Rec e = insert(5, "e0");
    step("insert e=5:e0                 [L2]");
    objects.delete(b);
    step("delete(b)   // b1 L1 < stack L2 -> NODE pushed");
    stepUndoRedo("undo   // -> L1: restores b1 via delete list; cancels e (readObject skip check)");
    stepUndoRedo("undo   // -> L0: b1 swapped for b0 (cross-link); a,c0 restored from delete list");
    stepUndoRedo("redo   // -> L1: b0 swapped for b1; delete-list walk re-deletes a, c0");
    stepUndoRedo("redo   // -> L2: e restored (level == stackLevel branch); b1 re-deleted");

    // ---- Phase 3: disableRedo truncation via mutation ----
    stepUndoRedo("undo   // -> L1 (redoPossible = true)");
    insert(6, "f0");
    step("insert f=6:f0  // disableRedo: truncate stack above L1, remove e (L2) from map, null redo at L1");
    stepUndoRedo("redo   // MUST be false: stack truncated by the mutation");
    objects.popSnapshot();
    step("popSnapshot // L1 -> L0: merge delete lists (a, c0 re-added at L1 by S13 walk now merge down)");

    // ---- Phase 4: three-level save chain, undo down, popSnapshot re-link ----
    objects.saveForUndo(a);
    step("saveForUndo(a)  // a at L0, stack L0 -> no-op branch (level < stackLevel false)");
    objects.generateSnapshot();
    step("generateSnapshot              [L1]");
    objects.saveForUndo(a);
    a.value = "a2";
    step("saveForUndo(a); a.value=a2    // old a1-node at L0");
    objects.generateSnapshot();
    step("generateSnapshot              [L2]");
    objects.saveForUndo(a);
    a.value = "a3";
    step("saveForUndo(a); a.value=a3    // old a2-node at L1 (three-node chain)");
    stepUndoRedo("undo   // -> L1: a3 swapped for a2-node");
    objects.popSnapshot();
    step("popSnapshot // L1 -> L0: re-link branch check (a2-node at L0 with redoObject at L1?)");
    stepUndoRedo("undo   // -> L0 if possible (else false) — residual state probe");
    objects.generateSnapshot();
    step("generateSnapshot              [L1]");
    insert(7, "g0");
    step("insert g=7:g0                 [L1]");
    stepUndoRedo("undo   // -> L0: g cancelled (created-new node stays in map invisible)");
    step("end");

    // ---- Phase 5 (FRESH instance): popSnapshot merge with CONTENT — both sides ----
    // Appended AFTER the original S30 "end" so the S1-S30 lines stay
    // byte-identical to the first capture (the committed pins quote them).
    // The residual state above cannot reach a content-bearing merge, so a
    // second instance drives it directly: a (L0 node) and b's second-save
    // old node (L1, with undoObject = the first-save clone) both sit in the
    // TOP delete list at pop time. The merge must send a down as its NODE
    // (level 0 < stackLevel-1) and b's entry as its UNDOOBJECT (level 1 not
    // < 1) — the undo right after the pop restores from the MERGED list, so
    // the restored values pin which side each entry took.
    mphase5();
  }

  private void mstep(String what, UndoableObjects m, StringBuilder extra) {
    step++;
    System.out.println("S" + step + " " + what);
    if (extra != null) {
      System.out.println("    " + extra);
    }
    StringBuilder sb = new StringBuilder("    iter=[");
    java.util.Iterator<UndoableObjects.UndoableObjectNode> it = m.startReadObject();
    while (true) {
      UndoableObjects.Storable next = m.readObject(it);
      if (next == null) {
        break;
      }
      if (sb.length() > "    iter=[".length()) {
        sb.append(", ");
      }
      sb.append(next);
    }
    sb.append(']');
    System.out.println(sb);
  }

  private void mphase5() {
    UndoableObjects m = new UndoableObjects();
    Rec ma = new Rec(1, "a0");
    m.insert(ma);
    Rec mb = new Rec(2, "b0");
    m.insert(mb);
    mstep("P5 insert a=1:a0 b=2:b0   [L0]", m, null);
    m.generateSnapshot();
    m.saveForUndo(mb);
    mb.value = "b1";
    mstep("P5 snapshot; saveForUndo(b); b.value=b1", m, null);
    m.generateSnapshot();
    m.saveForUndo(mb);
    mb.value = "b2";
    mstep("P5 snapshot; saveForUndo(b); b.value=b2  // b_old1 (b1, L1) undoObject=b_old (b0, L0)", m, null);
    m.delete(ma);
    mstep("P5 delete(a)   // a L0 < stack L2 -> NODE into TOP list", m, null);
    m.delete(mb);
    mstep("P5 delete(b)   // b live L2 == stack -> UNDOOBJECT (b_old1) into TOP list", m, null);
    boolean popped = m.popSnapshot();
    mstep("P5 popSnapshot // merge TOP into second-top: a stays NODE (0 < 1); b_old1's UNDOOBJECT b_old (1 !< 1) -> popped=" + popped, m, null);
    java.util.LinkedList<UndoableObjects.Storable> cancelled = new java.util.LinkedList<>();
    java.util.LinkedList<UndoableObjects.Storable> restored = new java.util.LinkedList<>();
    boolean undone = m.undo(cancelled, restored);
    mstep("P5 undo -> " + undone + " cancelled=" + cancelled + " restored=" + restored + "  // restores from the MERGED list", m, null);
  }

  public static void main(String[] args) {
    new UndoSpike().run();
  }
}
