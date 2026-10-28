# A pet in a garden that a decisions API moves

- Date: 2026-10-28
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 7, roles beside the relay that an outside API moves: the garden of a room (`xmoq-garden`), its pet (`xmoq-pet`), their tracks (`xmoq-pet-tracks`, `@xmoq/pet`) and the page `/pet` of the demo.

## Objective

A person's page acts on the garden of its room, never on the pet: it puts food or the ball where the floor is clicked, and pets, calls or speaks as things that happen there. The garden publishes what lies on its floor and what happened; the pet perceives that over the relay as any page does, decides what to do, and acts on the garden in its turn (eats the food, takes the ball), which the garden takes away. Does this go round, through the relay, by the pet's own table and by the decisions API of `pomesaka/claude-decisions`?

A first version had the pages send commands to the pet; the person chose this one on the same day, the pet perceiving the garden instead (that version is kept as a change of its own, not merged).

**Loopback, one machine.**

## Expected

Said before the runs (in the session, not in a file): a bowl put down is in the garden within a second; the pet goes for it, sends `eat`, and the garden has it no longer; the same with the ball and `take`; petting is an event, which the pet approaches for.

## Environment

macOS 14.4, Chromium of Playwright 1.64.0 (`playwright-core` 1.64.0), headless, one page of `/pet` as `ann` in a room of its own. Demos of their own: `bun demo/serve.ts --relay-port 35443 --port 35080 --rooms-port 35787` (the pet by its table), and, started by the person, the same on 36443, 36080 and 36787 with `--decisions-url http://127.0.0.1:3333/v1/decisions --pet-log FILE`, the token given in `DECISIONS_API_TOKEN`. The decisions API ran at `http://127.0.0.1:3333` (claude-decisions, `claude-haiku-5-5`), after its owner tuned it for the pet's questions on the same day.

## Commands

The page waits 5 s, chooses **food** and clicks the floor at 30 % across and 75 % down, then reads what it has of the floor and of the pet 1.5 s, 5.5 s and 12.5 s after; chooses **ball** and clicks at 75 % across and 50 % down, and reads 2.5 s and 6.5 s after; presses **pet** and reads 2.5 s after.

## Observed results

**By the pet's table** (no API):

| After | The pet | The floor |
|---|---|---|
| food, 1.5 s | `eat` towards (244, 278), `やった！`, mood 3 | the bowl at (244, 278) |
| food, 5.5 s | `eat` at the bowl | empty: the pet sent `eat`, and the garden took the bowl away |
| ball, 2.5 s | at (908, 741), `ボール！`, mood 4 | empty: the pet ran there and took the ball |
| pet, 2.5 s | `approach` to (500, 150), `♥` | empty |

**With the API**: the pet printed `DECISIONS_WARMED took_ms=2831` at its start. For each happening it printed `PERCEIVED` when the garden showed it, `THINKING`, and the decision about 3 to 4 s after, while the page had it sitting with `?`:

| Happening | Decided (`source=api`) | Then |
|---|---|---|
| a bowl put down | `eat`, mood 3, `やった！` | `ACTED_SENT kind=eat thing=1`, and the next garden had the bowl gone |
| the ball thrown | `chase`, mood 4, `ボール！` | `ACTED_SENT kind=take thing=2`, and the ball gone |
| ann petted the pet | `approach`, `♥` | |

**Three bowls at once** (by the table, after the pet was made to look at the floor again when it goes back to itself): the page put three bowls with three clicks, and read the floor every 2 s. The pet ate the nearest first, then printed `LOOKING floor=2`, decided `eat` and ate the next, then `LOOKING floor=1` and the last: 2 bowls left at 2 s, 1 at 10 s, none at 18 s, and it walked about after. Before that change it ate one and wandered with two left.

The same with the API: each look was asked of it (`LOOKING floor=2`, then `DECIDED source=api why=answered reaction=eat`), the pet sitting with `?` for each answer; the three bowls (ids 1, 2 and 3) were eaten one after another and the floor was empty 26 s after they were put down, against 18 s by the table.

## Limitations and blockers

- The ball the pet takes is gone from the floor; there is no action yet to drop it again.
- When the pet subscribed to a room's `garden` before the garden had published it, the SUBSCRIBE was refused (0x10) and asked again a second later; the RENDEZVOUS_TIMEOUT parameter was not used.
- One machine, one pet; nothing was tried across a relay that moves the sessions (GOAWAY), nor a garden or a pet that restarts (the ids of a garden begin at 1 again).

## Open questions raised

None.
