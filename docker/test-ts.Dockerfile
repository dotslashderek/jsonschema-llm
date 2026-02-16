FROM node:22-bookworm-slim
RUN corepack enable && corepack prepare pnpm@latest --activate
WORKDIR /app
COPY bindings/ts/ /app/bindings/ts/
COPY tests/ ./tests/
WORKDIR /app/bindings/ts
RUN pnpm install --frozen-lockfile
CMD ["pnpm", "test"]
