using System.Buffers.Binary;
using System.Text;
using System.Text.Json;
using System.Runtime.CompilerServices;
using Wasmtime;

[assembly: InternalsVisibleTo("JsonSchemaLlmTests")]

namespace JsonSchemaLlm;

/// <summary>
/// WASI-backed wrapper for json-schema-llm.
///
/// Uses wasmtime-dotnet to load the universal WASI binary and exposes
/// Convert() and Rehydrate() as C# methods.
///
/// Concurrency: Each Engine owns its own Wasmtime Store. NOT thread-safe.
/// </summary>
public sealed class JsonSchemaLlmEngine : IDisposable
{
    private const int JslResultSize = 12; // 3 × u32 (LE)
    private const int StatusOk = 0;
    private const int StatusError = 1;
    private const int ExpectedAbiVersion = 1;

    private readonly Engine _engine;
    private readonly Module _module;
    private readonly Linker _linker;
    private bool _abiVerified;

    public JsonSchemaLlmEngine(string? wasmPath = null)
    {
        var path = wasmPath
            ?? Environment.GetEnvironmentVariable("JSL_WASM_PATH")
            ?? Path.Combine(
                AppDomain.CurrentDomain.BaseDirectory,
                "..", "..", "..", "..", "..",
                "target", "wasm32-wasip1", "release", "json_schema_llm_wasi.wasm");

        _engine = new Engine();
        _module = Module.FromFile(_engine, path);
        _linker = new Linker(_engine);
        _linker.DefineWasi();
    }

    public void Dispose()
    {
        _module.Dispose();
        _engine.Dispose();
    }

    private static readonly JsonSerializerOptions KebabCaseOptions = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.KebabCaseLower,
        DictionaryKeyPolicy = JsonNamingPolicy.KebabCaseLower,
    };

    public JsonElement Convert(object schema, object? options = null)
    {
        var schemaJson = JsonSerializer.Serialize(schema);
        // Normalize PascalCase/camelCase option keys to kebab-case for WASI binary
        var optsJson = options != null
            ? JsonSerializer.Serialize(options, KebabCaseOptions)
            : "{}";
        return CallJsl("jsl_convert", schemaJson, optsJson);
    }

    public JsonElement Rehydrate(object data, object codec, object schema)
    {
        var dataJson = JsonSerializer.Serialize(data);
        var codecJson = JsonSerializer.Serialize(codec);
        var schemaJson = JsonSerializer.Serialize(schema);
        return CallJsl("jsl_rehydrate", dataJson, codecJson, schemaJson);
    }

    public JsonElement ListComponents(object schema)
    {
        var schemaJson = JsonSerializer.Serialize(schema);
        return CallJsl("jsl_list_components", schemaJson);
    }

    public JsonElement ExtractComponent(object schema, string pointer, object? options = null)
    {
        var schemaJson = JsonSerializer.Serialize(schema);
        var optsJson = options != null
            ? JsonSerializer.Serialize(options, KebabCaseOptions)
            : "{}";
        return CallJsl("jsl_extract_component", schemaJson, pointer, optsJson);
    }

    public JsonElement ConvertAllComponents(object schema, object? convertOptions = null,
        object? extractOptions = null)
    {
        var schemaJson = JsonSerializer.Serialize(schema);
        var convOptsJson = convertOptions != null
            ? JsonSerializer.Serialize(convertOptions, KebabCaseOptions)
            : "{}";
        var extOptsJson = extractOptions != null
            ? JsonSerializer.Serialize(extractOptions, KebabCaseOptions)
            : "{}";
        return CallJsl("jsl_convert_all_components", schemaJson, convOptsJson, extOptsJson);
    }

    internal JsonElement CallJsl(string funcName, params string[] jsonArgs)
    {
        // Fresh store per call
        var config = new WasiConfiguration().WithInheritedStandardOutput().WithInheritedStandardError();
        using var store = new Store(_engine);
        store.SetWasiConfiguration(config);
        var instance = _linker.Instantiate(store, _module);

        var memory = instance.GetMemory("memory")
            ?? throw new InvalidOperationException("No memory export");

        // ABI version handshake (once per engine lifetime)
        if (!_abiVerified)
        {
            var abiFn = instance.GetFunction<int>("jsl_abi_version")
                ?? throw new InvalidOperationException(
                    "Incompatible WASM module: missing required 'jsl_abi_version' export");
            var version = abiFn();
            if (version != ExpectedAbiVersion)
                throw new InvalidOperationException(
                    $"ABI version mismatch: binary={version}, expected={ExpectedAbiVersion}");
            _abiVerified = true;
        }

        var jslAlloc = instance.GetFunction<int, int>("jsl_alloc")
            ?? throw new InvalidOperationException("No jsl_alloc export");
        var jslFree = instance.GetAction<int, int>("jsl_free")
            ?? throw new InvalidOperationException("No jsl_free export");
        var jslResultFree = instance.GetAction<int>("jsl_result_free")
            ?? throw new InvalidOperationException("No jsl_result_free export");

        // Allocate and write arguments
        var allocs = new List<(int ptr, int len)>();
        var flatArgs = new List<ValueBox>();
        var resultPtr = 0;

        try
        {
            foreach (var arg in jsonArgs)
            {
                var bytes = Encoding.UTF8.GetBytes(arg);
                var ptr = jslAlloc(bytes.Length);
                if (ptr == 0 && bytes.Length > 0)
                    throw new InvalidOperationException($"jsl_alloc returned null for {bytes.Length} bytes");
                bytes.CopyTo(memory.GetSpan(ptr, bytes.Length));
                allocs.Add((ptr, bytes.Length));
                flatArgs.Add(ptr);
                flatArgs.Add(bytes.Length);
            }

            // Call function
            var func = instance.GetFunction(funcName)
                ?? throw new InvalidOperationException($"No {funcName} export");
            resultPtr = (int)(func.Invoke(flatArgs.ToArray()) ?? throw new InvalidOperationException("null result"));
            if (resultPtr == 0)
                throw new InvalidOperationException($"{funcName} returned null result pointer");

            // Read JslResult (12 bytes: 3 × LE u32)
            var resultBytes = memory.GetSpan(resultPtr, JslResultSize).ToArray();
            var status = BinaryPrimitives.ReadInt32LittleEndian(resultBytes.AsSpan(0, 4));
            var payloadPtr = BinaryPrimitives.ReadInt32LittleEndian(resultBytes.AsSpan(4, 4));
            var payloadLen = BinaryPrimitives.ReadInt32LittleEndian(resultBytes.AsSpan(8, 4));

            // Read payload
            var payloadStr = Encoding.UTF8.GetString(memory.GetSpan(payloadPtr, payloadLen));

            using var payload = JsonDocument.Parse(payloadStr);

            if (status == StatusError)
            {
                var root = payload.RootElement;
                throw new JslException(
                    root.GetProperty("code").GetString() ?? "unknown",
                    root.GetProperty("message").GetString() ?? "unknown error",
                    root.TryGetProperty("path", out var path) ? path.GetString() ?? "" : "");
            }

            return payload.RootElement.Clone();
        }
        finally
        {
            // Always free guest memory, even on exception
            if (resultPtr != 0) jslResultFree(resultPtr);
            foreach (var (ptr, len) in allocs) jslFree(ptr, len);
        }
    }
}

public class JslException : Exception
{
    public string Code { get; }
    public string Path { get; }

    public JslException(string code, string message, string path = "")
        : base($"jsl error [{code}]{(string.IsNullOrEmpty(path) ? "" : $" at {path}")}: {message}")
    {
        Code = code;
        Path = path;
    }
}
