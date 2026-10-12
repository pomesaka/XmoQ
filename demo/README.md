# Demo

A page publishes video through the own relay, and other pages watch it.

```sh
mise run demo
```

Then, in Chromium (the pages need WebTransport and WebCodecs with H.264):

1. Open <http://localhost:8080/publish>. It paints a clock and publishes it; <http://localhost:8080/publish?source=camera> publishes the camera.
2. Open <http://localhost:8080/watch> in one or more other tabs. A viewer that comes while another watches fetches the Group so far from the relay and has a picture at once; <http://localhost:8080/watch?join=no> waits for the next key frame instead, a second at most.

**The publishing page may be a tab in the background**: its clock runs in a worker, which a browser does not slow down as it does the timers of a page nobody looks at. (Before, a publisher in a background tab sent one frame a second, and no sound that was in time.) The first line under each picture has the frames a second, and the watching page a line on the sound: what goes to the loudspeaker, and how loud.

**Where the delay comes from** is the second line under the watching page's picture: how long a frame took to get there (encoder, link, relay, decoder), how long it waited in the buffer, and how long for the next repaint. On one machine nearly all of it is the buffer. The buffer is set with the buttons and the slider beside "sound" while watching (or `?buffer=50`); "auto" (`?buffer=auto`, a trial, not the default) lets it follow how late things come, 400 ms at most. The default is 50 ms. The sound waits 30 ms at least, also where the buffer is less: with no wait at all its pieces come after their time and nothing sounds. With a camera, the publishing page says how long a frame took from the camera to the encoder where the browser tells; that part is not in the viewer's numbers.

**On a link that loses packets** the relay's congestion control decides whether the picture stays live: `bun demo/serve.ts --congestion bbr` (the relay's default is cubic; [record](../docs/experiments/2026-10-11-the-demo-over-a-lossy-link-with-bbr.md)). `/publish?picture=busy&bitrate=2500000` is a picture that fills its bitrate, and `--relay-log FILE` keeps the relay's lines.

**On a link with less room than the video** `bun demo/serve.ts --delivery-patience-ms 500` starts the relay with its experiment of giving up a Group a viewer does not have in time: the picture then stays about half a second behind and does not fall further, and the page shows more frames than without ([record](../docs/experiments/2026-10-12-a-page-behind-a-link-with-too-little-room-and-a-relay-that-gives-groups-up.md)).

`/publish?keyint=2000` makes a Group two seconds long (a second when left out), and `/watch?join=no` waits for the next key frame where the page otherwise fetches the Group it comes into ([record](../docs/experiments/2026-10-12-the-first-picture-of-a-viewer-who-comes-late-fetch-or-wait.md)).

**Two versions of the video (a trial)**: `/publish?ladder=1` encodes the picture a second time at 320x180 and 500 kbit/s and has both tracks in one alternate group of the catalog; `/watch?abr=1` plays the smaller one while the picture comes late and the larger one again when it has come in time for a while ([record](../docs/experiments/2026-10-12-two-versions-of-the-video-and-a-viewer-that-changes-between-them.md)). Without the two flags the demo is as before.

`/chat?room=R&name=a&peers=a,b` is a try of chat as MoQ tracks beside the room's WebSocket, driven by a script and linked from nowhere ([record](../docs/experiments/2026-10-12-chat-as-moq-tracks-beside-the-rooms-websocket.md)).

`bun demo/check.ts` checks a running demo in headless Chromium: the frames a second, and that sound leaves the watching page.

Ctrl-C stops the web server and the relay. `bun demo/serve.ts --port N --relay-port M` chooses other ports.

The publishing page sends a second track with sound, Opus: a quiet tone that goes up and down (`?audio=mic` takes the microphone, `?audio=none` sends none). On the watching page, click "sound" to hear it: a browser lets a page make sound only after a click.

**The tracks are found in a catalog.** The publishing page puts out an MSF catalog (`createCatalogPublisher` of `@xmoq/msf`) with its video track (`avc3.42E01F`) and, unless `?audio=none`, its audio track (`opus`); the watching page reads it on every session (`followCatalog`) and plays the first track with the role `video` and the first with the role `audio`, by the names and codecs it finds there. Others can read it: `xmoq-moqt-client --catalog-video`, and `mlmsub -videoname video -audioname audio` of moqlivemock ([record](../docs/experiments/2026-10-12-the-demo-has-a-catalog.md)).

**Picture and sound are played together, by the times their Objects carry.** The playing is `createPlayout` of `@xmoq/media`; the page draws, counts and shows. The watching page holds each frame and each piece of audio for 150 ms, counted from the fastest one that came (`/watch?buffer=N` for another time, in milliseconds; 0 plays each as it comes, and then most of the audio is too late to be sounded). Every 2 seconds the publishing page shows a white square and sounds a beep at the same moment, for 200 ms; the lines under the picture say how far apart the two came out, and how long a frame and a piece took from the publisher's clock to this page.

**A room** comes with it: the spike in `spikes/durable-objects-room`, a Durable Object run locally with Miniflare (`mise run demo` installs it there the first time). Under the picture are a chat and how many are connected, and the watching page learns from the room where the broadcast is; `?room=NAME` on both pages is another room. The broadcast is known to the room as long as the publishing page is open.

**A session that ends** (the relay stopped, the link gone for more than 6 seconds): both pages say so in their first line and connect again every 2 seconds (`keepPublishing` and `keepPlaying` of `@xmoq/media`).

**With tokens** (an experiment; how the relay authorizes is undecided): `bun demo/serve.ts --auth` makes a key for the run and gives it to the relay and to the rooms. The relay then refuses a request without a token; the pages ask the room for one (`pub` to publish, `sub` to watch) and give it to their connection, which puts it into every request. The relay looks at the token in SETUP as well, and closes a session without one: `?token=no` on a page asks for none, and the page says that the relay closed the session as not authorized.

No catalog. The times are compared on one machine, where the two pages have the same clock.
