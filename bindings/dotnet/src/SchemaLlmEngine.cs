using System.Text.Json;

namespace JsonSchemaLlm;

/// <summary>
/// High-level facade for json-schema-llm WASI operations.
///
/// <para>
/// Provides the consumer-friendly API: typed results, options records,
/// and resource lifecycle management via <see cref="IDisposable"/>.
/// </para>
///
/// <example>
/// <code>
/// using var engine = SchemaLlmEngine.Create();
/// var result = engine.Convert(schema, new ConvertOptions { Target = "openai-strict" });
/// var rehydrated = engine.Rehydrate(data, result.Codec, schema);
/// </code>
/// </example>
/// </summary>
public sealed class SchemaLlmEngine : IDisposable
{
    private readonly JsonSchemaLlmEngine _inner;

    private SchemaLlmEngine(JsonSchemaLlmEngine inner)
    {
        _inner = inner;
    }

    /// <summary>
    /// Create a new SchemaLlmEngine with automatic WASM discovery.
    ///
    /// <para>Resolution cascade:</para>
    /// <list type="number">
    /// <item>Explicit <paramref name="wasmPath"/></item>
    /// <item><c>JSL_WASM_PATH</c> environment variable</item>
    /// <item>Repo-relative fallback (dev/CI)</item>
    /// </list>
    /// </summary>
    public static SchemaLlmEngine Create(string? wasmPath = null)
    {
        return new SchemaLlmEngine(new JsonSchemaLlmEngine(wasmPath));
    }

    /// <summary>Convert a JSON Schema to LLM-compatible form.</summary>
    public ConvertResult Convert(object schema, ConvertOptions? options = null)
    {
        var raw = options is not null
            ? _inner.Convert(schema, options.ToDictionary())
            : _inner.Convert(schema);
        return ConvertResult.FromJson(raw);
    }

    /// <summary>Rehydrate LLM output back to the original schema shape.</summary>
    public RehydrateResult Rehydrate(object data, object codec, object schema)
    {
        var raw = _inner.Rehydrate(data, codec, schema);
        return RehydrateResult.FromJson(raw);
    }

    /// <summary>List all extractable component JSON Pointers in a schema.</summary>
    public ListComponentsResult ListComponents(object schema)
    {
        var raw = _inner.ListComponents(schema);
        return ListComponentsResult.FromJson(raw);
    }

    /// <summary>Extract a single component from a schema by JSON Pointer.</summary>
    public ExtractResult ExtractComponent(object schema, string pointer, ExtractOptions? options = null)
    {
        var optDict = options?.ToDictionary();
        var raw = _inner.ExtractComponent(schema, pointer, optDict);
        return ExtractResult.FromJson(raw);
    }

    /// <summary>Convert all components in a schema at once.</summary>
    public ConvertAllResult ConvertAllComponents(
        object schema, ConvertOptions? convertOptions = null, ExtractOptions? extractOptions = null)
    {
        var convDict = convertOptions?.ToDictionary();
        var extDict = extractOptions?.ToDictionary();
        var raw = _inner.ConvertAllComponents(schema, convDict, extDict);
        return ConvertAllResult.FromJson(raw);
    }

    /// <summary>Release WASM resources.</summary>
    public void Dispose() => _inner.Dispose();
}
