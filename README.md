# evoeditor

English | [日本語](README.ja.md)

A text editor written in Rust for the terminal. Its first goal is comfortable editing of Japanese text, with files saved safely in their original encoding and line endings.

> **The latest release is v0.2.0 (2026-11-04).** Downloads are on [GitHub Releases](https://github.com/evoref/evoeditor/releases): a zip for Windows 11 (x86_64) and static `tar.gz` builds for Linux (x86_64 / aarch64). Installation, updating, signature verification and known limitations are described in the bundled README ([dist/README.en.md](dist/README.en.md); Japanese: [dist/README.md](dist/README.md)).

## Status

- **v0.1 (minimal core)** was released as v0.1.0 on 2026-10-27. It runs in Windows Terminal and the console host. It opens a text file, searches and replaces, and saves safely in the original encoding, BOM and line endings. Besides UTF-8 and UTF-16, it handles Shift_JIS, EUC-JP and ISO-2022-JP.
- **v0.2** was released as v0.2.0 on 2026-11-04. It added:
  - Linux support
  - an English user interface
  - automatic saving of unsaved changes (swap) and recovery
  - multiple documents
  - regular-expression search and replace
  - wrapping and themes
  - a help screen

  macOS is not supported yet.
- **v0.3** is in development. It reorganizes and speeds up the internals without changing behavior, and fixes data-safety and correctness bugs ([docs/08-roadmap.md](docs/08-roadmap.md) §2.4).

Changes are listed in [CHANGELOG.en.md](CHANGELOG.en.md) (Japanese: [CHANGELOG.md](CHANGELOG.md)).

## Verifying downloads

Each release has a `SHA256SUMS` file signed with minisign (`SHA256SUMS.minisig`). The public key is below. It is the same as [minisign.pub](minisign.pub) in this repository; compare the two before you trust it.

```text
untrusted comment: minisign public key 39CA49EFC7BC926D
RWRtkrzH70nKORpBFPEtLy6elo0xP5XsEli9SU/OaxnBmNM8o14GKMhv
```

```sh
minisign -Vm SHA256SUMS -P RWRtkrzH70nKORpBFPEtLy6elo0xP5XsEli9SU/OaxnBmNM8o14GKMhv
```

The full steps for each OS are in the bundled README.

## Specification

The specification is in [docs/](docs/README.md) and is written in Japanese.

**It is published as a work in progress.** It may change, and items marked "提案中" (proposed) are not yet decided.

The specification of a released version is the `docs/` of that version's tag (or of `main`).

- Entry point: [docs/README.md](docs/README.md)
- Decision records (ADR) and open questions (OQ): [docs/10-decisions.md](docs/10-decisions.md)
- Roadmap: [docs/08-roadmap.md](docs/08-roadmap.md)

## Branches

- `main`: releases only. Only released versions are merged here.
- `develop`: the main line of development. Changes come in from work branches through pull requests.

See §9.1 of [docs/11-dev-workflow.md](docs/11-dev-workflow.md) for details.

## Contributing

**Pull requests from outside the project are not accepted yet.** Bug reports and feedback are welcome in Issues.

Do not report vulnerabilities in public issues; use the contact in [SECURITY.md](SECURITY.md).

The contribution policy is in [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[Apache License 2.0](LICENSE). See [NOTICE](NOTICE) for attributions.

---

The Japanese version ([README.ja.md](README.ja.md)) is authoritative. If the two differ, the Japanese version applies.
