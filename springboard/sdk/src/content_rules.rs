//! Content rules: block and allow rules that a page's requests are checked against, written with a url-filter subset
//! (`*`, `^`, a leading `||`, a leading or trailing `|`). The model lives in `bifrost_ui::content_rules`; this module
//! re-exports it for `bifrost_app`. The rules are not enforced in the web view yet, so an app can check its own requests.

pub use bifrost_ui::content_rules::{Action, ResourceType, Rule, RuleList, Verdict};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_rule_matches_a_request_through_the_sdk_path() {
        let mut list = RuleList::new();
        list.add(Rule::block("||ads.example^"));
        let verdict = list.check("https://ads.example/banner.js", "https://news.example/", ResourceType::Script);
        assert_eq!(verdict, Verdict::Block);
        let other = list.check("https://news.example/story", "https://news.example/", ResourceType::Document);
        assert_eq!(other, Verdict::Allow);
    }
}
