FROM node:22-bookworm-slim
WORKDIR /app
COPY bindings/ts-wasi/ ./bindings/ts-wasi/
COPY tests/ ./tests/
WORKDIR /app/bindings/ts-wasi
RUN npm install
CMD ["npx", "jest", "--verbose"]
