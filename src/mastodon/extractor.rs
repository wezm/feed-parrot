use std::sync::LazyLock;

use fancy_regex::Regex;

const MAX_DOMAIN_LENGTH: usize = 253;

const USERNAME_RE: &str = "((?i)[a-z0-9_]+(?:[a-z0-9_.-]+[a-z0-9_]+)?)"; // i

// Ruby also supports the following non-POSIX character classes:
//
// /[[:word:]]/ - A character in one of the following Unicode general categories Letter, Mark, Number, Connector_Punctuation
const WORD: &str = r"\p{Letter}\p{Mark}\p{Number}\p{Connector_Punctuation}";

const AT_SIGNS: &str = "[@＠]";

// https://github.com/twitter/twitter-text/blob/30e2430d90cff3b46393ea54caf511441983c260/rb/lib/twitter-text/regex.rb#L94
// Excludes 0xd7 from the range (the multiplication sign, confusable with "x").
// Also excludes 0xf7, the division sign
const LATIN_ACCENT_CHARS: &[(char, char)] = &[
    ('\u{00c0}', '\u{00d6}'),
    ('\u{00d8}', '\u{00f6}'),
    ('\u{00f8}', '\u{00ff}'),
    ('\u{0100}', '\u{024f}'),
    ('\u{0253}', '\u{0254}'),
    ('\u{0256}', '\u{0257}'),
    ('\u{0259}', '\u{0259}'),
    ('\u{025b}', '\u{025b}'),
    ('\u{0263}', '\u{0263}'),
    ('\u{0268}', '\u{0268}'),
    ('\u{026f}', '\u{026f}'),
    ('\u{0272}', '\u{0272}'),
    ('\u{0289}', '\u{0289}'),
    ('\u{028b}', '\u{028b}'),
    ('\u{02bb}', '\u{02bb}'),
    ('\u{0300}', '\u{036f}'),
    ('\u{1e00}', '\u{1eff}')
];

static MENTION_RE: LazyLock<Regex> = LazyLock::new(|| {
    let expr = format!(r"(?<![=/[{word}]])@{USERNAME_RE}(?:@([{word}.-]+[{word}]+))?", word = WORD);
    Regex::new(&expr).unwrap()
});

static END_MENTION_MATCH: LazyLock<Regex> = LazyLock::new(|| {
    let latin_accents = LATIN_ACCENT_CHARS.iter().copied().fold(String::new(), |mut string, (start, end)| {
        string.push(start);
        if start != end {
            string.push('-');
            string.push(end);
        }
        string
    });
    let expr = format!(r"(?i)\A(?:{AT_SIGNS}|[{latin_accents}]+|://)");
    Regex::new(&expr).unwrap()
});

struct Mention<'a>(fancy_regex::Captures<'a>);

fn detect_mentions(text: &str) -> Option<impl Iterator<Item=fancy_regex::Result<Mention<'_>>>> {
    if !text.contains('@') {
        return None;
    }

    Some(MENTION_RE.captures_iter(text)
        .filter_map(|captures| {
            let captures = match captures {
                Ok(captures) => captures,
                Err(err) => return Some(Err(err))
            };
            let mention = Mention(captures);

            let after = text.get(mention.end()..)?;
            let after_matches = match dbg!(END_MENTION_MATCH.is_match(after)) {
                Ok(x) => x,
                Err(err) => return Some(Err(err)),
            };

            if after_matches || domain_too_long(mention.domain()) {
                return None;
            }
            Some(Ok(mention))
        }))
}

fn domain_too_long(domain: Option<&str>) -> bool {
    matches!(domain, Some(domain) if domain.chars().count() > MAX_DOMAIN_LENGTH)
}

impl Mention<'_> {
    pub fn start(&self) -> usize {
        // NOTE(unwrap): Safe as capture zero should always exist
        self.0.get(0).unwrap().start()
    }

    pub fn end(&self) -> usize {
        // NOTE(unwrap): Safe as capture zero should always exist
        self.0.get(0).unwrap().end()
    }

    pub fn as_str(&self) -> &str {
        &self.0[0]
    }

    pub fn username(&self) -> &str {
        &self.0[1]
    }

    pub fn domain(&self) -> Option<&str> {
        self.0.get(2).map(|m| m.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_mentions<'a>(iter: Option<impl Iterator<Item=fancy_regex::Result<Mention<'a>>>>) -> Vec<String> {
        match iter {
            None => Vec::new(),
            Some(iter) => iter.map(|mention| mention.map(|m| m.as_str().to_string()).unwrap()).collect()
        }
    }

    #[test]
    fn test_detect_mentions() {
        assert_eq!(collect_mentions(detect_mentions("@wezm")), &["@wezm".to_string()]);
        assert_eq!(collect_mentions(detect_mentions("some @wezm")), &["@wezm".to_string()]);
        assert_eq!(collect_mentions(detect_mentions("some @wezm.")), &["@wezm".to_string()]);
        assert_eq!(collect_mentions(detect_mentions("some @wezm@mastodon.decentralied.social.")), &["@wezm@mastodon.decentralied.social".to_string()]);
        assert_eq!(collect_mentions(detect_mentions("@one @two @three ok")), &["@one".to_string(), "@two".to_string(), "@three".to_string()]);
        assert_eq!(collect_mentions(detect_mentions("test @test@wòw.com ok")), &["@test@wòw.com".to_string()]);
        assert_eq!(collect_mentions(detect_mentions("test @Test@Wòw.Com ok")), &["@Test@Wòw.Com".to_string()]);
        assert_eq!(collect_mentions(detect_mentions("An email: test@example.com")), Vec::<String>::new());
        assert_eq!(collect_mentions(detect_mentions("An email: @user@")), Vec::<String>::new());
        assert_eq!(collect_mentions(detect_mentions("")), Vec::<String>::new());
        assert_eq!(collect_mentions(detect_mentions("@test@example-domain.com.au")), &["@test@example-domain.com.au".to_string()]);
    }

    #[test]
    fn test_detect_mentions_after() {
        assert_eq!(collect_mentions(detect_mentions("@user@")), Vec::<String>::new());
        assert_eq!(collect_mentions(detect_mentions("@café")), Vec::<String>::new());
    }

    #[test]
    fn extract_username_and_domain() {
        let mentions = detect_mentions("some @one @two@example.com ").unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(mentions[0].username(), "one");
        assert_eq!(mentions[0].domain(), None);

        assert_eq!(mentions[1].username(), "two");
        assert_eq!(mentions[1].domain(), Some("example.com"));
    }
}
