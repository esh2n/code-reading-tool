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

Languages bundled today: Rust, Go, Python, JavaScript, TypeScript (and TSX),
Java, C, C++, C#, Ruby, PHP. Adding one is a row in
`crates/crt-treesitter/src/grammars.rs` plus, where the grammar's own tags
query misses functions or calls, a query file in
`crates/crt-treesitter/queries/<language>/`.

## Configure a model

`crt` has no default endpoint or model. Its configuration is one file shared
by the CLI and every editor: `config.toml` in the platform configuration
directory (`~/.config/crt/` on Linux, `~/Library/Application Support/crt/` on
macOS), or the file given with `--config`. `crt config`, `:CrConfig` in Neovim
and *Code Reading: Open Configuration* in VS Code print or open it, writing a
commented example first when there is none. Editors pick up changes on the
next read, without a restart. Editor-specific choices (whether explanations
are on, how many functions to read at once) are editor settings instead.

```toml
[llm]
base_url = "https://your-endpoint.example/v1"   # any OpenAI-compatible endpoint
model = "your-model"
api_key_command = ["security", "find-generic-password", "-s", "your-service", "-w"]
output_language = "English"                     # e.g. "Japanese"

[[llm.fallback]]                                # optional, tried when the above is down
base_url = "https://another.example/v1"
model = "another-model"
```

The API key never goes in the file. Choose one source, or none for an
endpoint without keys:

| Setting | Where the key comes from |
|---|---|
| `api_key_env = "NAME"` | an environment variable. Only programs started from a shell that sets it see it; an editor started from the Dock or a launcher does not. |
| `api_key_command = ["program", "arg", ...]` | the first line a command prints. It runs once per server, the first time a model is called, and may take up to 60 s (time to answer a password manager's prompt). If it fails, it is not run again until the configuration changes; what it printed never appears in messages or logs. |

Any tool that prints a secret works with `api_key_command`, for example
`["security", "find-generic-password", "-s", "NAME", "-w"]` (macOS Keychain),
`["secret-tool", "lookup", "service", "NAME"]` (Linux Secret Service),
`["op", "read", "op://Vault/Item/credential"]` (1Password),
`["bw", "get", "password", "NAME"]` (Bitwarden) or `["pass", "show", "NAME"]`.

Endpoint-specific request options go in `[llm.extra_body]`; they are merged
into every request (never over `model`, `messages` or `response_format`) and
are part of the cache key. For example, to turn off a Qwen3 model's thinking
mode on a server that honours chat-template arguments:

```toml
[llm.extra_body]
chat_template_kwargs = { enable_thinking = false }
```

The endpoint must support structured output (`response_format` with a JSON
Schema); `crt` refuses to fall back to free-form text.

## Use

### CLI

```sh
crt analyze path/to/file.go              # structural facts as JSON, no model
crt read path/to/file.go --func Copy     # per-line notes for one function (or --line N);
                                         # --progress prints each note as it arrives
crt scenarios path/to/file.go --func Copy  # normal, boundary and concurrent walk-throughs
crt cached path/to/file.go               # what is already explained
crt render path/to/file.go --out p.html  # one HTML page; --read-missing writes what is missing
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
| `:CrScenario[!]` | pick a scenario and see its steps beside the code; `<CR>` jumps to a step. The first use for a function asks the model to write them (`!` writes them again) |
| `:CrToggle` | turn explanations off or on. Off: nothing at line ends and no model calls unless you ask (`:CrRead`, `:CrScenario`). Start off with `enabled = false` in `setup()` |
| `:CrConfig` | open the shared configuration file |
| diagnostics | concurrency scenarios, with the lines that interleave as related locations |

Notes appear one by one while the model writes them. Scenarios are written
only when asked for, so opening a file costs one short request per function.

A local model that serves one request at a time gains nothing from parallel
reads: pass `max_parallel = 1` to `setup()`.

For a statusline, `require("code-reading").status()` returns `crt on`,
`crt off` or `crt …` (writing). With lualine, clicking it can switch:
`{ require("code-reading").status, on_click = function() require("code-reading").toggle() end }`.

### VS Code

Install the `.vsix` for your platform from a release (it contains `crt`).
The status bar shows whether explanations are on; click it to switch. Off
means nothing at line ends and no model calls unless you ask. The choice is
kept in the `codeReading.enabled` setting. Commands: *Code Reading: Explain
Function at Cursor*, *Show Scenario*, *Open Configuration*, *Turn
Explanations On or Off*. Settings: `codeReading.enabled`,
`codeReading.configPath`, `codeReading.cacheDir`, `codeReading.autoRead`,
`codeReading.maxParallel`, `codeReading.serverPath`.

## Develop

```sh
cargo test --workspace            # includes LSP and headless Neovim end-to-end tests
scripts/check-layers.sh           # dependency direction between crates
cd editors/vscode && pnpm install && pnpm test:unit && pnpm test:e2e
```

Design: [`docs/spec.md`](docs/spec.md), [`docs/architecture.md`](docs/architecture.md),
rulings in [`docs/decisions/`](docs/decisions/), evidence in
[`docs/research/`](docs/research/).
