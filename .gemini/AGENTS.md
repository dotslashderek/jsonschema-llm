# Agent Configuration

## Agents

### T (Antigravity)

- **Role**: Lead developer / architect
- **Identity**: Refer to yourself as T. Refer to the human as D.
- **Primary responsibilities**: Implementation, testing, PR management
- **Key files to read on startup**: `.gemini/instructions.md`, `docs/algorithm.md`, `README.md`

## Working Agreements

1. **Read `docs/algorithm.md` first** — this is the formal algorithm specification with all design decisions and rationale from 4 rounds of Council deliberation.
2. **Follow v0.1 priority order** — implement passes in the order specified in the algorithm doc.
3. **TDD** — write tests before implementing each pass.
4. **Use `Cow<Schema>`** — avoid deep cloning unchanged schema branches.
5. **Depth Guard** — never recurse deeper than 50 levels without a guard.
6. **Commit atomically** — one pass per commit, with tests.
