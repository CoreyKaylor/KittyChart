# C API

The C library is `crates/kittychart-ffi`. The header is
[`include/kittychart.h`](../crates/kittychart-ffi/include/kittychart.h), generated from the Rust
source by cbindgen; `just header` regenerates it, and a test fails when it's stale.

```sh
cargo build --release -p kittychart-ffi
# → target/release/libkittychart_ffi.{a,so,dylib} (kittychart_ffi.dll + .lib on Windows)
cc app.c -Icrates/kittychart-ffi/include target/release/libkittychart_ffi.a -lm -lpthread -ldl
```

Release archives contain the header, the static and shared libraries, and the CLI.

```c
#include "kittychart.h"

kc_chart *chart = kc_chart_from_json("{\"type\":\"bar\",\"data\":{\"labels\":[\"a\",\"b\"],"
                                     "\"datasets\":[{\"label\":\"x\",\"data\":[1,2]}]}}");
if (!chart) { fprintf(stderr, "%s\n", kc_last_error()); return 1; }

kc_show(chart, NULL);                                     /* draw in the terminal */

kc_render_options ro = { .width = 800, .scale = 2, .background = "white" };
kc_bytes png;
if (kc_render_png(chart, &ro, &png) == 0) {
    fwrite(png.data, 1, png.len, file);
    kc_bytes_free(png);
}
kc_chart_free(chart);
```

| Function | |
|---|---|
| `kc_chart_from_json(json)` | Parse a Chart.js config. NULL on error. |
| `kc_chart_set_json(chart, json)` | Replace the config (new data). Unchanged on error. |
| `kc_render_png(chart, opts, &bytes)` | PNG in memory. |
| `kc_render_rgba(chart, opts, &image)` | Raw RGBA8 pixels. |
| `kc_render_text(chart, cols, rows, mode, &bytes)` | Block/Braille text with ANSI colors (NUL-terminated). |
| `kc_show(chart, opts)` | Detect the terminal and draw at the cursor (Unix). |
| `kc_bytes_free(bytes)` | Free returned memory (including `kc_image.rgba`). |
| `kc_last_error()` | Message for the last failure on this thread. |
| `kc_chart_free(chart)`, `kc_version()` | |

Conventions:
- Functions return 0 on success and -1 on failure. Options pointers may be NULL, and zero or
  NULL fields mean "default".
- Returned buffers are NUL-terminated (not counted in `len`).
- Panics never cross the boundary.

The tests compile [`tests/c/smoke.c`](../crates/kittychart-ffi/tests/c/smoke.c) against the
header and check its PNG against the Rust render byte for byte. They also check it under
AddressSanitizer locally, and run [`examples/show.c`](../crates/kittychart-ffi/examples/show.c)
in Ghostty's VT core.
