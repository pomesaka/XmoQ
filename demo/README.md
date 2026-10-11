# Demo

A page publishes video through the own relay, and other pages watch it.

```sh
mise run demo
```

Then, in Chromium (the pages need WebTransport and WebCodecs with H.264):

1. Open <http://localhost:8080/publish>. It paints a clock and publishes it; <http://localhost:8080/publish?source=camera> publishes the camera.
2. Open <http://localhost:8080/watch> in one or more other tabs. The picture begins with the next key frame, a second at most.

Ctrl-C stops the web server and the relay. `bun demo/serve.ts --port N --relay-port M` chooses other ports.

No catalog, no audio, no buffer: a frame is drawn when it is decoded. The line under the picture has what was counted.
