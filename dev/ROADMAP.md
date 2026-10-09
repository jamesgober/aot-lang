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

## v1.0.1 - Patch: complete, versioned object code (DONE, 2026-10-08)
Fixes ISSUES **H02** from `_lexersketch/ISSUES.md`. No public API change; the object-code
byte format changes (bug fix: 1.0.0 bytes could not represent a branch).

Delivered:
- **H02.** Each function record now carries a header (magic `AOTB`, format version `1`)
  and the complete label table, so jump targets resolve from the bytes. Format specified
  in `docs/API.md#object-code-format`; the SemVer byte-stability promise now reads
  "within a format version".
- Decoder shipped **tests-only** (`tests/support/decode.rs`, also compiled into the unit
  tests). A public decoder would be new API, which a patch cannot add.
- Round-trip tests compare every decoded record with the lowered `Program` on all of its
  fields. codegen-lang 1.0 has no public `Program` constructor and no `labels()`
  accessor, so the comparison projects the program through its public accessors
  (`register_count`, `params`, `label_offset` for every id, `ops`) plus the symbol name.
- End-to-end execution tests: looping programs decoded from the image and run, checked
  against IR evaluation.

Decisions recorded here per the anti-deferral rule:
- **Public decoder / loader:** moved out of the patch (new API). If wanted in `1.x`, it is
  a minor release (`aot_lang::decode` or similar). The long-term answer is the native AOT
  path below, which emits real machine code instead of this bytecode container.
- **Label table, not resolved offsets:** jump targets stay label ids, and the table maps
  them to op indices. This keeps the op stream identical to codegen-lang's `Program`, so
  the record is lossless and a decoder can rebuild the exact program.

Dependency wiring: `codegen-lang` floor raised to `1.0.1` (H01 parallel-move fix: 1.0.0
miscompiled permuting back edges, which reached aot-lang images); `ir-lang = "1"` and
`linker-lang = "1"` unchanged.

Replacement: aot-lang's bytecode-container design is superseded by the native backend
in **Phase 5** of the LexerSketch roadmap (step 5.7, aot-lang 2.0: our backend to
object-lang to the linker to a real executable). This patch only makes the 1.x output
correct and self-describing.
