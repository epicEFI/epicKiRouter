package t2agent;

import java.util.Collection;
import net.bytebuddy.asm.Advice;
import net.bytebuddy.implementation.bytecode.assign.Assigner;

/**
 * M5-T2 (buglog 189 lever (a); SOURCES COMMITTED at rust/harness/oracle/t2agent/ - the BUILT
 * FAT JAR is the uncommitted part, built per README.md): the OnMethodExit advice for
 * completeShape — logs the result row, and at the fork-face rows (the band trigger in T2Log)
 * fires the north-band tree dump. The return is typed Collection with DYNAMIC typing; the band
 * dump is read-only (leaf walk + getters only). Exceptional exits: ByteBuddy's default
 * onThrowable=true fires this advice on a thrown completeShape too — the result is then null,
 * which T2Log.exitRow logs as size=-1 with no room columns, so enter/exit row pairing stays
 * balanced.
 */
public class T2Exit {

  @Advice.OnMethodExit
  public static void exit(
      @Advice.This(typing = Assigner.Typing.DYNAMIC) Object self,
      @Advice.Argument(value = 0, typing = Assigner.Typing.DYNAMIC) Object room,
      @Advice.Argument(1) int netNumber,
      @Advice.Enter int rowNo,
      @Advice.Return(typing = Assigner.Typing.DYNAMIC, readOnly = false) Collection<?> result) {
    if (rowNo >= 0) {
      T2Log.exitRow(self, room, netNumber, result, rowNo);
    }
    // readOnly semantics (ByteBuddy 1.17.7): readOnly=true (the default) is the documented
    // read-only mode — an advice assignment to the parameter would be silently DROPPED; it is
    // NOT required to declare the supertype Collection<?> (read-only permits it). This advice
    // never assigns the parameter, so either flag inlines identically; readOnly=false is kept
    // only to pin the already-verified inlining - do not flip it without re-verifying the
    // inlined typing. Transparency: T2Log never throws and only reads.
  }
}
