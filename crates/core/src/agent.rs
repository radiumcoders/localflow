//! Talking to a coding agent (Claude Code, Codex...) in a terminal: a
//! dictation that is only a command ("stop", "clear the chat") presses keys
//! instead of being pasted, and one that ends with "send it" sends the prompt.

use std::sync::LazyLock;

use regex::Regex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Esc: interrupt the agent.
    Stop,
    /// Esc twice: Claude Code's rewind (Codex's backtrack) to an earlier prompt.
    Rewind,
    /// A slash command, typed and sent.
    Slash(&'static str),
    /// Enter: send what's in the prompt already.
    Send,
}

impl Command {
    /// For the pill.
    pub fn describe(&self) -> &'static str {
        match self {
            Command::Stop => "Esc",
            Command::Rewind => "Esc Esc (rewind)",
            Command::Slash(slash) => slash,
            Command::Send => "Sent",
        }
    }
}

/// The dictation's words, lowercase, without punctuation or "okay"/"please".
fn bare(said: &str) -> String {
    let words: String =
        said.to_lowercase().chars().map(|c| if c.is_alphanumeric() || c == '\'' { c } else { ' ' }).collect();
    let words = words.split_whitespace().collect::<Vec<_>>().join(" ");
    let words = words.trim_start_matches("okay ").trim_start_matches("ok ").trim_start_matches("please ");
    words.trim_end_matches(" please").to_string()
}

/// The command a whole dictation is, for this agent ("claude", "codex"...),
/// or None when it's a prompt ("stop using the old API" is a prompt).
pub fn command(said: &str, agent: &str) -> Option<Command> {
    Some(match bare(said).as_str() {
        "stop" | "stop it" | "stop that" | "stop stop" | "cancel" | "cancel that" | "hold on" | "wait" | "wait wait"
        | "interrupt" | "escape" => Command::Stop,
        "undo" | "undo that" | "go back" | "rewind" | "take that back" => Command::Rewind,
        "clear" | "clear it" | "clear the chat" | "clear the context" | "clear the conversation" | "new chat"
        | "new conversation" | "start over" => Command::Slash(match agent {
            "codex" | "opencode" | "crush" => "/new",
            _ => "/clear",
        }),
        "compact" | "compact it" | "compact the chat" | "compact the context" | "compact the conversation" => {
            Command::Slash(match agent {
                "gemini" | "qwen" => "/compress",
                "aider" => return None,
                _ => "/compact",
            })
        }
        "send" | "send it" | "submit" | "submit it" | "enter" | "press enter" => Command::Send,
        _ => return None,
    })
}

/// When to press Enter after pasting into a coding agent (LOCALFLOW_SUBMIT).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Submit {
    /// When the dictation ends with "send it" (the default).
    Said,
    /// After every dictation.
    Always,
    Never,
}

impl Submit {
    pub fn parse(value: Option<&str>) -> Submit {
        match value.map(|v| v.trim().to_lowercase()).as_deref() {
            Some("always" | "agents" | "1" | "on") => Submit::Always,
            Some("never" | "off" | "0" | "no") => Submit::Never,
            _ => Submit::Said,
        }
    }

    /// The env file's value (None for the default).
    pub fn value(self) -> Option<&'static str> {
        match self {
            Submit::Said => None,
            Submit::Always => Some("always"),
            Submit::Never => Some("never"),
        }
    }
}

/// "…, and send it." at the end: the words that ask to send, not the prompt.
/// Kept narrow ("the form users submit" is a prompt); a closing </quiet>
/// (a muttered last segment) stays.
static SEND: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)[\s,.;:!?-]*\b(?:(?:and|then|now|okay|ok|go ahead and)\s+)?(?:send|submit)\s+(?:it|that|this)(?:\s+now)?[\s.!]*(</quiet>)?\s*$|[\s,.;:!?-]*\b(?:and|then)\s+(?:send|submit)[\s.!]*(</quiet>)?\s*$",
    )
    .unwrap()
});

/// The dictation without a closing "send it", and whether it had one.
pub fn split_send(said: &str) -> (String, bool) {
    match SEND.captures(said) {
        Some(c) => {
            let quiet = c.get(1).or(c.get(2)).map_or("", |m| m.as_str());
            (format!("{}{quiet}", &said[..c.get(0).unwrap().start()]), true)
        }
        None => (said.to_string(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_dictation_commands() {
        assert_eq!(command("Stop.", "claude"), Some(Command::Stop));
        assert_eq!(command("Okay, clear the chat.", "claude"), Some(Command::Slash("/clear")));
        assert_eq!(command("Clear the chat", "codex"), Some(Command::Slash("/new")));
        assert_eq!(command("Compact", "claude"), Some(Command::Slash("/compact")));
        assert_eq!(command("compact the context", "gemini"), Some(Command::Slash("/compress")));
        assert_eq!(command("compact", "aider"), None);
        assert_eq!(command("undo that", "claude"), Some(Command::Rewind));
        assert_eq!(command("Send it.", "claude"), Some(Command::Send));
        assert_eq!(command("Stop using the old API in the auth module.", "claude"), None);
    }

    #[test]
    fn send_at_the_end() {
        assert_eq!(split_send("Fix the login bug, and send it."), ("Fix the login bug".into(), true));
        assert_eq!(split_send("fix the login bug send it"), ("fix the login bug".into(), true));
        assert_eq!(split_send("Run the tests and send."), ("Run the tests".into(), true));
        assert_eq!(split_send("Rename it. Submit that now!"), ("Rename it".into(), true));
        assert_eq!(split_send("check it <quiet>okay send it</quiet>"), ("check it <quiet></quiet>".into(), true));
        for prompt in ["Fix the form users submit.", "Write the email I want to send.", "Send it to the API and log it.",
                       "Don't resend it"] {
            assert_eq!(split_send(prompt), (prompt.into(), false), "{prompt}");
        }
    }

    #[test]
    fn submit_setting() {
        assert_eq!(Submit::parse(None), Submit::Said);
        assert_eq!(Submit::parse(Some("always")), Submit::Always);
        assert_eq!(Submit::parse(Some("off")), Submit::Never);
        for s in [Submit::Said, Submit::Always, Submit::Never] {
            assert_eq!(Submit::parse(s.value()), s);
        }
    }
}
