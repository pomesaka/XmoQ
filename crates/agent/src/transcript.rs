//! Which transcript the reader follows: `speech` of
//! `xmoq/transcript/<prefix>`, where the prefix is the one a transcriber
//! listens under.

/// The prefix whose transcript is followed: `--transcript` where it is
/// given, else the room's (the `transcript` its room of a broadcast or of a
/// meeting answers with), else the meeting's (`xmoq/meet/<room>`), else the
/// broadcast's namespace; `None` where none of them is there.
///
/// WHY the flag first: it overrides, for a transcriber run another way than
/// the room says. WHY the room before the meeting's own: the room is where
/// the prefix is set (`MEETING_TRANSCRIPT_PREFIX`), which can be above the
/// meeting's (`xmoq/meet` for one transcriber of every room). WHY the
/// broadcast last: a transcriber that listens under the broadcast's
/// namespace alone is the narrowest.
#[must_use]
pub fn prefix(
    flag: Option<&[String]>,
    hello: Option<&[String]>,
    meeting: Option<&str>,
    broadcast: Option<&[String]>,
) -> Option<Vec<String>> {
    flag.or(hello)
        .map(<[String]>::to_vec)
        .or_else(|| {
            // The meeting's prefix is `xmoq_viewer::meeting`'s, as the
            // reader's namespace of the meeting is.
            meeting.map(xmoq_viewer::meeting::prefix)
        })
        .or_else(|| broadcast.map(<[String]>::to_vec))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(text: &str) -> Vec<String> {
        text.split('/').map(str::to_string).collect()
    }

    #[test]
    fn the_prefix_of_the_transcript_is_the_first_there() {
        let (flag, hello, broadcast) =
            (fields("x/flag"), fields("xmoq/meet/r"), fields("xmoq/demo"));
        // (the flag, the hello, the meeting, the broadcast; the prefix)
        let cases = [
            (
                Some(&flag),
                Some(&hello),
                Some("m"),
                Some(&broadcast),
                Some("x/flag"),
            ),
            (
                None,
                Some(&hello),
                Some("m"),
                Some(&broadcast),
                Some("xmoq/meet/r"),
            ),
            (None, None, Some("m"), Some(&broadcast), Some("xmoq/meet/m")),
            (None, None, None, Some(&broadcast), Some("xmoq/demo")),
            (Some(&flag), None, None, None, Some("x/flag")),
            // None of them.
            (None, None, None, None, None),
        ];
        for (flag, hello, meeting, broadcast, expected) in cases {
            assert_eq!(
                prefix(
                    flag.map(Vec::as_slice),
                    hello.map(Vec::as_slice),
                    meeting,
                    broadcast.map(Vec::as_slice)
                ),
                expected.map(fields),
                "{flag:?} {hello:?} {meeting:?} {broadcast:?}"
            );
        }
    }
}
