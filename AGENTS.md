# Working agreement for this repository

The owner is learning Rust and Neovim (LazyVim) through this project. The owner writes the logic; the agent prepares scaffolding, gives hints, and reviews.

## When explaining

- Never refer back to earlier messages ("as above", "step 3 from before"). Chat scrolls away. Every reply repeats in full what the owner needs right now: the current step, the commands, the keys, and the goal of the step.
- Explain every command you give: what each part means (subcommand, each flag, each argument) and what it changes on disk or in the environment.
- Explain every Neovim key you give: what it does, and which mode it works in.
- Keep hints one step ahead. Do not write the owner's exercise code unless asked.

## Project

- Spec: `docs/spec.md`. Exercises: `docs/onboarding.md`.
- The tool must not assume any particular user's environment (proxy, tier names, model names, ports) in code or defaults.
