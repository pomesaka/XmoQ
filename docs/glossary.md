# Glossary

The words of broadcasts and meetings that the code and the documents use, in the sense they have here. The code names its types after them.

The words of MoQ Transport (Publisher, Subscriber, Relay, Upstream, Track, Group, Object and the rest) mean what draft 18 says, in [§1.2 Terms and Definitions](https://www.ietf.org/archive/id/draft-ietf-moq-transport-18.html#section-1.2) and [§2 Object Data Model](https://www.ietf.org/archive/id/draft-ietf-moq-transport-18.html#section-2), and are not written again here. The names of the parts of the code, such as a published track or the gatherer, are in [the architecture](architecture.md).

| Word | Meaning |
|---|---|
| Room | Where people meet by name: a broadcast, its chat and a meeting are each of one room, under a namespace with the room's name in it (`xmoq/chat/<room>`, `xmoq/meet/<room>`). The room's own state (where the broadcast is, how many are connected, the seats of a meeting) is held by the room's server, over a WebSocket |
| Broadcast | Video and sound from one publisher to many viewers: the tracks under one namespace, described by one catalog. MSF-01 uses the word without defining it. A **registered broadcast** is where a page told the room its broadcast is (the relay's URL, the certificate's hash, the namespace), for as long as the page stays connected to the room. The **broadcast namespaces of the room** are those in which the room would give tokens for a broadcast; they are not the registered one, which any page can name. Proposed: the room gives a token for any namespace that is not a meeting's today |
| Meeting | Many to many within a room, of 49 participants at most: each publishes under `xmoq/meet/<room>/<id>` and receives the others |
| Participant | A person or an agent that takes part in a room's meeting or chat. In a meeting it is known by its id, in the chat by its name |
| Chat line | A line of a room's chat, numbered in the order it reached the gatherer; every viewer sees the lines of a room in that order. What one participant said, before it is numbered, is that participant's line (`said`) |
| Transcript | The words read from the sound of a broadcast or a meeting, a piece for each stretch of voice |
