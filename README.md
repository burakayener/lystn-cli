# lystn-cli

Listen to your AI coding assistant. **Lystn** speaks the replies from **Claude
Code** and **Codex** aloud — short spoken summaries so you can keep moving
instead of reading walls of text.

## Install

```bash
npm install -g lystn-cli
lystn wire        # wires Claude Code + Codex
lystn login       # sign in (Google)
```

That's it — no Python, no compiler. The install downloads a small native binary
for your OS (macOS, Windows, Linux).

## Commands

- `lystn wire` / `lystn unwire` — wire (or remove) the hook for Claude Code and Codex
- `lystn login` / `lystn logout` — sign in / clear your key
- `lystn config show` / `lystn config set <key> <value>`
- `lystn mute` / `lystn unmute` · `lystn speed <0.5–3.0>` · `lystn volume <0–100>`

Learn more: https://lystn.space

---

## For maintainers — how this package works

This npm package is a thin launcher. On install, `scripts/postinstall.js`
downloads the matching prebuilt **Rust** binary from this repo's GitHub Release
(`lystn-<target>`), and `bin/lystn.js` runs it.

**Repo layout** (this is the public `lystn-cli` repo):

```
lystn-cli/
├── cli-rs/                     # Rust source (the actual client)
├── bin/lystn.js                # launcher → runs the downloaded binary
├── scripts/postinstall.js      # downloads the binary for this OS
├── package.json
└── .github/workflows/release.yml   # builds + uploads the binaries on a tag
```

**Cutting a release**

1. Bump `version` in `package.json` (e.g. `0.3.0`).
2. `git tag v0.3.0 && git push --tags` → CI builds all 4 binaries and attaches
   them to the `v0.3.0` Release.
3. `npm publish` (the postinstall pulls from the `v0.3.0` Release).
