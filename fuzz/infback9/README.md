# infback9: the Deflate64 reference

`infback9.c`, `infback9.h`, `inffix9.h`, `inflate9.h`, `inftree9.c` and
`inftree9.h` come from `contrib/infback9` of
[zlib](https://github.com/madler/zlib) at tag `v1.3.2`. This is Mark
Adler's decoder for Deflate64 (ZIP method 9). The `inflate64` fuzz target
compares this crate's Deflate64 decoder against it.

One change, marked in `inftree9.c`: an empty code (all lengths zero) is
accepted. This 2003 code rejected it. zlib fixed the same bug in its own
`inftrees.c` in 1.2.1.1 (January 2004): "Fix a big fat bug in inftrees.c
that prevented decoding valid dynamic blocks with only literals and no
distance codes". The fix never reached `infback9`. RFC 1951 §3.2.7 allows
such blocks, and this crate's decoder accepts them, so the fuzzer found
the difference within seconds of its first run. The lines added are zlib
1.3.2's. The table built for an empty code still fails if a distance is
decoded with it. (Python's `inflate64`, another `infback9` descendant, has
the same bug.)

The other two files are not part of zlib:
- `zutil.h` stands in for zlib's internal header of that name. It has just
  the macros and declarations these files use, so they build against zlib's
  public `zlib.h` alone. `build.rs` takes `zlib.h` from `libz-sys`.
- `reference.c` runs `inflateBack9` on a whole buffer, with an output limit,
  for the fuzz target.

zlib's licence, which covers the six zlib files:

> Copyright (C) 1995-2026 Jean-loup Gailly and Mark Adler
>
> This software is provided 'as-is', without any express or implied
> warranty.  In no event will the authors be held liable for any damages
> arising from the use of this software.
>
> Permission is granted to anyone to use this software for any purpose,
> including commercial applications, and to alter it and redistribute it
> freely, subject to the following restrictions:
>
> 1. The origin of this software must not be misrepresented; you must not
>    claim that you wrote the original software. If you use this software
>    in a product, an acknowledgment in the product documentation would be
>    appreciated but is not required.
> 2. Altered source versions must be plainly marked as such, and must not be
>    misrepresented as being the original software.
> 3. This notice may not be removed or altered from any source distribution.
>
> Jean-loup Gailly        Mark Adler
> jloup@gzip.org          madler@alumni.caltech.edu
