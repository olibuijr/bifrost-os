//! The trust sheet: what the shell shows when the Browser reaches a site whose TLS certificate the system refuses.
//!
//! The first card says which site and why (expired, issued by an authority the system does not trust, issued for another
//! name), and offers Cancel and Show Certificate. Show Certificate opens the certificate panel: the chain the server
//! sent, leaf first, with one certificate selected, a Details disclosure with its subject, issuer, serial number and
//! validity, and a Done button. The models are `bifrost_ui::certificate_panel` (what is shown and when a certificate is
//! valid) and `bifrost_core::x509` (the fields of the DER the server sent).
//!
//! The sheet only informs. There is no button that loads the page anyway: the connection stays refused, so a person who
//! taps through cannot be handed to a site that failed to prove who it is. A trust exception for one site belongs in
//! a per-site store that Settings can list and remove from, and does not exist yet.
//!
//! The sheet is a pure model plus its drawing (`TrustSheet::new`, `hit`, `press`, `draw`), so it is testable without a
//! screen. Which refused chain to show comes from `deildu::http::take_rejection`, through the Browser.

use super::ui::{
    AlertCard, Ui, Weight, CARD_BG, LABEL, LABEL_2, LINK, MAC_BODY_PT, MAC_CALLOUT_PT, MAC_TITLE3_PT, SELECTION, SYSTEM_GREEN,
    SYSTEM_ORANGE, SYSTEM_RED, WHITE, FIELD_BG,
};
use crate::deildu::http::TlsRejection;
use crate::gfx::{rgba, Image, Surface};
use bifrost_core::plist::hex_encode;
use bifrost_core::x509;
use bifrost_ui::certificate_panel::{CertificateInfo, CertificatePanel, Validity};

/// Card width and row height of the certificate panel, in points.
const CARD_W_PT: f32 = 480.0;
const ROW_PT: f32 = 28.0;
/// Inner margin of the card, the gap above its buttons, and the corner radii (window 17, selection plate 9), in points.
const PAD_PT: f32 = 16.0;
const GAP_PT: f32 = 8.0;
const SHEET_RADIUS_PT: f32 = 17.0;
const PLATE_RADIUS_PT: f32 = 9.0;
const BUTTON_W_PT: f32 = 96.0;
const DOT_PT: f32 = 8.0;
const CHEVRON_PT: f32 = 8.0;
/// Share of the card width the label column of the detail rows takes.
const LABEL_COLUMN: f32 = 0.30;

/// The certificates of a refused chain as the panel model wants them. `None` when any certificate cannot be read, since
/// a chain shown in part would mislead.
pub fn chain_info(chain: &[Vec<u8>]) -> Option<Vec<CertificateInfo>> {
    chain
        .iter()
        .map(|der| {
            let c = x509::parse(der).ok()?;
            let name = |n: &x509::Name| if n.is_empty() { "(no name)".to_string() } else { n.to_string() };
            // The leading zero byte of an unsigned integer is encoding, not part of the number people compare.
            let digits = match c.serial.as_slice() {
                [0, rest @ ..] if !rest.is_empty() => rest,
                all => all,
            };
            let serial = if digits.is_empty() { "00".to_string() } else { hex_encode(digits).to_uppercase() };
            Some(CertificateInfo { subject: name(&c.subject), issuer: name(&c.issuer), serial, not_before: c.not_before as i64, not_after: c.not_after as i64 })
        })
        .collect()
}

/// Which card is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// The question: which site, why, Cancel or Show Certificate.
    Alert,
    /// The certificate panel.
    Certificate,
}

/// What a tap can land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Cancel,
    ShowCertificate,
    Done,
    /// A certificate of the chain, by index.
    Chain(usize),
    Disclosure,
}

/// The sheet for one refused connection.
pub struct TrustSheet {
    host: String,
    reason: String,
    stage: Stage,
    alert: AlertCard,
    panel: Option<CertificatePanel>,
    disclosed: bool,
    /// Unix seconds, the time validity is judged at.
    now: i64,
}

impl TrustSheet {
    /// The sheet for `rejection`, judging validity at `now` (Unix seconds).
    pub fn new(ui: &Ui, rejection: &TlsRejection, now: i64) -> Self {
        let panel = chain_info(&rejection.chain).and_then(|info| CertificatePanel::new(info).ok());
        let mut labels = vec![ui.text("Cancel", MAC_BODY_PT, Weight::Regular, LINK)];
        if panel.is_some() {
            labels.push(ui.text("Show Certificate", MAC_BODY_PT, Weight::Semibold, LINK));
        }
        let title = format!("Can't verify the identity of {}", rejection.host);
        let message = format!("{} Your information could be exposed if you continue, so {} was not opened.", rejection.reason, rejection.host);
        TrustSheet {
            host: rejection.host.clone(),
            reason: rejection.reason.clone(),
            stage: Stage::Alert,
            alert: AlertCard::new(ui, &title, &message, labels),
            panel,
            disclosed: false,
            now,
        }
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }

    pub fn stage(&self) -> Stage {
        self.stage
    }

    pub fn panel(&self) -> Option<&CertificatePanel> {
        self.panel.as_ref()
    }

    pub fn is_disclosed(&self) -> bool {
        self.disclosed
    }

    // ---- layout of the certificate card, in px ----

    fn detail_rows(&self) -> usize {
        if self.disclosed {
            self.panel.as_ref().and_then(|p| p.rows(p.selected(), self.now).ok()).map_or(0, |r| r.len())
        } else {
            0
        }
    }

    /// The card (x, y, w, h) centred on a screen of `w` by `h` px.
    fn card(&self, ui: &Ui, w: i32, h: i32) -> (f32, f32, f32, f32) {
        let chain = self.panel.as_ref().map_or(0, |p| p.chain().len());
        let rows = chain + 1 + self.detail_rows();
        let ch = ui.pt(PAD_PT) * 2.0 + ui.pt(ROW_PT) * (rows as f32 + 2.0) + ui.pt(GAP_PT) * 3.0;
        let cw = ui.pt(CARD_W_PT).min(w as f32 - ui.pt(PAD_PT) * 2.0);
        (((w as f32 - cw) / 2.0).round(), ((h as f32 - ch) / 2.0).round(), cw, ch)
    }

    /// Row `k` of the card's list, counting the title as row 0 and the chain from row 1: (x, y, w, h).
    fn row(&self, ui: &Ui, card: (f32, f32, f32, f32), k: usize) -> (f32, f32, f32, f32) {
        let (x, y, cw, _) = card;
        (x + ui.pt(PAD_PT), y + ui.pt(PAD_PT) + ui.pt(ROW_PT) * k as f32 + if k > 0 { ui.pt(GAP_PT) } else { 0.0 }, cw - 2.0 * ui.pt(PAD_PT), ui.pt(ROW_PT))
    }

    fn chain_len(&self) -> usize {
        self.panel.as_ref().map_or(0, |p| p.chain().len())
    }

    fn disclosure_row(&self, ui: &Ui, card: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
        let r = self.row(ui, card, 1 + self.chain_len());
        (r.0, r.1 + ui.pt(GAP_PT), r.2, r.3)
    }

    fn detail_row(&self, ui: &Ui, card: (f32, f32, f32, f32), i: usize) -> (f32, f32, f32, f32) {
        let r = self.disclosure_row(ui, card);
        (r.0, r.1 + ui.pt(ROW_PT) * (i + 1) as f32, r.2, r.3)
    }

    fn done_button(&self, ui: &Ui, card: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
        let (x, y, cw, ch) = card;
        let bw = ui.pt(BUTTON_W_PT);
        (x + cw - ui.pt(PAD_PT) - bw, y + ch - ui.pt(PAD_PT) - ui.pt(ROW_PT), bw, ui.pt(ROW_PT))
    }

    /// What is under (x, y) on a screen of `w` by `h` px.
    pub fn hit(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<Hit> {
        let inside = |(rx, ry, rw, rh): (f32, f32, f32, f32)| x >= rx && x < rx + rw && y >= ry && y < ry + rh;
        match self.stage {
            Stage::Alert => self.alert.hit(ui, w, h, x, y).and_then(|i| match i {
                0 => Some(Hit::Cancel),
                1 if self.panel.is_some() => Some(Hit::ShowCertificate),
                _ => None,
            }),
            Stage::Certificate => {
                let card = self.card(ui, w, h);
                if inside(self.done_button(ui, card)) {
                    return Some(Hit::Done);
                }
                if inside(self.disclosure_row(ui, card)) {
                    return Some(Hit::Disclosure);
                }
                (0..self.chain_len()).find(|&i| inside(self.row(ui, card, 1 + i))).map(Hit::Chain)
            }
        }
    }

    /// The centre of the thing `hit` names on a screen of `w` by `h` px, for tests.
    pub fn centre(&self, ui: &Ui, w: i32, h: i32, hit: Hit) -> Option<(f32, f32)> {
        let mid = |(x, y, rw, rh): (f32, f32, f32, f32)| (x + rw / 2.0, y + rh / 2.0);
        match (self.stage, hit) {
            (Stage::Alert, Hit::Cancel) => Some(self.alert.button_centre(ui, w, h, 0)),
            (Stage::Alert, Hit::ShowCertificate) if self.panel.is_some() => Some(self.alert.button_centre(ui, w, h, 1)),
            (Stage::Certificate, Hit::Done) => Some(mid(self.done_button(ui, self.card(ui, w, h)))),
            (Stage::Certificate, Hit::Disclosure) => Some(mid(self.disclosure_row(ui, self.card(ui, w, h)))),
            (Stage::Certificate, Hit::Chain(i)) if i < self.chain_len() => Some(mid(self.row(ui, self.card(ui, w, h), 1 + i))),
            _ => None,
        }
    }

    /// The person tapped `hit`. Returns true when the sheet is finished and should go away.
    pub fn press(&mut self, hit: Hit) -> bool {
        match (self.stage, hit) {
            (Stage::Alert, Hit::Cancel) | (Stage::Certificate, Hit::Done) => return true,
            (Stage::Alert, Hit::ShowCertificate) if self.panel.is_some() => self.stage = Stage::Certificate,
            (Stage::Certificate, Hit::Chain(i)) => {
                if let Some(p) = self.panel.as_mut() {
                    let _ = p.select(i);
                }
            }
            (Stage::Certificate, Hit::Disclosure) => self.disclosed = !self.disclosed,
            _ => {}
        }
        false
    }

    // ---- drawing ----

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32) {
        match self.stage {
            Stage::Alert => self.alert.draw(s, ui, w, h),
            Stage::Certificate => self.draw_certificate(s, ui, w, h),
        }
    }

    /// `text` in colour `color` at (x, y) px, cut to `max_w` px, vertically centred in a row of height `rh`.
    fn put(s: &mut Surface, ui: &Ui, (x, y, rh): (f32, f32, f32), text: &str, pt: f32, weight: Weight, color: u32, max_w: f32) -> Image {
        let img = ui.text(&ui.fit(text, pt, max_w.max(0.0)), pt, weight, WHITE);
        s.blit_colored(&img, x as i32, (y + (rh - img.h as f32) / 2.0) as i32, color, 255);
        img
    }

    fn draw_certificate(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32) {
        s.fill_rrect(0.0, 0.0, w as f32, h as f32, 0.0, rgba(0, 0, 0, 110));
        let card = self.card(ui, w, h);
        let (cx, cy, cw, ch) = card;
        s.fill_rrect(cx, cy, cw, ch, ui.pt(SHEET_RADIUS_PT), CARD_BG.now());
        let Some(panel) = self.panel.as_ref() else { return };
        let title = self.row(ui, card, 0);
        Self::put(s, ui, (title.0, title.1, title.3), &format!("Certificate for {}", self.host), MAC_TITLE3_PT, Weight::Semibold, LABEL.now(), title.2);
        for (i, cert) in panel.chain().iter().enumerate() {
            let row = self.row(ui, card, 1 + i);
            let selected = i == panel.selected();
            if selected {
                s.fill_rrect(row.0, row.1, row.2, row.3, ui.pt(PLATE_RADIUS_PT), SELECTION.now());
            }
            let validity = cert.validity(self.now);
            let dot = match validity {
                Validity::Valid => SYSTEM_GREEN,
                Validity::NotYetValid => SYSTEM_ORANGE,
                Validity::Expired => SYSTEM_RED,
            };
            let indent = ui.pt(GAP_PT) * (1.0 + 1.5 * i as f32);
            let d = ui.pt(DOT_PT);
            s.fill_rrect(row.0 + indent, row.1 + (row.3 - d) / 2.0, d, d, d / 2.0, dot);
            let (main, side) = if selected { (WHITE, WHITE) } else { (LABEL.now(), LABEL_2.now()) };
            let status = ui.text(validity.text(), MAC_CALLOUT_PT, Weight::Regular, WHITE);
            let text_x = row.0 + indent + d + ui.pt(GAP_PT);
            Self::put(s, ui, (text_x, row.1, row.3), &cert.subject, MAC_BODY_PT, Weight::Regular, main, row.0 + row.2 - ui.pt(GAP_PT) * 2.0 - status.w as f32 - text_x);
            s.blit_colored(&status, (row.0 + row.2 - ui.pt(GAP_PT) - status.w as f32) as i32, (row.1 + (row.3 - status.h as f32) / 2.0) as i32, side, 255);
        }
        let dr = self.disclosure_row(ui, card);
        let c = ui.pt(CHEVRON_PT);
        let (bx, by) = (dr.0 + ui.pt(GAP_PT), dr.1 + (dr.3 - c) / 2.0);
        let half = c / 2.0;
        for k in 0..(CHEVRON_PT as usize / 2) {
            let k = k as f32 * ui.pt(1.0);
            if self.disclosed {
                s.fill_rrect(bx + k, by + k + half / 2.0, c - 2.0 * k, ui.pt(1.0), 0.0, LABEL_2.now());
            } else {
                s.fill_rrect(bx + k + half / 2.0, by + k, ui.pt(1.0), c - 2.0 * k, 0.0, LABEL_2.now());
            }
        }
        Self::put(s, ui, (dr.0 + ui.pt(GAP_PT) + c + ui.pt(GAP_PT), dr.1, dr.3), "Details", MAC_BODY_PT, Weight::Regular, LABEL.now(), dr.2);
        if let Ok(rows) = panel.rows(panel.selected(), self.now) {
            if self.disclosed {
                for (i, (label, value)) in rows.iter().enumerate() {
                    let r = self.detail_row(ui, card, i);
                    let split = r.2 * LABEL_COLUMN;
                    Self::put(s, ui, (r.0 + ui.pt(GAP_PT), r.1, r.3), label, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), split - ui.pt(GAP_PT) * 2.0);
                    Self::put(s, ui, (r.0 + split, r.1, r.3), value, MAC_CALLOUT_PT, Weight::Regular, LABEL.now(), r.2 - split - ui.pt(GAP_PT));
                }
            }
        }
        let b = self.done_button(ui, card);
        s.fill_rrect(b.0, b.1, b.2, b.3, b.3 / 2.0, FIELD_BG.now());
        let done = ui.text("Done", MAC_BODY_PT, Weight::Semibold, LINK);
        s.blit_colored(&done, (b.0 + (b.2 - done.w as f32) / 2.0) as i32, (b.1 + (b.3 - done.h as f32) / 2.0) as i32, LINK, 255);
    }
}

/// A DER certificate for tests: subject and issuer common names and the validity period in Unix seconds.
#[cfg(test)]
pub(crate) fn test_certificate(subject: &str, issuer: &str, not_before: u64, not_after: u64) -> Vec<u8> {
    use bifrost_core::der;
    use bifrost_core::oid::Oid;
    let alg = Oid::parse("1.2.840.10045.4.3.2").unwrap();
    let key_alg = der::sequence(&[der::oid(&Oid::parse("1.2.840.10045.2.1").unwrap()).unwrap(), der::oid(&Oid::parse("1.2.840.10045.3.1.7").unwrap()).unwrap()]);
    let spki = der::sequence(&[key_alg, der::bit_string(0, &[4u8; 65])]);
    let t = x509::Template {
        serial: &[0x00, 0x8a, 0x01],
        signature_algorithm: &alg,
        issuer: &[("CN", issuer)],
        subject: &[("CN", subject)],
        not_before,
        not_after,
        spki: &spki,
        dns_names: &[],
        authority: false,
    };
    x509::encode(&t, &[9, 9, 9]).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: i32 = 1200;
    const H: i32 = 1920;
    const NOW: i64 = 1_800_000_000;

    fn ui() -> Ui {
        crate::shell::tests::ui()
    }

    fn rejection_for(host: &str, chain: Vec<Vec<u8>>) -> TlsRejection {
        TlsRejection { host: host.into(), chain, reason: "The certificate has expired.".into() }
    }

    fn rejection(chain: Vec<Vec<u8>>) -> TlsRejection {
        rejection_for("bad.example", chain)
    }

    fn two_certificates() -> Vec<Vec<u8>> {
        let leaf = test_certificate("bad.example", "Example CA", 1_700_000_000, 1_790_000_000);
        let root = test_certificate("Example CA", "Example CA", 1_000_000_000, 2_000_000_000);
        vec![leaf, root]
    }

    fn tap(sheet: &mut TrustSheet, ui: &Ui, hit: Hit) -> bool {
        let (x, y) = sheet.centre(ui, W, H, hit).expect("the sheet has this control");
        assert_eq!(sheet.hit(ui, W, H, x, y), Some(hit));
        sheet.press(hit)
    }

    #[test]
    fn the_chain_the_server_sent_is_read_into_the_panel_model_leaf_first() {
        let info = chain_info(&two_certificates()).unwrap();
        assert_eq!(info[0].subject, "CN=bad.example");
        assert_eq!(info[0].issuer, "CN=Example CA");
        assert_eq!(info[0].serial, "8A01");
        assert_eq!((info[0].not_before, info[0].not_after), (1_700_000_000, 1_790_000_000));
        assert_eq!(info.len(), 2);
        assert!(chain_info(&[vec![1, 2, 3]]).is_none(), "a certificate that cannot be read stops the whole chain");
    }

    #[test]
    fn the_first_card_names_the_site_and_offers_cancel_and_show_certificate() {
        let ui = ui();
        let mut sheet = TrustSheet::new(&ui, &rejection(two_certificates()), NOW);
        assert_eq!((sheet.stage(), sheet.host()), (Stage::Alert, "bad.example"));
        assert_eq!(sheet.reason(), "The certificate has expired.");
        assert_eq!(sheet.hit(&ui, W, H, 2.0, 2.0), None);
        assert!(!tap(&mut sheet, &ui, Hit::ShowCertificate));
        assert_eq!(sheet.stage(), Stage::Certificate);
    }

    #[test]
    fn cancel_ends_the_sheet() {
        let ui = ui();
        let mut sheet = TrustSheet::new(&ui, &rejection(two_certificates()), NOW);
        assert!(tap(&mut sheet, &ui, Hit::Cancel));
    }

    #[test]
    fn a_chain_that_cannot_be_read_offers_only_cancel() {
        let ui = ui();
        let mut sheet = TrustSheet::new(&ui, &rejection(vec![vec![0, 1, 2]]), NOW);
        assert!(sheet.panel().is_none());
        assert_eq!(sheet.centre(&ui, W, H, Hit::ShowCertificate), None);
        assert!(!sheet.press(Hit::ShowCertificate), "the sheet stays on its first card");
        assert_eq!(sheet.stage(), Stage::Alert);
        assert!(tap(&mut sheet, &ui, Hit::Cancel));
    }

    #[test]
    fn the_certificate_card_selects_a_certificate_and_discloses_its_details() {
        let ui = ui();
        let mut sheet = TrustSheet::new(&ui, &rejection(two_certificates()), NOW);
        tap(&mut sheet, &ui, Hit::ShowCertificate);
        assert_eq!(sheet.panel().unwrap().selected(), 0);
        tap(&mut sheet, &ui, Hit::Chain(1));
        assert_eq!(sheet.panel().unwrap().selected(), 1, "the issuer is selected");
        assert!(!sheet.is_disclosed());
        let closed = sheet.card(&ui, W, H).3;
        tap(&mut sheet, &ui, Hit::Disclosure);
        assert!(sheet.is_disclosed());
        let grown = sheet.card(&ui, W, H).3 - closed;
        assert!((grown - 4.0 * ui.pt(ROW_PT)).abs() < 0.01, "subject, issuer, serial number and status: {grown}");
        let rows = sheet.panel().unwrap().rows(1, NOW).unwrap();
        assert_eq!(rows[0], ("Subject", "CN=Example CA".to_string()));
        assert_eq!(rows[3], ("Status", "Valid".to_string()));
        let leaf = sheet.panel().unwrap().rows(0, NOW).unwrap();
        assert_eq!(leaf[3].1, "Expired", "the leaf ended before NOW");
        tap(&mut sheet, &ui, Hit::Disclosure);
        assert!(!sheet.is_disclosed());
        assert!(tap(&mut sheet, &ui, Hit::Done));
    }

    #[test]
    fn controls_of_the_other_card_are_not_hit() {
        let ui = ui();
        let mut sheet = TrustSheet::new(&ui, &rejection(two_certificates()), NOW);
        assert_eq!(sheet.centre(&ui, W, H, Hit::Done), None);
        assert!(!sheet.press(Hit::Done), "Done belongs to the certificate card");
        assert!(!sheet.press(Hit::Chain(0)), "nothing to select on the first card");
        assert_eq!(sheet.stage(), Stage::Alert);
    }

    #[test]
    fn both_cards_draw_without_panicking_and_paint_over_the_screen() {
        let ui = ui();
        let mut sheet = TrustSheet::new(&ui, &rejection(two_certificates()), NOW);
        let mut px = vec![0u32; (W * H) as usize];
        {
            let mut s = Surface { w: W as usize, h: H as usize, stride: W as usize, px: &mut px, y0: 0 };
            sheet.draw(&mut s, &ui, W, H);
        }
        assert_ne!(px[(H / 2 * W + W / 2) as usize], 0, "the first card covers the middle");
        sheet.press(Hit::ShowCertificate);
        sheet.press(Hit::Disclosure);
        let mut px = vec![0u32; (W * H) as usize];
        let mut s = Surface { w: W as usize, h: H as usize, stride: W as usize, px: &mut px, y0: 0 };
        sheet.draw(&mut s, &ui, W, H);
        assert_ne!(s.px[(H / 2 * W + W / 2) as usize], 0, "the certificate card covers the middle");
    }

    #[test]
    fn a_refused_certificate_in_the_browser_raises_the_sheet_and_cancel_clears_it() {
        use crate::deildu::http::record_rejection;
        use crate::shell::browser::{fetch::tests::Fake, store, Browser};
        use crate::shell::{AppId, Event, Screen};
        let mut sh = crate::shell::tests::shell();
        let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-trust-sheet")).join(format!("{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        store::save_session(&dir, &["https://shell-refused.test/".to_string()], 0);
        record_rejection(rejection_for("shell-refused.test", two_certificates()));
        sh.browser = Some(Browser::new(&sh.ui, 1200, 1920, dir, Fake::new(&[]), sh.wall, 0.0));
        sh.screen = Screen::App(AppId::Browser);
        for k in 0..400 {
            sh.tick(100.0 + 20.0 * k as f64);
            if sh.trust_sheet.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let sheet = sh.trust_sheet.as_ref().expect("the Browser's refused page raises the trust sheet");
        assert_eq!((sheet.host(), sheet.stage()), ("shell-refused.test", Stage::Alert));
        assert_eq!(sh.overlay_blocker(), Some("trust-sheet"));
        // Show Certificate, then Done, through the shell's touch path
        let tap = |sh: &mut crate::shell::Shell, hit: Hit, t: f64| {
            let (x, y) = sh.trust_sheet.as_ref().unwrap().centre(&sh.ui, sh.w, sh.h, hit).unwrap();
            sh.event(Event::Down(x, y), t);
            sh.event(Event::Up(x, y), t + 40.0);
        };
        tap(&mut sh, Hit::ShowCertificate, 9000.0);
        assert_eq!(sh.trust_sheet.as_ref().unwrap().stage(), Stage::Certificate);
        tap(&mut sh, Hit::Done, 9100.0);
        assert!(sh.trust_sheet.is_none(), "Done dismisses the sheet");
        sh.tick(9200.0);
        assert!(sh.trust_sheet.is_none(), "the same refusal does not raise it again");
    }
}
