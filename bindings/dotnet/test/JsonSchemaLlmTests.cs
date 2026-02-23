using System.Text.Json;
using JsonSchemaLlm;
using Xunit;

namespace JsonSchemaLlm.Tests;

public class JsonSchemaLlmTests : IDisposable
{
    private readonly SchemaLlmEngine _engine;

    public JsonSchemaLlmTests()
    {
        _engine = SchemaLlmEngine.Create();
    }

    public void Dispose() => _engine.Dispose();

    [Fact]
    public void ConvertSimple()
    {
        var schema = new Dictionary<string, object>
        {
            ["type"] = "object",
            ["properties"] = new Dictionary<string, object>
            {
                ["name"] = new Dictionary<string, object> { ["type"] = "string" },
                ["age"] = new Dictionary<string, object> { ["type"] = "integer", ["minimum"] = 0 }
            },
            ["required"] = new[] { "name", "age" }
        };

        var result = _engine.Convert(schema);
        Assert.NotEmpty(result.ApiVersion);
        Assert.NotEqual(JsonValueKind.Undefined, result.Schema.ValueKind);
        Assert.NotEqual(JsonValueKind.Undefined, result.Codec.ValueKind);
    }

    [Fact]
    public void ConvertWithOptions()
    {
        var schema = new Dictionary<string, object>
        {
            ["type"] = "object",
            ["properties"] = new Dictionary<string, object>
            {
                ["name"] = new Dictionary<string, object> { ["type"] = "string" }
            }
        };

        var result = _engine.Convert(schema, new ConvertOptions { Target = "openai-strict" });
        Assert.NotEmpty(result.ApiVersion);
        Assert.NotEqual(JsonValueKind.Undefined, result.Schema.ValueKind);
    }

    [Fact]
    public void ConvertError()
    {
        // Use internal raw engine for FFI error path testing
        using var raw = new JsonSchemaLlmEngine();
        var ex = Assert.Throws<JslException>(() =>
            raw.CallJsl("jsl_convert", "NOT VALID JSON", "{}"));
        Assert.NotEmpty(ex.Code);
    }

    [Fact]
    public void Roundtrip()
    {
        var schema = new Dictionary<string, object>
        {
            ["type"] = "object",
            ["properties"] = new Dictionary<string, object>
            {
                ["name"] = new Dictionary<string, object> { ["type"] = "string" },
                ["age"] = new Dictionary<string, object> { ["type"] = "integer", ["minimum"] = 0 }
            },
            ["required"] = new[] { "name", "age" }
        };

        var convertResult = _engine.Convert(schema);

        var data = new Dictionary<string, object> { ["name"] = "Ada", ["age"] = 36 };
        var rehydrated = _engine.Rehydrate(data, convertResult.Codec, schema);

        Assert.NotEmpty(rehydrated.ApiVersion);
        Assert.Equal("Ada", rehydrated.Data.GetProperty("name").GetString());
    }

    [Fact]
    public void RehydrateError()
    {
        using var raw = new JsonSchemaLlmEngine();
        Assert.Throws<JslException>(() =>
            raw.CallJsl("jsl_rehydrate",
                "{\"key\":\"value\"}", "NOT VALID JSON", "{\"type\":\"object\"}"));
    }

    [Fact]
    public void MultipleCalls()
    {
        var schema = new Dictionary<string, object>
        {
            ["type"] = "object",
            ["properties"] = new Dictionary<string, object>
            {
                ["x"] = new Dictionary<string, object> { ["type"] = "number" }
            }
        };

        for (var i = 0; i < 5; i++)
        {
            var result = _engine.Convert(schema);
            Assert.NotEqual(JsonValueKind.Undefined, result.Schema.ValueKind);
        }
    }

    [Fact]
    public void ListComponents()
    {
        var schema = new Dictionary<string, object>
        {
            ["$defs"] = new Dictionary<string, object>
            {
                ["Pet"] = new Dictionary<string, object> { ["type"] = "string" },
                ["Tag"] = new Dictionary<string, object> { ["type"] = "integer" }
            }
        };

        var result = _engine.ListComponents(schema);
        Assert.NotEmpty(result.ApiVersion);
        Assert.Equal(2, result.Components.Length);
    }

    [Fact]
    public void ExtractComponent()
    {
        var schema = new Dictionary<string, object>
        {
            ["$defs"] = new Dictionary<string, object>
            {
                ["Pet"] = new Dictionary<string, object>
                {
                    ["type"] = "object",
                    ["properties"] = new Dictionary<string, object>
                    {
                        ["name"] = new Dictionary<string, object> { ["type"] = "string" }
                    }
                }
            }
        };

        var result = _engine.ExtractComponent(schema, "#/$defs/Pet");
        Assert.NotEmpty(result.ApiVersion);
        Assert.Equal("#/$defs/Pet", result.Pointer);
        Assert.NotEqual(JsonValueKind.Undefined, result.Schema.ValueKind);
    }

    [Fact]
    public void ConvertAllComponents()
    {
        var schema = new Dictionary<string, object>
        {
            ["$defs"] = new Dictionary<string, object>
            {
                ["A"] = new Dictionary<string, object> { ["type"] = "string" },
                ["B"] = new Dictionary<string, object> { ["type"] = "integer" }
            }
        };

        var result = _engine.ConvertAllComponents(schema);
        Assert.NotEmpty(result.ApiVersion);
        Assert.NotEqual(JsonValueKind.Undefined, result.Full.ValueKind);
        Assert.NotEqual(JsonValueKind.Undefined, result.Components.ValueKind);
    }
}
