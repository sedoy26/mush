# Releasing mush-cli (GitHub)

The workflow [.github/workflows/release.yml](../.github/workflows/release.yml) builds **`mush-cli`** for Linux x86_64, Windows x86_64, and macOS (aarch64 + x86_64) and uploads them to a **GitHub Release** when you push a **version tag**.

A tag points at a **commit**, not a branch name. You can publish **v0.0.1 from `refactoring/rust` before merging** to `main`: push the tag while that commit is the tip of your branch (or any ancestor you intend to ship). After the Release workflow succeeds, merge the branch (or fast-forward `main`) so the default branch includes the same commit.

## Cut v0.0.1 from `refactoring/rust` (before merge)

1. Commit the release workflow, `rust/Cargo.toml` version **0.0.1**, and docs on **`refactoring/rust`** and push the branch:

   ```bash
   git checkout refactoring/rust
   git push -u origin refactoring/rust
   ```

2. Create the annotated tag on the commit you want to ship (must match `v*`):

   ```bash
   git tag -a v0.0.1 -m "Release v0.0.1"
   git push origin v0.0.1
   ```

3. Open **Actions** on GitHub and wait for the **Release** workflow to finish.
4. Open **Releases** and confirm the four binaries are attached.
5. Merge **`refactoring/rust` → default branch** (e.g. PR or local merge) so `main` carries the same tree as the tagged release.

If you prefer to merge first, you can tag after the merge instead—the Release workflow behaves the same as long as the tag’s commit contains `.github/workflows/release.yml` and the bumped `rust/Cargo.toml`.

`workflow_dispatch` on the same workflow only runs the **build** matrix (no Release job) when the ref is not a tag—useful to verify CI without shipping.

## Troubleshooting: tag exists but only “Source code” (no binaries / no Release)

GitHub always shows **zip/tar source** for any tag. A real **Release** with attached binaries only appears after the **Release** workflow finishes and the `release` job runs.

1. **Open Actions → “Release”** on the repo. If there is **no run** for your tag push:
   - Confirm the tag points at a commit that **contains** `.github/workflows/release.yml` (same tree you tested locally).
   - Confirm **GitHub Actions** is enabled (Settings → Actions → General).
   - Some teams merge the workflow file to the default branch once so the workflow is registered; then re-push the tag if needed (`git push origin :refs/tags/v0.0.1` then `git push origin v0.0.1`).

2. If a workflow run exists but is **red**: open the failed **build** matrix job (Linux / Windows / macOS). The **release** job is skipped until all four targets succeed.

3. After fixing the workflow or builds, **delete the remote tag and push it again** so the `push: tags` trigger fires (moving a tag with `--force` also works if you accept rewriting the tag).

## Version number

Workspace version lives in [Cargo.toml](Cargo.toml) under `[workspace.package] version`. Keep it in sync with the tag you push (e.g. tag `v0.0.1` ↔ version `0.0.1`).
