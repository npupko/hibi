## Summary

<!-- What does this change, and why? -->

## Related issues

Closes #

## Checklist

- [ ] `cargo fmt --all --check` is clean
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` is clean
- [ ] `cargo test --workspace` passes
- [ ] `cargo build --workspace --no-default-features` builds
- [ ] `design/SPEC.md` and the code agree; if one changed, the other changed in this PR
- [ ] Schemas regenerated if a record type changed (`cargo run -p hibi-cli --example write-schemas`)
- [ ] CLI reference regenerated if a command or option changed (`cargo run -p hibi-cli --example cli-reference`)
- [ ] No file under `cases/` was edited to make a test pass
- [ ] The PR title follows [Conventional Commits](https://www.conventionalcommits.org/)
