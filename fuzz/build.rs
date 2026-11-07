//! Builds zlib's infback9 (Mark Adler's Deflate64 decoder, in infback9/),
//! the reference for the `inflate64` target.

fn main() {
    let mut build = cc::Build::new();
    build.files(["infback9/infback9.c", "infback9/inftree9.c", "infback9/reference.c"]).include("infback9");
    // zlib.h, from the zlib that libz-sys builds.
    if let Some(include) = std::env::var_os("DEP_Z_INCLUDE") {
        build.include(include);
    }
    build.compile("infback9");
    println!("cargo:rerun-if-changed=infback9");
}
