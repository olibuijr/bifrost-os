use bifrost_ui::content_sniff::{sniff, SNIFF_LEN};

#[test]
fn recognizes_binary_signatures_and_shared_containers() {
    assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest"), Some("public.png"));
    assert_eq!(sniff(b"%PDF-1.7"), Some("com.adobe.pdf"));
    assert_eq!(sniff(b"PK\x03\x04zip"), Some("public.zip-archive"));
    assert_eq!(sniff(b"BZh9block"), Some("public.bzip2-archive"));
}

#[test]
fn identifies_tar_and_media_containers_at_their_defined_offsets() {
    let mut tar = vec![0; 262];
    tar[257..262].copy_from_slice(b"ustar");
    assert_eq!(sniff(&tar), Some("public.tar-archive"));

    let mut movie = vec![0; 12];
    movie[4..8].copy_from_slice(b"ftyp");
    movie[8..12].copy_from_slice(b"qt  ");
    assert_eq!(sniff(&movie), Some("bifrost.player-movie"));
}

#[test]
fn recognizes_markup_scripts_and_plain_utf8_text() {
    assert_eq!(sniff(b"  <!doctype html><html>"), Some("public.html"));
    assert_eq!(sniff(b"<?xml version='1.0'?><SVG></SVG>"), Some("public.svg-image"));
    assert_eq!(sniff(b"#!/usr/bin/env python3 -u\nprint(1)"), Some("public.python-script"));
    assert_eq!(sniff(b"#!/usr/bin/env -S python3 -u\nprint(1)"), Some("public.python-script"));
    assert_eq!(sniff(b"#!/bin/bash -e\necho ok"), Some("public.shell-script"));
    assert_eq!(sniff("naïve text".as_bytes()), Some("public.plain-text"));
}

#[test]
fn unknown_binary_empty_and_nul_containing_data_are_not_text() {
    assert_eq!(sniff(&[]), None);
    assert_eq!(sniff(b"\0\xff\x01"), None);
    assert_eq!(sniff(b"text\0tail"), None);
    assert_eq!(sniff(b"#!/usr/bin/env ruby\nputs 1"), None);
}

#[test]
fn only_the_leading_window_is_considered() {
    let mut bytes = vec![b'x'; SNIFF_LEN + 8];
    bytes[SNIFF_LEN..].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    assert_eq!(sniff(&bytes), Some("public.plain-text"));
    assert_eq!(SNIFF_LEN, 512);
}
