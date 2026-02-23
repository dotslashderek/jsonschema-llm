using System.Text.Json;
using System.Text.Json.Nodes;
using JsonSchemaLlm;
using Xunit;

namespace JsonSchemaLlm.Tests;

/// <summary>
/// Conformance fixture tests for the WASI-backed jsonschema-llm wrapper.
/// Loads fixtures from tests/conformance/fixtures.json and runs each fixture
/// through the appropriate engine method, asserting expected outcomes.
/// </summary>
public class ConformanceTests : IDisposable
{
    private static readonly string FixturesPath = Path.Combine(
        AppDomain.CurrentDomain.BaseDirectory,
        "..", "..", "..", "..", "..", "..", "tests", "conformance", "fixtures.json");

    private readonly SchemaLlmEngine _engine;
    private static readonly JsonDocument Fixtures;

    static ConformanceTests()
    {
        var json = File.ReadAllText(FixturesPath);
        Fixtures = JsonDocument.Parse(json);
    }

    public ConformanceTests()
    {
        _engine = new SchemaLlmEngine();
    }

    public void Dispose() => _engine.Dispose();

    // -----------------------------------------------------------------------
    // Fixture data sources
    // -----------------------------------------------------------------------

    public static IEnumerable<object[]> ConvertFixtureIds()
    {
        foreach (var fx in Fixtures.RootElement
                     .GetProperty("suites")
                     .GetProperty("convert")
                     .GetProperty("fixtures")
                     .EnumerateArray())
        {
            yield return new object[] { fx.GetProperty("id").GetString()! };
        }
    }

    public static IEnumerable<object[]> RoundtripFixtureIds()
    {
        foreach (var fx in Fixtures.RootElement
                     .GetProperty("suites")
                     .GetProperty("roundtrip")
                     .GetProperty("fixtures")
                     .EnumerateArray())
        {
            yield return new object[] { fx.GetProperty("id").GetString()! };
        }
    }

    public static IEnumerable<object[]> RehydrateErrorFixtureIds()
    {
        foreach (var fx in Fixtures.RootElement
                     .GetProperty("suites")
                     .GetProperty("rehydrate_error")
                     .GetProperty("fixtures")
                     .EnumerateArray())
        {
            yield return new object[] { fx.GetProperty("id").GetString()! };
        }
    }

    private static JsonElement GetFixture(string suite, string fixtureId)
    {
        foreach (var fx in Fixtures.RootElement
                     .GetProperty("suites")
                     .GetProperty(suite)
                     .GetProperty("fixtures")
                     .EnumerateArray())
        {
            if (fx.GetProperty("id").GetString() == fixtureId)
                return fx;
        }
        throw new ArgumentException($"Fixture not found: {fixtureId}");
    }

    // -----------------------------------------------------------------------
    // Convert suite
    // -----------------------------------------------------------------------

    [Theory]
    [MemberData(nameof(ConvertFixtureIds))]
    public void ConformanceConvert(string fixtureId)
    {
        var fx = GetFixture("convert", fixtureId);
        var input = fx.GetProperty("input");
        var expected = fx.GetProperty("expected");

        // Error case: schema_raw → raw FFI
        if (input.TryGetProperty("schema_raw", out var schemaRaw))
        {
            Assert.True(expected.GetProperty("is_error").GetBoolean());

            var rawOptsJson = input.TryGetProperty("options", out var opts)
                ? opts.GetRawText()
                : "{}";

            var ex = Assert.Throws<JslException>(() =>
                _engine.CallJsl("jsl_convert", schemaRaw.GetString()!, rawOptsJson));

            if (expected.TryGetProperty("error_has_keys", out var errorKeys))
            {
                foreach (var key in errorKeys.EnumerateArray())
                {
                    var k = key.GetString()!;
                    if (k == "code") Assert.NotEmpty(ex.Code);
                    if (k == "message") Assert.NotEmpty(ex.Message);
                }
            }

            if (expected.TryGetProperty("error_code", out var errorCode))
            {
                Assert.Equal(errorCode.GetString(), ex.Code);
            }
            return;
        }

        // Normal convert: pass schema dict and options dict
        var schemaDict = JsonSerializer.Deserialize<Dictionary<string, object>>(
            input.GetProperty("schema").GetRawText())!;

        Dictionary<string, object>? optionsDict = null;
        if (input.TryGetProperty("options", out var optionsEl) &&
            optionsEl.GetRawText() != "{}")
        {
            optionsDict = JsonSerializer.Deserialize<Dictionary<string, object>>(
                optionsEl.GetRawText());
        }

        var schemaJson = JsonSerializer.Serialize(schemaDict);
        var optsJson = optionsDict != null
            ? JsonSerializer.Serialize(optionsDict, new JsonSerializerOptions { PropertyNamingPolicy = JsonNamingPolicy.KebabCaseLower, DictionaryKeyPolicy = JsonNamingPolicy.KebabCaseLower })
            : "{}";
        var result = _engine.CallJsl("jsl_convert", schemaJson, optsJson);
        AssertConvertExpected(result, expected);
    }

    private static void AssertConvertExpected(JsonElement result, JsonElement expected)
    {
        if (expected.TryGetProperty("has_keys", out var hasKeys))
        {
            foreach (var key in hasKeys.EnumerateArray())
            {
                Assert.True(result.TryGetProperty(key.GetString()!, out _),
                    $"result missing key: {key.GetString()}");
            }
        }

        if (expected.TryGetProperty("apiVersion", out var apiVersion))
        {
            Assert.Equal(apiVersion.GetString(), result.GetProperty("apiVersion").GetString());
        }

        if (expected.TryGetProperty("schema_has_properties", out _))
        {
            Assert.True(result.GetProperty("schema").TryGetProperty("properties", out _));
        }

        if (expected.TryGetProperty("codec_has_schema_uri", out _))
        {
            Assert.True(result.TryGetProperty("codec", out _));
        }
    }

    // -----------------------------------------------------------------------
    // Roundtrip suite
    // -----------------------------------------------------------------------

    [Theory]
    [MemberData(nameof(RoundtripFixtureIds))]
    public void ConformanceRoundtrip(string fixtureId)
    {
        var fx = GetFixture("roundtrip", fixtureId);
        var input = fx.GetProperty("input");
        var expected = fx.GetProperty("expected");

        var schemaDict = JsonSerializer.Deserialize<Dictionary<string, object>>(
            input.GetProperty("schema").GetRawText())!;

        Dictionary<string, object>? optionsDict = null;
        if (input.TryGetProperty("options", out var optionsEl) &&
            optionsEl.GetRawText() != "{}")
        {
            optionsDict = JsonSerializer.Deserialize<Dictionary<string, object>>(
                optionsEl.GetRawText());
        }

        var schemaJson = JsonSerializer.Serialize(schemaDict);
        var optsJson = optionsDict != null
            ? JsonSerializer.Serialize(optionsDict, new JsonSerializerOptions { PropertyNamingPolicy = JsonNamingPolicy.KebabCaseLower, DictionaryKeyPolicy = JsonNamingPolicy.KebabCaseLower })
            : "{}";
        var convertResult = _engine.CallJsl("jsl_convert", schemaJson, optsJson);
        var codec = convertResult.GetProperty("codec");

        var dataDict = JsonSerializer.Deserialize<Dictionary<string, object>>(
            input.GetProperty("data").GetRawText())!;

        var dataJson = JsonSerializer.Serialize(dataDict);
        var codecJson = JsonSerializer.Serialize(codec);
        var rehydrateResult = _engine.CallJsl("jsl_rehydrate", dataJson, codecJson, schemaJson);

        if (expected.TryGetProperty("has_keys", out var hasKeys))
        {
            foreach (var key in hasKeys.EnumerateArray())
            {
                Assert.True(rehydrateResult.TryGetProperty(key.GetString()!, out _),
                    $"result missing key: {key.GetString()}");
            }
        }

        if (expected.TryGetProperty("apiVersion", out var apiVersion))
        {
            Assert.Equal(apiVersion.GetString(),
                rehydrateResult.GetProperty("apiVersion").GetString());
        }

        if (expected.TryGetProperty("data", out var expectedData))
        {
            var actualData = rehydrateResult.GetProperty("data");
            var expectedNode = JsonNode.Parse(expectedData.GetRawText());
            var actualNode = JsonNode.Parse(actualData.GetRawText());
            Assert.True(JsonNode.DeepEquals(expectedNode, actualNode),
                $"Data mismatch.\nExpected: {expectedData.GetRawText()}\nActual:   {actualData.GetRawText()}");
        }

        if (expected.TryGetProperty("data_user_name", out var userName))
        {
            Assert.Equal(userName.GetString(),
                rehydrateResult.GetProperty("data")
                    .GetProperty("user")
                    .GetProperty("name").GetString());
        }

        if (expected.TryGetProperty("data_value", out var dataValue))
        {
            Assert.Equal(dataValue.GetDouble(),
                rehydrateResult.GetProperty("data")
                    .GetProperty("value").GetDouble(), 3);
        }

        if (expected.TryGetProperty("warnings_is_array", out _))
        {
            Assert.True(rehydrateResult.TryGetProperty("warnings", out var warnings));
            Assert.Equal(JsonValueKind.Array, warnings.ValueKind);
        }
    }

    // -----------------------------------------------------------------------
    // Rehydrate error suite
    // -----------------------------------------------------------------------

    [Theory]
    [MemberData(nameof(RehydrateErrorFixtureIds))]
    public void ConformanceRehydrateError(string fixtureId)
    {
        var fx = GetFixture("rehydrate_error", fixtureId);
        var input = fx.GetProperty("input");
        var expected = fx.GetProperty("expected");

        Assert.True(expected.GetProperty("is_error").GetBoolean());

        var dataJson = input.GetProperty("data").GetRawText();
        var schemaJson = input.GetProperty("schema").GetRawText();
        var codecArg = input.TryGetProperty("codec_raw", out var codecRaw)
            ? codecRaw.GetString()!
            : "{}";

        var ex = Assert.Throws<JslException>(() =>
            _engine.CallJsl("jsl_rehydrate", dataJson, codecArg, schemaJson));

        if (expected.TryGetProperty("error_has_keys", out var errorKeys))
        {
            foreach (var key in errorKeys.EnumerateArray())
            {
                var k = key.GetString()!;
                if (k == "code") Assert.NotEmpty(ex.Code);
                if (k == "message") Assert.NotEmpty(ex.Message);
            }
        }
    }
}
