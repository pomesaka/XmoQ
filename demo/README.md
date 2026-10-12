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

`bun demo/check.ts` checks a running demo in headless Chromium: the frames a second, and that sound leaves the watching page.

Ctrl-C stops the web server and the relay. `bun demo/serve.ts --port N --relay-port M` chooses other ports.

The publishing page sends a second track with sound, Opus: a quiet tone that goes up and down (`?audio=mic` takes the microphone, `?audio=none` sends none). On the watching page, click "sound" to hear it: a browser lets a page make sound only after a click.

**Picture and sound are played together, by the times their Objects carry.** The playing is `createPlayout` of `@xmoq/media`; the page draws, counts and shows. The watching page holds each frame and each piece of audio for 150 ms, counted from the fastest one that came (`/watch?buffer=N` for another time, in milliseconds; 0 plays each as it comes, and then most of the audio is too late to be sounded). Every 2 seconds the publishing page shows a white square and sounds a beep at the same moment, for 200 ms; the lines under the picture say how far apart the two came out, and how long a frame and a piece took from the publisher's clock to this page.

**A room** comes with it: the spike in `spikes/durable-objects-room`, a Durable Object run locally with Miniflare (`mise run demo` installs it there the first time). Under the picture are a chat and how many are connected, and the watching page learns from the room where the broadcast is; `?room=NAME` on both pages is another room. The broadcast is known to the room as long as the publishing page is open.

**A session that ends** (the relay stopped, the link gone for more than 6 seconds): both pages say so in their first line and connect again every 2 seconds (`keepPublishing` and `keepPlaying` of `@xmoq/media`).

**With tokens** (an experiment; how the relay authorizes is undecided): `bun demo/serve.ts --auth` makes a key for the run and gives it to the relay and to the rooms. The relay then refuses a request without a token; the pages ask the room for one (`pub` to publish, `sub` to watch) and give it to their connection, which puts it into every request. The relay looks at the token in SETUP as well, and closes a session without one: `?token=no` on a page asks for none, and the page says that the relay closed the session as not authorized.

No catalog. The times are compared on one machine, where the two pages have the same clock.
