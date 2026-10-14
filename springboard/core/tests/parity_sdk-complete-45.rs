//! Parity sdk-complete-45: printer interfaces that can carry print data, the printer name for a list, and the process
//! records parsed from procfs text with their parent chains and start and exit events. Headless: no device is opened
//! and no file is read.

use bifrost_core::process_info::{parent_chain, parse_cmdline, parse_stat, snapshot_events, ProcStat, ProcessEvent};
use bifrost_core::usb_descriptors::{EndpointDescriptor, InterfaceDescriptor};
use bifrost_core::usb_printer::{can_print, parse_device_id};

const BULK: u8 = 0x02;
const INTERRUPT: u8 = 0x03;

fn endpoint(address: u8, attributes: u8) -> EndpointDescriptor {
    EndpointDescriptor { address, attributes, max_packet_size: 64, interval: 0 }
}

fn interface(class: u8, endpoints: Vec<EndpointDescriptor>) -> InterfaceDescriptor {
    InterfaceDescriptor { number: 0, alternate: 0, class, subclass: 1, protocol: 2, endpoints }
}

fn device_id(body: &str) -> Vec<u8> {
    let total = body.len() + 2;
    let mut out = vec![(total >> 8) as u8, total as u8];
    out.extend_from_slice(body.as_bytes());
    out
}

#[test]
fn a_printer_interface_with_a_bulk_out_endpoint_can_print() {
    let iface = interface(7, vec![endpoint(0x02, BULK), endpoint(0x81, BULK)]);
    assert!(can_print(&iface));
}

#[test]
fn a_printer_interface_with_only_a_bulk_in_endpoint_cannot_print() {
    let iface = interface(7, vec![endpoint(0x81, BULK)]);
    assert!(!can_print(&iface), "print data has to go out, so a bulk-out pipe is needed");
}

#[test]
fn an_interrupt_out_endpoint_is_not_a_print_data_path() {
    let iface = interface(7, vec![endpoint(0x03, INTERRUPT)]);
    assert!(!can_print(&iface));
}

#[test]
fn an_interface_of_another_class_cannot_print_even_with_bulk_pipes() {
    let iface = interface(0xff, vec![endpoint(0x02, BULK), endpoint(0x81, BULK)]);
    assert!(!can_print(&iface));
}

#[test]
fn the_printer_name_joins_the_manufacturer_and_the_model() {
    let id = parse_device_id(&device_id("MFG:Canon;MDL:LBP2900;CMD:PCL,PS;")).unwrap();
    assert_eq!(id.display_name().as_deref(), Some("Canon LBP2900"));
}

#[test]
fn a_model_that_already_starts_with_the_manufacturer_is_shown_alone() {
    let id = parse_device_id(&device_id("MFG:HP;MDL:HP LaserJet 1020;")).unwrap();
    assert_eq!(id.display_name().as_deref(), Some("HP LaserJet 1020"));
}

#[test]
fn the_long_keys_and_a_missing_manufacturer_still_give_a_name() {
    let long = parse_device_id(&device_id("MANUFACTURER:Acme;MODEL:Office Jet;")).unwrap();
    assert_eq!(long.display_name().as_deref(), Some("Acme Office Jet"));
    let model_only = parse_device_id(&device_id("MDL:Printer X;")).unwrap();
    assert_eq!(model_only.display_name().as_deref(), Some("Printer X"));
    let maker_only = parse_device_id(&device_id("MFG:Acme;")).unwrap();
    assert_eq!(maker_only.display_name().as_deref(), Some("Acme"));
}

#[test]
fn a_device_id_with_neither_manufacturer_nor_model_has_no_name() {
    let id = parse_device_id(&device_id("CMD:PCL;")).unwrap();
    assert_eq!(id.display_name(), None);
    let blank = parse_device_id(&device_id("MFG: ;MDL:  ;")).unwrap();
    assert_eq!(blank.display_name(), None, "blank values do not count as a name");
}

#[test]
fn a_stat_line_gives_pid_name_state_and_parent() {
    let p = parse_stat("42 (my app) S 1 42 42 0 -1 4194560").unwrap();
    assert_eq!(p, ProcStat { pid: 42, comm: "my app".into(), state: 'S', ppid: 1 });
    assert!(!p.is_zombie());
}

#[test]
fn a_command_name_with_parentheses_runs_from_the_first_to_the_last_bracket() {
    let p = parse_stat("7 (a (b) c) R 6 7 7 0").unwrap();
    assert_eq!((p.comm.as_str(), p.state, p.ppid), ("a (b) c", 'R', 6));
}

#[test]
fn a_zombie_is_reported_as_one() {
    assert!(parse_stat("9 (done) Z 1 9").unwrap().is_zombie());
}

#[test]
fn a_malformed_stat_line_is_refused() {
    assert_eq!(parse_stat("42 my app S 1"), None, "no parentheses");
    assert_eq!(parse_stat("42 ) x ("), None, "the brackets are reversed");
    assert_eq!(parse_stat("x (app) S 1"), None, "the pid is not a number");
    assert_eq!(parse_stat("42 (app) S"), None, "the parent is missing");
}

#[test]
fn a_command_line_splits_on_nul_bytes_and_keeps_empty_arguments() {
    assert_eq!(parse_cmdline(b"/usr/bin/app\0--flag\0value\0"), ["/usr/bin/app", "--flag", "value"]);
    assert_eq!(parse_cmdline(b"a\0\0"), ["a", ""]);
    assert_eq!(parse_cmdline(b"a"), ["a"], "a missing final NUL still ends the argument");
    assert!(parse_cmdline(b"").is_empty(), "a kernel thread has no arguments");
}

fn table() -> Vec<ProcStat> {
    vec![
        ProcStat { pid: 1, comm: "init".into(), state: 'S', ppid: 0 },
        ProcStat { pid: 10, comm: "shell".into(), state: 'S', ppid: 1 },
        ProcStat { pid: 20, comm: "terminal".into(), state: 'S', ppid: 10 },
        ProcStat { pid: 30, comm: "tool".into(), state: 'R', ppid: 20 },
    ]
}

#[test]
fn the_parent_chain_runs_from_the_nearest_parent_up_to_init() {
    assert_eq!(parent_chain(&table(), 30), [20, 10, 1]);
    assert!(parent_chain(&table(), 1).is_empty(), "init has no parent");
    assert!(parent_chain(&table(), 99).is_empty(), "an unknown process has no chain");
}

#[test]
fn the_parent_chain_stops_when_a_parent_is_missing_or_repeats() {
    let gone = vec![ProcStat { pid: 30, comm: "tool".into(), state: 'S', ppid: 77 }];
    assert_eq!(parent_chain(&gone, 30), [77], "the parent is named even when it is not in the table");
    let cycle = vec![
        ProcStat { pid: 5, comm: "a".into(), state: 'S', ppid: 6 },
        ProcStat { pid: 6, comm: "b".into(), state: 'S', ppid: 5 },
    ];
    assert_eq!(parent_chain(&cycle, 5), [6], "a cycle in the table ends the walk");
}

#[test]
fn two_snapshots_give_the_exits_and_the_starts_between_them() {
    let before = table();
    let mut after = table();
    after.retain(|p| p.pid != 20);
    after.push(ProcStat { pid: 40, comm: "editor".into(), state: 'R', ppid: 10 });
    let events = snapshot_events(&before, &after);
    assert_eq!(
        events,
        vec![
            ProcessEvent::Exited { pid: 20, command: "terminal".into() },
            ProcessEvent::Exited { pid: 30, command: "tool".into() },
            ProcessEvent::Started { pid: 40, parent: 10, command: "editor".into() },
        ]
    );
}

#[test]
fn a_pid_reused_by_another_command_is_an_exit_and_a_start() {
    let before = vec![ProcStat { pid: 40, comm: "a".into(), state: 'S', ppid: 1 }];
    let after = vec![ProcStat { pid: 40, comm: "b".into(), state: 'S', ppid: 1 }];
    assert_eq!(
        snapshot_events(&before, &after),
        vec![
            ProcessEvent::Exited { pid: 40, command: "a".into() },
            ProcessEvent::Started { pid: 40, parent: 1, command: "b".into() },
        ]
    );
}

#[test]
fn identical_snapshots_give_no_events() {
    assert!(snapshot_events(&table(), &table()).is_empty());
}
