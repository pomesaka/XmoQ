# Reactions from page to page, and how many are in the room

- Date: 2026-10-27
- Commit SHA: the change that adds this record, on top of the gatherer's `reactions` and `present` (`lnzqltwm`, #506) and `9179dd4b`
- Roadmap item: Phase 4, "Implement chat; evaluate reactions"

## Objective

A reaction pressed on one page of the demo reaches every page of the room over the gatherer's track `reactions`, and the line under the picture says how many are in the room's chat, from the gatherer's `present` (decided on 2026-10-26). Before this, the interop runs of the chat had the gatherer subscribe to the pages' `reacted` and publish `present`, and no run had sent a reaction from one page to another.

## Expected

The three pages of one room say 3 in the chat; the reactions one page presses stand on all three; a page that presses more than 5 in a second keeps the rest back and says why; a page that leaves takes the count down by one.

## Environment

macOS, arm64; one machine. `bun demo/serve.ts --port 38080 --relay-port 38443 --rooms-port 38787` (the own relay, the rooms, the gatherer `xmoq-chat` with `--reactions-per-second 5 --reaction-batch-ms 100 --reaction-kinds 16`, with tokens); headless Chromium of playwright-core 1.64.0, each page in a context of its own. Two scripts of this run, not in the repository: one opened `/publish`, `/watch` and `/watch` of one room and had a watching page press 👍, 👍 and ❤️; the other opened two watching pages of another room, had one press 👏 seven times as fast as the clicks go, and closed it.

## Result

| What | Seen |
|---|---|
| The count, 6 s after three pages opened | `room “…”: 3 in the chat` on all three (`data-present` 3) |
| 👍, 👍, ❤️ pressed on one watching page | on the publishing page, the presser's and the other watching page alike, 600 ms after: `👍👍❤️`, 3 counted |
| 👏 pressed 7 times at once | the presser sent 5, and its button says `not sent: more than 5 reactions in a second`; the other page counted 5 |
| One of two pages closed | the other says 1 in the chat 4 s after (it said 2) |

Each was seen once. The time from a press to the other pages was not measured.

## Not looked at

- A terminal viewer and an agent are counted when they take part in the chat, and do not send reactions; not run here.
- More than three pages, and reactions from several pages in one while (the counts of one Object of `reactions` with more than one emoji from more than one sender).
- `--reaction-kinds`: no while had more than two kinds.
