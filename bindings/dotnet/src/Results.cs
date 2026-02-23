using System.Text.Json;

namespace JsonSchemaLlm;

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// <summary>Options for schema conversion.</summary>
public sealed record ConvertOptions
{
    public string? Target { get; init; }
    public string? Mode { get; init; }
    public int? MaxDepth { get; init; }
    public int? RecursionLimit { get; init; }
    public string? Polymorphism { get; init; }

    internal Dictionary<string, object> ToDictionary()
    {
        var dict = new Dictionary<string, object>();
        if (Target is not null) dict["target"] = Target;
        if (Mode is not null) dict["mode"] = Mode;
        if (MaxDepth is not null) dict["max-depth"] = MaxDepth.Value;
        if (RecursionLimit is not null) dict["recursion-limit"] = RecursionLimit.Value;
        if (Polymorphism is not null) dict["polymorphism"] = Polymorphism;
        return dict;
    }
}

/// <summary>Options for component extraction.</summary>
public sealed record ExtractOptions
{
    public int? MaxDepth { get; init; }

    internal Dictionary<string, object> ToDictionary()
    {
        var dict = new Dictionary<string, object>();
        if (MaxDepth is not null) dict["max-depth"] = MaxDepth.Value;
        return dict;
    }
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// <summary>Result of a schema conversion operation.</summary>
public sealed record ConvertResult
{
    public required string ApiVersion { get; init; }
    public required JsonElement Schema { get; init; }
    public required JsonElement Codec { get; init; }

    internal static ConvertResult FromJson(JsonElement root) => new()
    {
        ApiVersion = root.GetProperty("apiVersion").GetString()!,
        Schema = root.GetProperty("schema").Clone(),
        Codec = root.GetProperty("codec").Clone(),
    };
}

/// <summary>Warning produced during rehydration.</summary>
public sealed record RehydrateWarning
{
    public required string DataPath { get; init; }
    public required string SchemaPath { get; init; }
    public required string Message { get; init; }
}

/// <summary>Result of a rehydration operation.</summary>
public sealed record RehydrateResult
{
    public required string ApiVersion { get; init; }
    public required JsonElement Data { get; init; }
    public IReadOnlyList<RehydrateWarning> Warnings { get; init; } = [];

    internal static RehydrateResult FromJson(JsonElement root)
    {
        var warnings = new List<RehydrateWarning>();
        if (root.TryGetProperty("warnings", out var warningsEl) &&
            warningsEl.ValueKind == JsonValueKind.Array)
        {
            foreach (var w in warningsEl.EnumerateArray())
            {
                warnings.Add(new RehydrateWarning
                {
                    DataPath = w.TryGetProperty("dataPath", out var dp) ? dp.GetString() ?? "" : "",
                    SchemaPath = w.TryGetProperty("schemaPath", out var sp) ? sp.GetString() ?? "" : "",
                    Message = w.TryGetProperty("message", out var msg) ? msg.GetString() ?? "" : "",
                });
            }
        }

        return new()
        {
            ApiVersion = root.GetProperty("apiVersion").GetString()!,
            Data = root.GetProperty("data").Clone(),
            Warnings = warnings,
        };
    }
}

/// <summary>Result of listing extractable components.</summary>
public sealed record ListComponentsResult
{
    public required string ApiVersion { get; init; }
    public required string[] Components { get; init; }

    internal static ListComponentsResult FromJson(JsonElement root)
    {
        var components = root.GetProperty("components").EnumerateArray()
            .Select(c => c.GetString()!)
            .ToArray();

        return new()
        {
            ApiVersion = root.GetProperty("apiVersion").GetString()!,
            Components = components,
        };
    }
}

/// <summary>Result of extracting a single component.</summary>
public sealed record ExtractResult
{
    public required string ApiVersion { get; init; }
    public required JsonElement Schema { get; init; }
    public required string Pointer { get; init; }
    public required int DependencyCount { get; init; }
    public string[] MissingRefs { get; init; } = [];

    internal static ExtractResult FromJson(JsonElement root)
    {
        var missingRefs = root.TryGetProperty("missingRefs", out var mr) && mr.ValueKind == JsonValueKind.Array
            ? mr.EnumerateArray().Select(r => r.GetString()!).ToArray()
            : [];

        return new()
        {
            ApiVersion = root.GetProperty("apiVersion").GetString()!,
            Schema = root.GetProperty("schema").Clone(),
            Pointer = root.GetProperty("pointer").GetString()!,
            DependencyCount = root.GetProperty("dependencyCount").GetInt32(),
            MissingRefs = missingRefs,
        };
    }
}

/// <summary>Result of converting all components in one call.</summary>
public sealed record ConvertAllResult
{
    public required string ApiVersion { get; init; }
    public required JsonElement Full { get; init; }
    public required JsonElement Components { get; init; }
    public JsonElement? ComponentErrors { get; init; }

    internal static ConvertAllResult FromJson(JsonElement root) => new()
    {
        ApiVersion = root.GetProperty("apiVersion").GetString()!,
        Full = root.GetProperty("full").Clone(),
        Components = root.GetProperty("components").Clone(),
        ComponentErrors = root.TryGetProperty("componentErrors", out var ce) && ce.ValueKind != JsonValueKind.Null
            ? ce.Clone()
            : null,
    };
}
