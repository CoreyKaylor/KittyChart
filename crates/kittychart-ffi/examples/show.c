/* Draw a chart in the terminal from C.
 *
 *   cargo build --release -p kittychart-ffi
 *   cc examples/show.c -Iinclude -Ltarget/release -lkittychart_ffi -lm -lpthread -ldl -o show
 *   ./show [config.json]
 */
#include <stdio.h>
#include <stdlib.h>
#include "kittychart.h"

static char *read_file(const char *path) {
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    rewind(f);
    char *s = malloc((size_t)n + 1);
    if (s && fread(s, 1, (size_t)n, f) == (size_t)n) s[n] = '\0';
    fclose(f);
    return s;
}

int main(int argc, char **argv) {
    char *json = argc > 1 ? read_file(argv[1]) : NULL;
    kc_chart *chart = kc_chart_from_json(json ? json :
        "{\"type\":\"line\",\"data\":{\"labels\":[\"Mon\",\"Tue\",\"Wed\",\"Thu\",\"Fri\"],"
        "\"datasets\":[{\"label\":\"Requests\",\"data\":[120,190,170,250,210],\"fill\":true}]}}");
    free(json);
    if (!chart) {
        fprintf(stderr, "kittychart: %s\n", kc_last_error());
        return 1;
    }
    kc_show_options opts = { .cols = 60 };
    int rc = kc_show(chart, &opts);
    if (rc != 0) fprintf(stderr, "kittychart: %s\n", kc_last_error());
    kc_chart_free(chart);
    return rc == 0 ? 0 : 1;
}
