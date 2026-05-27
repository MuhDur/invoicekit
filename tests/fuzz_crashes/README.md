# Fuzz crash reproductions

When `.github/workflows/fuzz.yml` finds a crashing input, the CI job
uploads it as an artifact and the on-call agent reproduces locally,
minimizes the input, and lands it here as
`tests/fuzz_crashes/{target}-{short-hash}.input` plus a unit test in
the owning crate that loads the same bytes and asserts the panic is
gone.

Empty today (T-008 ships the harness; this directory fills as crashes
are found and fixed).
