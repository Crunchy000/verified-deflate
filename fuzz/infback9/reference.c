/* Runs zlib's infback9 (Mark Adler's Deflate64 decoder) on a whole buffer,
 * for verified-deflate's differential fuzzing. Not part of zlib. */
#include <stdlib.h>
#include "zutil.h"
#include "infback9.h"

voidpf zcalloc(voidpf opaque, unsigned items, unsigned size) {
    (void)opaque;
    return calloc(items, size);
}

void zcfree(voidpf opaque, voidpf ptr) {
    (void)opaque;
    free(ptr);
}

/* All input is given up front, so asking for more means it ran out. */
static unsigned no_more_input(void *desc, z_const unsigned char **next) {
    (void)desc;
    *next = Z_NULL;
    return 0;
}

struct sink {
    unsigned char *buf;
    size_t len;
    size_t limit;
};

static int put(void *desc, unsigned char *data, unsigned len) {
    struct sink *s = desc;
    if (len > s->limit - s->len)
        return 1; /* more than the limit: stop */
    memcpy(s->buf + s->len, data, len);
    s->len += len;
    return 0;
}

/* Decodes the raw Deflate64 stream in[0..in_len) into out (room for limit
 * bytes). Returns 0 when the stream is complete and valid, with its length
 * in *out_len and the input bytes it used in *consumed; 1 when its output
 * would pass the limit; -1 when it is invalid or truncated; -2 on an
 * allocation or setup failure. */
int reference_inflate9(const unsigned char *in, size_t in_len, unsigned char *out, size_t limit,
                       size_t *out_len, size_t *consumed) {
    z_stream strm;
    struct sink sink = {out, 0, limit};
    unsigned char *window;
    int ret;

    if (in_len > 0xffffffffu)
        return -2;
    window = malloc(65536);
    if (window == NULL)
        return -2;
    memset(&strm, 0, sizeof strm);
    if (inflateBack9Init(&strm, window) != Z_OK) {
        free(window);
        return -2;
    }
    strm.next_in = (z_const unsigned char *)in;
    strm.avail_in = (uInt)in_len;
    ret = inflateBack9(&strm, no_more_input, NULL, put, &sink);
    *out_len = sink.len;
    *consumed = strm.next_in == Z_NULL ? in_len : in_len - strm.avail_in;
    inflateBack9End(&strm);
    free(window);
    if (ret == Z_STREAM_END)
        return 0;
    if (ret == Z_BUF_ERROR && strm.next_in != Z_NULL)
        return 1; /* put() stopped it */
    return ret == Z_MEM_ERROR ? -2 : -1;
}
