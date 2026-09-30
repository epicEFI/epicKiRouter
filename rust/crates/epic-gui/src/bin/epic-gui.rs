//! The epic-gui bin: the desktop shell's entry face. DEFAULT-OFF:
//! without the `desktop` feature this file compiles to a 5-line stub
//! (the hint + exit 1) so the default workspace build never compiles
//! the wgpu tree (the census-pinnability law's bin face).

#[cfg(feature = "desktop")]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match epic_gui::shell::parse_launch_args(args.into_iter()) {
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
