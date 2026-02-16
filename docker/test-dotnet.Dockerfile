FROM mcr.microsoft.com/dotnet/sdk:8.0-bookworm-slim
WORKDIR /app
COPY bindings/dotnet/ ./bindings/dotnet/
COPY tests/ ./tests/
WORKDIR /app/bindings/dotnet
RUN dotnet restore test/JsonSchemaLlmTests.csproj
CMD ["dotnet", "test", "test/JsonSchemaLlmTests.csproj", "-v", "normal"]
