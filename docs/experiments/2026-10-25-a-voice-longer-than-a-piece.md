# A voice longer than a piece

- Date: 2026-10-25
- Commit SHA: `ec75e33f` (`main`), and a change to `crates/speech/src/whisper.rs` made for the runs alone and not kept (below)
- Roadmap item: Phase 6, an agent reads what is said in a meeting.

## Objective

The cutter ends a piece after 600 ms without a voice, and at 15 s at the longest (`crates/speech/README.md`). An agent reads what is said in a meeting from the transcript. When somebody speaks for more than 15 s without a pause of 600 ms, are words lost where the voice is cut, and is the reading of a long piece cut short?

It came from the demo for agents (`mise run agent-demo`): each of its pieces was 15,000 ms long and read as one "Hello from the relay.". Its page sends `fixtures/speech/hello-from-the-relay.wav` as Chromium's fake microphone (`--use-file-for-fake-audio-capture`), which plays the file again and again without a gap. The file is 1.18 s, its voice from 0.068 s to 1.144 s, so the quiet between two plays is about 0.11 s.

## Expected

Not written before the runs. The question was whether words are lost at the cut of 15 s, and whether `set_single_segment(true)` (`Whisper::text`), which has whisper.cpp give one segment for a piece, leaves out what would have been a second segment.

## Environment

macOS 14.4 on an Apple M3 Pro, the release build of the example `read-wav` of `xmoq-speech`. That example cuts and reads as the transcriber does: Opus at 48 kHz, decoded to 16 kHz, judged with Silero (`ggml-silero-v5.1.2.bin`), and read with a language code. `small-q5_1` and `large-v3-turbo-q5_0`, on Metal.

The voices:

- The demo's voice played 25 times, with no gap and with 1.5 s of silence between plays (as the interop's `publishVoice` sends it).
- A paragraph of English, 27.4 s (`say -v Samantha`), and one of Japanese, 41.9 s (`say -v Kyoko`), each said straight through. `say` pauses between sentences for less than 600 ms, so neither had a pause the cutter ends a piece at.
  - English: "This morning the team met to talk about the new relay. Maria said that the servers in Tokyo were ready, but the ones in Frankfurt still needed a newer certificate. Ken asked how many viewers one machine could carry, and nobody knew the answer yet. We agreed to measure it next week with five hundred test clients. After that, Lisa showed the design for the chat window, which now keeps the last two hundred lines. Finally, we decided to move the launch from Thursday to the following Monday."
  - Japanese: 「今朝のミーティングでは、新しい中継サーバーについて話し合いました。東京のサーバーは準備ができていますが、フランクフルトのサーバーはまだ証明書を更新する必要があると、田中さんが報告しました。山本さんは、一台のマシンで何人の視聴者を支えられるかを質問しましたが、まだ誰も答えを知りませんでした。来週、五百台の試験用クライアントで測ることにしました。そのあと、佐藤さんがチャット画面の新しいデザインを見せてくれました。最後に、公開日を木曜日から翌週の月曜日に移すことに決めました。」

To compare with `set_single_segment(false)`, `whisper.rs` was changed for these runs to read it from an environment variable, and the change was then undone.

## Result

**The demo's voice**: with no gap, 29.6 s were two pieces, 0 to 15.00 s and 15.00 to 29.62 s. Each piece had the sentence about 12 times, and each was read as one "Hello from the relay.", by both models and with `set_single_segment` true and false. With 1.5 s of silence, each play was a piece of its own (0 to 1.40 s, 2.56 to 4.10 s, ...), read once each. So the demo's pieces of 15 s come from how its page sends the voice, not from the model of voice activity. The sentence said again and again was read once; that whisper.cpp folds a repeated sentence into one is what the outputs show, and was not looked into further.

**The paragraphs**: the English one was two pieces (0 to 15.00 s, 15.00 to 27.38 s), the Japanese one three (0 to 15.00, 15.00 to 30.00, 30.00 to 41.88 s). No piece ended at a sentence's end. Every word was there, with these differences at the cuts and elsewhere:

| Model | Language | At the cut of 15 s | At the cut of 30 s | Elsewhere |
|---|---|---|---|---|
| `small-q5_1` | en | "next week." ends the first piece and "week with 500 ..." begins the second: "week" twice | | "500" and "200" as figures |
| `large-v3-turbo-q5_0` | en | The same as `small-q5_1` | | The same |
| `small-q5_1` | ja | 「…必要があると田中さんが」 ends the first, 「王国しました…」 begins the second: 「報告」 read as 「王国」 | 「…測ることにしました」 / 「その後、…」: nothing missing | 「けさ」 for 「今朝」, 「くらいアント」 for 「クライアント」, no punctuation in the first two pieces |
| `large-v3-turbo-q5_0` | ja | 「…田中さんが報告しています。」 ends the first, 「報告しました。」 begins the second: 「報告」 twice, the first made up as 「しています」 | Nothing missing | 「1台」, 「500台」 as figures |

- **No word was lost at a cut.** A word that the cut fell in was read on both sides of it (English, and Japanese with turbo), or read wrong on the side where it was whole (Japanese with small).
- **`set_single_segment(true)` did not cut a reading short**: with `false` the text of every piece was the same, character for character, for both models and both languages. A piece of 15 s is read whole as one segment.
- The time to read a piece of 15 s was 0.51 to 0.57 s with `small-q5_1` and 1.18 to 1.32 s with turbo.

## What it means

A reader of the transcript (an agent) gets all of a long speech, in pieces of 15 s. At each cut one word may come twice, or be read wrong. Nothing was changed for it.

## What was not run

- A person speaking, a microphone, a room's noise: the voices were `say`'s, which pause less between sentences than people do.
- More cuts: of the three here, two fell in a word (both at 15 s), and the one at 30 s lost nothing.
- The transcriber over a relay: the example cuts and reads as the transcriber does, without the relay, the tracks or the readers' threads.
- `--language auto`.
