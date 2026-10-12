fn main() {
    if let Err(e) = bifrost_chrome::menubar::run() {
        eprintln!("bifrost-menubar: {e}");
        std::process::exit(1);
    }
}
