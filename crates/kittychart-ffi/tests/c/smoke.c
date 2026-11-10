/* Exercises the whole C API. Usage: smoke OUT.png. Prints "ok" and exits 0 on success. */
#include <stdio.h>
#include <string.h>
#include "kittychart.h"

#define CHECK(cond) do { if (!(cond)) { fprintf(stderr, "FAIL %s:%d: %s (%s)\n", __FILE__, __LINE__, #cond, \
    kc_last_error() ? kc_last_error() : "no error"); return 1; } } while (0)

/* Scriptable options: color bars by their data index (red for index 1, else blue). */
static int calls = 0;
static const char *color_by_index(const char *id, const char *ctx, void *user) {
    (void)user;
    calls++;
    if (strcmp(id, "color") != 0) return NULL;
    return strstr(ctx, "\"dataIndex\":1,") ? "\"#ff0000\"" : "\"#0000ff\"";
}

static const char *BAR =
    "{\"type\":\"bar\",\"data\":{\"labels\":[\"Q1\",\"Q2\",\"Q3\"],"
    "\"datasets\":[{\"label\":\"Revenue\",\"data\":[12,19,7]}]}}";

int main(int argc, char **argv) {
    CHECK(argc == 2);
    CHECK(strlen(kc_version()) > 0);

    /* Errors: NULL return plus a message, and the next success doesn't need a reset. */
    CHECK(kc_chart_from_json("{not json") == NULL);
    CHECK(kc_last_error() != NULL && strlen(kc_last_error()) > 0);
    CHECK(kc_chart_from_json(NULL) == NULL);

    kc_chart *chart = kc_chart_from_json(BAR);
    CHECK(chart != NULL);

    /* RGBA: default options are 800 wide, aspect ratio 2. */
    kc_image img;
    CHECK(kc_render_rgba(chart, NULL, &img) == 0);
    CHECK(img.width == 800 && img.height == 400 && img.rgba.len == 800u * 400u * 4u);
    kc_bytes_free(img.rgba);

    kc_render_options ro = { .width = 400, .height = 300, .scale = 2, .background = "white" };
    CHECK(kc_render_rgba(chart, &ro, &img) == 0);
    CHECK(img.width == 800 && img.height == 600);
    CHECK(img.rgba.data[3] == 255); /* opaque background */
    kc_bytes_free(img.rgba);

    ro.background = "not-a-color";
    CHECK(kc_render_rgba(chart, &ro, &img) == -1);
    CHECK(strstr(kc_last_error(), "not-a-color") != NULL);

    /* PNG, for the test to compare with the Rust render. */
    kc_bytes png;
    CHECK(kc_render_png(chart, NULL, &png) == 0);
    CHECK(png.len > 8 && memcmp(png.data, "\x89PNG", 4) == 0);
    FILE *f = fopen(argv[1], "wb");
    CHECK(f != NULL);
    CHECK(fwrite(png.data, 1, png.len, f) == png.len);
    fclose(f);
    kc_bytes_free(png);

    /* SVG: a NUL-terminated document. */
    kc_bytes svg;
    CHECK(kc_render_svg(chart, NULL, &svg) == 0);
    CHECK(strncmp((const char *)svg.data, "<svg", 4) == 0 && strstr((const char *)svg.data, "</svg>") != NULL);
    kc_bytes_free(svg);

    /* Text: NUL-terminated, 40 columns of cells. */
    kc_bytes text;
    CHECK(kc_render_text(chart, 40, 0, NULL, &text) == 0);
    CHECK(strlen((const char *)text.data) == text.len);
    CHECK(strstr((const char *)text.data, "Revenue") != NULL);
    kc_bytes_free(text);
    CHECK(kc_render_text(chart, 40, 10, "dots", &text) == -1);

    /* Update in place; a bad update leaves the chart unchanged. */
    CHECK(kc_chart_set_json(chart, "{\"type\":\"nope\"}") == -1);
    CHECK(kc_chart_set_json(chart,
        "{\"type\":\"line\",\"data\":{\"labels\":[\"a\",\"b\"],\"datasets\":[{\"label\":\"Updated\",\"data\":[1,2]}]}}") == 0);
    CHECK(kc_render_text(chart, 40, 0, "braille", &text) == 0);
    CHECK(strstr((const char *)text.data, "Updated") != NULL);
    kc_bytes_free(text);

    /* Callbacks: a scripted chart renders exactly like its static equivalent. */
    kc_chart *scripted = kc_chart_from_json(
        "{\"type\":\"bar\",\"data\":{\"labels\":[\"a\",\"b\",\"c\"],"
        "\"datasets\":[{\"data\":[3,5,4],\"backgroundColor\":{\"__kc_fn\":\"color\"}}]}}");
    kc_chart *fixed = kc_chart_from_json(
        "{\"type\":\"bar\",\"data\":{\"labels\":[\"a\",\"b\",\"c\"],"
        "\"datasets\":[{\"data\":[3,5,4],\"backgroundColor\":[\"#0000ff\",\"#ff0000\",\"#0000ff\"]}]}}");
    CHECK(scripted && fixed);
    CHECK(kc_chart_set_dispatch(scripted, color_by_index, NULL) == 0);
    kc_image a, b;
    CHECK(kc_render_rgba(scripted, NULL, &a) == 0 && kc_render_rgba(fixed, NULL, &b) == 0);
    CHECK(calls >= 3 && a.rgba.len == b.rgba.len && memcmp(a.rgba.data, b.rgba.data, a.rgba.len) == 0);
    kc_bytes_free(a.rgba);
    kc_bytes_free(b.rgba);
    kc_chart_free(scripted);
    kc_chart_free(fixed);

    kc_bytes empty = { NULL, 0 };
    kc_bytes_free(empty);
    kc_chart_free(chart);
    kc_chart_free(NULL);
    puts("ok");
    return 0;
}
