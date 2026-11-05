use super::sys::*;
use super::*;

#[test]
fn every_material_kind_sits_at_its_wire_value() {
    for (i, k) in MaterialKind::ALL.iter().enumerate() {
        assert_eq!(*k as usize, i);
    }
}

#[test]
fn an_older_apps_material_byte_decodes_with_the_default_state() {
    let m = Material { kind: MaterialKind::Titlebar, rect: UiRect { x: 0., y: 0., w: 10., h: 10. }, radius: 0., state: MaterialState::FollowsWindowActiveState, blending: BlendingMode::BehindWindow, emphasized: false, corners: None };
    let bytes = ToShell::Materials(vec![m]).encode();
    // the kind byte of a state-less sender is the bare kind
    assert!(bytes.windows(1).any(|b| b[0] == 1));
    assert_eq!(ToShell::decode(&bytes), Ok(ToShell::Materials(vec![m])));
}

#[test]
fn the_extended_material_round_trips_through_the_request() {
    let mut m = Material::new(MaterialKind::HudWindow, UiRect { x: 1., y: 2., w: 30., h: 40. }, 6.);
    m.blending = BlendingMode::WithinWindow;
    m.emphasized = true;
    m.corners = Some(Corners { tl: 1., tr: 2., br: 3., bl: 4. });
    assert_eq!(ToShell::decode(&ToShell::Materials(vec![m]).encode()), Ok(ToShell::Materials(vec![m])));
    let back = Material::from_request(&m.request());
    assert_eq!((back.kind, back.blending, back.emphasized, back.corners), (m.kind, m.blending, true, m.corners));
    // unknown flag bits and negative corners are refused
    let mut bad = ToShell::Materials(vec![Material::new(MaterialKind::Menu, UiRect { x: 0., y: 0., w: 1., h: 1. }, 0.)]).encode();
    *bad.last_mut().unwrap() = 8;
    assert_eq!(ToShell::decode(&bad), Err(DecodeError::BadValue));
}

#[test]
fn legacy_kinds_are_the_two_an_older_shell_decodes() {
    for k in MaterialKind::ALL {
        assert!(matches!(k.legacy(), MaterialKind::Sidebar | MaterialKind::Titlebar));
    }
    assert_eq!(MaterialKind::Sidebar.legacy(), MaterialKind::Sidebar);
}

#[test]
fn an_unknown_kind_or_state_is_refused() {
    let m = Material { kind: MaterialKind::Selection, rect: UiRect { x: 0., y: 0., w: 1., h: 1. }, radius: 0., state: MaterialState::Inactive, blending: BlendingMode::BehindWindow, emphasized: false, corners: None };
    let good = ToShell::Materials(vec![m]).encode();
    let at = good.iter().position(|&b| b == (13 | 2 << 6)).unwrap();
    for bad_byte in [14u8, 3 << 6] {
        let mut bad = good.clone();
        bad[at] = bad_byte;
        assert_eq!(ToShell::decode(&bad), Err(DecodeError::BadValue));
    }
}

#[test]
fn pasteboard_encode_never_panics_or_shortens() {
    // a multibyte char straddling byte 128 of the type (IPC-2)
    let m = ToApp::Pasteboard { change_count: 1, items: vec![("é".repeat(100), vec![])] };
    let Ok(ToApp::Pasteboard { items, .. }) = ToApp::decode(&m.encode()) else { panic!("decode") };
    assert!(items[0].0.len() <= 128 && !items[0].0.is_empty());
    // a payload that does not fit is left out whole, never cut (IPC-5)
    let big = ToShell::PasteboardSet { items: vec![("public.png".into(), vec![7; 50_000]), ("public.utf8-plain-text".into(), b"hi".to_vec())] };
    let bytes = big.encode();
    assert!(bytes.len() < MAX_PACKET);
    let Ok(ToShell::PasteboardSet { items }) = ToShell::decode(&bytes) else { panic!("decode") };
    assert_eq!(items, vec![("public.utf8-plain-text".to_string(), b"hi".to_vec())]);
    // a type over 128 bytes is refused on decode
    let mut p = Vec::new();
    u32_(&mut p, tag::PASTEBOARD_SET);
    u32_(&mut p, 1);
    str_(&mut p, &"t".repeat(129));
    u32_(&mut p, 0);
    assert_eq!(ToShell::decode(&p), Err(DecodeError::BadValue));
}

#[test]
fn pasteboard_blob_messages_round_trip_and_refuse_bad_layouts() {
    let types = vec![("public.png".to_string(), 100_000u32), ("public.utf8-plain-text".to_string(), 2)];
    let a = ToApp::PasteboardBlob { change_count: 9, types: types.clone() };
    assert_eq!(ToApp::decode(&a.encode()), Ok(a));
    let s = ToShell::PasteboardBlob { types: types.clone() };
    assert_eq!(ToShell::decode(&s.encode()), Ok(s));
    let c = ToApp::PasteboardChanged { change_count: 4 };
    assert_eq!(ToApp::decode(&c.encode()), Ok(c));
    assert_eq!(ToShell::decode(&ToShell::PasteboardRead.encode()), Ok(ToShell::PasteboardRead));
    // a layout over the blob limits, with a control character, or with too many items is refused
    for bad in [
        vec![("t".to_string(), (pbblob::MAX_ITEM + 1) as u32)],
        vec![("t\0".to_string(), 1)],
        vec![("t".to_string(), 1); pbblob::MAX_ITEMS + 1],
    ] {
        let mut p = Vec::new();
        u32_(&mut p, tag::PASTEBOARD_BLOB);
        u32_(&mut p, bad.len() as u32);
        for (t, n) in &bad {
            str_(&mut p, t);
            u32_(&mut p, *n);
        }
        assert_eq!(ToShell::decode(&p), Err(DecodeError::BadValue));
    }
}

#[test]
fn a_pasteboard_type_with_a_control_character_is_left_out() {
    let m = ToShell::PasteboardSet { items: vec![("\0/x".into(), b"a".to_vec()), ("public.utf8-plain-text".into(), b"ok".to_vec())] };
    // the sender leaves it out ...
    assert_eq!(ToShell::decode(&m.encode()), Ok(ToShell::PasteboardSet { items: vec![("public.utf8-plain-text".into(), b"ok".to_vec())] }));
    // ... and so does the receiver, whatever a hostile sender put on the wire
    let mut p = Vec::new();
    u32_(&mut p, tag::PASTEBOARD_SET);
    u32_(&mut p, 2);
    str_(&mut p, "a\u{7}/b");
    u32_(&mut p, 1);
    p.push(1);
    str_(&mut p, "public.png");
    u32_(&mut p, 1);
    p.push(2);
    assert_eq!(ToShell::decode(&p), Ok(ToShell::PasteboardSet { items: vec![("public.png".into(), vec![2])] }));
}

#[test]
fn oversized_service_reply_is_refused_not_cut() {
    let m = ToApp::ServiceReply { request: 3, ok: true, body: "x".repeat(20_000) };
    let bytes = m.encode();
    assert!(bytes.len() <= MAX_PACKET);
    assert_eq!(ToApp::decode(&bytes), Ok(ToApp::ServiceReply { request: 3, ok: false, body: "body too large".into() }));
    let fits = ToApp::ServiceReply { request: 3, ok: true, body: "y".repeat(15_000) };
    assert_eq!(ToApp::decode(&fits.encode()), Ok(fits));
}

#[test]
fn short_memfd_is_measurable_and_shrink_seal_holds() {
    let fd = shm_create("seal-test", 4096).unwrap();
    assert_eq!(shm_len(&fd).unwrap(), 4096);
    seal_shrink(&fd).unwrap();
    assert!(shm_resize(&fd, 0).is_err(), "a sealed memfd cannot shrink");
    assert!(shm_resize(&fd, 8192).is_ok());
}

#[test]
fn try_send_does_not_block_and_oversized_packets_are_errors() {
    let (a, b) = socketpair_pair().unwrap();
    set_nonblocking(a.fd()).unwrap();
    let pkt = vec![0u8; 16 * 1024];
    let mut sent = 0;
    let err = loop {
        match a.try_send(&pkt, &[]) {
            Ok(()) => sent += 1,
            Err(e) => break e,
        }
        assert!(sent < 100_000, "a never-read socket must fill up");
    };
    assert_eq!(err.kind(), std::io::ErrorKind::WouldBlock);
    drop(b);
    let (c, d) = socketpair_pair().unwrap();
    c.send(&vec![1u8; 20 * 1024], &[]).unwrap();
    assert!(d.recv(true).is_err(), "a 20 KiB packet is a protocol error, not a cut Packet");
}
