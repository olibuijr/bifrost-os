fn main() {
    if let Err(e) = bifrost_chrome::dock::run() {
        eprintln!("bifrost-dock: {e}");
        std::process::exit(1);
    }
}
