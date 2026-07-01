# aot-lang - Roadmap

> Path from scaffold to a stable 1.0. Hard parts are front-loaded; each phase has hard exit criteria.
> Master plan: ../../_strategy/LANG_COLLECTION.md
>
> **Anti-deferral rule:** no listed hard task moves to a later phase unless this file records the move and the reason.

## v0.1.0 - Scaffold (DONE)
Compiles, CI green, structure correct, no domain logic.
- [x] Manifest, README, CHANGELOG, REPS, dual license, CI, deny, clippy, rustfmt.

## v0.2.0 - Core (THE HARD PART, NOT DEFERRED) (DONE)
Ahead-of-time compilation of IR into a single linked image: lower each function to
object code with codegen-lang, encode it, and lay the objects out with linker-lang.
Dependencies (ir, codegen, linker) are wired here, where first used.
Exit criteria:
- [x] Every public item has rustdoc + a runnable example.
- [x] Core invariants property-tested (encode/decode round-trip over generated functions).

## v1.0.0 - API freeze (DONE)
Public surface stable and frozen until 2.0. No breaking change from v0.2.0.
- [x] docs/API.md marked stable; SemVer promise recorded.
- [x] Full test + benchmark suite green on all three platforms.
- [x] Hardening: exact, overflow-safe object-code sizing; expanded property, layout,
      and error-path tests; runnable examples.
