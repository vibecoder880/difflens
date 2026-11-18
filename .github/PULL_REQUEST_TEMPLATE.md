## Summary

What does this PR do? One or two sentences.

## Changes

- 

## Checks

- [ ] `cargo fmt --check -p difflens-core`
- [ ] `cargo clippy -p difflens-core --all-targets -- -D warnings`
- [ ] `cargo test -p difflens-core`
- [ ] `npm test` (vitest + EN/VI i18n parity)
- [ ] `npm run build`
- [ ] Conventional commit title (`feat:`/`fix:`/`chore:`/`docs:`/…)
- [ ] No new network egress outside `core/src/download/` (`scripts/audit-egress.sh` passes)

## Notes for reviewers

Anything subtle: pin bumps, fixture changes, migration concerns.
