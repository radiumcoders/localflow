//! Snippets: a dictation that is only a trigger ("my email") pastes the text
//! saved for it in snippets.txt, word for word, instead of being cleaned up.
//! Only the whole dictation counts: "my email is broken" is a dictation, and
//! the cleanup would rewrite a snippet put inside one anyway.

use crate::Localflow;

/// A line of snippets.txt: 'my email => jay@example.com'.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    /// What you say, as written in the file.
    pub trigger: String,
    /// What's pasted, with each \n in the file a new line.
    pub text: String,
}

/// The snippets in a snippets.txt: 'trigger => text', one a line, # for a
/// comment. A line without a trigger or a text is skipped.
pub fn parse(file: &str) -> Vec<Snippet> {
    file.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let (trigger, text) = line.split_once("=>")?;
            let (trigger, text) = (trigger.trim(), text.trim());
            (!bare(trigger).is_empty() && !text.is_empty())
                .then(|| Snippet { trigger: trigger.to_string(), text: text.replace("\\n", "\n") })
        })
        .collect()
}

/// The words, lowercase, without punctuation, one space apart. An apostrophe
/// is dropped ("don't" is "dont"); anything else between words is a space
/// ("sign-off" is "sign off").
fn bare(said: &str) -> String {
    let words: String = said
        .to_lowercase()
        .chars()
        .filter(|c| !matches!(c, '\'' | '’'))
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    words.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Said before a trigger, and not part of it: "insert my email".
const VERBS: &[&str] = &["insert ", "paste ", "type "];

/// The snippet a whole dictation asks for, or None when it's a dictation.
/// A trigger that starts with one of the verbs still matches as written.
pub fn snippet<'a>(said: &str, snippets: &'a [Snippet]) -> Option<&'a Snippet> {
    let said = bare(said);
    let without_verb = VERBS.iter().find_map(|verb| said.strip_prefix(verb)).unwrap_or(&said);
    snippets.iter().find(|s| {
        let trigger = bare(&s.trigger);
        trigger == said || trigger == without_verb
    })
}

impl Localflow {
    /// Your snippets, read on every dictation like words.txt (none without
    /// the file).
    pub fn snippets(&self) -> Vec<Snippet> {
        std::fs::read_to_string(self.cfg.user_snippets()).map(|file| parse(&file)).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snip(trigger: &str, text: &str) -> Snippet {
        Snippet { trigger: trigger.into(), text: text.into() }
    }

    #[test]
    fn parses_the_file() {
        let file = "# Things I say a lot\n\
                    my email => jay@example.com\n\
                    \n\
                    \x20  sign off   =>   Thanks,\\nJay  \n\
                    no arrow here\n\
                    => no trigger\n\
                    no text =>\n\
                    ?! => only punctuation\n\
                    #commented => out\n\
                    arrow => a => b\n";
        assert_eq!(parse(file), [
            snip("my email", "jay@example.com"),
            snip("sign off", "Thanks,\nJay"),
            snip("arrow", "a => b"),
        ]);
        assert!(parse("").is_empty());
    }

    #[test]
    fn whole_dictation_only() {
        let snippets = [snip("my email", "jay@example.com"), snip("Sign-off", "Thanks,\nJay"),
                        snip("type signature", "fn(&str) -> bool")];
        let found = |said: &str| snippet(said, &snippets).map(|s| s.text.as_str());
        assert_eq!(found("my email"), Some("jay@example.com"));
        assert_eq!(found("My email."), Some("jay@example.com"));
        assert_eq!(found("  MY   EMAIL!? "), Some("jay@example.com"));
        assert_eq!(found("Insert my email."), Some("jay@example.com"));
        assert_eq!(found("paste, my email"), Some("jay@example.com"));
        assert_eq!(found("Type my email"), Some("jay@example.com"));
        assert_eq!(found("Sign off."), Some("Thanks,\nJay"));
        assert_eq!(found("sign-off"), Some("Thanks,\nJay"));
        assert_eq!(found("Type signature."), Some("fn(&str) -> bool"));
        for dictation in ["My email is broken.", "Check my email", "email", "insert", "type type my email",
                          "please insert my email", "my e-mail", ""] {
            assert_eq!(found(dictation), None, "{dictation}");
        }
    }
}
