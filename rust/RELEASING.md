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

## Version number

Workspace version lives in [Cargo.toml](Cargo.toml) under `[workspace.package] version`. Keep it in sync with the tag you push (e.g. tag `v0.0.1` ↔ version `0.0.1`).
