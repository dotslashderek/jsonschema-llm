FROM golang:1.22-bookworm
WORKDIR /app
COPY bindings/go/ ./bindings/go/
COPY tests/ ./tests/
WORKDIR /app/bindings/go
RUN go mod download
CMD ["go", "test", "-v", "./..."]
