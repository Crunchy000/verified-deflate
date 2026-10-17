# kani-sample

A local stand-in for the `kani` crate. It turns every `#[kani::proof]`
harness into an ordinary `#[test]` that runs the harness thousands of times
with random `kani::any()` values, discarding runs whose `kani::assume`
conditions fail.

**This is not verification.** It samples; Kani checks every input. It is
here so that harnesses can be compiled and smoke-tested quickly, and on
machines where Kani cannot be installed:

```sh
RUSTFLAGS="--cfg kani --cfg kani_sample" cargo test --lib --release
```

`KANI_SAMPLE_ITERS` (default 3000) sets runs per harness and
`KANI_SAMPLE_SEED` varies the inputs. The real check is `cargo kani`.
