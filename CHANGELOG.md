<h1 align="center">
    <img width="90px" height="auto" src="https://raw.githubusercontent.com/jamesgober/jamesgober/main/media/icons/hexagon-3.svg" alt="Triple Hexagon">
    <br><b>CHANGELOG</b>
</h1>
<p>
  All notable changes to <code>aot-lang</code> will be documented in this file. The format is based on <a href="https://keepachangelog.com/en/1.1.0/">Keep a Changelog</a>,
  and this project adheres to <a href="https://semver.org/spec/v2.0.0.html/">Semantic Versioning</a>.
</p>

---

## [Unreleased]

### Added

### Changed

### Fixed

### Security

---

## [1.0.1] - 2026-10-08

A correctness patch. The object code now carries each function's label table under a
versioned header, so jump targets resolve from the bytes. No public API change; the
byte format changes, deliberately, because `1.0.0`'s bytes could not represent a
branch.

### Fixed

- **Lossy object code (ISSUES H02).** `encode` wrote a program's register count,
  parameters, and ops but never its label table. A `jump` or `jump_unless` names a
  label id, not an op index, so in `1.0.0` output those targets could not be
  resolved: every function with a branch or loop was unrecoverable from the image.
  The module docs called the encoding "lossless", and the crate docs and README
  called the image "ready to be … loaded"; both were false. Each function's record
  now carries the complete label table (`label_count`, then one `u32` op index per
  label id). The tests read every function back out of linked images and compare
  it field by field with the program codegen-lang lowered: name (from the symbol),
  register count, parameters, labels, and ops, with float constants compared by bit
  pattern.
- **No format version.** Every record now starts with the magic `AOTB` and a
  `u32` format version (`1`), so a reader can refuse bytes it does not understand
  instead of misreading them. `1.0.0`'s headerless layout is treated as version 0
  and is rejected at the magic.
- **Why a format change in a patch.** The SemVer promise said object-code bytes are
  stable for a given program. They change here because the `1.0.0` bytes were
  unusable for any program with control flow, which makes this a bug fix, not a
  feature. A record grows by 12 bytes (magic, version, label count) plus 4 bytes per
  label. `docs/API.md` now specifies the format and qualifies the stability promise
  as "within a format version".
- Corrected the claims: the crate docs, README, and `docs/API.md` no longer say
  the image is ready to be loaded. aot-lang ships no loader or interpreter; they now
  say the records are self-describing and specify the format a loader can be
  written against.
- Counts written to the format (`param_count`, `op_count`, label offsets) use a
  saturating `u32` narrowing instead of `as u32`. They are bounded by `u32`
  upstream; for a program that exceeded the bound, the record fails to decode
  instead of silently aliasing a smaller count.

### Changed

- **`codegen-lang` requirement raised to `1.0.1`** (from `1`). codegen-lang `1.0.0`
  miscompiled loop back edges that pass a header's own parameters in a different
  order (ISSUES H01, for example `jump H(b, a)`). aot-lang lowers through it, so
  the miscompile reached its images. The floor guarantees the fix rather than
  leaving it to lockfile resolution. Op streams change only for functions with such
  an edge, where the old ones were wrong.

### Added

- Tests-only decoder (`tests/support/decode.rs`), also compiled into the unit tests.
  It is not public API. It states the format independently of the encoder, checks
  every read, bounds every count by the bytes remaining, and refuses bad magic,
  unknown versions, unknown tags, truncation, and trailing bytes.
- `tests/format.rs`: round-trips through linked images, both named cases and a
  property over arbitrary generated control-flow graphs (1 to 6 blocks, loops,
  permuted back edges, several functions per image, arbitrary base address). Every
  decoded jump target resolves inside its record, records tile `.text` exactly, and
  `1.0.0`-layout bytes are refused.
- `tests/execute.rs`: looping programs run end to end through the image. Functions
  are decoded from `.text`, executed by a test interpreter, and compared with a
  direct IR evaluation. Covers `sum_to`, `fib`, a swap back edge, a 3-cycle rotation,
  a property over counted loops whose back edges permute, rotate, duplicate, and
  offset their variables, and a property over arbitrary CFGs. Run against
  codegen-lang `1.0.0`, the swap, rotation, and permuting-loop tests fail.
- Unit tests for the header, the label table, NaN payloads, every truncation length,
  hostile counts, and the unversioned `1.0.0` layout, plus a property that the
  decoder is total on arbitrary bytes.

---

## [1.0.0] - 2026-07-01

API freeze. The public surface is stable and frozen until `2.0`; it does not change
from `0.2.0`. This release ratifies the SemVer promise, hardens the object-code
encoding, and expands the test suite and examples.

### Added

- `docs/API.md` marked stable, with the `1.0` SemVer promise recorded.
- Runnable examples: `compile_and_inspect`, `multi_function_image`, `error_handling`.
- Property test that the encoded length is predicted exactly (the output buffer is
  allocated once and never grows).
- Integration tests for base-address invariance, many-function layout, and symbol
  names carrying punctuation.

### Changed

- Object-code sizing is now computed exactly and with saturating arithmetic, so
  `encode` allocates its buffer once and cannot overflow the capacity computation.
- Hot encoding paths are marked `#[inline]`.
- Stability language across the crate root, `README`, and `docs/API.md` now states
  the frozen `1.0` surface and the SemVer promise.

### Fixed

### Security

- No advisories. `cargo audit` and `cargo deny check` pass with the `1.x`
  dependency tree.

---

## [0.2.0] - 2026-06-30

The core release. aot-lang now compiles `ir-lang` functions ahead of time into a
single linked image, wiring `codegen-lang` and `linker-lang` into a two-stage
pipeline. The public surface is small and every item is documented with runnable
examples; it is designed to freeze at `1.0`.

### Added

- `compile` &mdash; ahead-of-time compile a single function into an [`Image`], with
  the function as the entry point.
- `Compiler` &mdash; a builder that accumulates functions and links them into one
  image, with configurable base address and entry point.
- `AotError` &mdash; the failure type, distinguishing the code-generation stage
  (`Codegen`) from the link stage (`Link`); implements `core::error::Error` and
  converts from both underlying errors. `#[non_exhaustive]`.
- Re-exports of `Image` and `OutputSection` from `linker-lang` as the result types.
- A deterministic, little-endian object-code encoding for the lowered bytecode,
  covered by an encode/decode round-trip property test.
- Dependencies wired: `ir-lang`, `codegen-lang`, `linker-lang` (all `1.x`).
- Benchmarks for single-function compilation and multi-function linking.
- `docs/API.md` documenting the full public surface with examples.

### Changed

- `Cargo.toml` metadata: corrected the `keywords`/`categories` arrays and sharpened
  the crate description to match the implemented scope.
- Aligned the `clippy.toml` MSRV (`1.85`) with `Cargo.toml`.

### Fixed

- Invalid TOML in the `keywords` and `categories` fields that blocked the manifest
  from parsing.

### Security

- No advisories. `cargo audit` and `cargo deny check` pass with the `1.x`
  dependency tree.

---

## [0.1.0] - 2026-06-18

Initial scaffold and repository bootstrap. No domain logic yet &mdash; this release establishes the structure, tooling, and quality gates the implementation will be built on.

### Added

- `Cargo.toml` with crate metadata, Rust 2024 edition, MSRV 1.85.
- Dual `Apache-2.0 OR MIT` license files.
- `README.md`, `CHANGELOG.md`, and a documentation skeleton.
- `REPS.md` compliance baseline.
- `.github/workflows/ci.yml` CI matrix; `deny.toml`, `clippy.toml`, `rustfmt.toml`.
- `dev/DIRECTIVES.md` and `dev/ROADMAP.md` (committed engineering standards + plan).

[Unreleased]: https://github.com/jamesgober/aot-lang/compare/v1.0.1...HEAD
[1.0.1]: https://github.com/jamesgober/aot-lang/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/jamesgober/aot-lang/compare/v0.2.0...v1.0.0
[0.2.0]: https://github.com/jamesgober/aot-lang/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/jamesgober/aot-lang/releases/tag/v0.1.0
