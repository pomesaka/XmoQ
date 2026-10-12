# Keys for the terminal viewer, and following the room's broadcast

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 3, terminal controls

## Objective

The viewer of the terminal (`spikes/terminal-av`, `xmoq-watch`) could be ended with Ctrl-C and nothing else, and stayed with the broadcast it found when it joined a room. Whether it can be worked with keys while it plays, leave the terminal as it found it on every way out, and follow a broadcast that ends and begins again.

## What was built

Nothing outside `spikes/terminal-av`.

- **Keys**, where stdin is a terminal: `q` or Ctrl-C ends; `+` and `-` change the volume by 0.1; `m` mutes; `t` or Enter begins a line for the chat, Enter sends it, Esc gives it up, Backspace takes a character. A fourth line under the picture shows the keys, or the line being typed.
- **The terminal's line mode**: canonical mode, echo and signals are turned off with `termios` (crate `libc`; the output's processing is left on). It is put back when the viewer ends: by `q`, Ctrl-C (a byte now, not a signal), SIGINT, SIGTERM and SIGHUP through the same way out, and by a hook where it panics. A signal that cannot be caught leaves the terminal as it is.
- **Following the room**: without a broadcast the viewer waits for one; when a track ends or the room names another broadcast it asks the room again, takes a new token and connects anew. A failure of the first try ends the viewer (a relay that turns it down is to be seen); a later one is tried again after a second.

## Commands

```sh
spikes/terminal-av/try-keys.sh                                        # the check, on a pseudo-terminal
ROOM=http://127.0.0.1:8787/rooms/demo spikes/terminal-av/watch.sh     # for a person
```

`try-keys.sh` starts a demo of its own with authorization and rooms, puts its publishing page into a room in headless Chromium, and runs the viewer on a pseudo-terminal (`keys-on-a-pty.py`), which presses the keys and reads the terminal's mode before, during and after. **Nobody looked at the pseudo-terminal**; what was checked is the viewer's log, the page's chat and the mode.

## Observed results

One run.

| What | Outcome |
|---|---|
| `+ +`, `m`, `m`, `-` from volume 0 | `VOLUME level=0.1`, `0.2`, `0.2 muted=true`, `0.2 muted=false`, `0.1` |
| `t`, `typed in the termx`, Backspace, `nal`, Enter | the viewer: `SAID typed in the termnal`; the page's chat: `1 terminal: typed in the termnal` |
| The page is loaded anew while the viewer watches | `ROOM_BROADCAST none`, the track ends after 237 frames, `ROOM_BROADCAST none: waiting`, then the new broadcast and a new token, and 269 more frames until `q` |
| The mode while watching | `icanon=0 echo=0 isig=0` |
| The mode after `q` | `icanon=1 echo=1 isig=1`, as before; exit 0 |
| After Ctrl-C | the same; exit 0 |
| After SIGTERM | the same; exit 0 |
| After a panic (made by an environment variable for this check) | the same; exit 101 |

`try.sh` and `try-room.sh` of the records before were run again and ended as they did.

## Not done

- Looking at it in Ghostty: whether the line under the picture reads well while frames are drawn over it, and what a terminal narrower than the picture does.
- Keys while no frame has been drawn yet are taken, and their line is not shown: it is written with a frame.
- A broadcast that moves to another relay (the same path as the one that was run, with another address; not run).
- Changing the size of the picture, pausing, seeking.

## Next action

A person tries the keys in Ghostty.
