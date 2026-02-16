using System.Buffers.Binary;
using System.Text;
using System.Text.Json;
using Wasmtime;

namespace JsonSchemaLlm;

/// <summary>
/// WASI-backed wrapper for jsonschema-llm.
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

    private readonly Engine _engine;
    private readonly Module _module;
    private readonly Linker _linker;

    public JsonSchemaLlmEngine(string? wasmPath = null)
    {
        var path = wasmPath
            ?? Environment.GetEnvironmentVariable("JSL_WASM_PATH")
            ?? Path.Combine(
                AppDomain.CurrentDomain.BaseDirectory,
                "..", "..", "..", "..", "..",
                "target", "wasm32-wasip1", "release", "jsonschema_llm_wasi.wasm");

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

    public JsonElement Convert(object schema, object? options = null)
    {
        var schemaJson = JsonSerializer.Serialize(schema);
        var optsJson = JsonSerializer.Serialize(options ?? new { });
        return CallJsl("jsl_convert", schemaJson, optsJson);
    }

    public JsonElement Rehydrate(object data, object codec, object schema)
    {
        var dataJson = JsonSerializer.Serialize(data);
        var codecJson = JsonSerializer.Serialize(codec);
        var schemaJson = JsonSerializer.Serialize(schema);
        return CallJsl("jsl_rehydrate", dataJson, codecJson, schemaJson);
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
        var jslAlloc = instance.GetFunction<int, int>("jsl_alloc")
            ?? throw new InvalidOperationException("No jsl_alloc export");
        var jslFree = instance.GetAction<int, int>("jsl_free")
            ?? throw new InvalidOperationException("No jsl_free export");
        var jslResultFree = instance.GetAction<int>("jsl_result_free")
            ?? throw new InvalidOperationException("No jsl_result_free export");

        // Allocate and write arguments
        var allocs = new List<(int ptr, int len)>();
        var flatArgs = new List<object>();

        foreach (var arg in jsonArgs)
        {
            var bytes = Encoding.UTF8.GetBytes(arg);
            var ptr = jslAlloc(bytes.Length);
            memory.Write(ptr, bytes);
            allocs.Add((ptr, bytes.Length));
            flatArgs.Add(ptr);
            flatArgs.Add(bytes.Length);
        }

        // Call function
        var func = instance.GetFunction(funcName)
            ?? throw new InvalidOperationException($"No {funcName} export");
        var resultPtr = (int)(func.Invoke(flatArgs.ToArray()) ?? throw new InvalidOperationException("null result"));

        // Read JslResult (12 bytes: 3 × LE u32)
        var resultBytes = new byte[JslResultSize];
        memory.Read(resultPtr, resultBytes);
        var status = BinaryPrimitives.ReadInt32LittleEndian(resultBytes.AsSpan(0, 4));
        var payloadPtr = BinaryPrimitives.ReadInt32LittleEndian(resultBytes.AsSpan(4, 4));
        var payloadLen = BinaryPrimitives.ReadInt32LittleEndian(resultBytes.AsSpan(8, 4));

        // Read payload
        var payloadBytes = new byte[payloadLen];
        memory.Read(payloadPtr, payloadBytes);
        var payloadStr = Encoding.UTF8.GetString(payloadBytes);

        // Free
        jslResultFree(resultPtr);
        foreach (var (ptr, len) in allocs) jslFree(ptr, len);

        using var payload = JsonDocument.Parse(payloadStr);

        if (status == StatusError)
        {
            var root = payload.RootElement;
            throw new JslException(
                root.GetString("code") ?? "unknown",
                root.GetString("message") ?? "unknown error",
                root.TryGetProperty("path", out var path) ? path.GetString() ?? "" : "");
        }

        return payload.RootElement.Clone();
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
