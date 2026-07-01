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

[Unreleased]: https://github.com/jamesgober/aot-lang/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/jamesgober/aot-lang/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/jamesgober/aot-lang/releases/tag/v0.1.0
