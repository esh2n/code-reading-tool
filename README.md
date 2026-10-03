# crt — read code by how it behaves

`crt` puts a short explanation at the end of every line of a function, in
Neovim and VS Code, and can walk a function through concrete inputs: a normal
case, boundary values, and two calls at once.

Every explanation says what it rests on:

- **fact** (●) — read off the syntax tree: "this line calls `add`".
- **guess** (◌) — the model's reasoning about behaviour, with the assumptions
  it makes. Nothing is executed; read guesses as guesses.

A note that claims a fact the code does not have is demoted to a guess before
you see it.

## How it works

One Rust binary, `crt`, does the work and serves editors over LSP:

1. Opening a file shows the structural facts at once (tree-sitter, bundled
   grammars).
2. Functions in view that have no cached explanation are sent to an
   OpenAI-compatible model with a strict JSON Schema, one function at a time.
3. Results are checked against the facts, cached by the function's content
   hash, and pushed to the editor. Editing a function invalidates only its
   explanation.

Languages bundled today: Rust, Go, Python, JavaScript.

## Configure a model

`crt` has no default endpoint or model. Create `config.toml` in the platform
configuration directory — `~/.config/crt/` on Linux,
`~/Library/Application Support/crt/` on macOS — or pass `--config`. When the
file is missing, `crt read` prints the exact path it looked at.

```toml
[llm]
base_url = "https://your-endpoint.example/v1"   # any OpenAI-compatible endpoint
model = "your-model"
api_key_env = "YOUR_API_KEY_VARIABLE"           # the variable's name; omit for keyless endpoints
output_language = "English"                     # e.g. "Japanese"

[[llm.fallback]]                                # optional, tried when the above is down
base_url = "https://another.example/v1"
model = "another-model"
```

The endpoint must support structured output (`response_format` with a JSON
Schema); `crt` refuses to fall back to free-form text.

## Use

### CLI

```sh
crt analyze path/to/file.go              # structural facts as JSON, no model
crt read path/to/file.go --func Copy     # explain one function (or --line N)
crt cached path/to/file.go               # what is already explained
crt render path/to/file.go --out p.html  # one HTML page; --read-missing explains the rest
crt languages
```

### Neovim (0.11+)

```lua
-- lazy.nvim
{
  "esh2n/code-reading-tool",
  config = function(plugin)
    vim.opt.rtp:append(plugin.dir .. "/editors/nvim")
    require("code-reading").setup({})
  end,
}
```

Run `:CrInstall` once to download `crt` for your platform (checked against
the release's SHA-256), or put `crt` on your `PATH`.

| Command / key | What it does |
|---|---|
| `K` (hover) | the full explanation, its assumptions, and the facts on that line |
| `:CrRead[!]` | explain the function under the cursor now (`!` asks the model again) |
| `:CrScenario` | pick a scenario and see its steps beside the code; `<CR>` jumps to a step |
| `:CrToggle` | hide or show the annotations |
| diagnostics | concurrency scenarios, with the lines that interleave as related locations |

### VS Code

Install the `.vsix` for your platform from a release (it contains `crt`).
Commands: *Code Reading: Explain Function at Cursor*, *Show Scenario*,
*Toggle Annotations*. Settings: `codeReading.configPath`, `codeReading.cacheDir`,
`codeReading.autoRead`, `codeReading.maxParallel`, `codeReading.serverPath`.

## Develop

```sh
cargo test --workspace            # includes LSP and headless Neovim end-to-end tests
scripts/check-layers.sh           # dependency direction between crates
cd editors/vscode && pnpm install && pnpm test:unit && pnpm test:e2e
```

Design: [`docs/spec.md`](docs/spec.md), [`docs/architecture.md`](docs/architecture.md),
rulings in [`docs/decisions/`](docs/decisions/), evidence in
[`docs/research/`](docs/research/).
