//! Rich pasteboard (`NativePasteboard`) on the shell side: items keyed by UTType id, mapped to MIME types for the Wayland
//! clipboard. The plain-text clipboard path (`clipboard::set_text`) stays the carrier for text.

use std::sync::Mutex;

static STATE: Mutex<(u32, Vec<(String, Vec<u8>)>)> = Mutex::new((0, Vec::new()));

/// UTType id to MIME type (`public.utf8-plain-text` to `text/plain;charset=utf-8`, and so on).
pub fn mime_for(uti: &str) -> &str {
    match uti {
        "public.utf8-plain-text" | "public.plain-text" => "text/plain;charset=utf-8",
        "public.html" => "text/html",
        "public.rtf" => "text/rtf",
        "public.png" => "image/png",
        "public.jpeg" => "image/jpeg",
        "public.file-url" | "public.url" => "text/uri-list",
        other => other,
    }
}

/// Replaces the contents (`clearContents` + `setData`), bumps the change count, and returns the text to publish on the
/// plain clipboard: the plain-text item, else the file URLs, else the HTML source.
pub fn set(items: Vec<(String, Vec<u8>)>) -> Option<String> {
    let text = ["public.utf8-plain-text", "public.file-url", "public.html"]
        .iter()
        .find_map(|t| items.iter().find(|(k, _)| k == t))
        .and_then(|(_, d)| String::from_utf8(d.clone()).ok());
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    s.0 = s.0.wrapping_add(1);
    s.1 = items;
    text
}

/// Offers the typed items to external Wayland clients: one `wl-copy` per MIME type would replace the last, so the
/// richest type is offered with `--type` (png, html, rtf, uri-list); plain text stays on the shared clipboard path.
/// Best effort: a missing `wl-copy` is ignored. Gap: a multi-MIME data source needs the presenter's data device.
pub fn offer_wayland(items: &[(String, Vec<u8>)]) {
    use std::io::Write;
    let pick = ["public.png", "public.html", "public.rtf", "public.file-url"].iter().find_map(|t| items.iter().find(|(k, _)| k == t));
    let Some((uti, data)) = pick else { return };
    let child = std::process::Command::new("wl-copy").args(["--type", mime_for(uti)]).stdin(std::process::Stdio::piped()).spawn();
    if let Ok(mut c) = child {
        if let Some(mut i) = c.stdin.take() { let _ = i.write_all(data); }
        std::thread::spawn(move || { let _ = c.wait(); });
    }
}

/// The change count and items now on the pasteboard.
pub fn snapshot() -> (u32, Vec<(String, Vec<u8>)>) {
    let s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    (s.0, s.1.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maps_and_counts() {
        assert_eq!(mime_for("public.png"), "image/png");
        let t = set(vec![("public.html".into(), b"<i>x</i>".to_vec())]);
        assert_eq!(t.as_deref(), Some("<i>x</i>"));
        assert!(snapshot().0 >= 1);
    }
}
