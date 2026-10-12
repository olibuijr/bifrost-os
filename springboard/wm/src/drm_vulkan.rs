//! The DRM backend's Vulkan entry: finds the render node of the scanout GPU and opens the render server on it. The
//! frame loop (GBM scanout ring, `Server::frame`, atomic commit with the submission's in-fence, `FrameClock`) is not
//! wired yet; `drm.rs` keeps scanning out through GLES and only logs the probe result (wave G2, `wm-drm-vulkan`).

use std::path::{Path, PathBuf};

use bifrost_gpu::PixelFormat;

use crate::render_server::server::Server;

/// The render node name (`renderD128`) among the entries of a card's `device/drm` sysfs directory, or `None`.
/// The card's directory lists `card0` and `renderD128` next to each other; only `renderD*` names count.
pub fn render_node_name(entries: &[String]) -> Option<String> {
    let mut nodes: Vec<&String> = entries.iter().filter(|e| is_render_node(e)).collect();
    nodes.sort();
    nodes.first().map(|s| (*s).clone())
}

fn is_render_node(name: &str) -> bool {
    name.strip_prefix("renderD").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The render node path for a primary node such as `/dev/dri/card0`: the `renderD*` entry under
/// `/sys/class/drm/card0/device/drm`, placed in the same `/dev/dri` directory. `None` when sysfs has no such entry.
pub fn render_node_for_card(card: &Path) -> Option<PathBuf> {
    let card_name = card.file_name()?.to_str()?;
    let sysfs = Path::new("/sys/class/drm").join(card_name).join("device/drm");
    let entries: Vec<String> = std::fs::read_dir(sysfs)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let node = render_node_name(&entries)?;
    Some(card.parent()?.join(node))
}

/// Opens the render server on the scanout GPU (`card`, the seat's primary GPU). Any failure is returned as text for
/// the log; the caller keeps the GLES path.
pub fn probe(card: &Path) -> Result<Server, String> {
    let node = render_node_for_card(card).ok_or_else(|| format!("no render node for {}", card.display()))?;
    Server::new(Some(&node), PixelFormat::Bgra8Unorm).map_err(|e| format!("{}: {e}", node.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_render_node_is_the_renderd_entry_next_to_the_card() {
        assert_eq!(render_node_name(&names(&["card0", "renderD128"])), Some("renderD128".into()));
    }

    #[test]
    fn card_and_control_entries_are_not_render_nodes() {
        assert_eq!(render_node_name(&names(&["card1", "controlD64"])), None);
    }

    #[test]
    fn with_two_render_nodes_the_lowest_number_wins() {
        assert_eq!(render_node_name(&names(&["renderD129", "card0", "renderD128"])), Some("renderD128".into()));
    }

    #[test]
    fn a_bare_renderd_prefix_without_a_number_is_rejected() {
        assert!(!is_render_node("renderD"));
        assert!(!is_render_node("renderDx"));
    }

    #[test]
    fn a_prefix_match_with_trailing_text_is_rejected() {
        assert!(!is_render_node("renderD128-old"));
        assert!(!is_render_node("xrenderD128"));
    }

    #[test]
    fn an_empty_listing_has_no_render_node() {
        assert_eq!(render_node_name(&[]), None);
    }

    #[test]
    fn a_three_digit_node_number_is_accepted() {
        assert!(is_render_node("renderD129"));
        assert!(is_render_node("renderD1000"));
    }
}
