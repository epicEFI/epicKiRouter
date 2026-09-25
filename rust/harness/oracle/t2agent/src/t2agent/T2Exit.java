package t2agent;

import java.util.Collection;
import net.bytebuddy.asm.Advice;
import net.bytebuddy.implementation.bytecode.assign.Assigner;

/**
 * SCRATCH (M5-T2): the OnMethodExit advice for completeShape — logs the result row, and at the
 * fork-face rows (net 21, single-result) triggers the north-band tree dump. The return is typed
 * Collection with DYNAMIC typing; the band dump is read-only (leaf walk + getters only).
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
    // readOnly=false is deliberate: ByteBuddy refuses an inlined exit advice that cannot write
    // the local; we never write it. Behavior stays transparent: T2Log never throws and only reads.
  }
}
