//! On-device launch-boundary verification; no graphics or audio.
fn main() {
    let args:Vec<_>=std::env::args().collect();
    let network=args.get(3).is_some_and(|s|s=="network");
    let mut cmd=springboard::bare::app_policy::command("is.bifrost.policy.probe",std::path::Path::new(&args[1]),std::path::Path::new(&args[2]),springboard::bare::app_policy::Policy{network,..Default::default()},false).unwrap();
    cmd.env("PROBE_NETWORK",if network{"1"}else{"0"});
    assert!(cmd.status().unwrap().success());
}
