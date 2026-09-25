package t2agent;

import java.lang.instrument.Instrumentation;
import net.bytebuddy.agent.builder.AgentBuilder;
import net.bytebuddy.matcher.ElementMatchers;

/**
 * M5-T2 (buglog 189 lever (a)): a premain javaagent that lands the buglog-189
 * recording instrument INSIDE the real CLI process. These SOURCES are committed here;
 * the BUILT FAT JAR stays uncommitted (build it from this tree per README.md into
 * logs/, which is git-ignored). The recording form and the interception seam (why an
 * in-place class transform; the exact SearchTreeManager hardcode + package-private
 * base-constructor blockage) are anchored in README.md; agent options live in T2Log's
 * options block. Coverage: the 45-degree tree (the bm06 fork face), the 90-degree tree,
 * and the base ShapeSearchTree arm - each completeShape call is instrumented exactly
 * once (the subclasses override without delegating to the base method).
 */
public class T2Agent {

  public static void premain(String args, Instrumentation inst) {
    T2Log.init(args);
    T2Log.registerShutdown();
    // ByteBuddy must be allowed to read the newest class files the JDK 25 frozen jar carries.
    System.setProperty("net.bytebuddy.experimental", "true");
    try {
      new AgentBuilder.Default()
          .with(AgentBuilder.RedefinitionStrategy.DISABLED)
          .disableClassFormatChanges()
          .type(
              ElementMatchers.named("app.freerouting.board.searchtree.ShapeSearchTree45Degree")
                  .or(
                      ElementMatchers.named(
                          "app.freerouting.board.searchtree.ShapeSearchTree90Degree"))
                  .or(
                      ElementMatchers.named(
                          "app.freerouting.board.searchtree.ShapeSearchTree")))
          .transform(
              (builder, typeDescription, classLoader, module, protectionDomain) ->
                  builder.visit(
                      net.bytebuddy.asm.Advice.to(T2Enter.class, T2Exit.class)
                          .on(
                              ElementMatchers.named("completeShape")
                                  .and(ElementMatchers.takesArguments(4)))))
          .installOn(inst);
      T2Log.logRaw("# T2Agent installed on " + inst);
    } catch (Throwable t) {
      T2Log.logRaw("# T2Agent INSTALL FAILED: " + t);
      for (StackTraceElement e : t.getStackTrace()) {
        T2Log.logRaw("#   at " + e);
      }
    }
  }
}
