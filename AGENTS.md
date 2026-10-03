# Working agreement for this repository

The goal is the product: a tool that reads code by how it behaves (see `docs/spec.md`). The agent builds; the owner decides and reviews. Learning Rust or Neovim is no longer a constraint on technical choices.

## When explaining

- Never refer back to earlier messages ("as above", "step 3 from before"). Chat scrolls away. Every reply repeats in full what the owner needs right now.
- When giving a command the owner must run, explain what it does and what it changes.

## Project

- Spec: `docs/spec.md`. Rulings: `docs/decisions/`. Research: `docs/research/` (start with the consolidated report). Screens: `docs/mocks/`.
- The tool must not assume any particular user's environment (proxy, tier names, model names, ports) in code or defaults.
- Every claim shown to the reader must cite the scenario result it rests on, or be marked as a guess. This is the product's core rule; never relax it for convenience.
