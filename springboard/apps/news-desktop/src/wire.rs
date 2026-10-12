//! Parity wiring for News: language and word tokenizing for article text, a link card built from the feed's
//! metadata, and the Reading List. The models live in the shared kit (`bifrost_app::controls`) and the SDK
//! (`bifrost_app::spellcheck`); this file adapts them to the feed items the app already holds and keeps the store.

use crate::feed::Item;
use bifrost_app::controls::language::{dominant_language, Language};
use bifrost_app::controls::linkmeta::{validate_url, Card, LinkMetadata, LinkView};
use bifrost_app::controls::reading_list::ReadingList;
use bifrost_app::controls::translation::LanguageAvailability;
use bifrost_app::spellcheck::words;
use bifrost_app::translation::{TranslationError, TranslationMemory, TranslationSession};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// The Reading List keeps this many pages; the oldest go first.
pub const READING_LIMIT: usize = 500;
/// The silent reading rate behind the "min read" figure.
pub const WORDS_PER_MINUTE: usize = 200;
/// The store file, next to the feed cache in the app's data directory.
const STORE: &str = "reading-list.txt";
/// The language codes the Translation model covers for News: Icelandic and English. No language pack ships with
/// BifrOSt yet, so no pair is installed (see `installed`).
pub const TRANSLATION_LANGUAGES: &[&str] = &["en", "is"];

/// The English name of a recognised language.
pub fn language_name(language: Language) -> &'static str {
    match language {
        Language::Arabic => "Arabic",
        Language::Chinese => "Chinese",
        Language::Danish => "Danish",
        Language::Dutch => "Dutch",
        Language::English => "English",
        Language::French => "French",
        Language::German => "German",
        Language::Greek => "Greek",
        Language::Hebrew => "Hebrew",
        Language::Icelandic => "Icelandic",
        Language::Italian => "Italian",
        Language::Japanese => "Japanese",
        Language::Korean => "Korean",
        Language::Norwegian => "Norwegian",
        Language::Polish => "Polish",
        Language::Portuguese => "Portuguese",
        Language::Russian => "Russian",
        Language::Spanish => "Spanish",
        Language::Swedish => "Swedish",
        Language::Thai => "Thai",
    }
}

/// The number of words in `text`, counted by the SDK tokenizer (runs of letters and digits, with inner apostrophes).
pub fn word_count(text: &str) -> usize {
    words(text).len()
}

/// Whole minutes to read `count` words: at least one for any text.
pub fn reading_minutes(count: usize) -> usize {
    count.div_ceil(WORDS_PER_MINUTE).max(1)
}

/// The facts line under an article: its language, word count and reading time, joined by a middle dot.
/// Empty when the text has no words. The language is left out when the text has no recognisable letters.
pub fn facts(text: &str) -> String {
    let count = word_count(text);
    if count == 0 {
        return String::new();
    }
    let mut parts = Vec::with_capacity(3);
    if let Some(language) = dominant_language(text) {
        parts.push(language_name(language).to_string());
    }
    parts.push(format!("{count} words"));
    parts.push(format!("{} min read", reading_minutes(count)));
    parts.join("  \u{b7}  ")
}

/// The rich link card for an article: its title, and the feed's name as the site. The feed gives the metadata,
/// so the model makes no request. None when the article's link is not an http or https address.
pub fn link_card(item: &Item) -> Option<Card> {
    let url = validate_url(&item.link).ok()?;
    let metadata = LinkMetadata {
        original_url: url.clone(),
        url,
        title: item.title.clone(),
        site_name: item.source.clone(),
        description: item.body.clone(),
        icon: None,
        image: (!item.image.is_empty()).then(|| item.image.clone()),
        video: None,
    };
    Some(LinkView::loaded(metadata).card())
}

/// The language pairs installed on this PC for News. BifrOSt ships no translation language packs yet, so the
/// availability is empty and every pair reports `NotInstalled` until a pack is added.
pub fn installed() -> LanguageAvailability {
    LanguageAvailability::new(TRANSLATION_LANGUAGES)
}

/// The source and target codes for an article text. The target is the other of English and Icelandic: an
/// Icelandic article goes to English, and any other article goes to Icelandic (where its source is unsupported).
pub fn translation_plan(text: &str) -> Result<(String, String), TranslationError> {
    if text.trim().is_empty() {
        return Err(TranslationError::NothingToTranslate);
    }
    let language = dominant_language(text).ok_or(TranslationError::UnableToIdentifyLanguage)?;
    let source = language.code().to_string();
    let target = String::from(if source == "en" { "is" } else { "en" });
    Ok((source, target))
}

/// Translates the title and preview of an article through a `TranslationSession` for its language pair. The session
/// checks that the pair is installed in `availability` before the engine runs, so the error is the framework's own.
/// Returns the translated (title, preview).
pub fn translate_article(
    item: &Item,
    availability: &LanguageAvailability,
    engine: TranslationMemory,
) -> Result<(String, String), TranslationError> {
    let (source, target) = translation_plan(&format!("{} {}", item.title, item.body))?;
    let session = TranslationSession::new(availability, &source, &target, Box::new(engine))?;
    let title = session.translate(&item.title)?;
    let body = if item.body.trim().is_empty() { String::new() } else { session.translate(&item.body)? };
    Ok((title, body))
}

/// Reads the store. A missing or damaged file gives an empty list.
pub fn load_reading_list(dir: &Path) -> ReadingList {
    let text = std::fs::read_to_string(dir.join(STORE)).unwrap_or_default();
    ReadingList::from_text(&text, READING_LIMIT)
}

/// Writes the store. A failed write only loses the change; the list stays in memory until the next save.
pub fn save_reading_list(dir: &Path, list: &ReadingList) {
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(dir.join(STORE), list.to_text());
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Saves the article, or removes it when it is already saved. Returns whether it is saved afterwards.
pub fn toggle(list: &mut ReadingList, item: &Item) -> bool {
    if list.contains(&item.link) {
        list.remove(&item.link);
        return false;
    }
    list.add(&item.link, &item.title, &item.body, now_secs()).is_ok()
}

/// The saved pages as feed rows (newest first), so the list draws them like headlines.
pub fn rows(list: &ReadingList) -> Vec<Item> {
    list.items()
        .iter()
        .map(|saved| Item {
            title: saved.title.clone(),
            link: saved.url.clone(),
            date: String::new(),
            body: saved.preview.clone(),
            source: "Reading List".to_string(),
            image: String::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn article(title: &str, link: &str, body: &str) -> Item {
        Item { title: title.into(), link: link.into(), date: String::new(), body: body.into(), source: "RÚV".into(), image: String::new() }
    }

    #[test]
    fn english_text_is_recognised_and_named() {
        let text = "The council met on Monday and the members agreed that the new bridge will open to traffic in the spring.";
        assert_eq!(dominant_language(text), Some(Language::English));
        assert!(facts(text).starts_with("English"));
    }

    #[test]
    fn icelandic_text_is_recognised() {
        let text = "Þetta er mjög góð frétt og það er ekki hægt að segja annað um það sem gerðist í gær.";
        assert_eq!(dominant_language(text), Some(Language::Icelandic));
        assert_eq!(language_name(Language::Icelandic), "Icelandic");
    }

    #[test]
    fn words_follow_the_sdk_tokenizer() {
        assert_eq!(word_count("Don't stop, 2 cats"), 4);
        assert_eq!(word_count(""), 0);
        assert_eq!(facts(""), "");
        assert_eq!(facts("!!! ..."), "");
    }

    #[test]
    fn reading_time_rounds_up_and_is_at_least_a_minute() {
        assert_eq!(reading_minutes(1), 1);
        assert_eq!(reading_minutes(200), 1);
        assert_eq!(reading_minutes(201), 2);
    }

    #[test]
    fn link_card_uses_the_article_and_refuses_other_schemes() {
        let card = link_card(&article("Frétt", "https://www.ruv.is/frett/1", "Preview")).unwrap();
        assert_eq!((card.title.as_str(), card.subtitle.as_str(), card.placeholder), ("Frétt", "RÚV", false));
        assert!(card.image.is_none());
        assert!(link_card(&article("A", "javascript:alert(1)", "")).is_none());
        assert!(link_card(&article("A", "", "")).is_none());
    }

    #[test]
    fn reading_list_toggles_round_trips_and_lists_newest_first() {
        let mut list = ReadingList::new(READING_LIMIT);
        let first = article("One", "https://www.mbl.is/a", "preview");
        assert!(toggle(&mut list, &first));
        assert!(toggle(&mut list, &article("Two", "https://www.mbl.is/b", "")));
        let items = rows(&list);
        assert_eq!(items.len(), 2);
        assert_eq!((items[0].link.as_str(), items[0].title.as_str()), ("https://www.mbl.is/b", "Two"));
        assert_eq!(items[1].body, "preview");
        assert_eq!(items[1].source, "Reading List");
        let again = ReadingList::from_text(&list.to_text(), READING_LIMIT);
        assert!(again.contains("https://www.mbl.is/a"));
        assert!(!toggle(&mut list, &first));
        assert!(!list.contains("https://www.mbl.is/a"));
    }

    #[test]
    fn unsupported_addresses_are_not_saved() {
        let mut list = ReadingList::new(READING_LIMIT);
        assert!(!toggle(&mut list, &article("Bad", "ftp://example.is/x", "")));
        assert!(list.is_empty());
    }

    #[test]
    fn a_missing_store_is_an_empty_list() {
        assert!(load_reading_list(Path::new("/nonexistent/bifrost-news-store")).is_empty());
    }

    #[test]
    fn translation_plan_picks_the_other_of_english_and_icelandic() {
        let is = "Þetta er mjög góð frétt og það er ekki hægt að segja annað um það sem gerðist í gær.";
        let en = "The council met on Monday and the members agreed that the new bridge will open to traffic in the spring.";
        assert_eq!(translation_plan(is), Ok(("is".to_string(), "en".to_string())));
        assert_eq!(translation_plan(en), Ok(("en".to_string(), "is".to_string())));
        assert_eq!(translation_plan("  "), Err(TranslationError::NothingToTranslate));
        assert_eq!(translation_plan("!!! ..."), Err(TranslationError::UnableToIdentifyLanguage));
    }

    #[test]
    fn no_language_pack_is_installed_so_a_translation_reports_not_installed() {
        let body = "Þetta er mjög góð frétt og það er ekki hægt að segja annað um það sem gerðist í gær.";
        let item = article("Frétt", "https://www.ruv.is/frett/1", body);
        let r = translate_article(&item, &installed(), TranslationMemory::new());
        assert_eq!(r, Err(TranslationError::NotInstalled));
        assert_eq!(
            installed().status("is", "en"),
            bifrost_app::controls::translation::AvailabilityStatus::Supported
        );
    }

    #[test]
    fn an_installed_pair_translates_the_title_and_preview_through_the_engine() {
        let body = "Þetta er mjög góð frétt og það er ekki hægt að segja annað um það sem gerðist í gær.";
        let item = article("Frétt", "https://www.ruv.is/frett/1", body);
        let mut availability = installed();
        availability.install("is", "en");
        let mut memory = TranslationMemory::new();
        memory.insert("is", "en", "Frétt", "News");
        memory.insert("is", "en", body, "This is very good news.");
        let (title, preview) = translate_article(&item, &availability, memory).unwrap();
        assert_eq!(title, "News");
        assert_eq!(preview, "This is very good news.");
    }

    #[test]
    fn a_phrase_missing_from_the_engine_is_an_internal_error() {
        let body = "Þetta er mjög góð frétt og það er ekki hægt að segja annað um það sem gerðist í gær.";
        let item = article("Frétt", "https://www.ruv.is/frett/1", body);
        let mut availability = installed();
        availability.install("is", "en");
        assert_eq!(translate_article(&item, &availability, TranslationMemory::new()), Err(TranslationError::InternalError));
    }
}
