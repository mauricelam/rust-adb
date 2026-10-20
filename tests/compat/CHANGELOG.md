# Compatibility Suite Changelog

All notable changes to the compatibility test suite (`tests/compat/`), test fixtures (`**/fixtures/`), or mutation suite (`mutation/`) will be documented in this file.

Any change to an existing test, fixture, or mutation patch requires a version bump in `tests/compat/VERSION` and a documented reason in this file.

## [1.0.0] - 2026-03-02

### Added
- Initial frozen release of the compatibility test suite (`tests/compat/`) and mutation testing framework (`mutation/`).
- Baseline mutation testing suite verifying 100% kill rate against C++ ADB reference.
