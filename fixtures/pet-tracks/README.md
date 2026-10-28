# The garden's tracks: their names and payloads

A room has a garden: what lies on its floor, and what happened there of late. Whoever is in the room, a person's page or the pet, is an actor that acts on the garden, and none acts on another: the garden (`xmoq-garden`) takes what each actor does, changes the garden, and publishes it; the pet (`xmoq-pet`) perceives the garden as any page does, decides what it does, and acts on the garden in its turn. `xmoq-pet-tracks` and `@xmoq/pet` read the tables in their tests, so that the same bytes give the same value, or the same reason why there is none. Written by hand from the decision; no other implementation writes these payloads.

A room is `xmoq/pet/<room>`, where the garden publishes `garden`. An actor announces `xmoq/pet/<room>/<name>` and publishes `acted` there. **The pet is the actor named `pet`**, which also publishes `state`, its body: where it is and what it does. A page that names itself `pet` is not taken for an actor. Each action, each garden and each state is Object 0 of a Group of its own.

**A place on the floor is two whole numbers from 0 to 1000**, across and into the room, and not fractions: `JSON.parse` and `serde_json` write a fraction or a whole number of a float differently (`1` and `1.0`). A time is milliseconds since the Unix epoch, a whole number from 0 to 2^53 - 1, by the clock of who wrote it. An id is a whole number from 0 to 2^53 - 1.

Columns are separated by a tab. A text column holds a JSON string literal, so that quotes, backslashes and line breaks are written as JSON writes them; `-` is "none".

## `names.tsv`

| Column | What it is |
|---|---|
| 1 | The case |
| 2 | The fields of the namespace, a JSON array of strings |
| 3 | The track's name, where the row is of a track; `-` where it is of a namespace |
| 4 | What it is read as: `actor <room> <name>` for the namespace of an actor (`pet` among them); `garden <room>` for the track of a room, `acted <room> <name>` for that of an actor, `state <room>` for the pet's; `none` for nobody's |

## `acted.tsv`

What an actor does. A person's page: `feed` (puts a bowl of food at `to`), `throw` (the ball to `to`), `call`, `pet` (strokes the pet), `talk` (`text`, from 1 to 120 bytes of UTF-8). The pet: `eat` (the bowl `thing`, which is then empty and taken away), `take` (the ball `thing`, which is then in its mouth and no longer on the floor). A kind has the members named here and no other of `to`, `text` and `thing`. Who may do which is the garden's to say, not the reader's: the reader reads any kind from any actor.

| Column | What it is |
|---|---|
| 1 | The case |
| 2 | The payload, as UTF-8 |
| 3 | The `kind` read |
| 4 | The `to` read, `[x,y]` |
| 5 | The `text` read, as a JSON string literal |
| 6 | The `thing` read |
| 7 | The `sentAt` read |
| 8 | Why it is no action: `not-an-object`, `kind`, `to`, `text`, `thing`, `sent-at` |
| 9 | `yes` where writing what was read gives the payload back, byte for byte |

## `garden.tsv`

The garden as it is now: `things` on its floor, each `{"id", "kind", "at", "by"}` (`kind` `bowl` or `ball`, `by` the name of the actor that put it there), at most 8; and `events`, what happened of late that leaves nothing on the floor, each `{"id", "kind", "by", "text", "at"}` (`kind` `call`, `pet` or `talk`; `text` the words of a `talk` and `null` otherwise; `at` when the garden took it), the newest last, at most 16. Ids go up in a room and are never used twice, things and events counted apart; a reader that read an event before knows it again by its id. `at` of the garden is when it was written.

| Column | What it is |
|---|---|
| 1 | The case |
| 2 | The payload, as UTF-8 |
| 3 | The `things` read, as compact JSON |
| 4 | The `events` read, as compact JSON |
| 5 | The `at` read |
| 6 | Why it is no garden: `not-an-object`, `things`, `events`, `at` |
| 7 | `yes` where writing what was read gives the payload back, byte for byte |

## `state.tsv`

The pet's body. It goes from `from` at `began` to `to` at `arrives` (the same place and time where it stays), facing `left` or `right`, doing one of `idle`, `walk`, `sit`, `sleep`, `jump`, `eat`, `chase`, `flee`, `approach`; `mood` from 0 (cross) to 4 (glad); `bubble` what it says, from 1 to 64 bytes of UTF-8, or `null`; `by` the name of whom it reacts to, or `null`; `at` when the pet wrote it. A reader places it at a moment between `began` and `arrives` by its own clock against `at`.

| Column | What it is |
|---|---|
| 1 | The case |
| 2 | The payload, as UTF-8 |
| 3 | The `from` read, `[x,y]` |
| 4 | The `to` read |
| 5 | The `began` read |
| 6 | The `arrives` read |
| 7 | The `facing` read |
| 8 | The `doing` read |
| 9 | The `mood` read |
| 10 | The `bubble` read, as a JSON string literal |
| 11 | The `by` read, as a JSON string literal |
| 12 | The `at` read |
| 13 | Why it is no state: `not-an-object`, `from`, `to`, `began`, `arrives`, `facing`, `doing`, `mood`, `bubble`, `by`, `at` |
| 14 | `yes` where writing what was read gives the payload back, byte for byte |
