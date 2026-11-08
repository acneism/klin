mod lists;
mod updater;

pub use lists::{Lists, ListsError};
pub use updater::{BLOCKLIST_URL, ENTITYLIST_URL, ListUpdater};

use std::sync::Arc;

use klin_shared::Url;
use klin_shared::tracking::BlockCategory;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    Off,
    PrivateOnly,
    All,
}

impl Scope {
    pub fn from_prefs(all: bool, private: bool) -> Self {
        if all {
            Scope::All
        } else if private {
            Scope::PrivateOnly
        } else {
            Scope::Off
        }
    }

    fn applies(self, private: bool) -> bool {
        match self {
            Scope::Off => false,
            Scope::PrivateOnly => private,
            Scope::All => true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub tracking: Scope,
    pub content_list: bool,
    pub social: bool,
    pub cryptomining: bool,
    pub fingerprinting: bool,
    pub email: Scope,
}

impl Policy {
    pub const STANDARD: Policy = Policy {
        tracking: Scope::PrivateOnly,
        content_list: false,
        social: true,
        cryptomining: true,
        fingerprinting: true,
        email: Scope::PrivateOnly,
    };

    pub const STRICT: Policy = Policy {
        tracking: Scope::All,
        content_list: true,
        social: true,
        cryptomining: true,
        fingerprinting: true,
        email: Scope::All,
    };
}

impl Default for Policy {
    fn default() -> Self {
        Policy::STANDARD
    }
}

#[derive(Clone, Debug)]
pub struct Blocker {
    lists: Arc<Lists>,
    policy: Policy,
}

pub fn base_domain(host: &str) -> &str {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if bare.parse::<std::net::IpAddr>().is_ok() {
        return host;
    }
    psl::domain_str(host).unwrap_or(host)
}

impl Blocker {
    pub fn new(lists: Arc<Lists>, policy: Policy) -> Self {
        Self { lists, policy }
    }

    pub fn policy(&self) -> Policy {
        self.policy
    }

    pub fn classify(&self, request: &Url, top: &Url, private: bool) -> Option<BlockCategory> {
        if !matches!(request.scheme(), "http" | "https" | "ws" | "wss") {
            return None;
        }
        let host = request.host_str()?;
        let top_host = top.host_str()?;
        if base_domain(host) == base_domain(top_host) {
            return None;
        }
        let flags = self.lists.flags(host);
        if flags == 0 || self.lists.same_entity(top_host, host) {
            return None;
        }
        let policy = &self.policy;
        let tracking_mask = if policy.content_list {
            lists::TRACKING | lists::CONTENT
        } else {
            lists::TRACKING
        };
        if policy.cryptomining && flags & lists::CRYPTOMINING != 0 {
            return Some(BlockCategory::Cryptomining);
        }
        if policy.fingerprinting
            && flags & lists::FINGERPRINTING != 0
            && flags & (lists::TRACKING | lists::CONTENT) != 0
        {
            return Some(BlockCategory::Fingerprinting);
        }
        if policy.social && flags & lists::SOCIAL != 0 {
            return Some(BlockCategory::Social);
        }
        if policy.tracking.applies(private) && flags & tracking_mask != 0 {
            return Some(BlockCategory::Tracking);
        }
        if policy.email.applies(private) && flags & lists::EMAIL != 0 {
            return Some(BlockCategory::Tracking);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCKLIST: &str = r#"{
        "license": "test",
        "categories": {
            "Advertising": [{"Ads Inc": {"https://ads.example/": ["ads.example", "adcdn.example"]}}],
            "Analytics": [{"Stats": {"https://stats.example/": ["stats.example"], "performance": "true"}}],
            "Social": [{"Friends": {"https://friends.example/": ["friends.example"]}}],
            "Content": [{"Video": {"https://video.example/": ["video.example"]}}],
            "Cryptomining": [{"Miner": {"https://miner.example/": ["miner.example"]}}],
            "FingerprintingInvasive": [{"Ads Inc": {"https://ads.example/": ["ads.example"]}}, {"Lonely": {"https://fp.example/": ["fp.example"]}}],
            "Email": [{"Mail": {"https://mail.example/": ["mailtrack.example"]}}]
        }
    }"#;

    const ENTITIES: &str = r#"{
        "license": "test",
        "entities": {
            "Friends": {"properties": ["friends.example", "friendsapp.example"], "resources": ["friends.example", "fbcdn.example"]}
        }
    }"#;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    fn blocker(policy: Policy) -> Blocker {
        Blocker::new(
            Arc::new(Lists::parse(BLOCKLIST, Some(ENTITIES)).unwrap()),
            policy,
        )
    }

    #[test]
    fn standard_blocks_social_miners_and_fingerprinters_everywhere() {
        let b = blocker(Policy::STANDARD);
        let top = u("https://news.example.org/article");
        let check = |url: &str, private| b.classify(&u(url), &top, private);
        assert_eq!(
            check("https://cdn.friends.example/sdk.js", false),
            Some(BlockCategory::Social)
        );
        assert_eq!(
            check("https://miner.example/m.js", false),
            Some(BlockCategory::Cryptomining)
        );
        assert_eq!(
            check("https://ads.example/fp.js", false),
            Some(BlockCategory::Fingerprinting)
        );
        assert_eq!(check("https://fp.example/x.js", false), None);
        assert_eq!(check("https://adcdn.example/banner.png", false), None);
        assert_eq!(
            check("https://adcdn.example/banner.png", true),
            Some(BlockCategory::Tracking)
        );
        assert_eq!(
            check("https://stats.example/s.js", true),
            Some(BlockCategory::Tracking)
        );
        assert_eq!(check("https://video.example/player.js", true), None);
        assert_eq!(
            check("https://mailtrack.example/p.gif", true),
            Some(BlockCategory::Tracking)
        );
        assert_eq!(check("https://mailtrack.example/p.gif", false), None);
        assert_eq!(check("https://unknown.example/x.js", true), None);
    }

    #[test]
    fn strict_blocks_tracking_content_in_all_windows() {
        let b = blocker(Policy::STRICT);
        let top = u("https://news.example.org/");
        assert_eq!(
            b.classify(&u("https://adcdn.example/b.png"), &top, false),
            Some(BlockCategory::Tracking)
        );
        assert_eq!(
            b.classify(&u("https://video.example/p.js"), &top, false),
            Some(BlockCategory::Tracking)
        );
    }

    #[test]
    fn first_party_and_same_entity_loads_are_allowed() {
        let b = blocker(Policy::STRICT);
        assert_eq!(
            b.classify(
                &u("https://ads.example/a.js"),
                &u("https://www.ads.example/"),
                false
            ),
            None
        );
        assert_eq!(
            b.classify(
                &u("https://fbcdn.example/a.js"),
                &u("https://friendsapp.example/"),
                false
            ),
            None
        );
        assert_eq!(
            b.classify(
                &u("https://friends.example/a.js"),
                &u("https://m.friends.example/"),
                false
            ),
            None
        );
        assert_eq!(
            b.classify(&u("data:text/plain,x"), &u("https://a.example/"), false),
            None
        );
    }

    #[test]
    fn base_domain_uses_public_suffix_list() {
        assert_eq!(base_domain("a.b.example.co.uk"), "example.co.uk");
        assert_eq!(base_domain("www.example.com"), "example.com");
        assert_eq!(base_domain("localhost"), "localhost");
        assert_eq!(base_domain("127.0.0.1"), "127.0.0.1");
        assert_eq!(base_domain("[::1]"), "[::1]");
    }
}
