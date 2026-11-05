use super::Manifest;

#[test]
fn service_tables_are_read_and_do_not_leak_into_the_app_keys() {
    let m = Manifest::parse(
        "id = \"is.olibuijr.preview.desktop\"\nname = \"Viewer\"\nversion = \"0.11.8\"\nexec = \"bifrost-viewer\"\n\n[[service]]\ntitle = \"Open Images in Viewer\"\nsend_types = [\"public.image\", \"public.pdf\"]\nreturn_types = []\nmethod = \"open_images\"\n\n[[service]]\ntitle = \"Copy Path\"\nsend_types = [\"bifrost.file-path-text\"]\nreturn_types = [\"public.utf8-plain-text\"]\nmethod = \"copy_path\"\n",
    )
    .unwrap();
    assert_eq!(m.id, "is.olibuijr.preview.desktop");
    assert_eq!(m.name, "Viewer");
    assert!(m.intents.is_empty());
    assert_eq!(m.services.len(), 2);
    assert_eq!(m.services[0].title, "Open Images in Viewer");
    assert_eq!(m.services[0].send_types, vec!["public.image".to_string(), "public.pdf".to_string()]);
    assert!(m.services[0].return_types.is_empty());
    assert_eq!(m.services[0].method, "open_images");
    assert_eq!(m.services[1].return_types, vec!["public.utf8-plain-text".to_string()]);
    assert_eq!(m.services[1].rpc_method(), "service.copy_path");
}

#[test]
fn a_manifest_without_services_has_none() {
    let m = Manifest::parse("id = \"a.b\"\nname = \"X\"\nexec = \"x\"\n").unwrap();
    assert!(m.services.is_empty());
}
