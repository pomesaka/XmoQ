# Chat as MoQ tracks, beside the room's WebSocket

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, "Decide chat transport, persistence and ordering". Material for the decision; nothing is decided here.

## Objective

Chat travels over the room's WebSocket (a Durable Object, in Miniflare). Can it travel as MoQ tracks through the own relay with what is there, and how do the two compare: does every line reach everybody, in the same order for everybody, does somebody who comes later see what was said before, and how long does a line take?

Expected, written before the runs: both deliver every line; the room gives one order to everybody (it numbers the lines), and MoQ tracks of several senders do not; a late joiner gets the room's history, and over MoQ what the relay's cache has; both take a few milliseconds on loopback.

## The shape that was tried

Every participant publishes one track, `chat`, in a namespace of its own, `xmoq/chat/<room>/<name>`. A line is an Object (JSON); all lines of a participant's session are one Group, on one stream that stays open, the Group ID the time the session began. Everybody subscribes to everybody's track, their own among them, with a Joining FETCH from the start of the Group (transport draft 18 §10.12.2), which the relay answers from its cache of the latest Group.

WHY this one: a participant is a publisher of what it says, as with video; a Group is what a subscriber can join at (§2.3), so "what was said before I came" is the Group so far, and the relay's cache of the latest Group serves it with no new part. MSF has nothing for chat (its catalog could name the track; none was written).

Not tried, and why:

- **A line a Group.** The relay keeps the latest Group of a track, so a late joiner would get each participant's last line and no more.
- **One track for the room that everybody publishes to.** Group IDs of several publishers would have to be agreed on, and the relay's handling of several publishers of one track is partial (implementation notes).
- **The room merges all lines into one track.** A Durable Object cannot open a WebTransport session to the relay; something else would have to be the publisher.

**What the relay lacks for it: SUBSCRIBE_NAMESPACE** (it answers NOT_SUPPORTED). With it a participant would subscribe to `xmoq/chat/<room>` and be told of every participant's namespace. Here the names of the participants were given to each page in its address. The room could tell them instead, as it tells where the broadcast is.

## Environment

macOS 14.4, arm64, loopback. Chromium 156.0.8078.4 (playwright-core 1.64.0), headless, three pages (`demo/chat.ts`, at `/chat?room=R&name=a&peers=a,b,c`). The own relay and the rooms as `bun demo/serve.ts` starts them. Every line is sent on both transports at the same moment, with its time in it; a page notes a line when it comes back to it, its own lines too.

## Commands

```sh
bun demo/serve.ts --port 45381 --relay-port 45382 --rooms-port 45384
# a script of this run: a and b say 20 lines each, 30 ms apart, at once; c comes; a, b and c say 20 each
```

## Result

Four runs, all the same but for the times.

| | Room's WebSocket | MoQ tracks |
|---|---|---|
| Lines that reached each page | all (40 of 40 on a and b, then 60 of 60 on a, b and c) | all, the same |
| The same order on every page | yes | **no**: a and b differed, and a and c, in every run |
| Each sender's lines in the order they were sent | yes | yes |
| c, who came after 40 lines | all 40, from the room's history, in the order a had them | all 40, by the Joining FETCH of each track; a's lines and b's each in order, not in the order a had the two together |
| From sent to shown, ms: median / 95th / most | 0.9 to 2.2 / 1.8 to 5.6 / 2.9 to 9.1 | 0.6 to 1.3 / 1.0 to 2.8 / 1.6 to 3.8 |

- The expectations held.
- **Order.** The room numbers the lines as it takes them, and every page shows that order. Over MoQ each page has the lines of each sender in order and puts the senders together as they arrive, which differs from page to page. One order for all would take something that gives it: the room, a clock all trust (the times in the lines are the senders' own), or one publisher.
- **History.** Over MoQ it is what the relay's cache holds of each participant's Group: gone when that participant leaves (the cache is kept while the publisher is there), at most 4,096 Objects and 8 MiB a track, and one participant's lines only. The room keeps the last 50 lines whoever is there.
- The times are of one machine with no link between; they say that neither adds a wait of its own, and nothing about a network.

## Not done

- A participant who leaves and comes back, and a relay that goes: the lines said meanwhile are not sent later.
- More than three participants; every page holds a subscription for each (n subscriptions a page, n squared in the room).
- A stream that stays open for a whole session, against the relay's limits on streams over hours.
- A viewer in a terminal; authorization (who may publish under `xmoq/chat/<room>/<name>`).
- Loss and delay. A line lost on its stream holds back that sender's later lines, and nobody else's.
