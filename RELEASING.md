# Public release workflow

This is the release procedure for publishing a Windie version on GitHub. A
release has two separate phases:

1. Prepare and review the release through a pull request into `main`.
2. After that pull request is merged, tag the merged commit. The tag starts the
   GitHub Actions packaging and publishing workflow.

Do not tag the release branch before the release pull request is merged. The
release tag must identify the commit that is actually on `main`.

## 1. Synchronize the release base

Use the repository's local `windie-2` development branch as the release base.
Before making release changes, update it from the remote `main` branch:

```bash
git fetch origin main
git switch windie-2
git pull --ff-only origin main
```

Confirm that the base is clean before editing:

```bash
git status --short --branch
git diff --exit-code
git diff --cached --exit-code
git submodule status
```

Do not continue while the base branch contains unrelated changes.

## 2. Verify vendor submodules

The Windie repository stores each submodule as an exact commit, not as a
floating branch. GitHub Actions must be able to fetch that exact commit from
the vendor repository.

For every submodule that changed in the release, inspect its state and verify
that its commit exists on the vendor remote:

```bash
for path in vendor/bifrost vendor/windie-inspector vendor/windie-landing-2nd; do
  echo "[$path]"
  git -C "$path" status --short --branch
  git -C "$path" fetch origin
  commit="$(git -C "$path" rev-parse HEAD)"
  git -C "$path" branch -r --contains "$commit"
done
```

A local-only vendor commit is not ready for release. Publish it through its
own vendor-repository branch and pull request, wait for that pull request to
merge, then pin Windie to the merged commit:

```bash
git -C vendor/<vendor> switch -c codex/<vendor-release-name>
git -C vendor/<vendor> push -u origin codex/<vendor-release-name>
```

The release-relevant vendor repositories use different stable branches and
therefore require separate vendor pull requests:

- `vendor/bifrost`: open the pull request against Bifrost's `dev` branch, then
  pin the merged commit from `origin/dev`.
- `vendor/windie-inspector`: open the pull request against Inspector's `main`
  branch, then pin the merged commit from `origin/main`.
- `vendor/windie-landing-2nd`: this submodule is not included in the current
  release packaging workflow and does not need a release pin unless the
  packaging workflow changes.

After the vendor pull request merges, update the pin from the vendor's merged
stable branch. For Bifrost:

```bash
git -C vendor/bifrost fetch origin dev
git -C vendor/bifrost switch --detach origin/dev
git add vendor/bifrost
```

For the Inspector:

```bash
git -C vendor/windie-inspector fetch origin main
git -C vendor/windie-inspector switch --detach origin/main
git add vendor/windie-inspector
```

The branch named in `.gitmodules` is only a tracking hint. The parent
repository still records one exact commit. A commit technically only needs to
be reachable from the vendor remote for CI to fetch it; Windie's release
policy additionally requires release-relevant vendor changes to be reviewed
and merged into the vendor repository's designated stable branch. The current
release packaging workflow uses `vendor/bifrost` and
`vendor/windie-inspector`; it does not package `vendor/windie-landing-2nd`.

Review all resulting pins before continuing:

```bash
git diff --submodule=log
git submodule status
```

## 3. Prepare version and changelog metadata

Replace `X.Y.Z` and `YYYY-MM-DD` below with the planned release version and
release date.

Update the version in the root `Cargo.toml`:

```toml
version = "X.Y.Z"
```

Update `CHANGELOG.md` with a user-facing section:

```markdown
## [X.Y.Z] - YYYY-MM-DD

- Describe the meaningful user-facing changes.
- Keep implementation details out unless they affect users, release
  reliability, or contributors.
```

Update the links at the bottom of the changelog:

```markdown
[Unreleased]: https://github.com/buiilding/Windie-Sandbox/compare/vX.Y.Z...HEAD
[X.Y.Z]: https://github.com/buiilding/Windie-Sandbox/releases/tag/vX.Y.Z
```

The release workflow extracts the text between `## [X.Y.Z]` and the next
version heading. The section must therefore exist and contain release notes.

## 4. Regenerate and verify Rust metadata

Run `cargo check` after changing `Cargo.toml`; this refreshes the root package
version in `Cargo.lock` when needed:

```bash
cargo check
cargo metadata --no-deps --format-version 1 | rg '"name":"windie"|"version":"X.Y.Z"'
git diff -- Cargo.toml Cargo.lock
```

Review the diff. Do not manually change unrelated lockfile entries.

## 5. Run release checks

Run the normal Rust checks locally:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

The correct Clippy lint is `-D warnings` (plural). Also run the frontend and
Inspector host checks when those components changed:

```bash
npm ci --prefix vendor/windie-inspector/frontend --legacy-peer-deps
npm run build --prefix vendor/windie-inspector/frontend
cargo check --manifest-path vendor/windie-inspector/host/Cargo.toml
```

Review the complete release diff, not only the version files:

```bash
git diff --check
git diff --stat
git diff
git diff --submodule=log
```

The pull request checks also validate the frontend, Rust, Inspector host, and
Windows build paths. A local macOS run does not replace the Windows CI job.

## 6. Commit and open the release pull request

Every release-preparation commit must include a meaningful changelog entry.
Stage the release metadata and any intentionally updated submodule pins:

```bash
git add CHANGELOG.md Cargo.toml Cargo.lock
git add vendor/<updated-submodule>  # only when this pin changed
git diff --cached --check
git commit -m "Prepare Windie vX.Y.Z release"
```

Before pushing, confirm that the release preparation is fully committed and
that no unrelated work remains:

```bash
git status --short --branch
git diff --exit-code
git diff --cached --exit-code
git submodule status
```

Following the repository branch rules, create the pushable branch from the
committed local `windie-2` state:

```bash
git switch -c codex/release-X.Y.Z
git push -u origin codex/release-X.Y.Z
```

Open a release pull request targeting `main`. A release pull request is the
exception to the normal issue-closing requirement, but its description should
still explain the release scope and verification performed:

```bash
gh pr create \
  --base main \
  --head codex/release-X.Y.Z \
  --title "Prepare Windie vX.Y.Z release" \
  --body-file release-pr.md
```

The pull request must include the complete diff and pass the required checks.
Wait for the maintainer to manually merge it into `main`.

## 7. Tag the merged commit

After the pull request is merged, synchronize the local release base again.
This prevents tagging the pre-merge branch by mistake:

```bash
git fetch origin main
git switch windie-2
git pull --ff-only origin main
git log -1 --oneline origin/main
```

Confirm that the version tag does not already exist, then create an annotated
tag and push it:

```bash
git tag --list vX.Y.Z
git tag -a vX.Y.Z -m "Release vX.Y.Z"
git push origin vX.Y.Z
```

The tag is created in the terminal and is not text that belongs in the pull
request description. It must point at the merged `main` commit.

## 8. Monitor and verify the public release

Pushing the `vX.Y.Z` tag triggers `.github/workflows/release.yml`. The workflow
builds these current targets:

- Linux x86_64
- Linux ARM64
- macOS x86_64
- macOS ARM64
- Windows x86_64

Monitor the workflow and release:

```bash
gh run list --workflow release.yml --limit 1
gh release view vX.Y.Z
```

The workflow publishes the platform archives and their SHA-256 files only
after every packaging job succeeds. Confirm that the release exists, that the
notes match the `CHANGELOG.md` section, and that all expected platform assets
are present before announcing the release.

If the workflow fails, diagnose and fix the failure in a follow-up pull
request. Do not move an existing release tag to a different commit without a
deliberate recovery decision.
