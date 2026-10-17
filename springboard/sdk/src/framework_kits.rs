//! App-facing names for the framework models that live in `bifrost_ui`.
//!
//! Each group below is one reference framework, and each module in it is the model that does the framework's job (a
//! scene graph, a sound space, a model store, a certificate panel). The models are pure and host-tested in `bifrost_ui`
//! so the shell and the apps share one implementation; this module gives an app one SDK path to all of them
//! (`bifrost_app::framework_kits::scenekit::scene3d::Scene`) without a direct dependency on the UI crate. The tests
//! here prove the paths resolve and that a model works when reached through them; the models' own tests are in their
//! modules.

/// 3D scene graph, cameras, lights, geometry, physics, particles, skinning, positional audio, a CPU rasteriser and
/// scene import.
pub mod scenekit {
    pub use bifrost_ui::{
        linalg, morph_skin, particle_system3d, physics3d, primitives3d, raster3d, scene3d, scene_action, scene_audio,
        scene_source, scene_view, shader_library, shading,
    };
}

/// 2D scene loop, nodes, actions, atlases, tile maps, physics, particles, constraints and warp geometry.
pub mod spritekit {
    pub use bifrost_ui::{
        emitter, node_constraint, physics, scene2d, scene_loop, sk_action, sk_atlas, sk_node, sk_tilemap, tilemap, warp,
    };
}

/// Spatial audio: listener and sources, occluders, rooms, distance models, sound events, envelopes and groups.
pub mod phase {
    pub use bifrost_ui::{sound_distance, sound_envelope, sound_event, sound_group, sound_room, sound_space};
}

/// Sound classification of files and streams, with results and observers.
pub mod sound_analysis {
    pub use bifrost_ui::{sound_analyzer, sound_classify, sound_stream};
}

/// Screen and app recording sessions, the clip buffer and broadcast.
pub mod replaykit {
    pub use bifrost_ui::{broadcast, clip_buffer, recording};
}

/// Model schema, the record store with unique and relation rules, change history, data stores and record sync.
pub mod swiftdata {
    pub use bifrost_ui::{data_store, history, model_schema, model_store, record_sync, relations};
}

/// Certificate panel and trust editor, the identity chooser and the keychain save and settings panels.
pub mod security_interface {
    pub use bifrost_ui::{certificate_panel, certificate_trust, identity_chooser, keychain_save, keychain_settings};
}

/// PDF display, pages, forms, selection, access permissions, overlays and annotations.
pub mod pdfkit {
    pub use bifrost_ui::{acroform, annotation, pdf_access, pdf_overlay, pdfform, pdfpage, pdfselect, pdfview};
}

/// Symbol effects and the layered symbol rendering model.
pub mod symbols {
    pub use bifrost_ui::{symbol_effect, symbol_layers};
}

/// Settings panes: the pane host with its lifecycle, the manifest format and pane links.
pub mod preference_panes {
    pub use bifrost_ui::{pane, pane_link};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_is_built_and_searched_through_the_sdk_path() {
        let mut scene = scenekit::scene3d::Scene::new();
        let root = scene.root();
        let camera = scene.add_child(root, "camera").expect("the root is in the scene");
        assert_eq!(scene.find("camera"), Some(camera));
        assert_eq!(scene.parent(camera), Some(root));
        assert_eq!(scene.node_count(), 2);
    }

    #[test]
    fn a_tile_set_hands_out_group_indices_through_the_sdk_path() {
        let mut set = spritekit::tilemap::TileSet::new();
        assert_eq!(set.add_group("grass"), 0);
        assert_eq!(set.add_group("water"), 1);
        assert_eq!(set.group_name(1), Some("water"));
    }

    #[test]
    fn a_listener_hears_a_source_straight_ahead_in_the_middle() {
        let listener = phase::sound_space::Listener::new([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0])
            .expect("a forward and an up vector that are not parallel");
        let heard = listener.hear([0.0, 0.0, 2.0]).expect("a source away from the listener is heard");
        assert!((heard.distance - 2.0).abs() < 1e-5);
        assert!(heard.pan.abs() < 1e-5);
    }

    #[test]
    fn a_classify_request_checks_its_labels_and_sizes_its_windows() {
        let request = sound_analysis::sound_classify::ClassifyRequest::new(1.0, 0.5, vec!["speech".to_string()])
            .expect("a one second window with half overlap");
        assert_eq!(request.window_frames(1000), 1000);
        assert_eq!(request.hop_frames(1000), 500);
        assert!(sound_analysis::sound_classify::ClassifyRequest::new(1.0, 0.5, Vec::new()).is_err());
    }

    #[test]
    fn a_recording_session_reports_how_long_it_ran() {
        let mut recorder = replaykit::recording::Recorder::new(true);
        recorder.start(1.0, true).expect("an available recorder starts");
        assert!(recorder.is_recording());
        let finished = recorder.stop(3.5).expect("a running session stops");
        assert_eq!(finished.duration, 2.5);
        assert!(finished.microphone);
        assert!(!recorder.is_recording());
    }

    #[test]
    fn a_model_store_inserts_and_fetches_records_through_the_sdk_path() {
        let mut store = swiftdata::model_store::Store::new();
        let id = store.insert("Note", &[("title", "one")]).expect("an entity without a schema takes any field");
        store.insert("Note", &[("title", "two")]).expect("a second record");
        let titles = store.fetch("Note", |r| r.field("title") == Some("one"));
        assert_eq!(titles.len(), 1);
        assert_eq!(store.get(id).and_then(|r| r.field("title")), Some("one"));
    }

    #[test]
    fn a_certificate_panel_reports_the_validity_of_the_chain() {
        use security_interface::certificate_panel::{CertificateInfo, CertificatePanel, Validity};
        let info = CertificateInfo {
            subject: "example.test".into(),
            issuer: "Example Root".into(),
            serial: "01".into(),
            not_before: 100,
            not_after: 200,
        };
        let panel = CertificatePanel::new(vec![info]).expect("a well formed one certificate chain");
        assert_eq!(panel.status(0, 150).expect("certificate 0"), Validity::Valid);
        assert_eq!(panel.status(0, 250).expect("certificate 0"), Validity::Expired);
        assert!(panel.chain_valid(150));
        assert!(!panel.chain_valid(50));
    }

    #[test]
    fn a_pdf_view_counts_its_pages_through_the_sdk_path() {
        let mut view = pdfkit::pdfview::PdfView::new(600.0, 800.0);
        view.set_pages(&[(612.0, 792.0), (612.0, 792.0)]).expect("two letter pages");
        assert_eq!(view.page_count(), 2);
        assert!(view.set_pages(&[(0.0, 792.0)]).is_err());
    }

    #[test]
    fn a_bounce_is_a_discrete_effect_that_ends_at_rest() {
        use symbols::symbol_effect::{Effect, Options, Pose, SymbolEffect};
        let effect = SymbolEffect::new(Effect::Bounce, Options::default(), 10.0);
        assert!(effect.is_discrete());
        assert!(effect.is_done(10.7));
        assert_eq!(effect.pose(11.0), Pose::REST);
    }

    #[test]
    fn a_pane_host_starts_with_nothing_selected_and_reads_a_manifest() {
        let host = preference_panes::pane::PaneHost::new();
        assert_eq!(host.selected(), None);
        assert!(preference_panes::pane::parse_manifest("name = demo\ntitle = Demo").is_ok());
        assert!(preference_panes::pane::parse_manifest("title = no name").is_err());
    }
}
