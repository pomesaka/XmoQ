//! Which transcript the reader follows: `speech` of
//! `xmoq/transcript/<prefix>`, where the prefix is the one a transcriber
//! listens under.

/// The fields a meeting's namespace begins with: its participants publish
/// under `xmoq/meet/<room>/<id>`.
const MEETING: [&str; 2] = ["xmoq", "meet"];

/// The prefix whose transcript is followed: `--transcript` where it is
/// given, else the room's (its hello), else the meeting's
/// (`xmoq/meet/<room>`), else the broadcast's namespace; `None` where none
/// of them is there.
///
/// WHY the flag first: whoever starts the reader knows best where the
/// transcriber listens. WHY the room before the meeting's own: the room is
/// where the prefix is set (`TRANSCRIPT_PREFIX`), and a meeting's default
/// is the same where it was set so. WHY the broadcast last: a transcriber
/// that listens under the broadcast's namespace alone is the narrowest.
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
            meeting.map(|room| {
                MEETING
                    .iter()
                    .map(|field| (*field).to_string())
                    .chain([room.to_string()])
                    .collect()
            })
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
            // A session at the room's relay for the chat alone, in a room
            // that names no transcript.
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
