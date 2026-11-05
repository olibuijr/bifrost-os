use super::*;
#[test]
fn review_registry_update_preserves_slots_and_old_snapshot_ownership() {
    let app=|id:&str,version:&str| Installed {manifest:Manifest{id:id.into(),version:version.into(),exec:"app".into(),..Manifest::default()},dir:PathBuf::from(format!("/apps/{id}/{version}"))};
    let old:std::sync::Arc<[Installed]>=vec![app("a","1"),app("b","1")].into();
    let current=merge_installed(&old,vec![app("b","2"),app("c","1"),app("a","2")]);
    assert_eq!(current.iter().map(|a|a.manifest.id.as_str()).collect::<Vec<_>>(),vec!["a","b","c"]);
    assert_eq!(current[0].exec_path(),PathBuf::from("/apps/a/2/app"));
    assert_eq!(current[1].manifest.version,"2");
    assert_eq!(old[0].manifest.version,"1");
    assert_eq!(merge_installed(&current,current.clone()),current);
}

#[test]
fn an_uninstalled_app_keeps_its_slot_but_loses_its_permissions() {
    let app=|id:&str| Installed {manifest:Manifest{id:id.into(),name:id.into(),exec:"app".into(),permissions:vec!["accessibility".into(),"storage".into()],..Manifest::default()},dir:PathBuf::from(format!("/nowhere/{id}"))};
    let old=vec![app("a"),app("b"),app("c")];
    let now=merge_installed(&old,vec![app("a"),app("c")]);
    assert_eq!(now.iter().map(|a|a.manifest.id.as_str()).collect::<Vec<_>>(),vec!["a","b","c"],"the slots do not move");
    assert!(now[1].is_removed() && now[1].manifest.permissions.is_empty() && now[1].manifest.exec.is_empty());
    assert_eq!(now[1].manifest.name,"b");
    assert!(!now[0].is_removed() && !now[2].is_removed());
    // installing it again fills the same slot
    let back=merge_installed(&now,vec![app("a"),app("b"),app("c")]);
    assert!(!back[1].is_removed() && back[1].manifest.permissions.len()==2);
    // an entry that is already removed stays removed and is stable
    assert_eq!(merge_installed(&now,vec![app("a"),app("c")]),now);
}

#[test]
fn packages_beyond_the_limit_are_not_added() {
    let app=|i:usize| Installed {manifest:Manifest{id:format!("app{i}"),exec:"app".into(),..Manifest::default()},dir:PathBuf::from(format!("/nowhere/{i}"))};
    let scanned:Vec<Installed>=(0..MAX_INSTALLED+3).map(app).collect();
    let all=merge_installed(&[],scanned);
    assert_eq!(all.len(),MAX_INSTALLED);
}
