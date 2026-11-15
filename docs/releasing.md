# Releasing FOSSPS

A release is made by pushing a version tag. `.github/workflows/release.yml` does the rest: it builds `fossps` and `fossps-server` for Linux, Windows and macOS, tries each download, and publishes them on the [releases page](https://github.com/tahuffman1s/FOSSPS/releases).

## Cutting a release

1. Set `version` under `[workspace.package]` in the root `Cargo.toml` (for example `0.1.0`), run `cargo check` so `Cargo.lock` follows, and merge that through a pull request as usual.
2. Tag the merged commit and push the tag:

   ```sh
   git switch main && git pull
   git tag v0.1.0
   git push origin v0.1.0
   ```

3. Watch it: `gh run watch` or the Actions tab. When it is green the release is public.

The tag has to be `v` followed by the version in `Cargo.toml`. If they differ the run stops in its first minute and says what to change, because the programs would otherwise report the wrong version.

A version with a dash in it (`0.2.0-rc.1`) is published as a pre-release, which GitHub does not offer as the latest one.

## What a release holds

| File | For |
| --- | --- |
| `fossps-linux-x86_64.tar.gz` | Linux, 64-bit Intel/AMD, glibc 2.35 or newer (Ubuntu 22.04, Fedora 36, Debian 12 and later) |
| `fossps-windows-x86_64.zip` | Windows 10 and 11, 64-bit; needs no Visual C++ installer |
| `fossps-macos-arm64.tar.gz` | macOS 11 or newer on Apple silicon (experimental: needs MoltenVK) |
| `SHA256SUMS` | checksums of the three, for `sha256sum --check SHA256SUMS` |

Each archive unpacks to one folder, `fossps-<version>-<system>`, holding `fossps`, `fossps-server`, a short `README.txt` (from `.github/release/README.txt`) and the licences.

The file names carry no version, so the newest release is always at the same address:

```
https://github.com/tahuffman1s/FOSSPS/releases/latest/download/fossps-linux-x86_64.tar.gz
```

## If a run fails

- **Before publishing** (a build or a check failed): the release was not made, or is still a draft nobody else can see. Fix the cause and use "Re-run failed jobs"; a draft left by the earlier run is filled and published. If the fix needs a new commit, delete the tag (`git push origin :v0.1.0`), delete the draft on the releases page if there is one, and tag the new commit.
- **After publishing**: a published release is never changed. Release the fix under the next version number.

## Trying the workflow without releasing

A pull request that changes `release.yml` or `.github/release/`, or a run started by hand (`gh workflow run release.yml`), builds and tries all three downloads and writes the checksums, but publishes nothing. The archives are attached to the run as artifacts.
