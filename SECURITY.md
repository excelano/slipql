# Security Policy

## Reporting a vulnerability

Please report suspected vulnerabilities privately through GitHub Security Advisories at https://github.com/excelano/slipql/security/advisories/new. If you would rather not use GitHub, email david.anderson@excelano.com instead. I aim to respond within seven days.

Please do not open public issues for security problems.

## Supported versions

The latest release receives security fixes. Older versions are not supported.

## What slipql can access

slipql is a CLI that runs locally on your machine. It lists the directory you name, opens every file there whose extension is `.slpc`, reads the metadata member out of each through the `slpc` library, and closes it. It never decompresses or reads a payload, never writes to any file it scans, makes no network calls, has no auth layer, and implements no administrative operations. It can only read files your operating-system user already has access to.

The metadata member is decompressed before it can be parsed, so a container is a claim on memory before it is known to be one. The `slpc` library bounds that read, and a container over the bound is skipped and reported rather than read. A program embedding the library sets the bound through `Options`.

## What slipql stores

slipql stores nothing outside the files you explicitly redirect output to. The interactive prompt keeps a line-editing history file under your configuration directory (`~/.config/slipql/` on Linux and macOS, `%APPDATA%\slipql\` on Windows); there is no other state, no telemetry, no analytics, and no remote logging.

## Verifying releases

Every GitHub release includes a `.sha256` file next to each archive listing its SHA-256 hash. Verify any download before running it:

    sha256sum slipql-x86_64-unknown-linux-gnu.tar.xz
    # compare against the value in slipql-x86_64-unknown-linux-gnu.tar.xz.sha256

Release artifacts are built by GitHub Actions from a tagged commit using the cargo-dist configuration in this repo (`dist-workspace.toml` and the generated `.github/workflows/release.yml`). The workflow and build configuration are public and auditable.
