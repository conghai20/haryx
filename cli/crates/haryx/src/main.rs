//! `haryx`: command-line entry point. Argument parsing and rendering only; logic lives in `haryx-core`.

fn main() {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("{} {} (claude-swap {})", haryx_core::PROG, haryx_core::VERSION, haryx_core::UPSTREAM_VERSION);
        return;
    }
    eprintln!("{}: not implemented yet", haryx_core::PROG);
    std::process::exit(2);
}
