# Releasing

The app version is also the library version (PRD 11.1). One SemVer number covers both.

## One-time setup

**Every key, certificate and password below is stored in the password vault "Lichtwart/Product 1".**
Nothing of it belongs in the repository (it is public). After putting a file into a secret, keep it
only in the vault and delete the local copy.

All signing secrets live in the GitHub environment **`release`** (Settings > Environments >
`release`). The `build` job reads only this environment; repository secrets or secrets in another
environment are not seen. Secrets cannot be copied or renamed between environments, set them again.

| Secret | Required | Content |
|---|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | yes | content of `~/.tauri/lichtwart-sim.key` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | yes | its password |
| `APPLE_CERTIFICATE` | all six or none | "Developer ID Application" `.p12`, base64 |
| `APPLE_CERTIFICATE_PASSWORD` | | export password of the `.p12` |
| `APPLE_SIGNING_IDENTITY` | | `Developer ID Application: Lichtwart GmbH (XD59B34DMF)` |
| `APPLE_API_KEY` | | App Store Connect API Key ID |
| `APPLE_API_ISSUER` | | App Store Connect Issuer ID |
| `APPLE_API_KEY_P8` | | `AuthKey_<KeyID>.p8`, base64 |

The commands below are for bash/zsh. **Nushell** appends a line break when it pipes a file or a
string into `gh secret set`. The updater key then fails with "Invalid symbol 10" (10 is the line
break). Set file-based secrets from bash/zsh with `<` (e.g. `bash -c 'gh secret set … < file'`), or
in Nushell pass the value with `--body (open ~/.tauri/lichtwart-sim.key | str trim)`. Empty values
(a wrong path in a pipe) are set without an error: check the timestamp in `gh secret list`.

`GITHUB_TOKEN` is provided by GitHub. The Apple Team ID (`XD59B34DMF`) is part of
`APPLE_SIGNING_IDENTITY`; API-key notarization does not need it separately. The workflow stops
early with a clear message when a required secret is empty.

### 1. GitHub CLI

The commands below use `gh`. With several GitHub accounts logged in, `gh` uses the active one; it
must be allowed to manage the repository's secrets:

```sh
gh auth status                       # shows the logged-in accounts and which one is active
gh auth switch -u lw-lueddemann      # make that account the active one
# or, for one command only:
GH_TOKEN=$(gh auth token -u lw-lueddemann) gh secret list -R Lichtwart/lw-data-simulator -e release
```

Pushing `.github/workflows/` needs the `workflow` scope: `gh auth refresh -h github.com -s workflow`.

### 2. Repository and update endpoint

The repository `Lichtwart/lw-data-simulator` is public (the updater downloads `latest.json` and the
installers without a token; `handoffs/` is ignored and not published).
`plugins.updater.endpoints` in `src-tauri/tauri.conf.json` points at
`https://github.com/Lichtwart/lw-data-simulator/releases/latest/download/latest.json`.
`node scripts/release-check.mjs vX.Y.Z` (first step of the Release workflow) fails when the
endpoint or the public key is a placeholder or the endpoint does not match `GITHUB_REPOSITORY`. CI
(no tag) only warns; `--strict` fails on it too.

### 3. Updater signing key (exactly one)

Generate it **once** and never regenerate it: installed apps accept only updates signed with the key
whose public half they carry. A new key means every installed app must be reinstalled by hand. (The
first key was generated twice by accident; the GitHub secret and the file then held different keys
and v0.1.0 cannot update. The key was replaced before v0.1.1.)

```sh
mkdir -p ~/.tauri
npx tauri signer generate -f -w ~/.tauri/lichtwart-sim.key   # asks for the password twice
cat ~/.tauri/lichtwart-sim.key        # private key (secret)
cat ~/.tauri/lichtwart-sim.key.pub    # public key
```

1. Put the `.key.pub` content (one base64 line) into `plugins.updater.pubkey` in
   `src-tauri/tauri.conf.json`.
2. Set the secrets from the file, so nothing is mixed up by copy and paste:

   ```sh
   gh secret set TAURI_SIGNING_PRIVATE_KEY -R Lichtwart/lw-data-simulator -e release < ~/.tauri/lichtwart-sim.key
   gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD -R Lichtwart/lw-data-simulator -e release   # asks for it
   ```
3. Store the key file, the `.pub` file and the password in the vault.

The key ID shows which key a file or signature belongs to (current key: `F0EA0FDB5A34733B`):

```sh
kid(){ echo "$1" | base64 -d | sed -n 2p | base64 -d | tail -c +3 | head -c 8 | xxd -p; }
kid "$(cat ~/.tauri/lichtwart-sim.key.pub)"                                              # key file
kid "$(node -p "require('./src-tauri/tauri.conf.json').plugins.updater.pubkey")"         # app config
kid "$(curl -sL https://github.com/Lichtwart/lw-data-simulator/releases/latest/download/latest.json \
  | node -e 'process.stdin.on("data",d=>console.log(Object.values(JSON.parse(d).platforms)[0].signature))')"  # release
```

All three must print the same bytes (little-endian, so `3b73345adb0feaf0` = `F0EA0FDB5A34733B`).

### 4. Apple signing and notarization (macOS)

Without the six `APPLE_*` secrets the macOS build is ad-hoc signed and not notarized (the workflow
prints a warning). macOS then blocks the first open of the downloaded app: System Settings > Privacy
& Security > "Open Anyway" (or `xattr -dr com.apple.quarantine "/Applications/Lichtwart Demo
Simulator.app"`). In-app updates still work, they are verified with the updater key. Setting only
some of the six fails the build.

Requirements: Apple Developer Program membership of Lichtwart GmbH (organization, Team ID
`XD59B34DMF`). Only the **Account Holder** can accept agreements and create Developer ID
certificates.

**a. Accept the current agreement.** If Xcode shows "Unable to process request - PLA Update
available", the Account Holder signs in at <https://developer.apple.com/account> and accepts the
updated Program License Agreement (banner at the top; open agreements also under
<https://appstoreconnect.apple.com> > Business).

**b. Create the "Developer ID Application" certificate.** Xcode > Settings > Accounts > Lichtwart
GmbH > Manage Certificates > **+** > **Developer ID Application**. (Not "Apple Development",
"Apple Distribution" or "Mac Installer Distribution": only Developer ID is accepted for apps
outside the App Store. If it is missing from the menu, the agreement is not accepted or the Apple
ID is not the Account Holder.) Alternative on the web: Certificates > **+** > Developer ID
Application > G2 Sub-CA with a CSR from Keychain Access > Certificate Assistant. Check:

```sh
security find-identity -v -p codesigning | grep "Developer ID Application"
# "Developer ID Application: Lichtwart GmbH (XD59B34DMF)"
```

**c. Export it as `.p12`.** Keychain Access > keychain **login** > tab **My Certificates** (in
"Certificates" the `.p12` format is greyed out because the private key is not included) > expand
"Developer ID Application: Lichtwart GmbH (XD59B34DMF)" (the private key must be underneath) >
right-click the certificate > Export > "Personal Information Exchange (.p12)". If `.p12` is still
greyed out, select the certificate and its key together and export both. The export password you
set here is used by the pipeline (`APPLE_CERTIFICATE_PASSWORD`), not by your Mac: use a long random
one from the vault.

**d. Create the App Store Connect API key (notarization).**
<https://appstoreconnect.apple.com/access/integrations/api> (the first time, the Account Holder
clicks "Request Access") > Team Keys > **+**, name "GitHub Notarization", access **Developer**.
Download `AuthKey_<KeyID>.p8` (possible only once) and note the **Key ID** and the **Issuer ID**
(above the list).

**e. Set the six secrets**, then store the `.p12`, its password and the `.p8` in the vault and
delete the local files:

```sh
R="-R Lichtwart/lw-data-simulator -e release"
base64 -i ~/Desktop/DeveloperID.p12 | gh secret set APPLE_CERTIFICATE $R
gh secret set APPLE_CERTIFICATE_PASSWORD $R           # asks for the .p12 export password
gh secret set APPLE_SIGNING_IDENTITY $R --body "Developer ID Application: Lichtwart GmbH (XD59B34DMF)"
gh secret set APPLE_API_KEY $R --body "<Key ID>"
gh secret set APPLE_API_ISSUER $R --body "<Issuer ID>"
base64 -i ~/Downloads/AuthKey_<KeyID>.p8 | gh secret set APPLE_API_KEY_P8 $R
gh secret list $R                                     # names only, never values
rm ~/Desktop/DeveloperID.p12 ~/Downloads/AuthKey_<KeyID>.p8
```

Check a notarized build after installing:
`spctl -a -vvv "/Applications/Lichtwart Demo Simulator.app"` must say
"accepted, source=Notarized Developer ID".

### 5. Protect the signing secrets

- Add **required reviewers** to the `release` environment: a pushed `v*` tag then waits for
  approval before any secret is available.
- Add a tag ruleset (Settings > Rules) that restricts who can create `v*` tags.
- The workflow also refuses tags whose commit is not on `main`.

## Local builds and the release overlay

`createUpdaterArtifacts` is `false` in `src-tauri/tauri.conf.json`, so `npm run tauri build` works on
a laptop without `TAURI_SIGNING_PRIVATE_KEY`. The Release workflow passes
`--config src-tauri/tauri.release.conf.json` (`bundle.createUpdaterArtifacts: true`) to get the signed
updater files and `latest.json`. `release-check.mjs` fails a tag when the overlay or that flag in the
workflow is missing. To try a signed build locally, export `TAURI_SIGNING_PRIVATE_KEY` (and its
password) and add the same `--config` argument.

## Cut a release

1. Move your notes into `## [Unreleased]` in `CHANGELOG.md`.
2. `node scripts/release.mjs 0.2.0` (for the first release: `0.1.0`, same version is fine). It
   must run on `main` with a clean working tree, and fails when `[Unreleased]` is empty or the
   version already has a section. It bumps `package.json`, `package-lock.json`,
   `src-tauri/Cargo.toml` (+ `Cargo.lock`), `tauri.conf.json` (if it holds a literal version) and
   turns "Unreleased" into a dated section (UTC date). It stops when a version pattern does not match.
3. Run `npm run check` and `node scripts/release-check.mjs v0.2.0`, then commit:
   `git commit -am "chore: release v0.2.0"`.
4. Tag with an **annotated** tag (`--follow-tags` skips lightweight tags):
   `git tag -a v0.2.0 -m "v0.2.0" && git push --follow-tags`.
5. The `Release` workflow:
   1. `verify`: the tag is on `main`, `release-check.mjs` passes (versions in all files, CHANGELOG
      section, no updater placeholders), and the notes are cut from the CHANGELOG.
   2. `create-release`: one **draft** release with the notes.
   3. `build` (needs approval of the `release` environment): universal macOS `.dmg` (signed and
      notarized, or ad-hoc signed without the Apple secrets) and Windows NSIS installer, signed updater artifacts and `latest.json`, all
      uploaded into the draft.
   4. `publish`: only when all builds succeeded, the draft is published (and marked latest).
   If a build fails, the draft stays unpublished: fix the problem, delete the draft release and the
   tag, and tag again. Versions with a pre-release suffix (`0.2.0-beta.1`) are published as
   pre-releases and never become "latest", so the updater ignores them.

CI also runs `node scripts/check-tauri-versions.mjs` (Tauri crate and npm package must match in
major.minor, otherwise `tauri build` refuses to run) and `node scripts/release-check.mjs` without a
tag. Both are part of `npm run check` (`npm run check:versions`). The `@tauri-apps/*` runtime
packages are pinned with `~` in `package.json`; update crate and package together.

Installed apps read `releases/latest/download/latest.json` on start and offer the update.

## Windows SmartScreen

The Windows installer is not code-signed. SmartScreen shows "Windows protected your PC". Users
click "More info", then "Run anyway". This is accepted for now (PRD 15). Updates installed by the
in-app updater are verified by the updater signature and do not show this warning.
