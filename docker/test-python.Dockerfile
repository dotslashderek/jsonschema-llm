FROM python:3.12-slim-bookworm
WORKDIR /app
COPY bindings/python-wasi/ ./bindings/python-wasi/
COPY tests/ ./tests/
WORKDIR /app/bindings/python-wasi
RUN pip install --no-cache-dir wasmtime pytest
CMD ["python", "-m", "pytest", "-v"]
