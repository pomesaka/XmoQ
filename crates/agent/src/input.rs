//! What a line of stdin asks for: a line of the chat, or a command.
//!
//! A line that begins with `/` is a command; one that begins with `//` is
//! said with one `/` taken off, so that the chat can have a line that
//! begins with `/`. WHY a mark at the beginning, and not a line of JSON:
//! an agent writes `echo hello` as easily as a person types it, and the
//! chat is what it does most.

/// What a line asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// A line said in the chat.
    Say(String),
    /// `/frame`: a picture of the broadcast now; in a meeting, `/frame
    /// <id>` names whose, and `/frame` alone is of the one who spoke last.
    /// Outside a meeting the word after it is not looked at.
    Frame(Option<String>),
    /// A command that is none of these, with its name.
    Unknown(String),
    /// An empty line, which asks for nothing.
    Nothing,
}

/// Reads one line of stdin, without its line break. A carriage return
/// before the line break (a line from Windows) is taken off as well.
#[must_use]
pub fn read(line: &str) -> Input {
    let line = line.strip_suffix('\r').unwrap_or(line);
    if line.is_empty() {
        return Input::Nothing;
    }
    if let Some(said) = line.strip_prefix("//") {
        return Input::Say(format!("/{said}"));
    }
    let Some(command) = line.strip_prefix('/') else {
        return Input::Say(line.to_string());
    };
    // WHY the words after the first that a command takes are let be: a
    // command with more words after it is still the command, as a person
    // would mean it.
    let mut words = command.split_whitespace();
    let name = words.next().unwrap_or_default();
    match name {
        "frame" => Input::Frame(words.next().map(ToString::to_string)),
        _ => Input::Unknown(name.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_of_stdin_is_said_or_is_a_command() {
        // (the line; what it asks for)
        let cases = [
            ("hello", Input::Say("hello".to_string())),
            ("hello\r", Input::Say("hello".to_string())),
            (" /frame", Input::Say(" /frame".to_string())),
            ("/frame", Input::Frame(None)),
            ("/frame p1", Input::Frame(Some("p1".to_string()))),
            ("/frame  p1  p2", Input::Frame(Some("p1".to_string()))),
            ("/frame\tp1", Input::Frame(Some("p1".to_string()))),
            ("//frame", Input::Say("/frame".to_string())),
            ("//", Input::Say("/".to_string())),
            ("///x", Input::Say("//x".to_string())),
            ("/quit", Input::Unknown("quit".to_string())),
            ("/", Input::Unknown(String::new())),
            ("", Input::Nothing),
            ("\r", Input::Nothing),
        ];
        for (line, input) in cases {
            assert_eq!(read(line), input, "{line:?}");
        }
    }
}
