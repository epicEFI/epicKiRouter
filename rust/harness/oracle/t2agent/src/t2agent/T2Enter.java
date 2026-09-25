package t2agent;

import net.bytebuddy.asm.Advice;
import net.bytebuddy.implementation.bytecode.assign.Assigner;

/**
 * M5-T2 (buglog 189 lever (a); SOURCES COMMITTED at rust/harness/oracle/t2agent/ - the BUILT
 * FAT JAR is the uncommitted part, built per README.md): the OnMethodEnter advice for
 * completeShape — logs the input row. All app types are passed as Object with DYNAMIC typing so
 * the advice class never needs the app classes to load; the body is INLINED into the target
 * method, so it runs with the target's access rights.
 */
public class T2Enter {

  @Advice.OnMethodEnter
  public static int enter(
      @Advice.This(typing = Assigner.Typing.DYNAMIC) Object self,
      @Advice.Argument(value = 0, typing = Assigner.Typing.DYNAMIC) Object room,
      @Advice.Argument(1) int netNumber,
      @Advice.Argument(value = 2, typing = Assigner.Typing.DYNAMIC) Object ignoreObject,
      @Advice.Argument(value = 3, typing = Assigner.Typing.DYNAMIC) Object ignoreShape) {
    return T2Log.enterRow(self, room, netNumber, ignoreObject, ignoreShape);
  }
}
