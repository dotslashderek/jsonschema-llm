FROM eclipse-temurin:22-jdk-jammy
WORKDIR /app
COPY bindings/java /app/bindings/java/
COPY tests/ ./tests/
WORKDIR /app/bindings/java
RUN ./gradlew --no-daemon dependencies 2>/dev/null || true
CMD ["./gradlew", "--no-daemon", "test"]
