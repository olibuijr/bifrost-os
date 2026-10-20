//! The app firewall's question and its Settings page.
//!
//! When an app the shell runs starts waiting for connections from the network and the app firewall has no choice for
//! it, the shell asks the person Allow or Deny on an alert card: "Do you want “X” to accept incoming network
//! connections?". Deny is the default, so an unanswered question never opens a port. The answer is saved through the
//! `app_firewall` service (`set`), which keeps `app-firewall.conf` and loads the nftables rules.
//!
//! Which app listens comes from the kernel: the shell reads `/proc/net/{tcp,tcp6,udp,udp6}` for sockets that wait for
//! traffic from elsewhere (`bifrost_core::app_firewall::parse_listeners`) and matches their inodes with the sockets
//! each hosted app, and the processes under it, holds open (`/proc/<pid>/fd`). It looks every [`SCAN_MS`]. An app asks
//! once per run; a Deny or Allow stays in the file, so it is not asked again after a restart either.
//!
//! An app whose choice is Block gets new ports it opens added to the rules without asking again: the choice was about
//! the app, not about one port.
//!
//! Settings > Network > Firewall lists the choices (`FirewallView`) and offers Allow, Block and Forget (the app asks
//! again) for each ([`Edit`]).

use super::list::{Group, Item, Spec, Tint};
use super::pages::{Action, PageId};
use super::ui::{AlertCard, Weight, LINK, MAC_BODY_PT};
use super::{apphost, Drawn, Shell};
use crate::gfx::{rgba, Surface};
use bifrost_core::app_firewall::{self, Policy, Port, Proto};
use bifrost_core::json::Json;
use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

/// How often the shell looks for a listening app, in milliseconds.
pub const SCAN_MS: f64 = 4000.0;
/// Questions that wait behind the sheet on screen; an app past this is asked on a later look.
const MAX_QUEUED: usize = 8;
/// How deep below an app's own process the look goes (a launcher that starts the real program, say).
const TREE_DEPTH: usize = 3;

const ALLOW: &str = "Allow";
const DENY: &str = "Deny";

/// An app waiting to be asked: its id, the name for the title and the ports it listens on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub app_id: String,
    pub name: String,
    pub ports: Vec<Port>,
}

/// The card's title.
pub fn title(ask: &Ask) -> String {
    format!("Do you want “{}” to accept incoming network connections?", ask.name)
}

/// The text under the title.
pub fn detail(ask: &Ask) -> String {
    format!("It is listening on {}. You can change this in Settings > Network > Firewall.", app_firewall::format_ports(&ask.ports))
}

/// The sheet on screen.
pub struct Sheet {
    pub ask: Ask,
    card: AlertCard,
}

/// What the person changed on the Firewall page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    Allow,
    Block,
    /// forget the choice: the app is asked again the next time it listens
    Forget,
}

/// One app's row on the Firewall page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FirewallView {
    pub app_id: String,
    pub name: String,
    pub policy: Policy,
    pub ports: String,
}

/// The firewall's state in the shell: the sheet, the questions behind it, the apps asked this run, and when the last
/// look was.
pub struct Firewall {
    sheet: Option<Sheet>,
    queue: VecDeque<Ask>,
    asked: BTreeSet<String>,
    scan_at: f64,
    /// where the process tables are; `/proc`, or a copy of its layout in tests
    proc_root: PathBuf,
}

impl Default for Firewall {
    fn default() -> Self {
        Firewall { sheet: None, queue: VecDeque::new(), asked: BTreeSet::new(), scan_at: f64::NEG_INFINITY, proc_root: PathBuf::from("/proc") }
    }
}

impl Firewall {
    #[cfg(test)]
    pub(super) fn with_proc_root(proc_root: PathBuf) -> Self {
        Firewall { proc_root, ..Firewall::default() }
    }
}

/// The ports `pid`, or a process under it, waits on for traffic from elsewhere, sorted. `proc_root` is `/proc`.
pub fn listening_ports(proc_root: &Path, pid: u32) -> Vec<Port> {
    let mut pids = vec![pid];
    let mut frontier = vec![pid];
    for _ in 0..TREE_DEPTH {
        let mut next = Vec::new();
        for p in &frontier {
            let tasks = std::fs::read_dir(proc_root.join(p.to_string()).join("task")).map(|d| d.flatten().map(|e| e.path()).collect::<Vec<_>>()).unwrap_or_default();
            for task in tasks {
                let text = std::fs::read_to_string(task.join("children")).unwrap_or_default();
                next.extend(text.split_whitespace().filter_map(|c| c.parse::<u32>().ok()).filter(|c| !pids.contains(c)));
            }
        }
        next.sort_unstable();
        next.dedup();
        pids.extend(&next);
        frontier = next;
    }
    let mut inodes = BTreeSet::new();
    for p in pids {
        let Ok(dir) = std::fs::read_dir(proc_root.join(p.to_string()).join("fd")) else { continue };
        for entry in dir.flatten() {
            if let Some(inode) = std::fs::read_link(entry.path()).ok().and_then(|l| app_firewall::socket_inode(&l.to_string_lossy())) {
                inodes.insert(inode);
            }
        }
    }
    if inodes.is_empty() {
        return Vec::new();
    }
    let mut ports = BTreeSet::new();
    for (file, proto) in [("tcp", Proto::Tcp), ("tcp6", Proto::Tcp), ("udp", Proto::Udp), ("udp6", Proto::Udp)] {
        let text = std::fs::read_to_string(proc_root.join("net").join(file)).unwrap_or_default();
        ports.extend(app_firewall::parse_listeners(&text, proto).into_iter().filter(|l| inodes.contains(&l.inode)).map(|l| l.port));
    }
    ports.into_iter().collect()
}

fn ports_json(ports: &[Port]) -> Json {
    Json::Arr(ports.iter().map(|p| Json::Str(p.text())).collect())
}

fn parse_ports(items: Option<&[Json]>) -> Vec<Port> {
    let Some(items) = items else { return Vec::new() };
    items.iter().filter_map(Json::as_str).filter_map(|t| app_firewall::parse_port_list(t).ok()).flatten().collect()
}

impl Shell {
    /// Whether the firewall question is on screen. It takes every touch while it is.
    pub(super) fn fw_showing(&self) -> bool {
        self.fw.sheet.is_some()
    }

    /// The policies the `app_firewall` service holds: (app id, policy, ports).
    fn fw_policies(&mut self, t: f64) -> Vec<(String, Policy, Vec<Port>)> {
        let Ok(answer) = self.service_call("", "app_firewall", "policies", "", t) else { return Vec::new() };
        let Some(list) = answer.get("policies").and_then(Json::as_array) else { return Vec::new() };
        list.iter()
            .filter_map(|p| {
                let policy = match p.str_field("policy").as_deref() {
                    Some("allow") => Policy::Allow,
                    Some("block") => Policy::Block,
                    _ => return None,
                };
                Some((p.str_field("app_id")?, policy, parse_ports(p.get("ports").and_then(Json::as_array))))
            })
            .collect()
    }

    /// Saves a choice for an app with the `app_firewall` service.
    fn fw_set(&mut self, app_id: &str, policy: Policy, ports: &[Port], t: f64) -> Result<(), String> {
        let body = Json::Obj(vec![
            ("app_id".into(), Json::Str(app_id.into())),
            ("policy".into(), Json::Str(if policy == Policy::Allow { "allow" } else { "block" }.into())),
            ("ports".into(), ports_json(ports)),
        ]);
        self.service_call("", "app_firewall", "set", &body.encode(), t).map(|_| ())
    }

    /// The hosted apps that run: (pid, app id, name). Only apps whose id the service accepts take part.
    fn fw_running(&self) -> Vec<(u32, String, String)> {
        let apps = apphost::installed();
        self.ext
            .iter()
            .enumerate()
            .filter_map(|(slot, host)| {
                let pid = host.as_ref()?.pid()?;
                let app = apps.get(slot)?;
                app_firewall::valid_app_id(&app.manifest.id).then(|| (pid, app.manifest.id.clone(), app.manifest.name.clone()))
            })
            .collect()
    }

    /// Every [`SCAN_MS`]: looks for hosted apps that listen, queues a question for each the firewall has no choice for,
    /// and shows the next question when none is on screen.
    pub(super) fn fw_tick(&mut self, t: f64) {
        if self.fw.sheet.is_none() {
            if let Some(ask) = self.fw.queue.pop_front() {
                self.fw_show(ask);
            }
        }
        if t - self.fw.scan_at < SCAN_MS {
            return;
        }
        self.fw.scan_at = t;
        let running = self.fw_running();
        self.fw_scan(&running, t);
    }

    /// One look at the running apps `(pid, app id, name)`.
    pub(super) fn fw_scan(&mut self, running: &[(u32, String, String)], t: f64) {
        let listening: Vec<(String, String, Vec<Port>)> =
            running.iter().map(|(pid, id, name)| (id.clone(), name.clone(), listening_ports(&self.fw.proc_root, *pid))).filter(|(_, _, ports)| !ports.is_empty()).collect();
        if listening.is_empty() {
            return;
        }
        let policies = self.fw_policies(t);
        for (app_id, name, ports) in listening {
            match policies.iter().find(|(id, _, _)| *id == app_id) {
                None => {
                    if self.fw.asked.contains(&app_id) || self.fw.queue.len() >= MAX_QUEUED {
                        continue;
                    }
                    self.fw.asked.insert(app_id.clone());
                    let ask = Ask { app_id, name, ports };
                    if self.fw.sheet.is_none() {
                        self.fw_show(ask);
                    } else {
                        self.fw.queue.push_back(ask);
                    }
                }
                // a blocked app's new ports are blocked too
                Some((_, Policy::Block, known)) if ports.iter().any(|p| !known.contains(p)) => {
                    let mut all = known.clone();
                    all.extend(ports.iter().filter(|p| !known.contains(p)));
                    all.sort();
                    if let Err(e) = self.fw_set(&app_id, Policy::Block, &all, t) {
                        eprintln!("springboard: {e}");
                    }
                }
                Some(_) => {}
            }
        }
    }

    fn fw_show(&mut self, ask: Ask) {
        let labels = [ALLOW, DENY].iter().map(|label| self.ui.text(label, MAC_BODY_PT, if *label == DENY { Weight::Semibold } else { Weight::Regular }, LINK)).collect();
        let card = AlertCard::new(&self.ui, &title(&ask), &detail(&ask), labels);
        self.fw.sheet = Some(Sheet { ask, card });
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// Whether a release at (x, y) ends a press that went down on the same button of the sheet.
    pub(super) fn fw_press_lands(&self, press: Option<(f32, f32)>, x: f32, y: f32) -> bool {
        super::press_lands(press, (x, y), |px, py| self.fw.sheet.as_ref().and_then(|s| s.card.hit(&self.ui, self.w, self.h, px, py)))
    }

    /// A touch on the sheet: a button answers it, anything else is ignored.
    pub(super) fn fw_touch(&mut self, x: f32, y: f32, up: bool, t: f64) {
        if up {
            let hit = self.fw.sheet.as_ref().and_then(|s| s.card.hit(&self.ui, self.w, self.h, x, y));
            match hit {
                Some(0) => self.fw_close(Policy::Allow, t),
                Some(1) => self.fw_close(Policy::Block, t),
                _ => {}
            }
        }
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// Takes the sheet down and saves the answer.
    fn fw_close(&mut self, policy: Policy, t: f64) {
        let Some(sheet) = self.fw.sheet.take() else { return };
        if let Err(e) = self.fw_set(&sheet.ask.app_id, policy, &sheet.ask.ports, t) {
            eprintln!("springboard: {e}");
        }
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// Draws the card over a dimmed screen.
    pub(super) fn fw_draw(&self, s: &mut Surface) {
        if let Some(sheet) = &self.fw.sheet {
            s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, rgba(0, 0, 0, 120));
            sheet.card.draw(s, &self.ui, self.w, self.h);
        }
    }

    /// The Firewall page's rows: each remembered app with its choice and ports, sorted by name.
    pub(super) fn fw_views(&mut self, t: f64) -> Vec<FirewallView> {
        let apps = apphost::installed();
        let mut views: Vec<FirewallView> = self
            .fw_policies(t)
            .into_iter()
            .map(|(app_id, policy, ports)| {
                let name = apps.iter().find(|a| a.manifest.id == app_id).map_or_else(|| app_id.clone(), |a| a.manifest.name.clone());
                FirewallView { app_id, name, policy, ports: app_firewall::format_ports(&ports) }
            })
            .collect();
        views.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.app_id.cmp(&b.app_id)));
        views
    }

    /// Carries out what the person changed on the Firewall page.
    pub(super) fn fw_edits(&mut self, edits: Vec<(String, Edit)>, t: f64) {
        for (app_id, edit) in edits {
            let policies = self.fw_policies(t);
            let ports = policies.iter().find(|(id, _, _)| *id == app_id).map(|(_, _, p)| p.clone()).unwrap_or_default();
            let result = match edit {
                Edit::Allow => self.fw_set(&app_id, Policy::Allow, &ports, t),
                Edit::Block => self.fw_set(&app_id, Policy::Block, &ports, t),
                Edit::Forget => {
                    // the app is asked again the next time it listens
                    self.fw.asked.remove(&app_id);
                    let body = Json::Obj(vec![("app_id".into(), Json::Str(app_id.clone()))]).encode();
                    self.service_call("", "app_firewall", "remove", &body, t).map(|_| ())
                }
            };
            if let Err(e) = result {
                eprintln!("springboard: {e}");
            }
        }
    }
}

/// Settings > Network > Firewall.
pub fn page(views: &[FirewallView]) -> Spec {
    let mut groups: Vec<Group> = views
        .iter()
        .map(|v| {
            let allowed = v.policy == Policy::Allow;
            let mut group = Group::new(vec![
                Item::new("Allow Incoming Connections").check(allowed, Action::Firewall(v.app_id.clone(), Edit::Allow)),
                Item::new("Block Incoming Connections").check(!allowed, Action::Firewall(v.app_id.clone(), Edit::Block)),
                Item::new("Forget This Choice").tint(Tint::Blue).action(Action::Firewall(v.app_id.clone(), Edit::Forget)),
            ])
            .header(&v.name);
            if !v.ports.is_empty() {
                group = group.footer(&format!("Listens on {}.", v.ports));
            }
            group
        })
        .collect();
    if groups.is_empty() {
        groups.push(Group::new(vec![Item::new("No Apps Yet").tint(Tint::Dim)]).footer("When an app starts waiting for connections from the network, you are asked whether to allow it. Your choices are listed here."));
    }
    Spec { id: PageId::Firewall, title: "Firewall".into(), large: false, groups }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::tests::{shell, unlock};
    use crate::shell::Event;
    use std::os::unix::fs::symlink;
    use std::sync::{Arc, Mutex};

    struct Record(Mutex<Vec<String>>);
    impl super::super::services::app_firewall::Applier for Record {
        fn apply(&self, ruleset: &str) -> Result<(), String> {
            self.0.lock().unwrap().push(ruleset.to_string());
            Ok(())
        }
    }

    /// A process table with one app (pid 4242) that holds a socket listening on `tcp_port`, plus another that holds none.
    fn proc_tree(tag: &str, tcp_port: u16) -> PathBuf {
        let root = std::env::temp_dir().join(format!("fw-proc-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("net")).unwrap();
        std::fs::create_dir_all(root.join("4242/fd")).unwrap();
        std::fs::create_dir_all(root.join("4243/fd")).unwrap();
        symlink("socket:[11111]", root.join("4242/fd/7")).unwrap();
        symlink("/dev/null", root.join("4242/fd/8")).unwrap();
        std::fs::write(
            root.join("net/tcp"),
            format!(
                "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n   0: 00000000:{tcp_port:04X} 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 11111 1 0 100 0 0 10 0\n   1: 00000000:0016 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 99999 1 0 100 0 0 10 0\n"
            ),
        )
        .unwrap();
        root
    }

    /// The shell with the firewall service kept in a scratch directory and a recording loader.
    fn firewall_shell(tag: &str, tcp_port: u16) -> (Shell, Arc<Record>) {
        let mut sh = shell();
        let dir = std::env::temp_dir().join(format!("fw-shell-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rec = Arc::new(Record(Mutex::new(Vec::new())));
        sh.services.retain(|s| s.name() != "app_firewall");
        sh.services.push(Box::new(super::super::services::app_firewall::AppFirewall::with_applier(dir.join(super::super::services::app_firewall::FILE), rec.clone())));
        sh.fw = Firewall::with_proc_root(proc_tree(tag, tcp_port));
        (sh, rec)
    }

    fn running() -> Vec<(u32, String, String)> {
        vec![(4242, "is.example.chat".into(), "Chat".into()), (4243, "is.example.quiet".into(), "Quiet".into())]
    }

    fn decision(sh: &mut Shell, id: &str) -> String {
        let body = Json::Obj(vec![("app_id".into(), Json::Str(id.into()))]).encode();
        sh.service_call("", "app_firewall", "decision", &body, 0.0).unwrap().str_field("decision").unwrap()
    }

    #[test]
    fn the_listening_ports_of_a_process_come_from_its_sockets() {
        let root = proc_tree("ports", 8080);
        assert_eq!(listening_ports(&root, 4242), vec![Port { proto: Proto::Tcp, number: 8080 }], "the other listener belongs to another process");
        assert!(listening_ports(&root, 4243).is_empty());
        assert!(listening_ports(&root, 1).is_empty(), "no such process");
        // a child of the app counts
        std::fs::create_dir_all(root.join("4242/task/4242")).unwrap();
        std::fs::write(root.join("4242/task/4242/children"), "4300 ").unwrap();
        std::fs::create_dir_all(root.join("4300/fd")).unwrap();
        symlink("socket:[99999]", root.join("4300/fd/3")).unwrap();
        assert_eq!(listening_ports(&root, 4242).len(), 2);
    }

    #[test]
    fn an_app_that_starts_listening_is_asked_and_the_answer_is_saved() {
        let (mut sh, rec) = firewall_shell("ask", 8080);
        unlock(&mut sh);
        sh.fw_scan(&running(), 1000.0);
        assert!(sh.fw_showing(), "the question is up");
        let ask = sh.fw.sheet.as_ref().unwrap().ask.clone();
        assert_eq!((ask.app_id.as_str(), ask.name.as_str()), ("is.example.chat", "Chat"));
        assert_eq!(title(&ask), "Do you want “Chat” to accept incoming network connections?");
        assert!(detail(&ask).contains("tcp/8080"));
        // it takes the touches: a tap outside the buttons changes nothing
        sh.event(Event::Down(5.0, 5.0), 1100.0);
        sh.event(Event::Up(5.0, 5.0), 1110.0);
        assert!(sh.fw_showing());
        assert_eq!(decision(&mut sh, "is.example.chat"), "ask");
        // Deny is the second button
        let (bx, by) = sh.fw.sheet.as_ref().unwrap().card.button_centre(&sh.ui, sh.w, sh.h, 1);
        sh.event(Event::Down(bx, by), 1200.0);
        sh.event(Event::Up(bx, by), 1210.0);
        assert!(!sh.fw_showing());
        assert_eq!(decision(&mut sh, "is.example.chat"), "block");
        let ruleset = {
            // the loader runs on a thread
            for _ in 0..100 {
                if !rec.0.lock().unwrap().is_empty() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            rec.0.lock().unwrap().last().cloned().unwrap_or_default()
        };
        assert!(ruleset.contains("tcp dport 8080 drop comment \"is.example.chat\""), "{ruleset}");
        // an app with a choice, and one that does not listen, are not asked
        sh.fw_scan(&running(), 9000.0);
        assert!(!sh.fw_showing());
    }

    #[test]
    fn allow_is_remembered_and_the_app_is_not_asked_twice() {
        let (mut sh, _rec) = firewall_shell("allow", 8080);
        sh.fw_scan(&running(), 1000.0);
        let (bx, by) = sh.fw.sheet.as_ref().unwrap().card.button_centre(&sh.ui, sh.w, sh.h, 0);
        sh.event(Event::Down(bx, by), 1200.0);
        sh.event(Event::Up(bx, by), 1210.0);
        assert_eq!(decision(&mut sh, "is.example.chat"), "allow");
        sh.fw_scan(&running(), 9000.0);
        assert!(!sh.fw_showing());
        // an app whose question is on screen is not queued a second time
        let (mut sh, _rec) = firewall_shell("once", 8080);
        sh.fw_scan(&running(), 1000.0);
        sh.fw_scan(&running(), 6000.0);
        assert!(sh.fw.queue.is_empty());
    }

    #[test]
    fn the_second_app_waits_behind_the_first_question() {
        let (mut sh, _rec) = firewall_shell("queue", 8080);
        let root = sh.fw.proc_root.clone();
        symlink("socket:[99999]", root.join("4243/fd/5")).unwrap();
        sh.fw_scan(&running(), 1000.0);
        assert_eq!(sh.fw.sheet.as_ref().unwrap().ask.app_id, "is.example.chat");
        assert_eq!(sh.fw.queue.len(), 1);
        let (bx, by) = sh.fw.sheet.as_ref().unwrap().card.button_centre(&sh.ui, sh.w, sh.h, 1);
        sh.event(Event::Down(bx, by), 1200.0);
        sh.event(Event::Up(bx, by), 1210.0);
        sh.fw_tick(1300.0);
        assert_eq!(sh.fw.sheet.as_ref().unwrap().ask.app_id, "is.example.quiet", "the next question comes up");
    }

    #[test]
    fn a_blocked_app_that_opens_another_port_has_it_blocked_without_asking() {
        let (mut sh, rec) = firewall_shell("grow", 8080);
        sh.fw_scan(&running(), 1000.0);
        let (bx, by) = sh.fw.sheet.as_ref().unwrap().card.button_centre(&sh.ui, sh.w, sh.h, 1);
        sh.event(Event::Down(bx, by), 1200.0);
        sh.event(Event::Up(bx, by), 1210.0);
        // the app opens a second listening socket
        let root = sh.fw.proc_root.clone();
        let mut tcp = std::fs::read_to_string(root.join("net/tcp")).unwrap();
        tcp.push_str("   2: 00000000:2328 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 22222 1 0 100 0 0 10 0\n");
        std::fs::write(root.join("net/tcp"), tcp).unwrap();
        symlink("socket:[22222]", root.join("4242/fd/9")).unwrap();
        sh.fw_scan(&running(), 9000.0);
        assert!(!sh.fw_showing(), "the choice was about the app");
        let views = sh.fw_views(9000.0);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].ports, "tcp/8080, tcp/9000");
        let _ = rec;
    }

    #[test]
    fn the_firewall_page_lists_the_choices_and_changes_them() {
        let (mut sh, _rec) = firewall_shell("page", 8080);
        sh.fw_scan(&running(), 1000.0);
        let (bx, by) = sh.fw.sheet.as_ref().unwrap().card.button_centre(&sh.ui, sh.w, sh.h, 1);
        sh.event(Event::Down(bx, by), 1200.0);
        sh.event(Event::Up(bx, by), 1210.0);
        let views = sh.fw_views(2000.0);
        assert_eq!(views, vec![FirewallView { app_id: "is.example.chat".into(), name: "is.example.chat".into(), policy: Policy::Block, ports: "tcp/8080".into() }]);
        let spec = page(&views);
        assert_eq!(spec.title, "Firewall");
        assert_eq!(spec.groups[0].header.as_deref(), Some("is.example.chat"));
        assert_eq!(spec.groups[0].footer.as_deref(), Some("Listens on tcp/8080."));
        assert_eq!(spec.groups[0].items[1].accessory, crate::shell::list::Accessory::Check(true));
        assert_eq!(spec.groups[0].items[0].action, Action::Firewall("is.example.chat".into(), Edit::Allow));
        sh.fw_edits(vec![("is.example.chat".into(), Edit::Allow)], 3000.0);
        assert_eq!(decision(&mut sh, "is.example.chat"), "allow");
        assert_eq!(sh.fw_views(3000.0)[0].ports, "tcp/8080", "the ports are kept");
        sh.fw_edits(vec![("is.example.chat".into(), Edit::Forget)], 4000.0);
        assert_eq!(decision(&mut sh, "is.example.chat"), "ask");
        sh.fw_scan(&running(), 9000.0);
        assert!(sh.fw_showing(), "a forgotten choice is asked again");
        assert!(page(&[]).groups[0].items[0].label.starts_with("No Apps"));
    }
}
