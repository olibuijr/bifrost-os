//! App Intents for the scanner: "scan a document" and "scan a page to a folder" descriptors that Search and Shortcuts
//! can list.
//!
//! Like `vpn_intents`, these only describe the actions. Nothing serves a call yet: the intended path is
//! `bifrost_ui::scanner::scan_to`, the same path the Scan page uses, with the device list from
//! `bifrost_ui::scanner::list`. Until that is wired, a call must not report a scan that did not run.

use crate::intents::{Intent, Param, ParamType};

/// Id of the intent that scans one document to a file in the Documents folder.
pub const SCAN_DOCUMENT_ID: &str = "scan_document";
/// Id of the intent that scans a page into a named folder.
pub const SCAN_TO_FOLDER_ID: &str = "scan_to_folder";

/// Parameter: the scanner to use, by its display name.
pub const DEVICE_PARAM: &str = "device";
/// Parameter: the output format, one of the cases in [`FORMAT_CASES`].
pub const FORMAT_PARAM: &str = "format";
/// Parameter: the folder the scan is saved into.
pub const FOLDER_PARAM: &str = "folder";

/// The output formats a scan can be saved in, in the order the Scan page lists them.
pub const FORMAT_CASES: [&str; 4] = ["pdf", "jpeg", "png", "tiff"];

/// The two scanner intents, in the order Search lists them.
pub fn scanner_intents() -> Vec<Intent> {
    let format = || ParamType::Enum(FORMAT_CASES.iter().map(|c| c.to_string()).collect());
    vec![
        Intent::new(SCAN_DOCUMENT_ID, "Scan Document", "Scans a document to a file in Documents.")
            .param(Param::new(DEVICE_PARAM, "Scanner", ParamType::String).optional())
            .param(Param::new(FORMAT_PARAM, "Format", format()).optional())
            .shortcut(),
        Intent::new(SCAN_TO_FOLDER_ID, "Scan to Folder", "Scans a page and saves it into the named folder.")
            .param(Param::new(FOLDER_PARAM, "Folder", ParamType::String))
            .param(Param::new(DEVICE_PARAM, "Scanner", ParamType::String).optional())
            .shortcut(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_intents_with_stable_ids() {
        let list = scanner_intents();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, SCAN_DOCUMENT_ID);
        assert_eq!(list[1].id, SCAN_TO_FOLDER_ID);
    }

    #[test]
    fn methods_use_the_intent_prefix() {
        let list = scanner_intents();
        assert_eq!(list[0].method(), "intent.scan_document");
        assert_eq!(list[1].method(), "intent.scan_to_folder");
    }

    #[test]
    fn scan_document_takes_optional_device_and_format() {
        let list = scanner_intents();
        let names: Vec<&str> = list[0].params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, [DEVICE_PARAM, FORMAT_PARAM]);
        assert!(list[0].params.iter().all(|p| p.optional));
    }

    #[test]
    fn scan_to_folder_needs_a_folder() {
        let list = scanner_intents();
        let folder = list[1].params.iter().find(|p| p.name == FOLDER_PARAM).expect("folder parameter");
        assert!(!folder.optional, "the folder is required");
    }

    #[test]
    fn format_lists_the_scan_page_formats() {
        let list = scanner_intents();
        match &list[0].params[1].ty {
            ParamType::Enum(cases) => assert_eq!(cases.as_slice(), FORMAT_CASES.map(String::from)),
            other => panic!("format should be an enum, got {other:?}"),
        }
    }

    #[test]
    fn both_are_shortcuts() {
        for i in scanner_intents() {
            assert!(i.shortcut, "{} should be a shortcut", i.id);
        }
    }
}
