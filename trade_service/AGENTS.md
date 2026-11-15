# Trade Service Development Checks

Keep the workspace's security and production boundaries. These rules only govern
development verification; they do not authorize deployment, SDK upgrades, wallet
access, production migrations, or transactions.

- On the current small VM, run `bash scripts/dev_check.sh check` before linking a
  new Rust test binary. Fix type errors before starting code generation.
- Use the bounded command in `docs/optimization/DEVELOPMENT_CHECKS.md` for linking
  and test execution on this VM. Request the required tool permission; do not
  bypass the memory limit by immediately retrying raw `cargo test`.
- The 2026-11-15 diagnosis already recorded two memory-limited builds and a
  bounded timeout on this VM. Do not repeat local test linking without changed
  resource prerequisites or an explicitly authorized diagnostic experiment.
  Type checks and shell-runner tests can proceed independently.
- Keep one Cargo operation at a time and retain the package-specific test debug
  override. Do not change global RUSTFLAGS or dependency profiles to save memory:
  that can trigger a complete dependency rebuild.
- Use focused tests while editing. Run complete concurrent/serial suites at a
  verified milestone, not after every partial edit. Never weaken tests to fit a
  resource budget; use a larger build environment when a bounded run cannot fit.
- Avoid `--quiet` for long checks. Preserve the run's log, exit code and time/RSS
  statistics under `target/dev-checks/`, which is already ignored by Git.
- An unknown tool session or a recorded `running` marker is not success. Inspect
  the run log, boot ID and actual process before deciding whether to wait or rerun.
  A busy lock is a reason to refuse duplicate work, not proof of task progress.
- A paused continuous goal stays paused. Diagnose an explicit new request without
  silently resuming the broader feature backlog.
