# A voice for the transcriber's interop run

`hello-from-the-relay.wav`: "Hello from the relay.", 1.18 s, one channel of 16-bit samples at 16,000 Hz (42 KB). It was made on macOS 14.4 on 2026-10-25 with

```sh
say -v Samantha -o hello.aiff "Hello from the relay."
afconvert -f WAVE -d LEI16@16000 -c 1 hello.aiff hello-from-the-relay.wav
```

`interop/transcriber.ts` hands its samples to a page (`publishVoice` of `interop/page.ts`), which makes them 48,000 Hz and publishes them as an Opus track. WHY 16,000 Hz and not the 48,000 Hz the page sends: a third of the bytes, and whisper.cpp reads 16,000 Hz anyway. WHY a file and not `say` at each run: CI runs on Linux, which has no `say`.
