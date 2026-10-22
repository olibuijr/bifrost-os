//! Cantonese input by Jyutping (the TYIM source): the letters typed are a Jyutping reading, a tone digit from 1 to 6
//! ends a syllable, and the candidates for the reading come from a seed table. Space picks the first candidate. Once a
//! reading ends in a tone digit, a further digit from 1 to 9 picks that candidate (so `nei5` then `2` is the second
//! candidate for nei5). A reading may hold several syllables (`nei5hou2`), and a reading without tones (`nei`) offers
//! the candidates of every toned reading that matches it.
//!
//! A digit counts as a tone only after a well-formed syllable (an initial and a final of the Jyutping scheme, or the
//! syllabic `m` and `ng`); after anything else it is not taken, so the controller commits the letters as typed.
//!
//! The table holds a few hundred common readings (single characters and everyday words). No full dictionary is
//! shipped, so rarer readings have no candidate yet; the table is data, and a larger dictionary replaces it without
//! changing the composer.

use super::han::Entry;
use super::Composer;

/// Jyutping initials, longest first so `ng` and `gw` win over `n` and `g`.
const INITIALS: &[&str] = &["ng", "gw", "kw", "b", "p", "m", "f", "d", "t", "n", "l", "g", "k", "h", "w", "z", "c", "s", "j"];

/// Jyutping finals, including the syllabic nasals `m` and `ng`.
const FINALS: &[&str] = &[
    "aa", "aai", "aau", "aam", "aan", "aang", "aap", "aat", "aak", "a", "ai", "au", "am", "an", "ang", "ap", "at", "ak",
    "e", "ei", "eu", "em", "eng", "ep", "ek", "i", "iu", "im", "in", "ing", "ip", "it", "ik", "o", "oi", "ou", "on",
    "ong", "ot", "ok", "u", "ui", "un", "ung", "ut", "uk", "oe", "oeng", "oek", "eoi", "eon", "eot", "yu", "yun", "yut",
    "m", "ng",
];

/// Common readings: syllables with their tone digit, joined for a word, and the characters they can stand for.
pub const JYUTPING: &[Entry] = &[
    Entry { reading: "nei5", text: &["你", "妳"] },
    Entry { reading: "hou2", text: &["好"] },
    Entry { reading: "nei5hou2", text: &["你好"] },
    Entry { reading: "ngo5", text: &["我"] },
    Entry { reading: "m4", text: &["唔"] },
    Entry { reading: "goi1", text: &["該"] },
    Entry { reading: "m4goi1", text: &["唔該"] },
    Entry { reading: "do1", text: &["多"] },
    Entry { reading: "ze6", text: &["謝"] },
    Entry { reading: "do1ze6", text: &["多謝"] },
    Entry { reading: "zou2", text: &["早"] },
    Entry { reading: "san4", text: &["晨", "神", "臣"] },
    Entry { reading: "zou2san4", text: &["早晨"] },
    Entry { reading: "jau5", text: &["有"] },
    Entry { reading: "mou5", text: &["冇", "武", "舞"] },
    Entry { reading: "hai6", text: &["係", "系"] },
    Entry { reading: "si6", text: &["是", "事", "市", "士"] },
    Entry { reading: "ge3", text: &["嘅"] },
    Entry { reading: "go2", text: &["嗰"] },
    Entry { reading: "go3", text: &["個"] },
    Entry { reading: "ni1", text: &["呢"] },
    Entry { reading: "zung1", text: &["中", "忠", "鐘"] },
    Entry { reading: "gwok3", text: &["國"] },
    Entry { reading: "zung1gwok3", text: &["中國"] },
    Entry { reading: "man4", text: &["文", "民", "聞"] },
    Entry { reading: "zung1man4", text: &["中文"] },
    Entry { reading: "jyut6", text: &["月", "粵"] },
    Entry { reading: "jyu5", text: &["語", "雨", "宇"] },
    Entry { reading: "jyut6jyu5", text: &["粵語"] },
    Entry { reading: "gwong2", text: &["廣"] },
    Entry { reading: "dung1", text: &["東"] },
    Entry { reading: "gwong2dung1", text: &["廣東"] },
    Entry { reading: "hoeng1", text: &["香"] },
    Entry { reading: "gong2", text: &["港", "講"] },
    Entry { reading: "hoeng1gong2", text: &["香港"] },
    Entry { reading: "sik6", text: &["食"] },
    Entry { reading: "jam2", text: &["飲"] },
    Entry { reading: "faan6", text: &["飯"] },
    Entry { reading: "sik6faan6", text: &["食飯"] },
    Entry { reading: "jat1", text: &["一", "壹"] },
    Entry { reading: "ji6", text: &["二", "貳"] },
    Entry { reading: "saam1", text: &["三"] },
    Entry { reading: "sei3", text: &["四"] },
    Entry { reading: "ng5", text: &["五"] },
    Entry { reading: "luk6", text: &["六"] },
    Entry { reading: "cat1", text: &["七"] },
    Entry { reading: "baat3", text: &["八"] },
    Entry { reading: "gau2", text: &["九"] },
    Entry { reading: "sap6", text: &["十"] },
    Entry { reading: "gam1", text: &["今"] },
    Entry { reading: "jat6", text: &["日"] },
    Entry { reading: "gam1jat6", text: &["今日"] },
    Entry { reading: "tin1", text: &["天"] },
    Entry { reading: "hei3", text: &["氣"] },
    Entry { reading: "tin1hei3", text: &["天氣"] },
    Entry { reading: "ho2", text: &["可"] },
    Entry { reading: "ji5", text: &["以", "已", "耳"] },
    Entry { reading: "ho2ji5", text: &["可以"] },
    Entry { reading: "bin1", text: &["邊"] },
    Entry { reading: "dou6", text: &["度", "道"] },
    Entry { reading: "bin1dou6", text: &["邊度"] },
    Entry { reading: "dou1", text: &["都"] },
    Entry { reading: "dak1", text: &["得"] },
    Entry { reading: "dik1", text: &["的"] },
    Entry { reading: "oi3", text: &["愛"] },
    Entry { reading: "loi4", text: &["來"] },
    Entry { reading: "heoi3", text: &["去"] },
    Entry { reading: "taai3", text: &["太", "泰"] },
    Entry { reading: "mat1", text: &["乜"] },
    Entry { reading: "je5", text: &["嘢"] },
    Entry { reading: "mat1je5", text: &["乜嘢"] },
    Entry { reading: "zou6", text: &["做"] },
    Entry { reading: "waa6", text: &["話"] },
    Entry { reading: "tung4", text: &["同"] },
    // More everyday readings: pronouns, particles, family, school, time, travel, food and greetings.
    Entry { reading: "ngo5dei6", text: &["我哋"] },
    Entry { reading: "nei5dei6", text: &["你哋"] },
    Entry { reading: "keoi5", text: &["佢"] },
    Entry { reading: "keoi5dei6", text: &["佢哋"] },
    Entry { reading: "me1", text: &["咩"] },
    Entry { reading: "dim2", text: &["點"] },
    Entry { reading: "gaai2", text: &["解"] },
    Entry { reading: "dim2gaai2", text: &["點解"] },
    Entry { reading: "gei2", text: &["幾"] },
    Entry { reading: "gei2do1", text: &["幾多"] },
    Entry { reading: "zo2", text: &["咗"] },
    Entry { reading: "gan2", text: &["緊"] },
    Entry { reading: "hai2", text: &["喺"] },
    Entry { reading: "hai2dou6", text: &["喺度"] },
    Entry { reading: "jan1", text: &["因"] },
    Entry { reading: "wai6", text: &["為"] },
    Entry { reading: "jan1wai6", text: &["因為"] },
    Entry { reading: "so2", text: &["所"] },
    Entry { reading: "so2ji5", text: &["所以"] },
    Entry { reading: "daan6", text: &["但"] },
    Entry { reading: "daan6hai6", text: &["但係"] },
    Entry { reading: "jyu4", text: &["如", "魚"] },
    Entry { reading: "gwo2", text: &["果"] },
    Entry { reading: "jyu4gwo2", text: &["如果"] },
    Entry { reading: "zau6", text: &["就"] },
    Entry { reading: "jau6", text: &["又"] },
    Entry { reading: "zoi3", text: &["再"] },
    Entry { reading: "gin3", text: &["見"] },
    Entry { reading: "zoi3gin3", text: &["再見"] },
    Entry { reading: "deoi3", text: &["對"] },
    Entry { reading: "zyu6", text: &["住"] },
    Entry { reading: "deoi3m4zyu6", text: &["對唔住"] },
    Entry { reading: "sai2", text: &["使"] },
    Entry { reading: "m4sai2", text: &["唔使"] },
    Entry { reading: "maan5", text: &["晚"] },
    Entry { reading: "on1", text: &["安"] },
    Entry { reading: "maan5on1", text: &["晚安"] },
    Entry { reading: "pang4", text: &["朋"] },
    Entry { reading: "pang4jau5", text: &["朋友"] },
    Entry { reading: "jan4", text: &["人"] },
    Entry { reading: "daai6", text: &["大"] },
    Entry { reading: "siu2", text: &["小"] },
    Entry { reading: "gaa1", text: &["家"] },
    Entry { reading: "daai6gaa1", text: &["大家"] },
    Entry { reading: "uk1", text: &["屋"] },
    Entry { reading: "kei2", text: &["企"] },
    Entry { reading: "uk1kei2", text: &["屋企"] },
    Entry { reading: "hok6", text: &["學"] },
    Entry { reading: "haau6", text: &["校"] },
    Entry { reading: "hok6haau6", text: &["學校"] },
    Entry { reading: "lou5", text: &["老"] },
    Entry { reading: "si1", text: &["師", "思", "絲"] },
    Entry { reading: "lou5si1", text: &["老師"] },
    Entry { reading: "sin1", text: &["先"] },
    Entry { reading: "saang1", text: &["生"] },
    Entry { reading: "sin1saang1", text: &["先生"] },
    Entry { reading: "seoi2", text: &["水"] },
    Entry { reading: "caa4", text: &["茶"] },
    Entry { reading: "jam2caa4", text: &["飲茶"] },
    Entry { reading: "gaa3", text: &["咖", "價"] },
    Entry { reading: "fe1", text: &["啡"] },
    Entry { reading: "gaa3fe1", text: &["咖啡"] },
    Entry { reading: "juk6", text: &["肉"] },
    Entry { reading: "syu1", text: &["書"] },
    Entry { reading: "din6", text: &["電"] },
    Entry { reading: "din6waa6", text: &["電話"] },
    Entry { reading: "nou5", text: &["腦"] },
    Entry { reading: "din6nou5", text: &["電腦"] },
    Entry { reading: "din6si6", text: &["電視"] },
    Entry { reading: "ce1", text: &["車"] },
    Entry { reading: "dei6", text: &["地"] },
    Entry { reading: "tit3", text: &["鐵"] },
    Entry { reading: "dei6tit3", text: &["地鐵"] },
    Entry { reading: "fei1", text: &["飛"] },
    Entry { reading: "gei1", text: &["機"] },
    Entry { reading: "fei1gei1", text: &["飛機"] },
    Entry { reading: "coeng4", text: &["場", "長"] },
    Entry { reading: "gei1coeng4", text: &["機場"] },
    Entry { reading: "cin2", text: &["錢"] },
    Entry { reading: "maai5", text: &["買"] },
    Entry { reading: "maai6", text: &["賣"] },
    Entry { reading: "gei2cin2", text: &["幾錢"] },
    Entry { reading: "ngan4", text: &["銀"] },
    Entry { reading: "hong4", text: &["行"] },
    Entry { reading: "ngan4hong4", text: &["銀行"] },
    Entry { reading: "nin4", text: &["年"] },
    Entry { reading: "sing1", text: &["星"] },
    Entry { reading: "kei4", text: &["期"] },
    Entry { reading: "sing1kei4", text: &["星期"] },
    Entry { reading: "si4", text: &["時"] },
    Entry { reading: "gaan1", text: &["間"] },
    Entry { reading: "si4gaan1", text: &["時間"] },
    Entry { reading: "ziu1", text: &["朝"] },
    Entry { reading: "ziu1zou2", text: &["朝早"] },
    Entry { reading: "je6", text: &["夜"] },
    Entry { reading: "haa6", text: &["下"] },
    Entry { reading: "zau3", text: &["晝"] },
    Entry { reading: "haa6zau3", text: &["下晝"] },
    Entry { reading: "ming4", text: &["明"] },
    Entry { reading: "ming4tin1", text: &["明天"] },
    Entry { reading: "zok6", text: &["昨"] },
    Entry { reading: "zok6jat6", text: &["昨日"] },
    Entry { reading: "ting1", text: &["聽"] },
    Entry { reading: "ting1jat6", text: &["聽日"] },
    Entry { reading: "tai2", text: &["睇"] },
    Entry { reading: "se2", text: &["寫"] },
    Entry { reading: "duk6", text: &["讀"] },
    Entry { reading: "sik1", text: &["識"] },
    Entry { reading: "zi1", text: &["知"] },
    Entry { reading: "soeng2", text: &["想"] },
    Entry { reading: "jiu3", text: &["要"] },
    Entry { reading: "hang4", text: &["行"] },
    Entry { reading: "leng3", text: &["靚"] },
    Entry { reading: "hoi1", text: &["開"] },
    Entry { reading: "sam1", text: &["心"] },
    Entry { reading: "hoi1sam1", text: &["開心"] },
    Entry { reading: "faai3", text: &["快"] },
    Entry { reading: "lok6", text: &["樂"] },
    Entry { reading: "faai3lok6", text: &["快樂"] },
    Entry { reading: "naan4", text: &["難"] },
    Entry { reading: "san1", text: &["新"] },
    Entry { reading: "gau6", text: &["舊"] },
    Entry { reading: "mo1", text: &["麼"] },
    Entry { reading: "maa3", text: &["嗎"] },
    Entry { reading: "aa3", text: &["呀"] },
    Entry { reading: "laa1", text: &["啦"] },
    Entry { reading: "laa3", text: &["喇"] },
    Entry { reading: "lo1", text: &["囉"] },
    Entry { reading: "aa1", text: &["吖"] },
    Entry { reading: "hoeng1gong2jan4", text: &["香港人"] },
    Entry { reading: "waa2", text: &["話"] },
    Entry { reading: "gwong2dung1waa2", text: &["廣東話"] },
    Entry { reading: "zung1gwok3jan4", text: &["中國人"] },
    Entry { reading: "pou2", text: &["普"] },
    Entry { reading: "tung1", text: &["通"] },
    Entry { reading: "pou2tung1waa2", text: &["普通話"] },
    Entry { reading: "jing1", text: &["英"] },
    Entry { reading: "jing1man4", text: &["英文"] },
    Entry { reading: "bun2", text: &["本"] },
    Entry { reading: "jat6bun2", text: &["日本"] },
    Entry { reading: "mat6", text: &["物"] },
    Entry { reading: "sik6mat6", text: &["食物"] },
    Entry { reading: "min6", text: &["麵"] },
    Entry { reading: "baau1", text: &["包"] },
    Entry { reading: "min6baau1", text: &["麵包"] },
    Entry { reading: "mai5", text: &["米"] },
    Entry { reading: "coi3", text: &["菜"] },
    Entry { reading: "saai3", text: &["晒"] },
    Entry { reading: "do1ze6saai3", text: &["多謝晒"] },
    Entry { reading: "gung1", text: &["恭", "工"] },
    Entry { reading: "hei2", text: &["喜"] },
    Entry { reading: "faat3", text: &["發"] },
    Entry { reading: "coi4", text: &["財"] },
    Entry { reading: "gung1hei2faat3coi4", text: &["恭喜發財"] },
    Entry { reading: "san1nin4", text: &["新年"] },
    Entry { reading: "san1nin4faai3lok6", text: &["新年快樂"] },
    Entry { reading: "saang1jat6", text: &["生日"] },
    Entry { reading: "saang1jat6faai3lok6", text: &["生日快樂"] },
    Entry { reading: "zok3", text: &["作"] },
    Entry { reading: "gung1zok3", text: &["工作"] },
    Entry { reading: "m4hou2", text: &["唔好"] },
    Entry { reading: "ji3", text: &["意"] },
    Entry { reading: "m4hou2ji3si1", text: &["唔好意思"] },
    Entry { reading: "bin1go3", text: &["邊個"] },
    Entry { reading: "joeng2", text: &["樣"] },
    Entry { reading: "dim2joeng2", text: &["點樣"] },
    Entry { reading: "gam3", text: &["咁"] },
    Entry { reading: "hou2m4hou2", text: &["好唔好"] },
    Entry { reading: "ling4", text: &["零"] },
    Entry { reading: "baak3", text: &["百"] },
    Entry { reading: "cin1", text: &["千"] },
    Entry { reading: "maan6", text: &["萬"] },
    Entry { reading: "loeng5", text: &["兩"] },
    Entry { reading: "bun3", text: &["半"] },
    Entry { reading: "jau5mou5", text: &["有冇"] },
    Entry { reading: "ngo5oi3nei5", text: &["我愛你"] },
];

/// Whether `s` (letters only, no tone digit) is a well-formed Jyutping syllable: an optional initial and a final, or a
/// syllabic nasal alone.
pub fn is_syllable(s: &str) -> bool {
    if FINALS.contains(&s) {
        return true;
    }
    INITIALS.iter().any(|i| s.strip_prefix(i).is_some_and(|rest| FINALS.contains(&rest)))
}

/// The tone digit of a character: 1 to 6, else None.
fn tone(ch: char) -> Option<u8> {
    match ch {
        '1'..='6' => Some(ch as u8 - b'0'),
        _ => None,
    }
}

/// The letters after the last digit of `reading`: the syllable being typed.
fn current_syllable(reading: &str) -> &str {
    reading.rsplit(|c: char| c.is_ascii_digit()).next().unwrap_or("")
}

/// The candidates for a reading. A reading with tone digits matches the table exactly; one without matches every
/// toned entry that has the same letters, in table order.
pub fn candidates(reading: &str) -> Vec<&'static str> {
    if reading.is_empty() {
        return vec![];
    }
    if reading.chars().any(|c| c.is_ascii_digit()) {
        return super::han::candidates(JYUTPING, reading).to_vec();
    }
    JYUTPING
        .iter()
        .filter(|e| e.reading.chars().filter(|c| !c.is_ascii_digit()).eq(reading.chars()))
        .flat_map(|e| e.text.iter().copied())
        .collect()
}

/// The Jyutping composer: letters and tone digits build the reading, and candidates are picked by space or a digit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JyutpingComposer {
    reading: String,
}

impl Composer for JyutpingComposer {
    fn feed(&mut self, ch: char) -> Option<String> {
        if ch.is_ascii_lowercase() {
            self.reading.push(ch);
            return Some(String::new());
        }
        if self.reading.is_empty() {
            return None;
        }
        let ends_in_tone = self.reading.ends_with(|c: char| c.is_ascii_digit());
        // a digit after a letter is a tone when the letters before it form a syllable
        if let (Some(_), false) = (tone(ch), ends_in_tone) {
            if is_syllable(current_syllable(&self.reading)) {
                self.reading.push(ch);
                return Some(String::new());
            }
            return None;
        }
        let list = candidates(&self.reading);
        let pick = match ch {
            ' ' => list.first(),
            '1'..='9' if ends_in_tone => list.get((ch as u8 - b'1') as usize),
            _ => None,
        }?;
        self.reading.clear();
        Some(pick.to_string())
    }

    fn backspace(&mut self) -> bool {
        self.reading.pop().is_some()
    }

    fn composed(&self) -> String {
        self.reading.clone()
    }

    fn reset(&mut self) {
        self.reading.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::type_keys;
    use super::super::JYUTPING as SOURCE;
    use super::*;

    fn typed(s: &str) -> JyutpingComposer {
        let mut c = JyutpingComposer::default();
        for ch in s.chars() {
            assert_eq!(c.feed(ch), Some(String::new()), "{ch} belongs to the reading");
        }
        c
    }

    #[test]
    fn syllables_are_checked_against_initials_and_finals() {
        for ok in ["nei", "hou", "ngo", "gwong", "kwai", "m", "ng", "aa", "zung", "jyut", "hoeng", "heoi", "dou"] {
            assert!(is_syllable(ok), "{ok}");
        }
        for bad in ["", "xyz", "nn", "gwx", "qu", "hoengg", "b"] {
            assert!(!is_syllable(bad), "{bad}");
        }
    }

    #[test]
    fn letters_and_a_tone_build_the_reading_and_space_picks_the_first_candidate() {
        let mut c = typed("nei5");
        assert_eq!(c.composed(), "nei5");
        assert_eq!(c.feed(' '), Some("你".to_string()));
        assert_eq!(c.composed(), "");
    }

    #[test]
    fn a_digit_after_the_tone_picks_that_candidate() {
        let mut c = typed("nei5");
        assert_eq!(c.feed('2'), Some("妳".to_string()));
        let mut d = typed("nei5");
        assert_eq!(d.feed('9'), None, "no ninth candidate, so the digit is literal");
        assert_eq!(d.composed(), "nei5", "the reading is kept for the controller to commit");
    }

    #[test]
    fn a_word_reading_has_its_own_candidates() {
        let mut c = typed("nei5hou2");
        assert_eq!(c.feed(' '), Some("你好".to_string()));
        let mut d = typed("m4goi1");
        assert_eq!(d.feed(' '), Some("唔該".to_string()));
    }

    #[test]
    fn a_reading_without_tones_offers_every_toned_match() {
        assert_eq!(candidates("jyut"), vec!["月", "粵"], "jyut6 is the one toned match");
        assert!(candidates("qqq").is_empty());
        assert_eq!(candidates("san"), vec!["晨", "神", "臣", "新"], "san4 first, then san1, in table order");
        let mut c = typed("hou");
        assert_eq!(c.feed(' '), Some("好".to_string()));
    }

    #[test]
    fn a_digit_after_something_that_is_not_a_syllable_is_not_a_tone() {
        let mut c = typed("xyz");
        assert_eq!(c.feed('1'), None);
        assert_eq!(c.composed(), "xyz");
        let mut idle = JyutpingComposer::default();
        assert_eq!(idle.feed('1'), None, "nothing typed, so the digit is literal");
    }

    #[test]
    fn a_syllabic_nasal_takes_a_tone() {
        let mut c = typed("ng5");
        assert_eq!(c.feed(' '), Some("五".to_string()));
        let mut d = typed("m4");
        assert_eq!(d.feed(' '), Some("唔".to_string()));
    }

    #[test]
    fn a_reading_without_candidates_ends_on_space() {
        // a valid syllable the table has no character for
        let mut c = typed("deu6");
        assert_eq!(c.feed(' '), None, "the controller commits the reading and inserts the space");
        assert_eq!(c.composed(), "deu6");
    }

    #[test]
    fn backspace_removes_the_last_key_tone_included() {
        let mut c = typed("nei5");
        assert!(c.backspace());
        assert_eq!(c.composed(), "nei");
        c.reset();
        assert!(!c.backspace());
    }

    #[test]
    fn every_table_reading_is_well_formed_and_has_candidates() {
        for e in JYUTPING {
            assert!(!e.text.is_empty(), "{}", e.reading);
            let mut rest = e.reading;
            while !rest.is_empty() {
                let end = rest.find(|c: char| c.is_ascii_digit()).expect("a tone digit ends each syllable");
                assert!(is_syllable(&rest[..end]), "{} in {}", &rest[..end], e.reading);
                assert!(tone(rest.as_bytes()[end] as char).is_some(), "{}", e.reading);
                rest = &rest[end + 1..];
            }
        }
    }

    #[test]
    fn no_reading_is_listed_twice() {
        for (i, e) in JYUTPING.iter().enumerate() {
            assert!(JYUTPING[..i].iter().all(|o| o.reading != e.reading), "{} is listed twice", e.reading);
        }
    }

    #[test]
    fn the_table_holds_a_few_hundred_readings() {
        assert!(JYUTPING.len() >= 200, "{} readings", JYUTPING.len());
    }

    #[test]
    fn everyday_words_have_candidates() {
        for (reading, first) in [
            ("dim2gaai2", "點解"),
            ("zoi3gin3", "再見"),
            ("m4hou2ji3si1", "唔好意思"),
            ("gung1hei2faat3coi4", "恭喜發財"),
            ("saang1jat6faai3lok6", "生日快樂"),
            ("pou2tung1waa2", "普通話"),
            ("keoi5", "佢"),
        ] {
            assert_eq!(candidates(reading).first().copied(), Some(first), "{reading}");
        }
    }

    #[test]
    fn a_toneless_reading_lists_each_tone_of_the_letters_in_table_order() {
        assert_eq!(candidates("gaa"), vec!["家", "咖", "價"]);
        assert_eq!(candidates("coeng"), vec!["場", "長"]);
    }

    #[test]
    fn a_typed_greeting_commits_as_one_word() {
        let (mut ctl, mut client) = type_keys(SOURCE, "zoi3gin3");
        ctl.handle(super::super::Key::Char(' '), &mut client);
        assert_eq!(client.committed, "再見");
    }

    #[test]
    fn the_controller_types_cantonese_into_a_field() {
        let (mut ctl, mut client) = type_keys(SOURCE, "nei5hou2");
        assert_eq!(client.marked, "nei5hou2");
        ctl.handle(super::super::Key::Char(' '), &mut client);
        assert_eq!(client.committed, "你好");
        assert_eq!(client.marked, "");
        assert_eq!(ctl.composed_string(), "");
    }
}
