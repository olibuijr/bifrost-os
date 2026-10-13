//! The fcitx5 bridge for CJK input. When fcitx5 is installed, switching to the Japanese (Romaji) source selects its
//! kana input method (`mozc`), and switching to a Latin source deactivates the engine, so apps that take their text
//! through fcitx5 follow the source. The shell's own Search field keeps its in-process kana composition
//! (`bifrost_ui::kana_method`); no dictionary is shipped, so the kana and pinyin candidates come from fcitx5 only.
//! Without fcitx5 on the PATH nothing runs.

use super::kb_layouts::Layout;
use std::ffi::OsStr;
use std::process::Command;

const REMOTE: &str = "fcitx5-remote";

/// The fcitx5 input method a source selects; `None` when the source uses none.
pub fn input_method(source: Layout) -> Option<&'static str> {
    match source {
        Layout::Japanese => Some("mozc"),
        Layout::Icelandic | Layout::English => None,
    }
}

/// The `fcitx5-remote` arguments for a switch to `source`: select the input method, or deactivate the engine.
pub fn remote_args(source: Layout) -> Vec<String> {
    match input_method(source) {
        Some(im) => vec!["-s".into(), im.into()],
        None => vec!["-c".into()],
    }
}

/// Whether `fcitx5-remote` is on the PATH.
pub fn installed() -> bool {
    installed_in(std::env::var_os("PATH").as_deref())
}

fn installed_in(path: Option<&OsStr>) -> bool {
    path.is_some_and(|p| std::env::split_paths(p).any(|dir| dir.join(REMOTE).is_file()))
}

/// Tells fcitx5 about a switch to `source`. The command runs on its own thread so the shell never waits for it.
pub fn follow(source: Layout) {
    if !installed() {
        return;
    }
    let args = remote_args(source);
    let _ = std::thread::Builder::new().name("fcitx5-bridge".into()).spawn(move || {
        let _ = Command::new(REMOTE).args(&args).status();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn japanese_selects_the_kana_method_and_latin_sources_deactivate_fcitx() {
        assert_eq!(input_method(Layout::Japanese), Some("mozc"));
        assert_eq!(remote_args(Layout::Japanese), vec!["-s".to_string(), "mozc".to_string()]);
        assert_eq!(input_method(Layout::English), None);
        assert_eq!(remote_args(Layout::Icelandic), vec!["-c".to_string()]);
    }

    #[test]
    fn no_path_or_an_empty_path_means_fcitx_is_not_installed() {
        assert!(!installed_in(None));
        assert!(!installed_in(Some(OsStr::new(""))));
        assert!(!installed_in(Some(OsStr::new("/nonexistent/bifrost-fcitx-test"))));
    }
}
