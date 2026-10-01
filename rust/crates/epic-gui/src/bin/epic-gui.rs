//! The epic-gui bin: the desktop shell's entry face. DEFAULT-OFF:
//! without the `desktop` feature this file compiles to a 5-line stub
//! (the hint + exit 1) so the default workspace build never compiles
//! the wgpu tree (the census-pinnability law's bin face).

#[cfg(feature = "desktop")]
fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match epic_gui::shell::parse_launch_args(argv.into_iter()) {
        Ok(epic_gui::shell::LaunchMode::Version) => {
            // The version face (M10-T6): printed and exited BEFORE any
            // eframe/wgpu work — no DISPLAY required (the shipped
            // desktop-only bin answers it headlessly). The line is the
            // single-sourced `shell::version_line` (pinned ungated in
            // shell.rs, AM5 lesson 14's derived-form equality).
            println!("{}", epic_gui::shell::version_line());
            std::process::exit(0);
        }
        Ok(mode) => match epic_gui::desktop::run(mode) {
            Ok(code) => std::process::exit(code),
            Err(text) => {
                eprintln!("epic-gui: {text}");
                std::process::exit(1);
            }
        },
        Err(usage) => {
            eprintln!("epic-gui: {usage}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(feature = "desktop"))]
fn main() {
    eprintln!(
        "epic-gui: the desktop shell is not compiled — the `desktop` feature is \
         default-off; rebuild with --features desktop"
    );
    std::process::exit(1);
}
