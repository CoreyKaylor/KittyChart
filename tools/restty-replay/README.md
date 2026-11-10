# restty replay

Checks whether [restty](https://github.com/wiedymi/restty) (a browser terminal built on Ghostty's
libghostty-vt) displays real KittyChart output. Nothing here draws images itself: the page feeds
recorded bytes to restty through its PTY transport interface, and shows what restty sends back.

```sh
./record.sh   # builds, records 6 streams + reference screenshots (kc-preview, libghostty-vt)
./serve.sh    # http://localhost:8765/  (npm install on first run; PORT=… to change)
```

On the page:

- Pick a stream: left is restty, right is the reference screenshot of the same run.
- "Replay paced" feeds the stream in chunks, so the animation frames play.
- The log at the bottom shows restty's replies:
  - the kitty support query answer (`\e_Gi=19267;OK`);
  - `EINVAL` for the temp-file and shared-memory probes, so KittyChart sends images inline;
  - mouse reports when you move the pointer over a viewer replay (SGR pixel mode, `\e[<35;x;yM`).

The pane is sized so restty's grid is exactly 100×30, the size the streams were recorded at; the
header shows the grid restty reports. `?stream=NAME` preselects a stream.
