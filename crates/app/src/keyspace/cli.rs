//! The CLI panel apart from drawing it: how a reply line is coloured and
//! how ↑ and ↓ walk the commands run, as the former web UI's `CliPanel.tsx` did.

/// What a reply line holds, which picks its colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyTone {
    Plain,
    Error,
    Number,
    String,
}

/// The tone of a reply line as redis-cli prints it, an item's `1) ` aside.
pub fn reply_tone(text: &str) -> ReplyTone {
    let body = strip_item_number(text);
    if body.starts_with("(error)") {
        ReplyTone::Error
    } else if body.starts_with("(integer)") || body.starts_with("(double)") {
        ReplyTone::Number
    } else if body.starts_with('"') {
        ReplyTone::String
    } else {
        ReplyTone::Plain
    }
}

/// `text` without a leading `  12) `.
fn strip_item_number(text: &str) -> &str {
    let rest = text.trim_start();
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits > 0 && rest[digits..].starts_with(") ") { &rest[digits + 2..] } else { text }
}

/// The commands run, oldest first, each once, and the one ↑/↓ recalled.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CliHistory {
    commands: Vec<String>,
    recall: Option<usize>,
}

impl CliHistory {
    /// `command` ran: it moves to the end, and recalling starts over.
    pub fn push(&mut self, command: &str) {
        self.commands.retain(|known| known != command);
        self.commands.push(command.to_owned());
        self.recall = None;
    }

    /// ↑ (`older`) or ↓: the text the prompt shows now, or `None` with no
    /// command run yet. Past the newest, the prompt is empty.
    pub fn browse(&mut self, older: bool) -> Option<String> {
        if self.commands.is_empty() {
            return None;
        }
        let index = match self.recall {
            None if older => Some(self.commands.len() - 1),
            None => None,
            Some(recall) if older => Some(recall.saturating_sub(1)),
            Some(recall) => Some(recall + 1),
        };
        match index.filter(|index| *index < self.commands.len()) {
            Some(index) => {
                self.recall = Some(index);
                Some(self.commands[index].clone())
            }
            None => {
                self.recall = None;
                Some(String::new())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_replies_by_what_they_hold() {
        assert_eq!(reply_tone("(error) ERR unknown command"), ReplyTone::Error);
        assert_eq!(reply_tone("(integer) 3"), ReplyTone::Number);
        assert_eq!(reply_tone("(double) 1.5"), ReplyTone::Number);
        assert_eq!(reply_tone("\"hello\""), ReplyTone::String);
        assert_eq!(reply_tone(" 2) \"b\""), ReplyTone::String, "a list item's number aside");
        assert_eq!(reply_tone("10) (integer) 1"), ReplyTone::Number);
        assert_eq!(reply_tone("OK"), ReplyTone::Plain);
        assert_eq!(reply_tone("1)x"), ReplyTone::Plain);
    }

    #[test]
    fn walks_the_commands_run_with_the_arrows() {
        let mut history = CliHistory::default();
        assert_eq!(history.browse(true), None, "nothing run yet");
        for command in ["GET a", "GET b", "GET a"] {
            history.push(command);
        }
        assert_eq!(history.browse(true).as_deref(), Some("GET a"), "each command once, the newest last");
        assert_eq!(history.browse(true).as_deref(), Some("GET b"));
        assert_eq!(history.browse(true).as_deref(), Some("GET b"), "the oldest stays");
        assert_eq!(history.browse(false).as_deref(), Some("GET a"));
        assert_eq!(history.browse(false).as_deref(), Some(""), "past the newest, an empty prompt");
        assert_eq!(history.browse(false).as_deref(), Some(""));
        assert_eq!(history.browse(true).as_deref(), Some("GET a"));
        history.push("PING");
        assert_eq!(history.browse(true).as_deref(), Some("PING"), "running starts over");
    }
}
