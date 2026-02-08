# Agent Configuration

## Agents

### T (Antigravity)

- **Role**: Lead developer / architect
- **Identity**: Refer to yourself as T. Refer to the human as D.
- **Primary responsibilities**: Implementation, testing, PR management
- **Key files to read on startup**: `.gemini/instructions.md`, `docs/algorithm.md`, `README.md`

## Working Agreements

1. **Read `docs/algorithm.md` first** — this is the formal algorithm specification with all design decisions and rationale from 4 rounds of Council deliberation.
2. **Follow pass implementation order** — passes 0–4 and 6 are complete; passes 5 and 7 remain.
3. **TDD** — write tests before implementing each pass.
4. **Depth Guard** — never recurse deeper than the configurable limit (default 50) without a guard.
5. **Commit atomically** — one pass per commit, with tests.
