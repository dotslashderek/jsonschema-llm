FROM eclipse-temurin:22-jdk-jammy
WORKDIR /app
COPY bindings/java-wasi/ ./bindings/java-wasi/
COPY tests/ ./tests/
WORKDIR /app/bindings/java-wasi
RUN ./gradlew --no-daemon dependencies 2>/dev/null || true
CMD ["./gradlew", "--no-daemon", "test"]
