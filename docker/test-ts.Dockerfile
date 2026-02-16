FROM node:22-bookworm-slim
RUN corepack enable && corepack prepare pnpm@latest --activate
WORKDIR /app
COPY bindings/ts-wasi/ ./bindings/ts-wasi/
COPY tests/ ./tests/
WORKDIR /app/bindings/ts-wasi
RUN pnpm install --frozen-lockfile
CMD ["pnpm", "test"]
