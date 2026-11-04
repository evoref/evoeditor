# evoeditor

[日本語](README.md) | English

A text editor written in Rust. Its first goal is comfortable editing of Japanese text in Windows terminals (Windows Terminal / the console host).

> **v0.1.0 (the minimal core) has been released.** The distribution is a zip for Windows 11 (x86_64) on [GitHub Releases](https://github.com/evoref/evoeditor/releases). Installation, updating, signature verification and known limitations are described in the bundled README (`dist/README.md`; the English version is `dist/README.en.md`).

## Status

- **v0.1 (minimal core)** was released as v0.1.0 on 2026-10-27. It supports Windows only and has a Japanese user interface. It opens one text file, searches and replaces, and saves safely in the original encoding and line endings. Besides UTF-8, it handles Shift_JIS, EUC-JP and ISO-2022-JP. Changes are listed in [CHANGELOG.en.md](CHANGELOG.en.md).
- The next version (v0.2) is in preparation. macOS / Linux support and an English user interface are planned for v0.2.

## Specification

The specification is in [docs/](docs/README.md) (in Japanese). **It is published as a work in progress.** It may change, and items marked "提案中" (proposed) are not yet decided. The specification of a released version is the `docs/` of that version's tag (or of `main`).

- Entry point: [docs/README.md](docs/README.md)
- Decision records (ADR) and open questions (OQ): [docs/10-decisions.md](docs/10-decisions.md)
- Roadmap: [docs/08-roadmap.md](docs/08-roadmap.md)

`main` holds only the specification of released versions. The version under development is in the `docs/` of `develop`.

## Branches

- `main`: releases only. Only released versions are merged here.
- `develop`: the main line of development. Changes come in from work branches through pull requests.

See §9.1 of [docs/11-dev-workflow.md](docs/11-dev-workflow.md) for details.

## Contributing

**Pull requests from outside the project are not accepted yet.** Bug reports and feedback are welcome in Issues. Do not report vulnerabilities in public issues; use the contact in [SECURITY.md](SECURITY.md). The contribution policy is in [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[Apache License 2.0](LICENSE). See [NOTICE](NOTICE) for attributions.

---

The Japanese version ([README.md](README.md)) is authoritative. If the two differ, the Japanese version applies.
