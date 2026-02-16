FROM python:3.12-slim-bookworm
WORKDIR /app
COPY bindings/python /app/bindings/python/
COPY tests/ ./tests/
WORKDIR /app/bindings/python
RUN pip install --no-cache-dir wasmtime pytest
CMD ["python", "-m", "pytest", "-v"]
