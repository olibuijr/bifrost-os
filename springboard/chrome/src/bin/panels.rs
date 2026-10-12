fn main() {
    if let Err(e) = bifrost_chrome::panels::run() {
        eprintln!("bifrost-panels: {e}");
        std::process::exit(1);
    }
}
