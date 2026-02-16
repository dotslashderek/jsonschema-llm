using System.Text.Json;
using JsonSchemaLlm;
using Xunit;

namespace JsonSchemaLlm.Tests;

public class JsonSchemaLlmTests : IDisposable
{
    private readonly JsonSchemaLlmEngine _engine;

    public JsonSchemaLlmTests()
    {
        _engine = new JsonSchemaLlmEngine();
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
        Assert.True(result.TryGetProperty("apiVersion", out _));
        Assert.True(result.TryGetProperty("schema", out _));
        Assert.True(result.TryGetProperty("codec", out _));
    }

    [Fact]
    public void ConvertError()
    {
        var ex = Assert.Throws<JslException>(() =>
            _engine.CallJsl("jsl_convert", "NOT VALID JSON", "{}"));
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
        var codec = convertResult.GetProperty("codec");

        var data = new Dictionary<string, object> { ["name"] = "Ada", ["age"] = 36 };
        var rehydrated = _engine.Rehydrate(data, codec, schema);

        Assert.True(rehydrated.TryGetProperty("apiVersion", out _));
        Assert.Equal("Ada", rehydrated.GetProperty("data").GetProperty("name").GetString());
    }

    [Fact]
    public void RehydrateError()
    {
        Assert.Throws<JslException>(() =>
            _engine.CallJsl("jsl_rehydrate",
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
            Assert.True(result.TryGetProperty("schema", out _));
        }
    }
}
