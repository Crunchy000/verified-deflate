/* A stand-in for zlib's internal zutil.h, written for verified-deflate's
 * fuzzing: just what infback9.c and inftree9.c use, so that they build
 * against zlib's public header alone. Not part of zlib.
 */
#ifndef REF9_ZUTIL_H
#define REF9_ZUTIL_H

#include <string.h>
#include "zlib.h"

#define local static
#define zmemcpy memcpy

/* infback9 falls back to these when the stream has no allocator. Renamed so
 * they cannot clash with the ones inside a zlib that is linked too. */
#define zcalloc ref9_zcalloc
#define zcfree ref9_zcfree
voidpf zcalloc(voidpf opaque, unsigned items, unsigned size);
void zcfree(voidpf opaque, voidpf ptr);

#define ZALLOC(strm, items, size) (*((strm)->zalloc))((strm)->opaque, (items), (size))
#define ZFREE(strm, addr) (*((strm)->zfree))((strm)->opaque, (voidpf)(addr))

#define Assert(cond, msg)
#define Trace(x)
#define Tracev(x)
#define Tracevv(x)
#define Tracec(c, x)
#define Tracecv(c, x)

#endif
