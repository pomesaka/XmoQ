//! What a line of stdin or of `inbox.txt` asks for: a line of the chat, or
//! a command.
//!
//! A line that begins with `/` is a command; one that begins with `//` is
//! said with one `/` taken off, so that the chat can have a line that
//! begins with `/`. WHY a mark at the beginning, and not a line of JSON:
//! an agent writes `echo hello` as easily as a person types it, and the
//! chat is what it does most.

/// The most bytes of a line. A longer one is not read, and is told as too
/// long. WHY a limit of its own, above the chat's 512 bytes: a line is kept
/// whole until its end comes, and the chat's limit is for what is said,
/// which is checked after.
pub const MOST_A_LINE: usize = 4096;

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

    /// What a line asks for, as the third column of the shared table writes it:
    /// its type, a space, and the rest as it is.
    fn written(input: &Input) -> String {
        match input {
            Input::Say(text) => format!("say {text}"),
            Input::Frame(id) => format!("frame {}", id.as_deref().unwrap_or("-")),
            Input::Unknown(name) => format!("unknown {name}"),
            Input::Nothing => "nothing".to_string(),
        }
    }

    /// `fixtures/agent-lines/inbox.tsv`, which `@xmoq/meeting` reads too: a
    /// line of `inbox.txt` of a page is read as a line of stdin here.
    #[test]
    fn a_line_of_stdin_is_said_or_is_a_command() {
        let table = include_str!("../../../fixtures/agent-lines/inbox.tsv");
        let mut rows = 0;
        for row in table.lines() {
            let [name, line, asks] = row.split('\t').collect::<Vec<_>>()[..] else {
                panic!("{row}");
            };
            let line: String = serde_json::from_str(line).unwrap();
            assert_eq!(written(&read(&line)), asks, "{name}");
            rows += 1;
        }
        assert!(rows > 0);
    }
}
