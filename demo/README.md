# Demo

A page publishes video through the own relay, and other pages watch it.

```sh
mise run demo
```

Then, in Chromium (the pages need WebTransport and WebCodecs with H.264):

1. Open <http://localhost:8080/publish>. It paints a clock and publishes it; <http://localhost:8080/publish?source=camera> publishes the camera.
2. Open <http://localhost:8080/watch> in one or more other tabs. A viewer that comes while another watches fetches the Group so far from the relay and has a picture at once; <http://localhost:8080/watch?join=no> waits for the next key frame instead, a second at most.

Ctrl-C stops the web server and the relay. `bun demo/serve.ts --port N --relay-port M` chooses other ports.

The publishing page sends a second track with sound, Opus: a quiet tone that goes up and down (`?audio=mic` takes the microphone, `?audio=none` sends none). On the watching page, click "sound" to hear it: a browser lets a page make sound only after a click.

**Picture and sound are played together, by the times their Objects carry.** The watching page holds each frame and each piece of audio for 150 ms, counted from the fastest one that came (`/watch?buffer=N` for another time, in milliseconds; 0 plays each as it comes, and then most of the audio is too late to be sounded). Every 2 seconds the publishing page shows a white square and sounds a beep at the same moment, for 200 ms; the lines under the picture say how far apart the two came out, and how long a frame and a piece took from the publisher's clock to this page.

**A room** comes with it: the spike in `spikes/durable-objects-room`, a Durable Object run locally with Miniflare (`mise run demo` installs it there the first time). Under the picture are a chat and how many are connected, and the watching page learns from the room where the broadcast is; `?room=NAME` on both pages is another room. The broadcast is known to the room as long as the publishing page is open.

**A session that ends** (the relay stopped, the link gone for more than 6 seconds): both pages say so in their first line and connect again every 2 seconds.

**With tokens** (an experiment; how the relay authorizes is undecided): `bun demo/serve.ts --auth` makes a key for the run and gives it to the relay and to the rooms. The relay then refuses a request without a token; the pages ask the room for one (`pub` to publish, `sub` to watch) and give it to their connection, which puts it into every request. `?token=no` on a page asks for none, and the page says that the relay refused. With `--auth-setup` as well, the relay looks at the token in SETUP and closes a session without one.

No catalog. The times are compared on one machine, where the two pages have the same clock.
