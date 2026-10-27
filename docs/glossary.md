# Glossary

The words of broadcasts and meetings that the code and the documents use, in the sense they have here. The code names its types after them.

The words of MoQ Transport (Publisher, Subscriber, Relay, Upstream, Track, Group, Object and the rest) mean what draft 18 says, in [§1.2 Terms and Definitions](https://www.ietf.org/archive/id/draft-ietf-moq-transport-18.html#section-1.2) and [§2 Object Data Model](https://www.ietf.org/archive/id/draft-ietf-moq-transport-18.html#section-2), and are not written again here. The names of the parts of the code, such as a published track or the gatherer, are in [the architecture](architecture.md).

| Word | Meaning |
|---|---|
| Room | Where people meet by name: a broadcast and its chat are of the room of a broadcast, a meeting and its chat of the room of a meeting, each under namespaces with the room's name in them (named in `xmoq-chat-tracks`, `xmoq-meeting-tracks` and `packages/room/src/core/rules.ts`). The room's server is asked over HTTP where the relay is, for tokens and for the seats of a meeting; what happens in the room goes over MoQ |
| Broadcast | Video and sound from one publisher to many viewers: the tracks under one namespace, described by one catalog. A room of a broadcast has one |
| Meeting | Many to many within a room: each participant publishes under a namespace of its own and receives the others |
| Participant | A person or an agent that takes part in a room's meeting or chat. In a meeting it is known by its id, in the chat by its name |
| Chat line | A line of a room's chat, numbered in the order it reached the gatherer; every viewer sees the lines of a room in that order. What one participant said, before it is numbered, is that participant's line (`said`) |
| Transcript | The words read from the sound of a broadcast or a meeting, a piece for each stretch of voice |
