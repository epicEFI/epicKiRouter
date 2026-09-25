package t2agent;

import java.lang.instrument.Instrumentation;
import net.bytebuddy.agent.builder.AgentBuilder;
import net.bytebuddy.matcher.ElementMatchers;

/**
 * M5-T2 (buglog 189 lever (a)): a premain javaagent that lands the buglog-189
 * recording instrument INSIDE the real CLI process. These SOURCES are committed here;
 * the BUILT FAT JAR stays uncommitted (build it from this tree per README.md into
 * logs/, which is git-ignored). The recording form is the M4-T12
 * instrument_T12RecordingTree45 subclass form ported to an in-place class transform: the CLI
 * constructs its class-1 autoroute tree as a hardcoded {@code new ShapeSearchTree45Degree(board, 1)}
 * (SearchTreeManager.java:154) and the base ShapeSearchTree constructor is package-private, so an
 * out-of-package subclass cannot even be constructed — the recording lives on the very same
 * completeShape method the T12 subclass would have hooked, via ByteBuddy Advice inlined into
 * ShapeSearchTree45Degree (and the ShapeSearchTree base for the 90Degree-free path).
 *
 * <p>Agent options (the -javaagent argument string): {@code log=<path>}, {@code net=<n>} (default
 * 21), {@code maxrows=<n>}.
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
