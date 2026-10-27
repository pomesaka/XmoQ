//! Words that are no words: what a reader of speech writes for a piece in
//! which nobody spoke, which the transcriber does not publish.
//!
//! WHY: a piece is cut where the judge of voice activity heard a voice,
//! and a breath, a hum or a noise can be taken for one. whisper.cpp then
//! writes a mark in brackets for the whole piece: `[BLANK_AUDIO]` (seen in a
//! meeting of the demo, 2026-10-27), and for noise, a hum and clicks of a
//! file read by `read-wav` with `ggml-tiny.bin` (2026-10-27) `[BIRDS
//! CHIRPING]`, `[MUSIC PLAYING]` and `(音楽)`. A reader of the transcript, an
//! agent above all, would take each for something said. WHY NOT words that
//! are often made up from noise (`ご視聴ありがとうございました`, `you`, as
//! `ggml-large-v3-turbo-q5_0.bin` wrote for the same files): they are also
//! what a person says, and the transcriber cannot tell the two apart. WHY
//! NOT a list of the marks: they are words of the model, not of
//! whisper.cpp (its source has none of them), and differ by model and
//! language; what they share is the brackets.

/// The pairs of brackets a mark is written in.
const BRACKETS: [(char, char); 4] = [('[', ']'), ('(', ')'), ('（', '）'), ('【', '】')];

/// Whether `text` has nothing but marks: what is left after every part in
/// [`BRACKETS`] is taken out has no letter and no digit of any script.
/// A part that is not closed is left in.
#[must_use]
pub fn only_marks(text: &str) -> bool {
    let mut rest = text;
    loop {
        let opened = rest.char_indices().find_map(|(at, c)| {
            BRACKETS
                .iter()
                .find(|(open, _)| *open == c)
                .map(|(_, close)| (at, c.len_utf8(), *close))
        });
        let Some((at, open_len, close)) = opened else {
            return !rest.chars().any(char::is_alphanumeric);
        };
        let (before, inside) = rest.split_at(at);
        let inside = inside.get(open_len..).unwrap_or_default();
        let Some(closed) = inside.find(close) else {
            return !rest.chars().any(char::is_alphanumeric);
        };
        if before.chars().any(char::is_alphanumeric) {
            return false;
        }
        rest = inside.get(closed + close.len_utf8()..).unwrap_or_default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_piece_of_marks_alone_is_told_from_one_with_words() {
        // (what the reader wrote; whether it is marks alone)
        let cases = [
            ("[BLANK_AUDIO]", true),
            ("[BIRDS CHIRPING]", true),
            ("[MUSIC PLAYING]", true),
            ("(音楽)", true),
            ("（笑）", true),
            ("【拍手】", true),
            ("[BLANK_AUDIO] [BLANK_AUDIO]", true),
            ("[Music] ♪", true),
            ("...", true),
            ("", true),
            // Words beside a mark are words.
            ("(笑) そうですね", false),
            ("そうですね (笑)", false),
            ("Hello from the relay.", false),
            ("ご視聴ありがとうございました", false),
            ("you", false),
            ("2", false),
            // A bracket that is not closed is no mark.
            ("(笑", false),
            ("[BLANK_AUDIO", false),
            // A pair of brackets of two kinds does not close.
            ("(笑]", false),
        ];
        for (text, expected) in cases {
            assert_eq!(only_marks(text), expected, "{text:?}");
        }
    }
}
